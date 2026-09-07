//! Runtime worker ownership independent of snapshot participation.

use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use serde::Serialize;

/// Handles retained until quiescence, snapshot publication, and final shutdown.
#[derive(Default)]
pub(crate) struct RuntimeWorkers {
    #[cfg(feature = "dynamodb")]
    pub(crate) dynamodb: Option<Arc<rustack_dynamodb_core::provider::RustackDynamoDB>>,
    #[cfg(feature = "events")]
    pub(crate) events: Option<Arc<rustack_events_core::provider::RustackEvents>>,
    #[cfg(feature = "sns")]
    pub(crate) sns: Option<Arc<rustack_sns_core::provider::RustackSns>>,
    #[cfg(feature = "lambda")]
    pub(crate) lambda: Option<Arc<rustack_lambda_core::provider::RustackLambda>>,
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
    pub(crate) ready: bool,
    pub(crate) services: BTreeMap<&'static str, &'static str>,
    delivery: BTreeMap<&'static str, DeliverySummary>,
}

impl RuntimeWorkers {
    /// Report live supervisor state, independently of historical delivery failures.
    pub(crate) fn diagnostics(&self) -> WorkerDiagnostics {
        let readiness: &[Option<(&'static str, bool)>] = &[
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
            ready: readiness.iter().flatten().all(|(_, ready)| *ready),
            services: readiness
                .iter()
                .flatten()
                .map(|(name, ready)| (*name, if *ready { "running" } else { "failed" }))
                .collect(),
            delivery: deliveries.iter().flatten().copied().collect(),
        }
    }

    /// Stop new cross-service work before collecting the supported snapshot state.
    pub(crate) async fn quiesce(&self, remaining: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + remaining;
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

    /// Final resource destruction. Call only after the snapshot decision.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        // After the snapshot decision, stop independent resource owners together:
        // a failed queue worker must not delay killing Lambda child processes.
        let (events_result, (), (), ()) = tokio::join!(
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
                    lambda.shutdown().await;
                }
            },
            async {
                #[cfg(feature = "sqs")]
                if let Some(sqs) = &self.sqs {
                    sqs.shutdown_all().await;
                }
            },
        );
        events_result
    }
}
