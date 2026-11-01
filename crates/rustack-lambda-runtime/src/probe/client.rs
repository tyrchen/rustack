//! One owned HTTP connection per request; never spawn a detached connection task.

use std::{env, future::pending, num::NonZeroU64, sync::Arc, time::Duration};

use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::{
    net::TcpStream,
    time::{Instant, sleep_until, timeout_at},
};

use super::{EndpointChallenge, ProbeError, REPORT_BYTES, collect, endpoint};
use crate::{metadata::deadline_from_unix_ms, protocol::AttemptId};

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}
async fn request(
    method: Method,
    path: &str,
    body: Bytes,
    deadline: Instant,
) -> Result<Reply, ProbeError> {
    let stream = timeout_at(deadline, TcpStream::connect("127.0.0.1:9001"))
        .await
        .map_err(|_| ProbeError::Deadline)??;
    let mut builder = http1::Builder::new();
    builder.max_buf_size(16384).max_headers(32);
    let (mut sender, connection) = timeout_at(deadline, builder.handshake(TokioIo::new(stream)))
        .await
        .map_err(|_| ProbeError::Deadline)??;
    tokio::pin!(connection);
    let operation = async {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:9001")
            .header("connection", "close")
            .header("content-type", "application/json")
            .body(Full::new(body))
            .map_err(|_| ProbeError::Protocol)?;
        let response = sender.send_request(request).await?;
        let (parts, body) = response.into_parts();
        let body = Limited::new(body, REPORT_BYTES)
            .collect()
            .await
            .map_err(|_| ProbeError::Protocol)?
            .to_bytes();
        Ok(Reply {
            status: parts.status,
            headers: parts.headers,
            body,
        })
    };
    tokio::pin!(operation);
    let mut finished = false;
    loop {
        tokio::select! {biased;
            ()=sleep_until(deadline)=>return Err(ProbeError::Deadline),
            result=&mut operation=>return result,
            result=&mut connection,if !finished=>{result?;finished=true;},
        }
    }
}
fn header<'a>(headers: &'a HeaderMap, key: &str, cap: usize) -> Result<&'a str, ProbeError> {
    let mut values = headers.get_all(key).iter();
    let value = values.next().ok_or(ProbeError::Protocol)?;
    if values.next().is_some() || value.as_bytes().len() > cap {
        return Err(ProbeError::Protocol);
    }
    value.to_str().map_err(|_| ProbeError::Protocol)
}
#[derive(Debug, Clone, Copy)]
enum Mode {
    Resource,
    Endpoint,
}
pub(super) async fn run() -> Result<(), ProbeError> {
    run_probe(Mode::Resource).await
}
pub(super) async fn run_endpoint() -> Result<(), ProbeError> {
    run_probe(Mode::Endpoint).await
}
async fn run_probe(mode: Mode) -> Result<(), ProbeError> {
    if !cfg!(target_os = "linux")
        || env::var("AWS_LAMBDA_RUNTIME_API").as_deref() != Ok("127.0.0.1:9001")
    {
        return Err(ProbeError::Configuration);
    }
    let challenge = request(
        Method::GET,
        "/2018-06-01/runtime/invocation/next",
        Bytes::new(),
        Instant::now() + Duration::from_secs(60),
    )
    .await?;
    if challenge.status != StatusCode::OK
        || (!matches!(mode, Mode::Endpoint) && challenge.body.as_ref() != b"{}")
    {
        return Err(ProbeError::Protocol);
    }
    let attempt: AttemptId = header(&challenge.headers, "lambda-runtime-aws-request-id", 36)?
        .parse()
        .map_err(|_| ProbeError::Protocol)?;
    let wall = header(&challenge.headers, "lambda-runtime-deadline-ms", 20)?
        .parse::<NonZeroU64>()
        .map_err(|_| ProbeError::Protocol)?;
    let deadline = deadline_from_unix_ms(wall).map_err(|_| ProbeError::Deadline)?;
    let observation = if matches!(mode, Mode::Endpoint) {
        match EndpointChallenge::decode(&challenge.body) {
            Ok(challenge) => endpoint::observe(&challenge, attempt, deadline)
                .await
                .and_then(|report| {
                    serde_json::to_vec(&report).map_err(|error| ProbeError::Report(Arc::new(error)))
                }),
            Err(error) => Err(error),
        }
    } else {
        collect::collect(attempt, deadline)
            .await
            .and_then(|report| {
                serde_json::to_vec(&report).map_err(|error| ProbeError::Report(Arc::new(error)))
            })
    };
    let (suffix, body) = match observation {
        Ok(bytes) if bytes.len() <= REPORT_BYTES => ("response", Bytes::from(bytes)),
        Ok(_) => return Err(ProbeError::Protocol),
        Err(_) => (
            "error",
            Bytes::from_static(
                br#"{"errorType":"Rustack.ProbeFailed","errorMessage":"runtime probe failed"}"#,
            ),
        ),
    };
    let reply = request(
        Method::POST,
        &format!("/2018-06-01/runtime/invocation/{attempt}/{suffix}"),
        body,
        deadline.min(Instant::now() + Duration::from_secs(5)),
    )
    .await?;
    if reply.status != StatusCode::ACCEPTED || !reply.body.is_empty() {
        return Err(ProbeError::Protocol);
    }
    // The host owns this one-shot function's retirement, including on rejection.
    pending().await
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    #[test]
    fn test_should_require_exact_single_bounded_ascii_probe_metadata() {
        let mut values = HeaderMap::new();
        assert!(header(&values, "probe", 3).is_err());
        values.insert("probe", HeaderValue::from_static("123"));
        assert_eq!(header(&values, "probe", 3).unwrap(), "123");
        assert!(header(&values, "probe", 2).is_err());
        values.append("probe", HeaderValue::from_static("456"));
        assert!(header(&values, "probe", 3).is_err());
        values.clear();
        values.insert("probe", HeaderValue::from_bytes(b"\xff").unwrap());
        assert!(header(&values, "probe", 3).is_err());
    }
    #[test]
    fn test_should_redact_probe_error_sources() {
        use std::error::Error as _;
        let error = ProbeError::from(std::io::Error::other("private-kernel-path"));
        assert!(error.source().is_some());
        assert!(!format!("{error:?} {error}").contains("private-kernel-path"));
    }
}
