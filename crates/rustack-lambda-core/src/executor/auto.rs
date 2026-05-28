//! Automatic Lambda executor selection.
//!
//! `AutoExecutor` keeps backend choice at invocation granularity. That lets a
//! single provider default to Squib for macOS-hosted Zip Lambdas while leaving
//! non-Zip packages and non-macOS hosts on the native path until Docker is
//! wired in.

use std::time::Duration;

use async_trait::async_trait;

use super::{
    Executor, ExecutorError, InvokeRequest, InvokeResponse, NativeExecutor, PackageType,
    SquibExecutor, SquibExecutorConfig,
};

/// Executor that chooses the concrete backend for each invocation.
#[derive(Debug)]
pub struct AutoExecutor {
    native: NativeExecutor,
    squib: SquibExecutor,
}

impl AutoExecutor {
    /// Build an automatic executor from native warm-pool settings and Squib
    /// microVM settings.
    #[must_use]
    pub fn new(
        max_warm_instances: usize,
        idle_timeout: Duration,
        init_timeout: Duration,
        squib_config: SquibExecutorConfig,
    ) -> Self {
        Self {
            native: NativeExecutor::new(max_warm_instances, idle_timeout, init_timeout),
            squib: SquibExecutor::new(squib_config),
        }
    }

    fn should_use_squib(req: &InvokeRequest) -> bool {
        cfg!(target_os = "macos") && req.package_type == PackageType::Zip
    }
}

#[async_trait]
impl Executor for AutoExecutor {
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        if Self::should_use_squib(&req) {
            self.squib.invoke(req).await
        } else {
            self.native.invoke(req).await
        }
    }

    async fn shutdown(&self) {
        self.squib.shutdown().await;
        self.native.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, time::Duration};

    use bytes::Bytes;

    use super::*;

    fn request(package_type: PackageType) -> InvokeRequest {
        InvokeRequest {
            function_arn: "arn:aws:lambda:us-east-1:000000000000:function:echo".to_owned(),
            function_name: "echo".to_owned(),
            qualifier: "$LATEST".to_owned(),
            runtime: Some("provided.al2023".to_owned()),
            handler: Some("bootstrap".to_owned()),
            architectures: vec!["arm64".to_owned()],
            package_type,
            code_root: Some(PathBuf::from("/tmp/code")),
            image_uri: None,
            environment: HashMap::new(),
            timeout: Duration::from_secs(1),
            memory_mb: 128,
            payload: Bytes::new(),
            capture_logs: false,
        }
    }

    #[test]
    fn test_should_default_macos_zip_to_squib() {
        assert_eq!(
            AutoExecutor::should_use_squib(&request(PackageType::Zip)),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn test_should_not_use_squib_for_image_package() {
        assert!(!AutoExecutor::should_use_squib(&request(
            PackageType::Image
        )));
    }
}
