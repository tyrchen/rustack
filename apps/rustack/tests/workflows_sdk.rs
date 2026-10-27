//! Explicit ordinary SDK qualification of the actual binary and Native runtime.
#![cfg(all(feature = "stepfunctions", feature = "lambda", unix))]
#![forbid(unsafe_code)]

#[path = "workflows_sdk/process.rs"]
mod process;
#[path = "workflows_sdk/runner.rs"]
mod runner;
#[path = "workflows_sdk/rust.rs"]
mod rust;

use std::{ffi::OsString, path::Path, time::Duration};

use anyhow::{Context, Error as AnyError, Result, ensure};
use process::{Server, assert_absent};
use runner::{Runner, interpreter};
use tokio::{
    fs,
    time::{Instant, timeout},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-stepfunctions-sdk"]
async fn test_should_accept_ordinary_rust_sdk_through_actual_runtime() -> Result<()> {
    let mut server = Server::start().await?;
    let result = match timeout(Duration::from_secs(120), rust::exercise(&server)).await {
        Ok(result) => result,
        Err(error) => Err(error.into()),
    };
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    if let Err(error) = &result {
        eprintln!("Rust SDK exercise failed: {error:#}");
    }
    let observed = cleanup?;
    result?;
    ensure!(observed > 0, "no actual bootstrap process was observed");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires uv and actual host bootstrap; run make test-stepfunctions-sdk"]
async fn test_should_accept_ordinary_boto3_through_actual_runtime() -> Result<()> {
    let python = interpreter()?;
    let mut server = Server::start().await?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = async {
        let arguments = [
            root.join("tests/workflows/stepfunctions_sdk.py")
                .into_os_string(),
            OsString::from(&server.endpoint),
            server.bootstrap.as_os_str().to_owned(),
            server.markers.as_os_str().to_owned(),
        ];
        let report = Runner::spawn(&python, &arguments)?
            .wait(Instant::now() + Duration::from_secs(120))
            .await?;
        if !report.status.success() {
            eprintln!("boto3 stderr: {}", String::from_utf8_lossy(&report.stderr));
        }
        ensure!(
            !report.forced && report.status.success(),
            "boto3 qualification failed or exceeded deadline"
        );
        let result: serde_json::Value = serde_json::from_slice(&report.stdout)?;
        ensure!(
            result["catalogCases"] == 17
                && result["m1Actions"] == 15
                && result["nestedCases"] == 6
                && result["nestedLifecycle"] == 2
                && result["structuredCases"] == 6
                && result["structuredLifecycle"] == 2
                && result["actualNative"] == true,
            "missing SDK acceptance evidence"
        );
        Ok(())
    }
    .await;
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    let observed = cleanup?;
    result?;
    ensure!(observed > 0, "no actual bootstrap process was observed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires pinned Python; run make test-stepfunctions-sdk"]
async fn test_should_reap_actual_python_after_observed_execution_timeout() -> Result<()> {
    for valid in [true, false] {
        let program = if valid {
            "import os,time; print(os.getpid(), flush=True); time.sleep(60)"
        } else {
            "import time; print('invalid-pid', flush=True); time.sleep(60)"
        };
        let arguments = [OsString::from("-c"), OsString::from(program)];
        let mut runner = Runner::spawn(&interpreter()?, &arguments)?;
        let barrier = runner.await_pid().await;
        let report = runner
            .wait(Instant::now() + Duration::from_millis(25))
            .await?;
        ensure!(
            report.forced && !report.status.success(),
            "did not stop the actual Python process"
        );
        assert_absent(report.pid).await?;
        if valid {
            ensure!(barrier? == report.pid, "wrong consumed PID");
        } else {
            ensure!(barrier.is_err(), "malformed PID barrier accepted");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-stepfunctions-sdk"]
async fn test_should_run_actual_nested_native_with_one_dispatch_slot() -> Result<()> {
    let mut server = Server::start_with_dispatch(1).await?;
    let result = match timeout(Duration::from_secs(60), rust::nested_only(&server)).await {
        Ok(result) => result,
        Err(error) => Err(error.into()),
    };
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    let observed = cleanup?;
    result?;
    ensure!(observed > 0, "no actual nested Native handler was observed");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual handler SDK fixtures; run make test-stepfunctions-sdk"]
async fn test_should_run_signed_rust_and_boto3_handler_callbacks_with_one_dispatch_slot()
-> Result<()> {
    let mut server = Server::start_with_dispatch(1).await?;
    let result = match timeout(Duration::from_secs(180), rust::callbacks_only(&server)).await {
        Ok(result) => result,
        Err(error) => Err(error.into()),
    };
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    if let Err(error) = &result {
        eprintln!("handler SDK callback qualification failed: {error:#}");
    }
    let observed = cleanup?;
    result?;
    ensure!(observed > 0, "no actual handler SDK process observed");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-stepfunctions-sdk"]
async fn test_should_join_actual_server_and_native_children_after_fixture_gate_release_failure()
-> Result<()> {
    let mut server = Server::start().await?;
    let operation = async {
        timeout(Duration::from_secs(30), rust::native_probe_only(&server)).await??;
        let private = server
            .markers
            .parent()
            .context("owned fixture root")?
            .to_owned();
        let mut native = Vec::new();
        let mut directory = fs::read_dir(&server.markers).await?;
        while let Some(entry) = directory.next_entry().await? {
            ensure!(
                native.len() < 64 && entry.metadata().await?.len() <= 11,
                "invalid PID evidence"
            );
            native.push(fs::read_to_string(entry.path()).await?.parse::<u32>()?);
        }
        fs::rename(
            private.join("callback-gates"),
            private.join("missing-callback-gates"),
        )
        .await?;
        Ok::<_, AnyError>((private, native))
    }
    .await;
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    assert_absent(server.pid).await?;
    let (private, native) = operation?;
    for pid in &native {
        assert_absent(*pid).await?;
    }
    ensure!(
        cleanup.is_err()
            && server
                .shutdown(Instant::now() + Duration::from_secs(1))
                .await
                .is_err(),
        "fixture gate failure was not retained"
    );
    ensure!(
        fs::try_exists(&private).await?,
        "failed qualification evidence was discarded"
    );
    fs::remove_dir_all(&private).await?;
    ensure!(!native.is_empty(), "no actual Native child observed");
    Ok(())
}
