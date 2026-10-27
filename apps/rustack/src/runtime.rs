//! Runtime worker ownership independent of snapshot participation.

#[cfg(not(all(
    any(feature = "events", feature = "sns", feature = "dynamodb"),
    any(feature = "lambda", feature = "sqs")
)))]
use std::future::{Ready, ready};
#[cfg(any(
    feature = "dynamodb",
    feature = "events",
    feature = "sns",
    feature = "lambda",
    feature = "sqs"
))]
use std::sync::Arc;
use std::{collections::BTreeMap, fmt};

#[cfg(any(
    feature = "events",
    feature = "sns",
    feature = "dynamodb",
    feature = "sqs"
))]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "lambda")]
use rustack_lambda_core::diagnostics::InvocationWorkStatus;
use serde::Serialize;

/// Handles retained until quiescence, snapshot publication, and final shutdown.
#[derive(Default)]
pub(crate) struct RuntimeWorkers {
    #[cfg(feature = "stepfunctions")]
    pub(crate) workflows: Option<crate::workflows::WorkflowStatus>,
    #[cfg(feature = "dynamodb")]
    pub(crate) dynamodb: Option<Arc<rustack_dynamodb_core::provider::RustackDynamoDB>>,
    #[cfg(feature = "events")]
    pub(crate) events: Option<Arc<rustack_events_core::provider::RustackEvents>>,
    #[cfg(feature = "sns")]
    pub(crate) sns: Option<Arc<rustack_sns_core::provider::RustackSns>>,
    #[cfg(feature = "lambda")]
    pub(crate) lambda: Option<Arc<rustack_lambda_core::provider::RustackLambda>>,
    #[cfg(all(feature = "lambda", unix))]
    pub(crate) docker: Option<rustack_lambda_core::docker::DockerRuntimeStatus>,
    #[cfg(feature = "sqs")]
    pub(crate) sqs: Option<Arc<rustack_sqs_core::provider::RustackSqs>>,
}

impl fmt::Debug for RuntimeWorkers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeWorkers")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeliverySummary {
    accepted: u64,
    delivered: u64,
    failed: u64,
    rejected: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerDiagnostics {
    #[cfg(feature = "lambda")]
    #[serde(skip_serializing_if = "Option::is_none")]
    lambda_admission: Option<InvocationWorkStatus>,
    #[cfg(all(feature = "lambda", unix))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) docker: Option<rustack_lambda_core::docker::DockerCapabilities>,
    pub(crate) ready: bool,
    pub(crate) services: BTreeMap<&'static str, &'static str>,
    delivery: BTreeMap<&'static str, DeliverySummary>,
}

impl RuntimeWorkers {
    /// Report live supervisor state, independently of historical delivery failures.
    pub(crate) fn diagnostics(&self) -> WorkerDiagnostics {
        let readiness: &[Option<(&'static str, bool)>] = &[
            #[cfg(feature = "lambda")]
            self.lambda
                .as_ref()
                .map(|provider| ("lambda", provider.is_ready())),
            #[cfg(feature = "stepfunctions")]
            self.workflows
                .as_ref()
                .map(|workflow| ("stepfunctions", workflow.is_ready())),
            #[cfg(feature = "events")]
            self.events
                .as_ref()
                .map(|provider| ("events", provider.is_ready())),
            #[cfg(feature = "sns")]
            self.sns
                .as_ref()
                .map(|provider| ("sns", provider.is_ready())),
            #[cfg(feature = "dynamodb")]
            self.dynamodb
                .as_ref()
                .map(|provider| ("dynamodb", provider.is_ready())),
            #[cfg(feature = "sqs")]
            self.sqs
                .as_ref()
                .map(|provider| ("sqs", provider.is_ready())),
        ];
        let deliveries: &[Option<(&'static str, DeliverySummary)>] = &[
            #[cfg(feature = "events")]
            self.events.as_ref().map(|provider| {
                let stats = provider.delivery_stats();
                (
                    "events",
                    DeliverySummary {
                        accepted: stats.accepted,
                        delivered: stats.delivered,
                        failed: stats.failed,
                        rejected: stats.rejected,
                    },
                )
            }),
            #[cfg(feature = "sns")]
            self.sns.as_ref().map(|provider| {
                let stats = provider.delivery_stats();
                (
                    "sns",
                    DeliverySummary {
                        accepted: stats.accepted,
                        delivered: stats.delivered,
                        failed: stats.failed,
                        rejected: stats.rejected,
                    },
                )
            }),
        ];
        WorkerDiagnostics {
            #[cfg(feature = "lambda")]
            lambda_admission: self.lambda.as_ref().map(|provider| provider.work_status()),
            #[cfg(all(feature = "lambda", unix))]
            docker: self
                .docker
                .as_ref()
                .map(rustack_lambda_core::docker::DockerRuntimeStatus::capabilities),
            ready: readiness.iter().flatten().all(|(_, ready)| *ready),
            services: readiness
                .iter()
                .flatten()
                .map(|(name, ready)| (*name, if *ready { "running" } else { "failed" }))
                .collect(),
            delivery: deliveries.iter().flatten().copied().collect(),
        }
    }

    #[cfg(feature = "stepfunctions")]
    pub(crate) fn workflow_capabilities(&self) -> Option<serde_json::Value> {
        self.workflows
            .as_ref()
            .map(crate::workflows::WorkflowStatus::capabilities)
    }

    /// Quiesce producers before the unique workflow supervisor drains.
    #[cfg(any(feature = "events", feature = "sns", feature = "dynamodb"))]
    pub(crate) async fn quiesce_producers(&self, deadline: tokio::time::Instant) -> Result<()> {
        check_deadline(deadline)?;
        #[cfg(feature = "events")]
        if let Some(events) = &self.events {
            tokio::time::timeout_at(deadline, events.quiesce())
                .await
                .context("EventBridge quiesce deadline exceeded")??;
        }
        #[cfg(feature = "sns")]
        if let Some(sns) = &self.sns {
            tokio::time::timeout_at(deadline, sns.quiesce())
                .await
                .context("SNS quiesce deadline exceeded")?;
        }
        #[cfg(feature = "dynamodb")]
        if let Some(dynamodb) = &self.dynamodb {
            tokio::time::timeout_at(deadline, dynamodb.quiesce())
                .await
                .context("DynamoDB quiesce deadline exceeded")??;
        }
        Ok(())
    }

    /// Downstream consumers stay available until workflow cleanup joins.
    #[cfg(any(feature = "lambda", feature = "sqs"))]
    pub(crate) async fn quiesce_consumers(&self, deadline: tokio::time::Instant) -> Result<()> {
        check_deadline(deadline)?;
        #[cfg(feature = "lambda")]
        if let Some(lambda) = &self.lambda {
            lambda
                .quiesce(deadline.saturating_duration_since(tokio::time::Instant::now()))
                .await?;
        }
        #[cfg(feature = "sqs")]
        if let Some(sqs) = &self.sqs {
            tokio::time::timeout_at(deadline, sqs.quiesce())
                .await
                .context("SQS quiesce deadline exceeded")??;
        }
        Ok(())
    }

    #[cfg(not(any(feature = "events", feature = "sns", feature = "dynamodb")))]
    pub(crate) fn quiesce_producers(&self, deadline: tokio::time::Instant) -> Ready<Result<()>> {
        tracing::debug!(workers = ?self.diagnostics(), "No producer services require quiescence");
        ready(check_deadline(deadline))
    }
    #[cfg(not(any(feature = "lambda", feature = "sqs")))]
    pub(crate) fn quiesce_consumers(&self, deadline: tokio::time::Instant) -> Ready<Result<()>> {
        tracing::debug!(workers = ?self.diagnostics(), "No consumer services require quiescence");
        ready(check_deadline(deadline))
    }

    /// Final resource destruction. Call only after the snapshot decision.
    pub(crate) async fn shutdown(&self, deadline: tokio::time::Instant) -> Result<()> {
        // After the snapshot decision, stop independent resource owners together:
        // a failed queue worker must not delay killing Lambda child processes.
        let (events_result, (), lambda_result, ()) = tokio::join!(
            async {
                #[cfg(feature = "events")]
                if let Some(events) = &self.events {
                    events.shutdown().await?;
                }
                Result::<()>::Ok(())
            },
            async {
                #[cfg(feature = "sns")]
                if let Some(sns) = &self.sns {
                    sns.shutdown().await;
                }
            },
            async {
                #[cfg(feature = "lambda")]
                if let Some(lambda) = &self.lambda {
                    lambda.shutdown_before(deadline).await?;
                }
                Result::<()>::Ok(())
            },
            async {
                #[cfg(feature = "sqs")]
                if let Some(sqs) = &self.sqs {
                    sqs.shutdown_all().await;
                }
            },
        );
        events_result?;
        lambda_result?;
        check_deadline(deadline)
    }
}

fn check_deadline(deadline: tokio::time::Instant) -> Result<()> {
    anyhow::ensure!(
        tokio::time::Instant::now() < deadline,
        "runtime quiescence deadline exceeded"
    );
    Ok(())
}

#[cfg(test)]
mod diagnostic_tests {
    #[cfg(feature = "lambda")]
    use rustack_lambda_core::{config::LambdaConfig, provider::RustackLambda};

    use super::*;

    #[test]
    fn test_should_omit_admission_counts_without_an_installed_lambda_provider() {
        let value = serde_json::to_value(RuntimeWorkers::default().diagnostics()).unwrap();
        assert!(value.get("lambdaAdmission").is_none());
    }

    #[cfg(feature = "lambda")]
    #[tokio::test]
    async fn test_should_project_the_actual_installed_lambda_admission_without_starting_work() {
        let provider = Arc::new(RustackLambda::new(LambdaConfig::default()));
        let workers = RuntimeWorkers {
            lambda: Some(provider.clone()),
            ..RuntimeWorkers::default()
        };
        let value = serde_json::to_value(workers.diagnostics()).unwrap();
        assert_eq!(
            value.get("lambdaAdmission"),
            Some(&serde_json::json!({"acceptedInvocations":0,
            "activeExecutions":0,"accepting":true,"limits":{"acceptedInvocations":128,
            "activeExecutions":32,"defaultFunctionExecutions":8}}))
        );
        assert_eq!(provider.work_status().active_executions, 0);
    }
}
