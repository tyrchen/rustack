use std::{sync::Arc, time::Duration};

use rcgen::generate_simple_self_signed;
use rstest::rstest;
use rustls::{
    RootCertStore, ServerConfig,
    crypto::aws_lc_rs::default_provider,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
};
use serde_json::to_vec;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
    sync::oneshot,
    task::spawn_blocking,
    time::{Instant, timeout},
};
use tokio_rustls::TlsAcceptor;

use super::{read_head, response};
use crate::probe::{
    ProbeError,
    endpoint::{
        observe_with_roots,
        types::{EndpointChallenge, GatewayInstanceId, GatewayReply},
    },
};

#[rstest]
#[case::valid(true, true)]
#[case::untrusted(false, true)]
#[case::wrong_name(true, false)]
#[tokio::test]
async fn test_should_verify_tls_ca_and_server_name_with_explicit_aws_lc(
    #[case] trusted: bool,
    #[case] named: bool,
) {
    let certificate = spawn_blocking(|| generate_simple_self_signed(vec!["localhost".into()]))
        .await
        .unwrap()
        .unwrap();
    let mut roots = RootCertStore::empty();
    if trusted {
        roots.add(certificate.cert.der().clone()).unwrap();
    }
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certificate.signing_key.serialize_der(),
    ));
    let server_config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.cert.der().clone()], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = if named { "localhost" } else { "127.0.0.1" };
    let id = GatewayInstanceId::generate().unwrap();
    let challenge = EndpointChallenge::new(format!("https://{host}:{port}").parse().unwrap(), id);
    let server = spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        match acceptor.accept(stream).await {
            Ok(mut stream) => {
                let request = read_head(&mut stream).await.unwrap();
                let _ = stream
                    .write_all(&response(200, &to_vec(&GatewayReply::new(id)).unwrap(), ""))
                    .await;
                let _ = stream.shutdown().await;
                Some(request)
            }
            Err(_) => None,
        }
    });
    let result =
        observe_with_roots(&challenge, Instant::now() + Duration::from_secs(5), roots).await;
    let observed = timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    if trusted && named {
        assert_eq!(result.unwrap().instance_id(), id);
        assert!(observed.is_some());
    } else {
        assert!(matches!(result, Err(ProbeError::Tls)));
        assert!(observed.is_none());
    }
}
#[tokio::test]
async fn test_should_close_owned_tls_handshake_on_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let challenge = EndpointChallenge::new(
        format!("https://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
        GatewayInstanceId::generate().unwrap(),
    );
    let (entered, saw) = oneshot::channel();
    let server = spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        entered.send(()).unwrap();
        let mut bytes = Vec::new();
        stream.take(65_537).read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.len() <= 65_536);
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
    assert!(matches!(client.await.unwrap(), Err(ProbeError::Deadline)));
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
