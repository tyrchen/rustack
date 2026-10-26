//! Automatic Lambda executor selection.
//!
//! `AutoExecutor` keeps backend choice at invocation granularity. That lets a
//! single provider default to Squib for macOS-hosted Zip Lambdas while leaving
//! non-Zip packages and non-macOS hosts on the native path until Docker is
//! wired in.

use std::time::Duration;

use async_trait::async_trait;

use super::{
    Executor, ExecutorError, InvokeRequest, InvokeResponse, NativeExecutor, SquibExecutor,
    SquibExecutorConfig,
};
use crate::{artifact::DeploymentArtifact, invocation::InvocationContext};

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
        cfg!(target_os = "macos") && matches!(req.revision.artifact, DeploymentArtifact::Zip(_))
    }
}

#[async_trait]
impl Executor for AutoExecutor {
    fn available(&self, request: &InvokeRequest) -> Result<(), ExecutorError> {
        if Self::should_use_squib(request) {
            self.squib.available(request)
        } else {
            self.native.available(request)
        }
    }

    async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        if Self::should_use_squib(&req) {
            self.squib.invoke(req, context).await
        } else {
            self.native.invoke(req, context).await
        }
    }

    async fn shutdown(&self) -> Result<(), ExecutorError> {
        let squib = self.squib.shutdown().await;
        let native = self.native.shutdown().await;
        squib.and(native)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::artifact::{Architecture, ResolvedImage};

    #[test]
    fn test_should_default_macos_zip_to_squib() {
        assert_eq!(
            AutoExecutor::should_use_squib(&InvokeRequest::fixture()),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn test_should_not_use_squib_for_image_package() {
        let mut request = InvokeRequest::fixture();
        Arc::make_mut(&mut request.revision).artifact =
            DeploymentArtifact::Image(ResolvedImage::fixture(Architecture::Arm64));
        assert!(!AutoExecutor::should_use_squib(&request));
    }
}
