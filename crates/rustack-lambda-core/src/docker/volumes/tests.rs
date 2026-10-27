use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::*;
use crate::{
    artifact::Architecture,
    docker::{
        DirectoryLease,
        code::tests::{elf, prepare, prepare_with, source, zip},
        tests::{Daemon, deadline},
    },
};

fn volume(name: &str, labels: &BTreeMap<String, String>) -> Value {
    json!({"Name":name,"Driver":"local","Mountpoint":"/daemon/private","Labels":labels,"Scope":"local","Options":{}})
}

#[tokio::test]
async fn test_should_create_observe_list_and_remove_only_explicit_local_volumes() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let name: DockerKey = "rustack-code-fixture".parse().unwrap();
    let labels = BTreeMap::from([("rustack.lambda.ownerId".into(), "recorded-owner".into())]);
    let pending = tokio::spawn({
        let (port, name, labels) = (port.clone(), name.clone(), labels.clone());
        async move { port.create_volume(&name, &labels, deadline()).await }
    });
    let request = daemon.requests.recv().await.unwrap();
    assert!(request.head.starts_with("POST /v1.53/volumes/create "));
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({"Name":name.as_str(),"Driver":"local","Labels":labels})
    );
    request
        .reply(
            201,
            &serde_json::to_vec(&volume(name.as_str(), &labels)).unwrap(),
        )
        .await;
    assert_eq!(pending.await.unwrap().unwrap().name, name.as_str());
    let pending = tokio::spawn({
        let (port, name) = (port.clone(), name.clone());
        async move { port.inspect_volume(&name, deadline()).await }
    });
    let request = daemon.requests.recv().await.unwrap();
    assert!(
        request
            .head
            .starts_with("GET /v1.53/volumes/rustack%2Dcode%2Dfixture ")
    );
    request
        .reply(
            200,
            &serde_json::to_vec(&volume(name.as_str(), &labels)).unwrap(),
        )
        .await;
    pending.await.unwrap().unwrap();
    let filter = OwnerFilter::new(uuid::Uuid::new_v4(), None, None).unwrap();
    let pending = tokio::spawn({
        let port = port.clone();
        async move { port.list_volumes(filter, deadline()).await }
    });
    let request = daemon.requests.recv().await.unwrap();
    assert!(request.head.starts_with("GET /v1.53/volumes?filters="));
    assert!(!request.head.contains("limit="));
    assert!(!request.head.contains("all="));
    request.reply(200, br#"{"Volumes":[],"Warnings":[]}"#).await;
    assert_eq!(pending.await.unwrap().unwrap(), [] as [Volume; 0]);
    let pending = tokio::spawn({
        let (port, name) = (port.clone(), name.clone());
        async move { port.remove_volume(&name, deadline()).await }
    });
    let request = daemon.requests.recv().await.unwrap();
    assert!(
        request
            .head
            .starts_with("DELETE /v1.53/volumes/rustack%2Dcode%2Dfixture?force=false ")
    );
    request.reply(204, b"").await;
    pending.await.unwrap().unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_upload_large_typed_tar_without_relaxing_json_or_response_limits() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let bootstrap = elf(Architecture::Arm64);
    let data = vec![b'x'; 2 * 1024 * 1024];
    let (_root, source) = source(zip(&[
        ("bootstrap", &bootstrap, 0o755),
        ("large", &data, 0o644),
    ]))
    .await;
    let code = prepare(source).await;
    let expected = code.bytes().clone();
    let key: DockerKey = "loader".parse().unwrap();
    let pending = tokio::spawn({
        let port = port.clone();
        async move { port.upload_code(&key, &code, deadline()).await }
    });
    let request = daemon.requests.recv().await.unwrap();
    assert!(request.head.starts_with(
        "PUT /v1.53/containers/loader/archive?path=%2Fvar%2Ftask&noOverwriteDirNonDir=true&\
         copyUIDGID=false "
    ));
    assert!(
        request
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/x-tar")
    );
    assert_eq!(request.body, expected);
    request.reply(200, b"").await;
    pending.await.unwrap().unwrap();
    assert!(matches!(
        port.call(
            Operation::CreateVolume,
            Method::POST,
            "/volumes/create".into(),
            Bytes::from(vec![0; 1024 * 1024 + 1]),
            StatusCode::CREATED,
            deadline()
        )
        .await,
        Err(DockerError::TooLarge)
    ));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_unsafe_labels_before_any_wire_request() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let name = "volume".parse().unwrap();
    for labels in [
        BTreeMap::from([("bad/key".into(), "value".into())]),
        BTreeMap::from([("valid".into(), "bad\nvalue".into())]),
        BTreeMap::from([("valid".into(), "a".repeat(257))]),
    ] {
        assert!(matches!(
            port.create_volume(&name, &labels, deadline()).await,
            Err(DockerError::Configuration)
        ));
    }
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_accepted_upload_admission_after_its_caller_is_abandoned() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let admission = Arc::new(Semaphore::new(1));
    let bootstrap = elf(Architecture::Arm64);
    let (_root, source) = source(zip(&[("bootstrap", &bootstrap, 0o755)])).await;
    let code = prepare_with(source, admission.clone()).await;
    let pending = tokio::spawn(async move {
        let key = "held-loader".parse().unwrap();
        port.upload_code(&key, &code, deadline()).await
    });
    let request = daemon.next().await;
    assert!(request.head.starts_with("PUT "));
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(admission.available_permits(), 0);
    request.reply(200, b"").await;
    wire.shutdown().await.unwrap();
    assert_eq!(admission.available_permits(), 1);
    daemon.close().await;
}

#[rstest]
#[case::missing(json!({"Warnings":[]}))]
#[case::null(json!({"Volumes":null}))]
#[case::warning(json!({"Volumes":[],"Warnings":["partial result"]}))]
#[case::row_kind(json!({"Volumes":[false]}))]
#[tokio::test]
async fn test_should_reject_incomplete_or_malformed_volume_inventories(#[case] response: Value) {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let scope = OwnerFilter::new(uuid::Uuid::new_v4(), None, None).unwrap();
    let pending = tokio::spawn(async move { port.list_volumes(scope, deadline()).await });
    daemon
        .next()
        .await
        .reply(200, &serde_json::to_vec(&response).unwrap())
        .await;
    assert!(pending.await.unwrap().is_err());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case::name("Name",json!("foreign"))]
#[case::driver("Driver",json!("plugin"))]
#[case::scope("Scope",json!("global"))]
#[case::options("Options",json!({"device":"/host/source"}))]
#[case::labels("Labels",json!({"foreign":"SECRET_VALUE"}))]
#[tokio::test]
async fn test_should_reject_volume_create_identity_or_policy_substitution(
    #[case] field: &str,
    #[case] value: Value,
) {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let name: DockerKey = "volume".parse().unwrap();
    let pending = tokio::spawn(async move {
        port.create_volume(&name, &BTreeMap::new(), deadline())
            .await
    });
    let mut response = volume("volume", &BTreeMap::new());
    response[field] = value;
    daemon
        .next()
        .await
        .reply(201, &serde_json::to_vec(&response).unwrap())
        .await;
    let error = pending.await.unwrap().unwrap_err();
    assert!(matches!(error, DockerError::Protocol));
    assert!(!format!("{error:?}").contains("SECRET_VALUE"));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_response_caps_and_connection_identity_fences_on_volume_operations() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let name: DockerKey = "volume".parse().unwrap();
    let pending = tokio::spawn({
        let port = port.clone();
        let name = name.clone();
        async move { port.inspect_volume(&name, deadline()).await }
    });
    daemon
        .next()
        .await
        .raw(b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n")
        .await;
    assert!(matches!(pending.await.unwrap(), Err(DockerError::TooLarge)));
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().to_owned(), deadline())
        .await
        .unwrap();
    let bound = port.bind_directory(&directory, deadline()).await.unwrap();
    daemon.identity_status.send_replace(404);
    let error = bound.inspect_volume(&name, deadline()).await.unwrap_err();
    assert!(matches!(error, DockerError::IdentityCheck(_)));
    assert!(daemon.requests.try_recv().is_err());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
