//! Real host-native bootstrap regressions for immutable artifacts and warm revisions.
use std::{
    collections::HashMap,
    io::{Cursor, Write},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rustack_lambda_core::{
    config::LambdaConfig,
    executor::ExecutorBackend,
    provider::{InvokeKind, InvokeOutcome, RustackLambda},
    storage::FunctionStore,
};
use rustack_lambda_model::{
    input::{
        CreateFunctionInput, PublishVersionInput, UpdateFunctionCodeInput,
        UpdateFunctionConfigurationInput,
    },
    types::{Environment, FunctionCode},
};

#[allow(clippy::disallowed_methods)] // Standalone fixture without async runtime defers the CARGO_BIN_EXE read to runtime.
fn package(marker: &str) -> String {
    // CARGO_BIN_EXE points at the fixture bootstrap when cargo builds test
    // executables; `cargo check --all-targets` only compiles this test without
    // materializing that binary, so defer the read to runtime.
    let Ok(binary) = std::fs::read(env!("CARGO_BIN_EXE_bootstrap")) else {
        return String::new();
    };
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "bootstrap",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .unwrap();
    writer.write_all(&binary).unwrap();
    writer
        .start_file("revision.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(marker.as_bytes()).unwrap();
    STANDARD.encode(writer.finish().unwrap().into_inner())
}
fn create(marker: &str) -> CreateFunctionInput {
    CreateFunctionInput {
        function_name: "revision-test".into(),
        runtime: Some("provided.al2023".into()),
        handler: Some("bootstrap".into()),
        role: "arn:aws:iam::000000000000:role/test".into(),
        timeout: Some(5),
        architectures: Some(vec![
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            }
            .into(),
        ]),
        code: FunctionCode {
            zip_file: Some(package(marker)),
            ..Default::default()
        },
        ..Default::default()
    }
}
async fn invoke(provider: &RustackLambda, version: Option<&str>) -> serde_json::Value {
    let InvokeOutcome::Sync(response) = provider
        .invoke("revision-test", version, b"{}", InvokeKind::RequestResponse)
        .await
        .unwrap()
    else {
        panic!("Expected synchronous result")
    };
    serde_json::from_slice(&response.payload).unwrap()
}

#[tokio::test]
async fn test_should_preserve_published_code_and_refresh_warm_revisions() {
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    provider.create_function(create("A")).await.unwrap();
    let a = invoke(&provider, None).await;
    assert_eq!(a["codeMarker"], "A");
    assert_eq!(invoke(&provider, None).await["pid"], a["pid"]);
    provider
        .publish_version("revision-test", &PublishVersionInput::default())
        .unwrap();
    provider
        .update_function_code(
            "revision-test",
            UpdateFunctionCodeInput {
                zip_file: Some(package("B")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let b = invoke(&provider, None).await;
    assert_eq!(b["codeMarker"], "B");
    assert_ne!(b["pid"], a["pid"]);
    assert_eq!(invoke(&provider, Some("1")).await["codeMarker"], "A");
    assert_eq!(invoke(&provider, Some("1")).await["codeMarker"], "A");
    provider
        .update_function_configuration(
            "revision-test",
            &UpdateFunctionConfigurationInput {
                environment: Some(Environment {
                    variables: Some(HashMap::from([("CONFIG_MARKER".into(), "updated".into())])),
                }),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(invoke(&provider, None).await["configMarker"], "updated");
    let before = provider
        .get_function("revision-test", None)
        .unwrap()
        .configuration
        .unwrap();
    assert!(
        provider
            .update_function_code(
                "revision-test",
                UpdateFunctionCodeInput {
                    zip_file: Some(STANDARD.encode(b"not ZIP")),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    let after = provider
        .get_function("revision-test", None)
        .unwrap()
        .configuration
        .unwrap();
    assert_eq!(before.code_sha256, after.code_sha256);
    assert_eq!(before.revision_id, after.revision_id);
    assert_eq!(invoke(&provider, None).await["codeMarker"], "B");
    provider
        .delete_function("revision-test", None)
        .await
        .unwrap();
    provider.create_function(create("C")).await.unwrap();
    assert_eq!(invoke(&provider, None).await["codeMarker"], "C");
    provider.quiesce(Duration::from_secs(2)).await.unwrap();
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_cancel_and_reap_native_event_on_quiesce_deadline() {
    let root = tempfile::tempdir().unwrap();
    let started = root.path().join("started");
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    let mut input = create("slow");
    input.timeout = Some(120);
    input.environment = Some(Environment {
        variables: Some(HashMap::from([
            ("SLEEP_SECS".into(), "60".into()),
            (
                "STARTED_FILE".into(),
                started.to_string_lossy().into_owned(),
            ),
        ])),
    });
    provider.create_function(input).await.unwrap();
    assert!(matches!(
        provider
            .invoke("revision-test", None, b"{}", InvokeKind::Event)
            .await
            .unwrap(),
        InvokeOutcome::Async { .. }
    ));
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        loop {
            tick.tick().await;
            match tokio::fs::read_to_string(&started).await {
                Ok(pid) if !pid.is_empty() => break pid,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {}
                Err(error) => panic!("Read start marker: {error}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(provider.quiesce(Duration::from_millis(500)).await.is_err());
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(pid.trim())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(
        !status.success(),
        "native child must be reaped before failed quiesce returns"
    );
    provider.shutdown().await;
}
