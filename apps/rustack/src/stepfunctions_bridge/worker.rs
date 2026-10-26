//! Retained start/result/cleanup sequence for exactly one actual provider call.

use std::{future::Future, pin::pin, sync::Arc};

use rustack_lambda_core::{executor::ExecutorError, invocation::InvocationHandle};
use rustack_stepfunctions_core::{LambdaTaskCommand, TaskFailure, TaskResponder};
use tokio::{sync::watch, task::JoinSet, time::Instant};

use super::{
    BridgeError, Shared, Uncertain, WorkerExit,
    mapping::{self, CallPlan, Mapped},
    signal,
};

async fn observe<F: Future>(
    future: F,
    core: &mut watch::Receiver<bool>,
    manager: &mut watch::Receiver<bool>,
    relay: &watch::Sender<bool>,
) -> F::Output {
    let mut future = pin!(future);
    loop {
        let core_stopped = signal(core);
        let manager_stopped = signal(manager);
        if core_stopped || manager_stopped {
            relay.send_replace(true);
        }
        tokio::select! {
            biased;
            _=core.changed(),if !core_stopped=>{},
            _=manager.changed(),if !manager_stopped=>{},
            result=&mut future=>return result,
        }
    }
}
fn checked(mapped: Result<Mapped, BridgeError>) -> Mapped {
    mapped.unwrap_or_else(|error| Mapped {
        result: Err(TaskFailure::service_exception()),
        diagnostic: Some(error),
    })
}
#[tracing::instrument(name = "stepfunctions.lambda", skip_all)]
pub(super) async fn invoke(
    shared: Arc<Shared>,
    command: LambdaTaskCommand,
    mut responder: TaskResponder,
    mut manager: watch::Receiver<bool>,
) -> WorkerExit {
    let mut core = responder.cancellation();
    let (relay, cancellation) = watch::channel(signal(&core) || signal(&manager));
    let plan = match CallPlan::compile(&command) {
        Ok(plan) => plan,
        Err(error) => {
            let mapped = checked(mapping::parameters(error));
            responder.finish(mapped.result);
            return WorkerExit {
                uncertain: None,
                diagnostic: mapped.diagnostic.map(Arc::new),
            };
        }
    };
    let start = shared.context.provider.start_invocation_owned(
        &plan.function,
        plan.qualifier.as_deref(),
        plan.payload.original_bytes(),
        Some(plan.deadline),
        cancellation,
    );
    let submission = observe(start, &mut core, &mut manager, &relay).await;
    let mut submission = match submission {
        Ok(submission) => submission,
        Err(error) => {
            let mapped = checked(mapping::service(error));
            responder.finish(mapped.result);
            return WorkerExit {
                uncertain: None,
                diagnostic: mapped.diagnostic.map(Arc::new),
            };
        }
    };
    let admitted = observe(submission.admitted(), &mut core, &mut manager, &relay).await;
    let failed_admission = admitted.is_err();
    let mut handle = submission.into_handle();
    if failed_admission {
        handle.cancel();
    }
    let mapped = match admitted {
        Ok(()) => {
            if responder.started().is_err() {
                relay.send_replace(true);
            }
            match observe(handle.wait_response(), &mut core, &mut manager, &relay).await {
                Ok(response) => {
                    let request_id = handle.id().to_string();
                    let optimized = plan.optimized;
                    let mut workers = JoinSet::new();
                    workers.spawn_blocking(move || {
                        mapping::response(&response, optimized, &request_id)
                    });
                    match workers.join_next().await {
                        Some(Ok(mapped)) => checked(mapped),
                        Some(Err(error)) => Mapped {
                            result: Err(TaskFailure::service_exception()),
                            diagnostic: Some(BridgeError::Worker(error)),
                        },
                        None => Mapped {
                            result: Err(TaskFailure::service_exception()),
                            diagnostic: Some(BridgeError::Unavailable),
                        },
                    }
                }
                Err(error) => checked(mapping::executor(error)),
            }
        }
        Err(error) => checked(mapping::service(error)),
    };
    let exit = cleanup(
        &shared,
        handle,
        responder,
        mapped,
        &mut core,
        &mut manager,
        &relay,
    )
    .await;
    // Keep the external cancellation relay alive through the owned cleanup join.
    drop(relay);
    exit
}

async fn cleanup(
    shared: &Shared,
    mut handle: InvocationHandle,
    responder: TaskResponder,
    mapped: Mapped,
    core: &mut watch::Receiver<bool>,
    manager: &mut watch::Receiver<bool>,
    relay: &watch::Sender<bool>,
) -> WorkerExit {
    if signal(core) || signal(manager) {
        relay.send_replace(true);
        handle.cancel();
    }
    let cleanup = match Instant::now().checked_add(shared.context.limits.cleanup) {
        Some(deadline) => observe(handle.wait_settlement(deadline), core, manager, relay).await,
        None => Err(ExecutorError::CleanupFailed(
            "Invalid cleanup deadline".into(),
        )),
    };
    match cleanup {
        Ok(()) => {
            responder.finish(mapped.result);
            WorkerExit {
                uncertain: None,
                diagnostic: mapped.diagnostic.map(Arc::new),
            }
        }
        Err(error) => {
            handle.cancel();
            let failure = mapped
                .result
                .err()
                .unwrap_or_else(TaskFailure::service_exception);
            let receipt = responder.quarantine(failure);
            let diagnostic = Arc::new(BridgeError::Executor(Arc::new(error)));
            shared.diagnostic.send_replace(Some(diagnostic.clone()));
            match observe(handle.wait_cleanup(), core, manager, relay).await {
                Ok(()) => {
                    receipt.settle();
                    WorkerExit {
                        uncertain: None,
                        diagnostic: Some(diagnostic),
                    }
                }
                Err(error) => WorkerExit {
                    uncertain: Some(Uncertain {
                        _handle: handle,
                        _receipt: receipt,
                    }),
                    diagnostic: Some(Arc::new(BridgeError::Executor(Arc::new(error)))),
                },
            }
        }
    }
}
