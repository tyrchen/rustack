//! HTTP proxy integration for API Gateway v2.
//!
//! Forwards requests to the configured HTTP endpoint and returns the response.
//! Redirects are returned verbatim; upstream authority is never selected by Location.

use bytes::Bytes;

use crate::{
    error::ApiGatewayV2ServiceError, provider::RustackApiGatewayV2, storage::IntegrationRecord,
};

/// Handle an HTTP proxy integration.
///
/// Forwards the request to the integration URI and returns the response.
pub async fn handle_http_proxy(
    provider: &RustackApiGatewayV2,
    integration: &IntegrationRecord,
    method: &http::Method,
    path: &str,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let base_uri = integration.integration_uri.as_deref().ok_or_else(|| {
        ApiGatewayV2ServiceError::Internal("HTTP integration has no URI".to_owned())
    })?;

    let base = reqwest::Url::parse(base_uri).map_err(|_| {
        ApiGatewayV2ServiceError::BadRequest("Invalid HTTP integration URL".to_owned())
    })?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.fragment().is_some()
    {
        return Err(ApiGatewayV2ServiceError::BadRequest(
            "HTTP integration requires an HTTP(S) authority without credentials or fragments"
                .to_owned(),
        ));
    }
    let target_url = reqwest::Url::parse(&format!("{base_uri}{path}")).map_err(|_| {
        ApiGatewayV2ServiceError::BadRequest("Invalid HTTP integration request URL".to_owned())
    })?;
    if target_url.origin() != base.origin() || target_url.fragment().is_some() {
        return Err(ApiGatewayV2ServiceError::BadRequest(
            "HTTP request must preserve integration authority".to_owned(),
        ));
    }
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| ApiGatewayV2ServiceError::BadRequest(format!("Invalid HTTP method: {e}")))?;

    let body_total_seconds = rustack_core::settings::budgets().body_total_seconds;
    let total = match integration.timeout_in_millis {
        Some(milliseconds) => {
            if !(1..=30_000).contains(&milliseconds) {
                return Err(ApiGatewayV2ServiceError::BadRequest(
                    "HTTP integration timeout must be between 1 and 30000 milliseconds".to_owned(),
                ));
            }
            let milliseconds = u64::try_from(milliseconds).map_err(|_| {
                ApiGatewayV2ServiceError::BadRequest("Invalid integration timeout".to_owned())
            })?;
            std::time::Duration::from_millis(milliseconds)
        }
        // Snapshot-imported or legacy records without a stored timeout still get a
        // wall-clock bound instead of an idle-only deadline.
        None => std::time::Duration::from_secs(body_total_seconds.min(30)),
    }
    .min(std::time::Duration::from_secs(body_total_seconds));
    let mut request = provider.http_client().request(reqwest_method, target_url);
    request = request.timeout(total);

    // Forward headers (skip host header)
    for (name, value) in headers {
        if name != "host" {
            if let Ok(v) = value.to_str() {
                request = request.header(name.as_str(), v);
            }
        }
    }

    if !body.is_empty() {
        request = request.body(body.to_vec());
    }

    let mut response = request.send().await.map_err(|e| {
        ApiGatewayV2ServiceError::IntegrationError(format!("HTTP proxy request failed: {e}"))
    })?;

    let status = response.status().as_u16();
    let resp_headers = response.headers().clone();
    let budgets = rustack_core::settings::budgets();
    let max_body = usize::try_from(budgets.upstream_body_bytes).map_err(|_| {
        ApiGatewayV2ServiceError::Internal("invalid upstream byte budget".to_owned())
    })?;
    let mut resp_body = Vec::new();
    let idle = std::time::Duration::from_secs(budgets.body_idle_seconds);
    let mut idle_deadline = tokio::time::Instant::now() + idle;
    let total_deadline = tokio::time::Instant::now() + total;
    while let Some(chunk) =
        tokio::time::timeout_at(total_deadline.min(idle_deadline), response.chunk())
            .await
            .map_err(|_| {
                ApiGatewayV2ServiceError::IntegrationError(
                    "HTTP proxy response deadline exceeded".to_owned(),
                )
            })?
            .map_err(|e| {
                ApiGatewayV2ServiceError::IntegrationError(format!(
                    "Failed to read HTTP proxy response: {e}"
                ))
            })?
    {
        if !chunk.is_empty() {
            idle_deadline = tokio::time::Instant::now() + idle;
        }
        rustack_core::http::append_bounded(&mut resp_body, &chunk, max_body as u64)
            .map_err(|error| ApiGatewayV2ServiceError::IntegrationError(error.to_string()))?;
    }

    let mut builder = http::Response::builder().status(status);
    for (name, value) in &resp_headers {
        builder = builder.header(name.as_str(), value.as_bytes());
    }

    builder
        .body(Bytes::from(resp_body))
        .map_err(|e| ApiGatewayV2ServiceError::Internal(format!("Failed to build response: {e}")))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    fn integration(uri: &str) -> IntegrationRecord {
        serde_json::from_value(serde_json::json!({
            "integrationId": "test", "integrationType": "HTTP_PROXY", "integrationUri": uri,
            "requestParameters": {}, "requestTemplates": {}, "responseParameters": {},
            "apiGatewayManaged": false, "timeoutInMillis": 100,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn test_should_ignore_environment_proxy_in_isolated_process() {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "execution::http_proxy::tests::test_should_return_all_redirects_without_contacting_second_origin"])
            .env("HTTP_PROXY", "http://127.0.0.1:9").env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9").env("http_proxy", "http://127.0.0.1:9")
            .env("https_proxy", "http://127.0.0.1:9").env("all_proxy", "http://127.0.0.1:9")
            .env_remove("NO_PROXY").env_remove("no_proxy").kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(10), child.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[tokio::test]
    async fn test_should_return_all_redirects_without_contacting_second_origin() {
        let provider =
            RustackApiGatewayV2::new(crate::config::ApiGatewayV2Config::default()).unwrap();
        for status in [301, 302, 303, 307, 308] {
            for style in ["absolute", "relative", "scheme-relative", "ipv6", "mapped"] {
                let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let location = match style {
                    "relative" => "/second".to_owned(),
                    "scheme-relative" => format!("//{}/second", second.local_addr().unwrap()),
                    "ipv6" => "http://[::1]:9/second".to_owned(),
                    "mapped" => "http://[::ffff:127.0.0.1]:9/second".to_owned(),
                    _ => format!("http://{}/second", second.local_addr().unwrap()),
                };
                let record = integration(&format!("http://{}", first.local_addr().unwrap()));
                let wire = format!(
                    "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: \
                     5\r\nConnection: close\r\n\r\nfirst"
                );
                let server = async {
                    let (mut socket, _) = first.accept().await.unwrap();
                    let mut buffer = [0; 4096];
                    assert!(socket.read(&mut buffer).await.unwrap() > 0);
                    socket.write_all(wire.as_bytes()).await.unwrap();
                };
                let headers = http::HeaderMap::new();
                let request =
                    handle_http_proxy(&provider, &record, &http::Method::GET, "/", &headers, &[]);
                let ((), result) = tokio::join!(server, request);
                let response = result.unwrap();
                assert_eq!(response.status().as_u16(), status);
                assert_eq!(
                    response.headers().get("location").unwrap(),
                    location.as_str()
                );
                assert_eq!(response.body().as_ref(), b"first");
                assert!(
                    tokio::time::timeout(Duration::from_millis(2), second.accept())
                        .await
                        .is_err()
                );
            }
        }
    }

    #[tokio::test]
    async fn test_should_apply_total_deadline_to_upstream_body_without_eof() {
        let provider =
            RustackApiGatewayV2::new(crate::config::ApiGatewayV2Config::default()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let record = integration(&format!("http://{}", listener.local_addr().unwrap()));
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nx")
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        };
        let headers = http::HeaderMap::new();
        let request = handle_http_proxy(&provider, &record, &http::Method::GET, "/", &headers, &[]);
        let ((), result) = tokio::join!(server, request);
        assert!(matches!(
            result,
            Err(ApiGatewayV2ServiceError::IntegrationError(_))
        ));
    }
}
