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

use anyhow::{Result, ensure};
use process::{Server, assert_absent};
use runner::{Runner, interpreter};
use tokio::time::{Instant, timeout};

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
