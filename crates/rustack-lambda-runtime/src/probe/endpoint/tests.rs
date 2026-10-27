use std::{io, time::Duration};

use rstest::rstest;
use rustls::RootCertStore;
use serde_json::to_vec;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
    sync::oneshot,
    task::JoinHandle,
    time::{Instant, timeout},
};

use super::{
    observe_with_roots,
    types::{EndpointChallenge, GatewayInstanceId, GatewayReply},
};
use crate::probe::ProbeError;

mod tls;

async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while bytes.len() < 8192 {
        let byte = stream.read_u8().await?;
        bytes.push(byte);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(io::Error::other("fixture request exceeded bound"))
}
fn response(status: u16, body: &[u8], headers: &str) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
async fn fixture(
    build: impl FnOnce(GatewayInstanceId) -> Vec<u8>,
) -> (EndpointChallenge, JoinHandle<Vec<u8>>) {
    let id = GatewayInstanceId::generate().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let reply = build(id);
    let server = spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_head(&mut stream).await.unwrap();
        let _ = stream.write_all(&reply).await;
        let _ = stream.shutdown().await;
        request
    });
    (EndpointChallenge::new(endpoint, id), server)
}
async fn join(server: JoinHandle<Vec<u8>>) -> Vec<u8> {
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn test_should_observe_independent_instance_without_sending_expected_identity() {
    let (challenge, server) =
        fixture(|id| response(200, &to_vec(&GatewayReply::new(id)).unwrap(), "")).await;
    let reply = observe_with_roots(
        &challenge,
        Instant::now() + Duration::from_secs(5),
        RootCertStore::empty(),
    )
    .await
    .unwrap();
    assert_eq!(reply.instance_id(), challenge.expected());
    let request = String::from_utf8(join(server).await).unwrap();
    assert!(request.starts_with("GET /_rustack/runtime-probe HTTP/1.1\r\n"));
    let id: String = challenge.expected().into();
    assert!(!request.contains(&id));
    assert!(request.to_ascii_lowercase().contains("connection: close"));
}
#[rstest]
#[case::redirect(0)]
#[case::wrong_instance(1)]
#[case::unknown_field(2)]
#[case::duplicate_field(3)]
#[case::encoding(4)]
#[case::unterminated(5)]
#[case::wrong_version(6)]
#[case::empty(7)]
#[case::overlong_declared_body(8)]
#[case::overlong_actual_body(9)]
#[case::headers(10)]
#[case::header_bytes(11)]
#[case::trailers(12)]
#[tokio::test]
async fn test_should_reject_untrusted_gateway_responses_without_peer_text(#[case] case: u8) {
    let (challenge,server)=fixture(|id|{
        let id:String=id.into();
        match case {
            0=>response(302,b"PRIVATE-PEER","Location: http://127.0.0.1:1/\r\n"),
            1=>response(200,&to_vec(&GatewayReply::new(GatewayInstanceId::generate().unwrap())).unwrap(),""),
            2=>response(200,format!("{{\"version\":1,\"instanceId\":\"{id}\",\"PRIVATE-PEER\":1}}").as_bytes(),""),
            3=>response(200,format!("{{\"version\":1,\"version\":1,\"instanceId\":\"{id}\"}}").as_bytes(),""),
            4=>response(200,&to_vec(&GatewayReply::new(id.parse().unwrap())).unwrap(),"Content-Encoding: gzip\r\n"),
            5=>b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec(),
            6=>response(200,format!("{{\"version\":2,\"instanceId\":\"{id}\"}}").as_bytes(),""),
            7=>response(200,b"",""),
            8=>response(200,&vec![b'x';4097],""),
            9=>{let mut bytes=b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();bytes.extend_from_slice(&vec![b'x';4097]);bytes},
            10=>response(200,b"{}",&"X-Fixture: x\r\n".repeat(33)),
            11=>response(200,b"{}",&format!("X-Fixture: {}\r\n","x".repeat(16_385))),
            _=>b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\n{}\r\n0\r\nPRIVATE-PEER: trailer\r\n\r\n".to_vec(),
        }
    }).await;
    let error = observe_with_roots(
        &challenge,
        Instant::now() + Duration::from_secs(5),
        RootCertStore::empty(),
    )
    .await
    .unwrap_err();
    assert!(!format!("{error:?}:{error}").contains("PRIVATE-PEER"));
    join(server).await;
}
#[rstest]
#[case::deadline(false)]
#[case::abandonment(true)]
#[tokio::test]
async fn test_should_close_owned_http_io_on_expiry_or_abandoned_caller(#[case] abandon: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let challenge = EndpointChallenge::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
        GatewayInstanceId::generate().unwrap(),
    );
    let (entered, saw) = oneshot::channel();
    let server = spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_head(&mut stream).await.unwrap();
        entered.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
    });
    let client = spawn(async move {
        observe_with_roots(
            &challenge,
            Instant::now() + Duration::from_secs(1),
            RootCertStore::empty(),
        )
        .await
    });
    saw.await.unwrap();
    if abandon {
        client.abort();
        assert!(client.await.unwrap_err().is_cancelled());
    } else {
        assert!(matches!(client.await.unwrap(), Err(ProbeError::Deadline)));
    }
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn test_should_not_follow_a_redirect_to_another_listener() {
    let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = other.local_addr().unwrap();
    let (challenge, server) =
        fixture(|_| response(302, b"", &format!("Location: http://{target}/\r\n"))).await;
    assert!(
        observe_with_roots(
            &challenge,
            Instant::now() + Duration::from_secs(5),
            RootCertStore::empty()
        )
        .await
        .is_err()
    );
    join(server).await;
    assert!(
        timeout(Duration::from_millis(50), other.accept())
            .await
            .is_err()
    );
}
