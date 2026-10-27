#[cfg(feature = "lambda")]
use std::sync::Arc;
use std::time::Duration;

use http::Method;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
#[cfg(feature = "lambda")]
use rustack_lambda_core::docker::{GatewayInstanceId, GatewayReply};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use super::GatewayService;
#[cfg(feature = "lambda")]
use crate::runtime::RuntimeWorkers;

async fn exchange(gateway: GatewayService, method: &str, path: &str) -> String {
    timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), gateway)
                .await
                .unwrap();
        };
        let client = async {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!(
                        "{method} {path} HTTP/1.1\r\nHost: fixture\r\nConnection: \
                         close\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut bytes = Vec::new();
            stream.take(8193).read_to_end(&mut bytes).await.unwrap();
            assert!(bytes.len() <= 8192);
            String::from_utf8(bytes).unwrap()
        };
        let ((), wire) = tokio::join!(server, client);
        wire
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn test_should_return_disabled_probe_404_without_claiming_an_identity() {
    let wire = exchange(
        GatewayService::new(Vec::new()),
        "GET",
        "/_rustack/runtime-probe",
    )
    .await;
    assert!(wire.starts_with("HTTP/1.1 404"));
    assert!(wire.contains("cache-control: no-store"));
    assert!(!wire.contains("instanceId"));
}
#[cfg(feature = "lambda")]
#[tokio::test]
async fn test_should_preserve_probe_identity_across_workers_clone_closed_and_saturated_admission() {
    let id = GatewayInstanceId::generate().unwrap();
    let gateway = GatewayService::new(Vec::new())
        .with_runtime_identity(id)
        .with_workers(Arc::new(RuntimeWorkers::default()))
        .health_only();
    let _permits = gateway
        .admission
        .clone()
        .acquire_many_owned(u32::try_from(gateway.budgets.requests).unwrap())
        .await
        .unwrap();
    gateway.state().drain();
    let wire = exchange(gateway.clone(), "GET", "/_rustack/runtime-probe").await;
    assert!(wire.starts_with("HTTP/1.1 200"));
    assert!(wire.contains("cache-control: no-store"));
    let (_, body) = wire.split_once("\r\n\r\n").unwrap();
    let reply: GatewayReply = serde_json::from_str(body).unwrap();
    assert_eq!(reply.instance_id(), id);
    assert!(
        exchange(gateway, "GET", "/bucket")
            .await
            .starts_with("HTTP/1.1 503")
    );
}
#[cfg(feature = "lambda")]
#[tokio::test]
async fn test_should_bound_head_reply_framing_and_reject_probe_business_methods() {
    let gateway = GatewayService::new(Vec::new())
        .with_runtime_identity(GatewayInstanceId::generate().unwrap());
    let wire = exchange(gateway.clone(), "HEAD", "/_rustack/runtime-probe").await;
    assert!(wire.starts_with("HTTP/1.1 200"));
    assert_eq!(wire.split_once("\r\n\r\n").unwrap().1, "");
    assert!(wire.contains("content-length:"));
    let wire = exchange(gateway, "POST", "/_rustack/runtime-probe").await;
    assert!(wire.starts_with("HTTP/1.1 405"));
    assert!(wire.contains("allow: GET, HEAD"));
}
#[test]
fn test_should_keep_probe_method_outside_normal_health_aliases() {
    assert!(!super::is_health_check(
        &Method::POST,
        "/_rustack/runtime-probe"
    ));
}
