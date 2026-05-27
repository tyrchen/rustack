//! Real Lambda invocation tests.
//!
//! These tests build a real Rust `bootstrap` binary (in
//! `tests/lambda-fixtures/echo-bootstrap`), wrap it in a zip, push it through
//! the rustack `RustackLambda::create_function` API, then call
//! `RustackLambda::invoke` and assert the round-trip.
//!
//! We deliberately bypass the AWS SDK + HTTP layer here — the SDK's TLS
//! initialization is brittle in CI sandboxes (rustls native roots), and the
//! HTTP layer is covered by the rest of the integration suite. What's new is
//! the executor pipeline: zip → extract → spawn process → runtime API
//! round-trip → response. These tests exercise exactly that.
//!
//! Tests are gated on `RUSTACK_LAMBDA_NATIVE_TESTS=1` so they don't fire
//! during a vanilla `cargo test` (they need cargo invocation + a host that
//! can run the fixture).

// std::process::Command and std::fs::read are intentional in this test file:
// the test orchestrator runs cargo to build a fixture binary and reads it
// once at startup. The disallowed-* lints exist to keep async runtime hot
// paths free of blocking I/O, which doesn't apply here.
#![allow(
    clippy::missing_panics_doc,
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::field_reassign_with_default
)]

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        path::PathBuf,
        process::Command,
        sync::{Arc, OnceLock},
        time::Duration,
    };

    use rustack_lambda_core::{
        config::LambdaConfig,
        executor::ExecutorBackend,
        provider::{InvokeKind, InvokeOutcome, RustackLambda},
        storage::FunctionStore,
    };
    use rustack_lambda_model::{input::CreateFunctionInput, types::FunctionCode};

    /// Build the fixture once per test process.
    fn bootstrap_path() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let manifest = env!("CARGO_MANIFEST_DIR");
            let workspace_root = PathBuf::from(manifest)
                .ancestors()
                .nth(2)
                .expect("workspace root")
                .to_path_buf();
            let status = Command::new(env!("CARGO"))
                .args(["build", "--release", "-p", "rustack-lambda-echo-bootstrap"])
                .current_dir(&workspace_root)
                .status()
                .expect("cargo build fixture");
            assert!(status.success(), "fixture build failed");

            let out = Command::new(env!("CARGO"))
                .args(["metadata", "--format-version=1", "--no-deps"])
                .current_dir(&workspace_root)
                .output()
                .expect("cargo metadata");
            let meta: serde_json::Value =
                serde_json::from_slice(&out.stdout).expect("metadata json");
            let target_dir = meta["target_directory"]
                .as_str()
                .expect("target_directory in metadata")
                .to_owned();
            let path = PathBuf::from(target_dir).join("release").join("bootstrap");
            assert!(path.exists(), "bootstrap not at {}", path.display());
            path
        })
        .clone()
    }

    /// Pack the fixture binary as a zip with one entry `bootstrap` (mode 755).
    fn fixture_zip() -> Vec<u8> {
        let bootstrap = bootstrap_path();
        let bytes = std::fs::read(&bootstrap).expect("read bootstrap");
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            w.start_file("bootstrap", opts).unwrap();
            w.write_all(&bytes).unwrap();
            w.finish().unwrap();
        }
        buf
    }

    fn host_arch_label() -> &'static str {
        match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "arm64",
            other => panic!("unsupported test host arch: {other}"),
        }
    }

    fn skip_unless_native_tests_enabled() -> bool {
        if std::env::var("RUSTACK_LAMBDA_NATIVE_TESTS").as_deref() == Ok("1") {
            return false;
        }
        eprintln!("set RUSTACK_LAMBDA_NATIVE_TESTS=1 to run lambda invoke integration tests");
        true
    }

    fn make_provider() -> Arc<RustackLambda> {
        let mut config = LambdaConfig::default();
        config.executor = ExecutorBackend::Native;
        config.init_timeout = Duration::from_secs(5);
        config.idle_timeout = Duration::from_mins(1);
        config.max_warm_instances = 2;

        let tmp = tempfile::Builder::new()
            .prefix("rustack-lambda-it-")
            .tempdir()
            .expect("tempdir");
        // Leak the tempdir so the path lives for the duration of the test —
        // dropping it deletes the unzipped bootstrap.
        let dir = tmp.keep();
        let store = FunctionStore::new(dir);
        Arc::new(RustackLambda::with_store(store, config))
    }

    fn create_input(name: &str, env: &[(&str, &str)]) -> CreateFunctionInput {
        use base64::Engine as _;
        let zip_b64 = base64::engine::general_purpose::STANDARD.encode(fixture_zip());
        let mut variables = std::collections::HashMap::new();
        for (k, v) in env {
            variables.insert((*k).to_owned(), (*v).to_owned());
        }
        let environment = if variables.is_empty() {
            None
        } else {
            Some(rustack_lambda_model::types::Environment {
                variables: Some(variables),
            })
        };
        CreateFunctionInput {
            function_name: name.to_owned(),
            runtime: Some("provided.al2023".to_owned()),
            role: "arn:aws:iam::000000000000:role/test-role".to_owned(),
            handler: Some("bootstrap".to_owned()),
            timeout: Some(5),
            architectures: Some(vec![host_arch_label().to_owned()]),
            code: FunctionCode {
                zip_file: Some(zip_b64),
                ..Default::default()
            },
            environment,
            ..Default::default()
        }
    }

    fn unique_name(prefix: &str) -> String {
        format!("{prefix}-{}", &uuid::Uuid::new_v4().to_string()[..8])
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires native bootstrap fixture build; gate via RUSTACK_LAMBDA_NATIVE_TESTS=1"]
    async fn test_should_invoke_native_echo_sync() {
        if skip_unless_native_tests_enabled() {
            return;
        }
        let provider = make_provider();
        let name = unique_name("echo");
        provider
            .create_function(create_input(&name, &[]))
            .await
            .expect("create_function");

        let outcome = provider
            .invoke(
                &name,
                None,
                br#"{"hello":"world"}"#,
                InvokeKind::RequestResponse,
            )
            .await
            .expect("invoke");
        let resp = match outcome {
            InvokeOutcome::Sync(r) => r,
            other => panic!("expected Sync, got {other:?}"),
        };
        assert_eq!(resp.status, 200);
        assert!(resp.function_error.is_none());
        let v: serde_json::Value = serde_json::from_slice(&resp.payload).expect("response is JSON");
        assert_eq!(v["echo"]["hello"], "world");
        assert!(!v["request_id"].as_str().unwrap().is_empty());

        provider.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires native bootstrap fixture build; gate via RUSTACK_LAMBDA_NATIVE_TESTS=1"]
    async fn test_should_invoke_dry_run_short_circuits_without_spawn() {
        if skip_unless_native_tests_enabled() {
            return;
        }
        let provider = make_provider();
        let name = unique_name("dryrun");
        provider
            .create_function(create_input(&name, &[]))
            .await
            .unwrap();

        let outcome = provider
            .invoke(&name, None, b"{}", InvokeKind::DryRun)
            .await
            .unwrap();
        assert!(matches!(outcome, InvokeOutcome::DryRun));
        provider.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires native bootstrap fixture build; gate via RUSTACK_LAMBDA_NATIVE_TESTS=1"]
    async fn test_should_invoke_event_returns_async_outcome() {
        if skip_unless_native_tests_enabled() {
            return;
        }
        let provider = make_provider();
        let name = unique_name("event");
        provider
            .create_function(create_input(&name, &[]))
            .await
            .unwrap();

        let outcome = provider
            .invoke(&name, None, b"{}", InvokeKind::Event)
            .await
            .unwrap();
        assert!(
            matches!(outcome, InvokeOutcome::Async { .. }),
            "expected Async, got {outcome:?}"
        );
        // Give the background task a moment to run + complete.
        tokio::time::sleep(Duration::from_millis(500)).await;
        provider.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires native bootstrap fixture build; gate via RUSTACK_LAMBDA_NATIVE_TESTS=1"]
    async fn test_should_propagate_function_error_as_unhandled() {
        if skip_unless_native_tests_enabled() {
            return;
        }
        let provider = make_provider();
        let name = unique_name("err");
        provider
            .create_function(create_input(&name, &[("FAIL_MODE", "panic")]))
            .await
            .unwrap();

        let outcome = provider
            .invoke(&name, None, b"{}", InvokeKind::RequestResponse)
            .await
            .unwrap();
        let resp = match outcome {
            InvokeOutcome::Sync(r) => r,
            other => panic!("expected Sync, got {other:?}"),
        };
        assert_eq!(resp.status, 200);
        assert_eq!(resp.function_error.as_deref(), Some("Unhandled"));
        let v: serde_json::Value = serde_json::from_slice(&resp.payload).expect("err body is JSON");
        assert_eq!(v["errorType"], "TestError");
        provider.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires native bootstrap fixture build; gate via RUSTACK_LAMBDA_NATIVE_TESTS=1"]
    async fn test_should_warm_reuse_instance_for_repeat_invocations() {
        if skip_unless_native_tests_enabled() {
            return;
        }
        let provider = make_provider();
        let name = unique_name("warm");
        provider
            .create_function(create_input(&name, &[]))
            .await
            .unwrap();

        let cold_start = std::time::Instant::now();
        let _ = provider
            .invoke(&name, None, b"{}", InvokeKind::RequestResponse)
            .await
            .unwrap();
        let cold = cold_start.elapsed();

        let warm_start = std::time::Instant::now();
        let _ = provider
            .invoke(&name, None, b"{}", InvokeKind::RequestResponse)
            .await
            .unwrap();
        let warm = warm_start.elapsed();

        eprintln!("cold: {cold:?}, warm: {warm:?}");
        assert!(
            warm * 2 < cold,
            "expected warm ({warm:?}) at least 2x faster than cold ({cold:?}) — process was not \
             reused",
        );
        provider.shutdown().await;
    }
}
