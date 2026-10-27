//! One owned Unix HTTP exchange; the driver is joined on every outcome.

use std::{path::Path, sync::Arc};

use bytes::Bytes;
use http::{
    Method, Request, Response, StatusCode,
    header::{
        CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HOST, TRANSFER_ENCODING,
        UPGRADE,
    },
};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::Incoming,
    client::conn::http1::{Builder, SendRequest},
    upgrade::on,
};
use hyper_util::rt::TokioIo;
use rustack_lambda_runtime::diagnostics::runtime_dispatch;
use tokio::{
    net::UnixStream,
    sync::watch,
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use tracing::instrument::WithSubscriber;

use super::{
    DockerError, MAX_BODY_BYTES, RawReply, RawRequest,
    binding::{DaemonBinding, DaemonIdentity},
    check_deadline,
};

pub(super) async fn exchange(
    path: &Path,
    request: RawRequest,
    deadline: Instant,
    mut stop: watch::Receiver<bool>,
) -> Result<RawReply, DockerError> {
    if Instant::now() >= deadline {
        return Err(DockerError::Deadline);
    }
    let connected = async {
        let stream = UnixStream::connect(path).await?;
        let pair = Builder::new()
            .max_headers(64)
            .max_buf_size(16 * 1024)
            .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await?;
        Ok::<_, DockerError>(pair)
    };
    let (mut sender, connection) = tokio::select! {
        biased;
        () = stopped(&mut stop) => return Err(DockerError::Stopped),
        result = timeout_at(deadline, connected) => result.map_err(|_| DockerError::Deadline)??,
    };
    if Instant::now() >= deadline {
        return Err(DockerError::Deadline);
    }
    if *stop.borrow() {
        return Err(DockerError::Stopped);
    }
    let expected = request.expected;
    let upgraded = request.expected == StatusCode::SWITCHING_PROTOCOLS;
    let mut driver = tokio::spawn(
        async move {
            if upgraded {
                connection.with_upgrades().await
            } else {
                // This fresh connection is never reused. Recover/drop its owned IO
                // instead of half-closing an already-disconnected Unix socket.
                connection.without_shutdown().await.map(drop)
            }
        }
        .with_subscriber(runtime_dispatch()),
    );
    let operation_stop = stop.clone();
    let operation = async {
        if let Some(binding) = &request.binding {
            check_identity(&mut sender, binding, deadline)
                .await
                .map_err(|source| DockerError::IdentityCheck(Arc::new(source)))?;
            sender.ready().await?;
        }
        check_deadline(deadline)?;
        if *operation_stop.borrow() {
            return Err(DockerError::Stopped);
        }
        let mut mutation = request.mutation;
        let mut builder = Request::builder()
            .method(request.method)
            .uri(request.target)
            .header(HOST, "docker")
            .header(CONTENT_LENGTH, request.body.len())
            .header(CONNECTION, if upgraded { "Upgrade" } else { "close" });
        if upgraded {
            builder = builder.header(UPGRADE, "tcp");
        }
        if !request.body.is_empty() {
            builder = builder.header(CONTENT_TYPE, "application/json");
        }
        let request = builder
            .body(Full::new(request.body))
            .map_err(|_| DockerError::Protocol)?;
        if let Some(mutation) = &mut mutation {
            mutation.issued();
        }
        let response = sender.send_request(request).await?;
        read_response(response, expected, deadline).await
    };
    let outcome = tokio::select! {
        biased;
        () = stopped(&mut stop) => Err(DockerError::Stopped),
        result = timeout_at(deadline, operation) => result.map_err(|_| DockerError::Deadline).and_then(|result| result),
    };
    drop(sender);
    let successful_upgrade = matches!(&outcome, Ok(RawReply::Upgrade(_)));
    let joined = join_driver(&mut driver, deadline, successful_upgrade, &mut stop).await;
    if matches!(&joined, Err(DockerError::Worker)) {
        drop(outcome);
        return Err(DockerError::Worker);
    }
    if Instant::now() >= deadline {
        drop(outcome);
        return Err(DockerError::Deadline);
    }
    if *stop.borrow() {
        drop(outcome);
        return Err(DockerError::Stopped);
    }
    match (outcome, joined) {
        (Ok(reply), Ok(())) => Ok(reply),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

async fn check_identity(
    sender: &mut SendRequest<Full<Bytes>>,
    binding: &DaemonBinding,
    deadline: Instant,
) -> Result<(), DockerError> {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/info")
        .header(HOST, "docker")
        .header(CONTENT_LENGTH, 0)
        .header(CONNECTION, "keep-alive")
        .body(Full::new(Bytes::new()))
        .map_err(|_| DockerError::Protocol)?;
    let response = sender.send_request(request).await?;
    let reply = read_response(response, StatusCode::OK, deadline).await?;
    if DaemonIdentity::decode(reply, deadline)? != binding.identity {
        return Err(DockerError::DaemonBinding);
    }
    Ok(())
}

async fn read_response(
    response: Response<Incoming>,
    expected: StatusCode,
    deadline: Instant,
) -> Result<RawReply, DockerError> {
    let upgraded = expected == StatusCode::SWITCHING_PROTOCOLS;
    check_headers(&response)?;
    let status = response.status();
    if upgraded && status == StatusCode::SWITCHING_PROTOCOLS {
        let headers = response.headers();
        if !headers
            .get(UPGRADE)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"tcp"))
            || !headers
                .get(CONNECTION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(',')
                        .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
                })
            || headers.contains_key(TRANSFER_ENCODING)
            || headers
                .get(CONTENT_LENGTH)
                .is_some_and(|value| value.as_bytes() != b"0")
        {
            return Err(DockerError::Protocol);
        }
        return Ok(RawReply::Upgrade(TokioIo::new(on(response).await?)));
    }
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return Err(DockerError::Protocol);
    }
    let body = collect(response.into_body(), deadline).await?;
    if status != expected {
        return Err(DockerError::Status(status));
    }
    Ok(RawReply::Body {
        status,
        bytes: body,
    })
}

async fn join_driver(
    driver: &mut JoinHandle<Result<(), hyper::Error>>,
    deadline: Instant,
    upgraded: bool,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), DockerError> {
    if upgraded {
        let joined = tokio::select! {
            biased;
            () = stopped(stop) => None,
            result = timeout_at(deadline, &mut *driver) => result.ok(),
        };
        if let Some(result) = joined {
            return result
                .map_err(|_| DockerError::Worker)?
                .map_err(DockerError::from);
        }
    }
    // Hyper connection IO is cancel-safe. Abort is followed by an actual join;
    // it never replaces ownership of a container or an external backend job.
    driver.abort();
    match driver.await {
        Ok(result) => result.map_err(DockerError::from),
        Err(error) if error.is_cancelled() => Ok(()),
        Err(_) => Err(DockerError::Worker),
    }
}

pub(super) async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stop| *stop).await;
}

fn check_headers(response: &Response<Incoming>) -> Result<(), DockerError> {
    let headers = response.headers();
    if headers.len() > 64 {
        return Err(DockerError::TooLarge);
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        total = total
            .checked_add(name.as_str().len())
            .and_then(|length| length.checked_add(value.as_bytes().len()))
            .ok_or(DockerError::TooLarge)?;
        if name.as_str().len() > 64 || value.as_bytes().len() > 4096 || total > 16 * 1024 {
            return Err(DockerError::TooLarge);
        }
        if !value
            .as_bytes()
            .iter()
            .all(|byte| *byte == b'\t' || (32..=126).contains(byte))
        {
            return Err(DockerError::Protocol);
        }
    }
    if headers.get_all(CONTENT_LENGTH).iter().count() > 1 {
        return Err(DockerError::Protocol);
    }
    if let Some(value) = headers.get(CONTENT_LENGTH) {
        let value = value.to_str().map_err(|_| DockerError::Protocol)?;
        if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(DockerError::Protocol);
        }
        if value.parse::<u64>().map_err(|_| DockerError::Protocol)?
            > u64::try_from(MAX_BODY_BYTES).map_err(|_| DockerError::TooLarge)?
        {
            return Err(DockerError::TooLarge);
        }
    }
    if headers
        .get(CONTENT_ENCODING)
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(DockerError::Protocol);
    }
    if headers
        .get(TRANSFER_ENCODING)
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"chunked"))
    {
        return Err(DockerError::Protocol);
    }
    Ok(())
}

async fn collect(mut body: Incoming, deadline: Instant) -> Result<Bytes, DockerError> {
    let mut output = Vec::new();
    let mut quantum = 0_u8;
    while let Some(frame) = body.frame().await {
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        let frame = frame?;
        let data = frame.into_data().map_err(|_| DockerError::Protocol)?;
        let length = output
            .len()
            .checked_add(data.len())
            .ok_or(DockerError::TooLarge)?;
        if length > MAX_BODY_BYTES {
            return Err(DockerError::TooLarge);
        }
        output
            .try_reserve(data.len())
            .map_err(|_| DockerError::Allocation)?;
        output.extend_from_slice(&data);
        quantum = quantum.saturating_add(1);
        if quantum == 64 {
            tokio::task::yield_now().await;
            quantum = 0;
        }
    }
    Ok(Bytes::from(output))
}
