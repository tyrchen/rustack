//! Lambda bootstrap fixture for rustack integration tests.
//!
//! Behavior is controlled by env vars set on the function:
//!
//! - default → echo `{"echo": <body>, "request_id": <id>}`
//! - `FAIL_MODE=panic` → POST `/error` with a known JSON shape
//! - `WORKFLOW_MODE=1` → checked computation; optional `WORKFLOW_FAIL_ONCE=1`
//! - `PROCESS_MARKER_DIR=path` → per-PID process ownership evidence
//! - `WORKFLOW_GATE_FILE=path` → bounded consumed-event gate for SDK revision tests
//! - `WORKFLOW_TOKEN_PORT=port` → local in-memory test collector; no token files/logs
//! - `WORKFLOW_INVOCATION_DIR=path` → atomic per-request consumed-event markers
//! - `INVOCATION_FILE=path` → per-event PID/request-ID marker after body receipt
//! - `SLEEP_SECS=N` → sleep N seconds before responding (for timeout tests)
//!
//! Uses synchronous `ureq` to keep the binary tiny and free of an async
//! runtime — the runtime API we talk to is in another process anyway.

#![allow(missing_docs)]

#[cfg(all(unix, feature = "provided-qualification"))]
mod provided_qualification;

use std::{
    io::{Error as IoError, Read as _},
    num::NonZeroU16,
    path::{Path, PathBuf},
    time::Duration,
};

use ureq::{Agent, AgentBuilder};

#[allow(clippy::disallowed_methods)] // This standalone bootstrap deliberately uses synchronous IO, not an async runtime.
fn main() {
    let api = runtime_api();
    let agent = AgentBuilder::new().timeout(Duration::from_mins(2)).build();
    #[cfg(all(unix, feature = "provided-qualification"))]
    let mut provided = match provided_qualification::Probe::selected() {
        Ok(provided) => provided,
        Err(error) => {
            eprintln!("Provided fixture setup: {error}");
            return;
        }
    };
    let fail_mode = std::env::var("FAIL_MODE").ok();
    let sleep_secs = sleep_seconds();

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
    if let Ok(directory) = std::env::var("PROCESS_MARKER_DIR") {
        let pid = std::process::id().to_string();
        let path = Path::new(&directory).join(format!("pid-{pid}"));
        if let Err(error) = std::fs::write(path, &pid) {
            eprintln!("Write process ownership marker: {error}");
            return;
        }
    }
    if let Ok(path) = std::env::var("STARTED_FILE") {
        if let Err(error) = std::fs::write(path, std::process::id().to_string()) {
            eprintln!("Write start marker: {error}");
            return;
        }
    }
    if !initialize(&agent, &api) {
        return;
    }
    let behavior = Behavior {
        workflow: workflow_mode,
        fail_once: workflow_fail_once,
        code_marker: code_marker.as_deref(),
        config_marker: config_marker.as_deref(),
    };
    loop {
        let (request_id, body) = match next_invocation(&agent, &api) {
            Ok(invocation) => invocation,
            Err(error) => {
                eprintln!("bootstrap: invocation receive failed: {error}");
                return;
            }
        };

        if let Ok(path) = std::env::var("INVOCATION_FILE") {
            let marker = serde_json::json!({"pid":std::process::id(),"request_id":request_id});
            if let Err(error) = std::fs::write(path, marker.to_string()) {
                eprintln!("Write invocation marker: {error}");
                return;
            }
        }
        if let Ok(directory) = std::env::var("WORKFLOW_INVOCATION_DIR") {
            let marker = consumed_marker(Path::new(&directory), &request_id).and_then(
                |(pending, complete, body)| {
                    std::fs::write(&pending, body)?;
                    std::fs::rename(pending, complete)
                },
            );
            if let Err(error) = marker {
                eprintln!("Write consumed invocation marker: {error}");
                return;
            }
        }
        if let Ok(path) = std::env::var("WORKFLOW_GATE_FILE") {
            if let Err(error) = wait_workflow_gate(Path::new(&path)) {
                eprintln!("Workflow fixture gate failed: {error}");
                return;
            }
        }
        if sleep_secs > 0 {
            std::thread::sleep(Duration::from_secs(sleep_secs));
        }

        if matches!(fail_mode.as_deref(), Some("panic")) {
            known_error(&agent, &api, &request_id);
            continue;
        }

        if !complete_invocation(
            &agent,
            &api,
            &request_id,
            &body,
            &behavior,
            #[cfg(all(unix, feature = "provided-qualification"))]
            provided.as_mut(),
        ) {
            return;
        }
    }
}

fn runtime_api() -> String {
    let Ok(api) = std::env::var("AWS_LAMBDA_RUNTIME_API") else {
        eprintln!("bootstrap: Runtime API endpoint missing");
        std::process::exit(78);
    };
    api
}

fn known_error(agent: &Agent, api: &str, request_id: &str) {
    let _ = post_runtime(
        agent,
        api,
        &format!("invocation/{request_id}/error"),
        r#"{"errorMessage":"boom","errorType":"TestError"}"#,
    );
}

#[derive(Debug)]
struct Behavior<'a> {
    workflow: bool,
    fail_once: bool,
    code_marker: Option<&'a str>,
    config_marker: Option<&'a str>,
}
fn complete_invocation(
    agent: &Agent,
    api: &str,
    request_id: &str,
    body: &str,
    behavior: &Behavior<'_>,
    #[cfg(all(unix, feature = "provided-qualification"))] provided: Option<
        &mut provided_qualification::Probe,
    >,
) -> bool {
    let parsed = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if !collect_callback_token(agent, api, request_id, &parsed) {
        return true;
    }
    let (kind, response) = invocation_response(
        &parsed,
        behavior.workflow,
        behavior.fail_once,
        request_id,
        behavior.code_marker,
        behavior.config_marker,
        api,
    );
    #[cfg(all(unix, feature = "provided-qualification"))]
    let mut response = response;
    #[cfg(all(unix, feature = "provided-qualification"))]
    if let Some(provided) = provided {
        if let Err(error) = provided.augment(&parsed, &mut response, request_id) {
            eprintln!("Provided fixture observation: {error}");
            let failure =
                serde_json::json!({"errorType":"ProvidedQualificationError","errorMessage":error});
            let _ = post_runtime(
                agent,
                api,
                &format!("invocation/{request_id}/error"),
                &failure.to_string(),
            );
            return false;
        }
    }
    let _ = post_runtime(
        agent,
        api,
        &format!("invocation/{request_id}/{kind}"),
        &response.to_string(),
    );
    true
}

fn collect_callback_token(
    agent: &Agent,
    api: &str,
    request_id: &str,
    event: &serde_json::Value,
) -> bool {
    if forward_callback_token(event).is_ok() {
        return true;
    }
    let _ = post_runtime(
        agent,
        api,
        &format!("invocation/{request_id}/error"),
        r#"{"errorType":"FixtureTokenCollection","errorMessage":"local collector failed"}"#,
    );
    false
}

fn forward_callback_token(event: &serde_json::Value) -> Result<(), &'static str> {
    let Ok(port) = std::env::var("WORKFLOW_TOKEN_PORT") else {
        return Ok(());
    };
    let port: NonZeroU16 = port.parse().map_err(|_| "invalid collector port")?;
    let token = event
        .get("token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| token.len() == 64)
        .ok_or("missing token")?;
    let collector = AgentBuilder::new().timeout(Duration::from_secs(2)).build();
    collector
        .post(&format!("http://127.0.0.1:{port}/token"))
        .send_string(token)
        .map_err(|_| "local token collector failed")?;
    Ok(())
}

fn sleep_seconds() -> u64 {
    std::env::var("SLEEP_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn initialize(agent: &Agent, api: &str) -> bool {
    if let Some(delay) = std::env::var("INIT_DELAY_MILLIS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_millis(delay));
    }
    if std::env::var("INIT_FAIL").is_ok_and(|value| value == "1") {
        let _ = post_runtime(
            agent,
            api,
            "init/error",
            r#"{"errorMessage":"secret-initialization-value","errorType":"TestInitError"}"#,
        );
        return false;
    }
    true
}

fn consumed_marker(
    directory: &Path,
    request_id: &str,
) -> Result<(PathBuf, PathBuf, String), IoError> {
    let valid = request_id.len() == 36
        && request_id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    if !valid {
        return Err(IoError::other("Invalid invocation marker request ID"));
    }
    let marker = serde_json::json!({"pid":std::process::id(),"request_id":request_id});
    Ok((
        directory.join(format!("{request_id}.pending")),
        directory.join(format!("{request_id}.json")),
        marker.to_string(),
    ))
}

fn wait_workflow_gate(path: &Path) -> Result<(), IoError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        if path.try_exists()? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(IoError::other("Workflow fixture gate expired"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn post_runtime(agent: &Agent, api: &str, route: &str, payload: &str) -> Result<(), IoError> {
    agent
        .post(&format!("http://{api}/2018-06-01/runtime/{route}"))
        .send_string(payload)
        .map(|_| ())
        .map_err(IoError::other)
}

fn next_invocation(agent: &Agent, api: &str) -> Result<(String, String), IoError> {
    let response = agent
        .get(&format!("http://{api}/2018-06-01/runtime/invocation/next"))
        .call()
        .map_err(IoError::other)?;
    let request_id = response
        .header("Lambda-Runtime-Aws-Request-Id")
        .ok_or_else(|| IoError::other("Runtime API request ID missing"))?
        .to_owned();
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body)?;
    Ok((request_id, body))
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
