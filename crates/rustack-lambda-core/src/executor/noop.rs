//! No-op executor that echoes the request payload.
//!
//! Preserves the legacy stub behavior so downstream tests and the
//! `LAMBDA_EXECUTOR=disabled` mode continue to work without any process or
//! container being started.

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::json;

use super::{Executor, ExecutorError, InvokeRequest, InvokeResponse};

/// Echoes the request payload back wrapped in a fake API Gateway response
/// shape — identical to the old hard-coded stub.
#[derive(Debug, Default, Clone)]
pub struct NoopExecutor;

impl NoopExecutor {
    /// Construct a new no-op executor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Executor for NoopExecutor {
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        // Mirror the old echo body shape so callers depending on it (e.g.
        // existing integration tests) don't notice the indirection.
        let body = json!({
            "statusCode": 200,
            "body": String::from_utf8_lossy(&req.payload),
        });
        let bytes = serde_json::to_vec(&body).map_err(|e| ExecutorError::Io(e.to_string()))?;
        Ok(InvokeResponse::success(Bytes::from(bytes), req.qualifier))
    }

    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::*;
    use crate::executor::PackageType;

    fn req(payload: &str) -> InvokeRequest {
        InvokeRequest {
            function_arn: "arn".into(),
            function_name: "fn".into(),
            qualifier: "$LATEST".into(),
            runtime: None,
            handler: None,
            architectures: vec!["x86_64".into()],
            package_type: PackageType::Zip,
            code_root: None,
            code_zip: None,
            image_uri: None,
            environment: HashMap::new(),
            timeout: Duration::from_secs(3),
            memory_mb: 128,
            payload: Bytes::from(payload.to_owned()),
            capture_logs: false,
        }
    }

    #[tokio::test]
    async fn test_should_echo_payload_in_legacy_shape() {
        let exec = NoopExecutor::new();
        let resp = exec.invoke(req("hello")).await.unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.function_error.is_none());
        let body: serde_json::Value = serde_json::from_slice(&resp.payload).unwrap();
        assert_eq!(body["statusCode"], 200);
        assert_eq!(body["body"], "hello");
        assert_eq!(resp.executed_version, "$LATEST");
    }
}
