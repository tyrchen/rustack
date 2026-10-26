//! Actual gateway workflow composition and unique resumable shutdown ownership.

use std::{error::Error, fmt, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rustack_cli::stepfunctions_api::{ApiError, ApiLimits, ApiSupervisor, StepFunctionsApi};
#[cfg(feature = "lambda")]
use rustack_cli::stepfunctions_bridge::{
    BridgeError, BridgeLimits, LambdaBridgeSupervisor, StepFunctionsLambdaBridge,
};
use rustack_core::settings::RuntimeBudgets;
#[cfg(feature = "lambda")]
use rustack_lambda_core::{executor::ExecutorBackend, provider::RustackLambda};
#[cfg(feature = "lambda")]
use rustack_stepfunctions_core::LambdaTaskPort;
use rustack_stepfunctions_core::{
    ExecutionLimits, ExecutionSupervisor, FrameCapacity, FrameLimits, HistoryLimits, Registry,
    RegistryCapacities, RegistryError, RegistryLimits, RegistrySupervisor, RuntimeLimits,
    StateSupport, TaskSupport, TenantScope,
};
use serde_json::{Value, json};
use tokio::time::Instant;

/// Shared health reads never acquire business admission or unique controllers.
#[derive(Debug, Clone)]
pub(crate) struct WorkflowStatus {
    api: Arc<StepFunctionsApi>,
    registry: Registry,
    budgets: RuntimeBudgets,
    #[cfg(feature = "lambda")]
    port: Option<Arc<StepFunctionsLambdaBridge>>,
}
impl WorkflowStatus {
    pub(crate) fn is_ready(&self) -> bool {
        let ready = self.api.is_ready() && self.registry.status().ready;
        #[cfg(feature = "lambda")]
        let ready = ready && self.port.as_ref().is_none_or(|port| port.is_ready());
        ready
    }
    pub(crate) fn capabilities(&self) -> Value {
        #[cfg(feature = "lambda")]
        let lambda = self.port.is_some();
        #[cfg(not(feature = "lambda"))]
        let lambda = false;
        json!({
            "type": "STANDARD", "queryLanguage": "JSONPath",
            "stateTypes": ["Pass", "Choice", "Wait", "Task", "Succeed", "Fail", "Parallel", "Map"],
            "inlineMapMaxConcurrency": 40,
            "lambdaRequestResponse": lambda, "nestedSync": false, "callbacks": false,
            "snapshot": "unsupported", "dockerQualified": false,
            "limits": {
                "apiAccepted": self.budgets.workflow_api_accepted,
                "apiWorkers": self.budgets.workflow_api_workers,
                "paginationBytes": self.budgets.workflow_pagination_bytes,
                "paginationTokens": self.budgets.workflow_pagination_tokens,
                "machines": self.budgets.workflow_registry_machines,
                "versions": self.budgets.workflow_registry_versions,
                "records": self.budgets.workflow_registry_records,
                "commands": self.budgets.workflow_registry_commands,
                "nameTombstones": self.budgets.workflow_name_tombstones,
                "retentionSeconds": self.budgets.workflow_retention_seconds,
                "openExecutions": self.budgets.workflow_open_executions,
                "evaluations": self.budgets.workflow_evaluations,
                "dispatches": self.budgets.workflow_dispatches,
                "maxDurationSeconds": self.budgets.workflow_max_duration_seconds,
                "transitions": self.budgets.workflow_transitions,
                "frames": self.budgets.workflow_frames,
                "framesPerExecution": self.budgets.workflow_frames_per_execution,
                "resultBytes": self.budgets.workflow_result_bytes,
                "resultBytesPerExecution": self.budgets.workflow_result_bytes_per_execution,
                "historyEvents": 25000, "historyBytes": self.budgets.workflow_history_bytes,
                "totalHistoryBytes": self.budgets.workflow_total_history_bytes,
                "lambdaAcceptedCalls": self.budgets.workflow_lambda_accepted_calls,
                "lambdaCleanupSeconds": self.budgets.workflow_lambda_cleanup_seconds,
                "drainSeconds": self.budgets.workflow_drain_seconds
            }
        })
    }
}
#[derive(Debug, Clone, Copy)]
enum Phase {
    Accepting,
    Api(Instant),
    ClosingApi,
    Grace(Instant),
    Registry,
    Bridge,
    Done,
}
/// Kept by main, outside `Arc<RuntimeWorkers>`; never protected by a mutex.
#[derive(Debug)]
pub(crate) struct WorkflowSupervisor {
    api: ApiSupervisor,
    registry: RegistrySupervisor,
    status: WorkflowStatus,
    #[cfg(feature = "lambda")]
    bridge: Option<LambdaBridgeSupervisor>,
    phase: Phase,
    failure: Option<WorkflowFailure>,
}
#[derive(Clone)]
struct WorkflowFailure(Arc<anyhow::Error>);
impl fmt::Display for WorkflowFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workflow cleanup failed")
    }
}
impl fmt::Debug for WorkflowFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl Error for WorkflowFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
pub(crate) struct WorkflowRuntime {
    pub(crate) api: Arc<StepFunctionsApi>,
    pub(crate) status: WorkflowStatus,
    pub(crate) owner: WorkflowSupervisor,
}
impl WorkflowRuntime {
    pub(crate) fn new(
        scope: TenantScope,
        budgets: RuntimeBudgets,
        #[cfg(feature = "lambda")] provider: Option<Arc<RustackLambda>>,
    ) -> Result<Self> {
        budgets.validate().context("validate workflow budgets")?;
        #[cfg(feature = "lambda")]
        let (port, bridge) =
            match provider.filter(|p| p.config().executor != ExecutorBackend::Disabled) {
                Some(provider) => {
                    let (port, owner) = StepFunctionsLambdaBridge::new(
                        provider,
                        scope.clone(),
                        BridgeLimits::new(
                            budgets.workflow_lambda_accepted_calls,
                            Duration::from_secs(budgets.workflow_lambda_cleanup_seconds),
                        )?,
                    )?;
                    (Some(port), Some(owner))
                }
                None => (None, None),
            };
        #[cfg(feature = "lambda")]
        let task_port = port
            .as_ref()
            .map(|port| port.clone() as Arc<dyn LambdaTaskPort>);
        #[cfg(not(feature = "lambda"))]
        let task_port = None;
        let tasks = if task_port.is_some() {
            TaskSupport::Lambda
        } else {
            TaskSupport::Disabled
        };
        let runtime = ExecutionSupervisor::new_with_frames(
            RuntimeLimits::new(
                budgets.workflow_open_executions,
                budgets.workflow_dispatches,
                budgets.workflow_evaluations,
            )?,
            HistoryLimits::new(25000, budgets.workflow_history_bytes)?,
            budgets.workflow_total_history_bytes,
            task_port,
            Arc::new(FrameCapacity::new(
                budgets.workflow_frames,
                budgets.workflow_result_bytes,
            )?),
            FrameLimits::new(
                budgets.workflow_frames_per_execution,
                budgets.workflow_result_bytes_per_execution,
            )?,
        )?;
        let (registry, owner) = Registry::new_with_state_support(
            scope.clone(),
            RegistryLimits::new(
                RegistryCapacities::new(
                    budgets.workflow_registry_machines,
                    budgets.workflow_registry_versions,
                    budgets.workflow_registry_records,
                    budgets.workflow_name_tombstones,
                )?,
                Duration::from_secs(budgets.workflow_retention_seconds),
                budgets.workflow_registry_commands,
            )?,
            runtime,
            ExecutionLimits::new(
                Duration::from_secs(budgets.workflow_max_duration_seconds),
                budgets.workflow_transitions,
            )?,
            StateSupport::Structured,
        )?;
        let (api, api_owner) = StepFunctionsApi::new(
            scope,
            tasks,
            registry.clone(),
            ApiLimits::new(
                budgets.workflow_api_accepted,
                budgets.workflow_api_workers,
                budgets.workflow_pagination_tokens,
                budgets.workflow_pagination_bytes,
            )?,
        )?;
        let status = WorkflowStatus {
            api: api.clone(),
            registry,
            budgets,
            #[cfg(feature = "lambda")]
            port,
        };
        Ok(Self {
            api,
            status: status.clone(),
            owner: WorkflowSupervisor {
                api: api_owner,
                registry: owner,
                status,
                #[cfg(feature = "lambda")]
                bridge,
                phase: Phase::Accepting,
                failure: None,
            },
        })
    }
}
impl WorkflowSupervisor {
    /// Resume the same controllers; a dropped/expired observer never restarts grace.
    pub(crate) async fn quiesce(&mut self, deadline: Instant) -> Result<()> {
        if matches!(self.phase, Phase::Accepting) {
            let cutoff =
                Instant::now() + Duration::from_secs(self.status.budgets.workflow_drain_seconds);
            self.api.close();
            self.phase = Phase::Api(cutoff);
        }
        loop {
            match self.phase {
                Phase::Accepting => anyhow::bail!("workflow shutdown phase invariant failed"),
                Phase::Api(_) | Phase::ClosingApi => {
                    let cutoff = if let Phase::Api(cutoff) = self.phase {
                        Some(cutoff)
                    } else {
                        None
                    };
                    match self.api.shutdown(deadline).await {
                        Ok(()) => {}
                        Err(ApiError::Deadline) => {
                            anyhow::bail!("workflow API shutdown observation expired")
                        }
                        Err(error) => self.fail(error.into()),
                    }
                    self.phase = cutoff.map_or(Phase::Registry, Phase::Grace);
                }
                Phase::Grace(cutoff) => {
                    match self.status.registry.wait_idle(cutoff.min(deadline)).await {
                        Err(RegistryError::Deadline) if Instant::now() < cutoff => {
                            anyhow::bail!("workflow grace observation expired")
                        }
                        Ok(()) | Err(RegistryError::Deadline) => {}
                        Err(error) => self.fail(error.into()),
                    }
                    self.phase = Phase::Registry;
                }
                Phase::Registry => {
                    match self.registry.shutdown(deadline).await {
                        Ok(()) => {}
                        Err(RegistryError::Deadline) => {
                            anyhow::bail!("workflow registry shutdown observation expired")
                        }
                        Err(error) => self.fail(error.into()),
                    }
                    self.phase = Phase::Bridge;
                }
                Phase::Bridge => {
                    #[cfg(feature = "lambda")]
                    if let Some(bridge) = &mut self.bridge {
                        if let Err(error) = bridge.shutdown(deadline).await {
                            if matches!(*error, BridgeError::Deadline) {
                                anyhow::bail!("workflow bridge shutdown observation expired");
                            }
                            self.fail(anyhow::anyhow!(error));
                        }
                    }
                    self.phase = Phase::Done;
                }
                Phase::Done => {
                    return match &self.failure {
                        Some(error) => Err(error.clone().into()),
                        None => Ok(()),
                    };
                }
            }
        }
    }
    /// Final cleanup skips grace and signals every owner on observer expiry.
    pub(crate) async fn shutdown(&mut self, deadline: Instant) -> Result<()> {
        self.api.close();
        self.phase = match self.phase {
            Phase::Accepting | Phase::Api(_) => Phase::ClosingApi,
            Phase::Grace(_) => Phase::Registry,
            phase => phase,
        };
        let result = self.quiesce(deadline).await;
        if result.is_err() {
            // No successful consistency cut is claimed; cancel retained owners
            // before main attempts provider destruction under the same deadline.
            self.registry.close();
            #[cfg(feature = "lambda")]
            if let Some(bridge) = &self.bridge {
                bridge.close();
            }
        }
        result
    }
    fn fail(&mut self, error: anyhow::Error) {
        if self.failure.is_none() {
            self.failure = Some(WorkflowFailure(Arc::new(error)));
        }
    }
}

#[cfg(test)]
mod tests;
