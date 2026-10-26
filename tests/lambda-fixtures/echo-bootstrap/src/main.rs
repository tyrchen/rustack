//! Lambda bootstrap fixture for rustack integration tests.
//!
//! Behavior is controlled by env vars set on the function:
//!
//! - default → echo `{"echo": <body>, "request_id": <id>}`
//! - `FAIL_MODE=panic` → POST `/error` with a known JSON shape
//! - `WORKFLOW_MODE=1` → checked computation; optional `WORKFLOW_FAIL_ONCE=1`
//! - `INVOCATION_FILE=path` → per-event PID/request-ID marker after body receipt
//! - `SLEEP_SECS=N` → sleep N seconds before responding (for timeout tests)
//!
//! Uses synchronous `ureq` to keep the binary tiny and free of an async
//! runtime — the runtime API we talk to is in another process anyway.

#![allow(missing_docs)]

use std::{io::Read as _, time::Duration};

#[allow(clippy::disallowed_methods)] // This standalone bootstrap deliberately uses synchronous IO, not an async runtime.
fn main() {
    let api = std::env::var("AWS_LAMBDA_RUNTIME_API")
        .expect("AWS_LAMBDA_RUNTIME_API must be set by the runtime");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_mins(2))
        .build();
    let fail_mode = std::env::var("FAIL_MODE").ok();
    let sleep_secs: u64 = std::env::var("SLEEP_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let code_marker = match std::fs::read_to_string("revision.txt") {
        Ok(marker) => Some(marker),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!("Read revision marker: {error}");
            return;
        }
    };
    let config_marker = std::env::var("CONFIG_MARKER").ok();
    let workflow_mode = std::env::var("WORKFLOW_MODE").is_ok_and(|value| value == "1");
    let workflow_fail_once = std::env::var("WORKFLOW_FAIL_ONCE").is_ok_and(|value| value == "1");
    if let Ok(path) = std::env::var("STARTED_FILE") {
        if let Err(error) = std::fs::write(path, std::process::id().to_string()) {
            eprintln!("Write start marker: {error}");
            return;
        }
    }
    if let Some(delay) = std::env::var("INIT_DELAY_MILLIS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_millis(delay));
    }
    if std::env::var("INIT_FAIL").is_ok_and(|value| value == "1") {
        let _ = agent
            .post(&format!("http://{api}/2018-06-01/runtime/init/error"))
            .send_string(
                r#"{"errorMessage":"secret-initialization-value","errorType":"TestInitError"}"#,
            );
        return;
    }
    loop {
        let next = agent
            .get(&format!("http://{api}/2018-06-01/runtime/invocation/next"))
            .call();
        let resp = match next {
            Ok(r) => r,
            Err(e) => {
                eprintln!("bootstrap: /next failed: {e}; exiting");
                return;
            }
        };
        let request_id = resp
            .header("Lambda-Runtime-Aws-Request-Id")
            .unwrap_or("")
            .to_owned();

        let mut body = String::new();
        if let Err(e) = resp.into_reader().read_to_string(&mut body) {
            eprintln!("bootstrap: read invocation body failed: {e}");
            return;
        }

        if let Ok(path) = std::env::var("INVOCATION_FILE") {
            let marker = serde_json::json!({"pid":std::process::id(),"request_id":request_id});
            if let Err(error) = std::fs::write(path, marker.to_string()) {
                eprintln!("Write invocation marker: {error}");
                return;
            }
        }
        if sleep_secs > 0 {
            std::thread::sleep(Duration::from_secs(sleep_secs));
        }

        if matches!(fail_mode.as_deref(), Some("panic")) {
            let _ = agent
                .post(&format!(
                    "http://{api}/2018-06-01/runtime/invocation/{request_id}/error"
                ))
                .send_string(r#"{"errorMessage":"boom","errorType":"TestError"}"#);
            continue;
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let (kind, response) = invocation_response(
            &parsed,
            workflow_mode,
            workflow_fail_once,
            &request_id,
            code_marker.as_deref(),
            config_marker.as_deref(),
            &api,
        );
        let _ = agent
            .post(&format!(
                "http://{api}/2018-06-01/runtime/invocation/{request_id}/{kind}"
            ))
            .send_string(&response.to_string());
    }
}

fn compute_workflow(
    input: &serde_json::Value,
    fail_once: bool,
    code_marker: Option<&str>,
    request_id: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let retry = input
        .get("retryCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if fail_once && retry == 0 {
        return Err(
            serde_json::json!({"errorMessage":"retry requested","errorType":"WorkflowTransient"}),
        );
    }
    let value = input
        .get("n")
        .and_then(serde_json::Value::as_i64)
        .and_then(|n| n.checked_mul(2))
        .ok_or_else(
            || serde_json::json!({"errorMessage":"invalid input","errorType":"WorkflowInputError"}),
        )?;
    Ok(
        serde_json::json!({"computed":value,"retryCount":retry,"request_id":request_id,"codeMarker":code_marker,"pid":std::process::id()}),
    )
}

fn invocation_response(
    input: &serde_json::Value,
    workflow: bool,
    fail_once: bool,
    request_id: &str,
    code_marker: Option<&str>,
    config_marker: Option<&str>,
    api: &str,
) -> (&'static str, serde_json::Value) {
    if workflow {
        return match compute_workflow(input, fail_once, code_marker, request_id) {
            Ok(body) => ("response", body),
            Err(error) => ("error", error),
        };
    }
    (
        "response",
        serde_json::json!({"echo":input,"request_id":request_id,"codeMarker":code_marker,"configMarker":config_marker,"pid":std::process::id(),"runtimeApi":api}),
    )
}
