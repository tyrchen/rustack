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
async fn test_should_wait_for_first_poll_before_starting_the_native_handler_budget() {
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            init_timeout: Duration::from_secs(3),
            ..Default::default()
        },
    );
    let mut input = create("slow-init");
    input.timeout = Some(1);
    input.environment = Some(Environment {
        variables: Some(HashMap::from([("INIT_DELAY_MILLIS".into(), "1200".into())])),
    });
    provider.create_function(input).await.unwrap();
    let started = tokio::time::Instant::now();
    assert_eq!(invoke(&provider, None).await["codeMarker"], "slow-init");
    assert!(started.elapsed() >= Duration::from_millis(1200));
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_join_stale_revision_api_connections_before_invoking_replacement_code() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    provider.create_function(create("old")).await.unwrap();
    let old = invoke(&provider, None).await;
    let address = old["runtimeApi"].as_str().unwrap();
    let mut held = tokio::net::TcpStream::connect(address).await.unwrap();
    held.write_all(b"POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 1\r\n\r\n").await.unwrap();
    // Keep the old API's body collector active while the revision is replaced.
    tokio::time::sleep(Duration::from_millis(50)).await;
    provider
        .update_function_code(
            "revision-test",
            UpdateFunctionCodeInput {
                zip_file: Some(package("new")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    assert_eq!(invoke(&provider, None).await["codeMarker"], "new");
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "replacement must await the old API connection deadline and join"
    );
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), held.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(reply.starts_with(b"HTTP/1.1 408"));
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(old["pid"].as_u64().unwrap().to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(!status.success());
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_time_out_and_reap_a_bootstrap_that_never_polls_the_runtime_api() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("started");
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            init_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    );
    let mut input = create("never-ready");
    input.timeout = Some(120);
    input.environment = Some(Environment {
        variables: Some(HashMap::from([
            ("INIT_DELAY_MILLIS".into(), "60000".into()),
            ("STARTED_FILE".into(), marker.to_string_lossy().into_owned()),
        ])),
    });
    provider.create_function(input).await.unwrap();
    let started = tokio::time::Instant::now();
    assert!(
        provider
            .invoke("revision-test", None, b"{}", InvokeKind::RequestResponse)
            .await
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    let pid = tokio::fs::read_to_string(marker).await.unwrap();
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(pid.trim())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(
        !status.success(),
        "initialization timeout must join child cleanup"
    );
    provider.shutdown().await;
}

#[tokio::test]
async fn test_should_report_native_initialization_error_without_echoing_its_payload() {
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    let mut input = create("init-error");
    input.environment = Some(Environment {
        variables: Some(HashMap::from([("INIT_FAIL".into(), "1".into())])),
    });
    provider.create_function(input).await.unwrap();
    let error = provider
        .invoke("revision-test", None, b"{}", InvokeKind::RequestResponse)
        .await
        .unwrap_err();
    assert!(!format!("{error} {error:?}").contains("secret-initialization-value"));
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_cancel_and_reap_a_bootstrap_while_waiting_for_first_poll() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("started");
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            init_timeout: Duration::from_secs(30),
            ..Default::default()
        },
    );
    let mut input = create("cancel-init");
    input.environment = Some(Environment {
        variables: Some(HashMap::from([
            ("INIT_DELAY_MILLIS".into(), "60000".into()),
            ("STARTED_FILE".into(), marker.to_string_lossy().into_owned()),
        ])),
    });
    provider.create_function(input).await.unwrap();
    let handle = provider
        .start_invocation("revision-test", None, b"{}", None)
        .await
        .unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tokio::fs::read_to_string(&marker).await {
                Ok(pid) if !pid.is_empty() => break pid,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {}
                Err(error) => panic!("Read initialization marker: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    handle
        .cancel_and_wait(tokio::time::Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(pid.trim())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(
        !status.success(),
        "cancelled startup must reap before settlement"
    );
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_join_shared_api_and_child_cleanup_after_cancelled_and_concurrent_native_shutdown_waiters()
 {
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    provider.create_function(create("shutdown")).await.unwrap();
    let response = invoke(&provider, None).await;
    let executor = provider.executor().clone();
    let mut waiting = Box::pin(executor.shutdown());
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    let (first, second) = tokio::join!(executor.shutdown(), executor.shutdown());
    first.unwrap();
    second.unwrap();
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(response["pid"].as_u64().unwrap().to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(!status.success());
    provider.shutdown().await;
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

#[cfg(unix)]
#[tokio::test]
async fn test_should_reap_cancelled_invocations_and_share_attempt_identity_with_runtime_api() {
    let root = tempfile::tempdir().unwrap();
    let started = root.path().join("started");
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    let mut input = create("cancellable");
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
    let handle = provider
        .start_invocation("revision-test", None, b"{}", None)
        .await
        .unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tokio::fs::read_to_string(&started).await {
                Ok(pid) if !pid.is_empty() => break pid,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {}
                Err(error) => panic!("Read start marker: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    handle
        .cancel_and_wait(tokio::time::Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    let status = tokio::process::Command::new("kill")
        .arg("-0")
        .arg(pid.trim())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(
        !status.success(),
        "cancel acknowledgement must follow process reaping"
    );

    provider
        .update_function_configuration(
            "revision-test",
            &UpdateFunctionConfigurationInput {
                environment: Some(Environment {
                    variables: Some(HashMap::new()),
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let handle = provider
        .start_invocation("revision-test", None, br#"{"value":42}"#, None)
        .await
        .unwrap();
    let id = handle.id().to_string();
    let result = handle.wait().await.unwrap();
    let response: serde_json::Value = serde_json::from_slice(&result.payload).unwrap();
    assert_eq!(response["request_id"], id);
    assert_eq!(response["echo"]["value"], 42);
    assert_ne!(response["pid"].as_u64().unwrap().to_string(), pid.trim());
    provider.quiesce(Duration::from_secs(2)).await.unwrap();
    provider.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_reap_successful_instances_before_returning_when_warm_retention_is_full() {
    for max_warm_instances in [0, 1] {
        let root = tempfile::tempdir().unwrap();
        let provider = RustackLambda::with_store(
            FunctionStore::new(root.path()),
            LambdaConfig {
                executor: ExecutorBackend::Native,
                max_warm_instances,
                ..Default::default()
            },
        );
        let mut input = create("retirement");
        input.environment = Some(Environment {
            variables: Some(HashMap::from([("SLEEP_SECS".into(), "1".into())])),
        });
        provider.create_function(input).await.unwrap();
        let first = provider
            .start_invocation("revision-test", None, b"{}", None)
            .await
            .unwrap();
        let second = provider
            .start_invocation("revision-test", None, b"{}", None)
            .await
            .unwrap();
        let (first, second) = tokio::join!(first.wait(), second.wait());
        let mut alive = 0;
        for result in [first.unwrap(), second.unwrap()] {
            let response: serde_json::Value = serde_json::from_slice(&result.payload).unwrap();
            let status = tokio::process::Command::new("kill")
                .arg("-0")
                .arg(response["pid"].as_u64().unwrap().to_string())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap();
            alive += usize::from(status.success());
        }
        assert_eq!(
            alive, max_warm_instances,
            "nonretained instances must already be reaped"
        );
        provider.shutdown().await;
    }
}

#[tokio::test]
async fn test_should_reject_native_event_architecture_mismatch_before_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_store(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            ..Default::default()
        },
    );
    let mut input = create("mismatched");
    input.architectures = Some(vec![
        if cfg!(target_arch = "aarch64") {
            "x86_64"
        } else {
            "arm64"
        }
        .into(),
    ]);
    provider.create_function(input).await.unwrap();
    assert!(
        provider
            .invoke("revision-test", None, b"{}", InvokeKind::Event)
            .await
            .is_err()
    );
    provider.quiesce(Duration::from_secs(1)).await.unwrap();
    provider.shutdown().await;
}
