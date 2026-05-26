//! Lambda bootstrap fixture for rustack integration tests.
//!
//! Behavior is controlled by env vars set on the function:
//!
//! - default → echo `{"echo": <body>, "request_id": <id>}`
//! - `FAIL_MODE=panic` → POST `/error` with a known JSON shape
//! - `SLEEP_SECS=N` → sleep N seconds before responding (for timeout tests)
//!
//! Uses synchronous `ureq` to keep the binary tiny and free of an async
//! runtime — the runtime API we talk to is in another process anyway.

#![allow(missing_docs)]

use std::{io::Read as _, time::Duration};

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
        let echo = serde_json::json!({
            "echo": parsed,
            "request_id": request_id,
        });
        let _ = agent
            .post(&format!(
                "http://{api}/2018-06-01/runtime/invocation/{request_id}/response"
            ))
            .send_string(&echo.to_string());
    }
}
