//! One container-origin HTTP(S) observation; all IO futures stay locally owned.

pub(super) mod entropy;
pub(super) mod types;

use std::{env::consts::ARCH, sync::Arc};

use bytes::{Bytes, BytesMut};
use http::{
    Request, StatusCode,
    header::{CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, HOST},
};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use rustls::{
    ClientConfig, RootCertStore, crypto::aws_lc_rs::default_provider, pki_types::ServerName,
};
use serde_json::from_slice;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    time::{Instant, timeout_at},
};
use tokio_rustls::TlsConnector;
use tracing::instrument::WithSubscriber;
use url::{Host, Url};
use webpki_roots::TLS_SERVER_ROOTS;

use self::types::{EndpointChallenge, EndpointReport, GATEWAY_PROBE_PATH, GatewayReply};
use super::{ProbeError, REPORT_BYTES};
use crate::{
    diagnostics::runtime_dispatch,
    protocol::{Architecture, AttemptId},
};

pub(super) async fn observe(
    challenge: &EndpointChallenge,
    attempt: AttemptId,
    deadline: Instant,
) -> Result<EndpointReport, ProbeError> {
    let roots = RootCertStore {
        roots: TLS_SERVER_ROOTS.to_vec(),
    };
    let observed = observe_with_roots(challenge, deadline, roots)
        .with_subscriber(runtime_dispatch())
        .await?;
    let architecture = match ARCH {
        "aarch64" => Architecture::Arm64,
        "x86_64" => Architecture::X86_64,
        _ => return Err(ProbeError::Configuration),
    };
    if Instant::now() >= deadline {
        return Err(ProbeError::Deadline);
    }
    Ok(EndpointReport::observed(
        attempt,
        architecture,
        observed.instance_id(),
    ))
}
async fn observe_with_roots(
    challenge: &EndpointChallenge,
    deadline: Instant,
    roots: RootCertStore,
) -> Result<GatewayReply, ProbeError> {
    let reply = timeout_at(deadline, async {
        let url =
            Url::parse(challenge.endpoint().as_str()).map_err(|_| ProbeError::Configuration)?;
        let host = match url.host().ok_or(ProbeError::Configuration)? {
            Host::Domain(name) => name.to_owned(),
            Host::Ipv4(ip) => ip.to_string(),
            Host::Ipv6(ip) => ip.to_string(),
        };
        let port = url
            .port_or_known_default()
            .ok_or(ProbeError::Configuration)?;
        let wire_host = url.host_str().ok_or(ProbeError::Configuration)?;
        let authority = match url.port() {
            Some(port) => format!("{wire_host}:{port}"),
            None => wire_host.to_owned(),
        };
        let stream = TcpStream::connect((host.as_str(), port)).await?;
        if url.scheme() == "https" {
            let name = ServerName::try_from(host).map_err(|_| ProbeError::Configuration)?;
            let config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| ProbeError::Tls)?
                .with_root_certificates(roots)
                .with_no_client_auth();
            let connector = TlsConnector::from(Arc::new(config));
            let stream = connector
                .connect(name, stream)
                .await
                .map_err(|_| ProbeError::Tls)?;
            exchange(stream, &authority).await
        } else {
            exchange(stream, &authority).await
        }
    })
    .await
    .map_err(|_| ProbeError::Deadline)??;
    if Instant::now() >= deadline {
        return Err(ProbeError::Deadline);
    }
    if reply.instance_id() != challenge.expected() {
        return Err(ProbeError::Protocol);
    }
    Ok(reply)
}
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    authority: &str,
) -> Result<GatewayReply, ProbeError> {
    let (mut sender, connection) = http1::Builder::new()
        .max_headers(32)
        .max_buf_size(16 * 1024)
        .handshake(TokioIo::new(stream))
        .await?;
    let connection = connection.without_shutdown();
    tokio::pin!(connection);
    let mut ended = false;
    let outcome = {
        let operation = async {
            let request = Request::builder()
                .method("GET")
                .uri(GATEWAY_PROBE_PATH)
                .header(HOST, authority)
                .header(CONNECTION, "close")
                .header(CONTENT_LENGTH, 0)
                .body(Full::new(Bytes::new()))
                .map_err(|_| ProbeError::Protocol)?;
            let response = sender.send_request(request).await?;
            if response.status() != StatusCode::OK
                || response.headers().contains_key(CONTENT_ENCODING)
            {
                return Err(ProbeError::Protocol);
            }
            let mut header_bytes = 0_usize;
            for (name, value) in response.headers() {
                header_bytes = header_bytes
                    .checked_add(name.as_str().len())
                    .and_then(|count| count.checked_add(value.as_bytes().len()))
                    .ok_or(ProbeError::Protocol)?;
            }
            if header_bytes > 16 * 1024 {
                return Err(ProbeError::Protocol);
            }
            let mut lengths = response.headers().get_all(CONTENT_LENGTH).iter();
            if let Some(length) = lengths.next() {
                if lengths.next().is_some()
                    || length
                        .to_str()
                        .ok()
                        .and_then(|value| value.parse::<usize>().ok())
                        .is_none_or(|length| length > REPORT_BYTES)
                {
                    return Err(ProbeError::Protocol);
                }
            }
            let mut body = response.into_body();
            let mut bytes = BytesMut::new();
            while let Some(frame) = body.frame().await {
                let data = frame?.into_data().map_err(|_| ProbeError::Protocol)?;
                if bytes
                    .len()
                    .checked_add(data.len())
                    .is_none_or(|size| size > REPORT_BYTES)
                {
                    return Err(ProbeError::Protocol);
                }
                bytes.extend_from_slice(&data);
            }
            from_slice::<GatewayReply>(&bytes).map_err(|_| ProbeError::Protocol)
        };
        tokio::pin!(operation);
        loop {
            tokio::select! {biased;
                result=&mut operation=>break result,
                result=&mut connection,if !ended=>{drop(result?);ended=true;},
            }
        }
    };
    drop(sender);
    if outcome.is_ok() && !ended {
        drop(connection.await?);
    }
    outcome
}

#[cfg(test)]
mod tests;
