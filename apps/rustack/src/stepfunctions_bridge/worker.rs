//! Retained start/result/cleanup sequence for exactly one actual provider call.

use std::{future::Future, pin::pin, sync::Arc};

use rustack_lambda_core::{
    executor::{ExecutorError, InvokeResponse},
    invocation::InvocationHandle,
};
use rustack_stepfunctions_core::{
    CallbackError, CallbackInvocationMetadata, CallbackInvocationReporter, LambdaTaskCommand,
    TaskFailure, TaskResponder,
};
use tokio::{sync::watch, task::JoinSet, time::Instant};

use super::{
    BridgeError, Shared, Uncertain, WorkerExit,
    mapping::{self, CallPlan, Mapped, ResponseMode},
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
    mut command: LambdaTaskCommand,
    mut responder: TaskResponder,
    mut manager: watch::Receiver<bool>,
) -> WorkerExit {
    let mut core = responder.cancellation();
    let (relay, cancellation) = watch::channel(signal(&core) || signal(&manager));
    let plan = CallPlan::compile(&command);
    let mut reporter = match command.take_callback_reporter() {
        Ok(reporter) => reporter,
        Err(error) => {
            responder.finish(Err(TaskFailure::service_exception()));
            return WorkerExit {
                uncertain: None,
                diagnostic: Some(Arc::new(BridgeError::Callback(Arc::new(error)))),
            };
        }
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            let mut mapped = checked(mapping::parameters(error));
            report(&mut reporter, &mut mapped, None, Instant::now());
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
            let mut mapped = checked(mapping::service(error));
            report(&mut reporter, &mut mapped, None, Instant::now());
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
    let (mut mapped, metadata, observed) = match admitted {
        Ok(()) => {
            if responder.started().is_err() {
                relay.send_replace(true);
            }
            match observe(handle.wait_response(), &mut core, &mut manager, &relay).await {
                Ok(response) => {
                    let observed = Instant::now();
                    let request_id = handle.id();
                    let (mapped, metadata) = map_response(
                        CallbackInvocationMetadata::new(request_id, &response.executed_version),
                        response,
                        plan.mode,
                        request_id.to_string(),
                        &mut core,
                        &mut manager,
                        &relay,
                    )
                    .await;
                    (mapped, metadata, observed)
                }
                Err(error) => (checked(mapping::executor(error)), None, Instant::now()),
            }
        }
        Err(error) => (checked(mapping::service(error)), None, Instant::now()),
    };
    report(&mut reporter, &mut mapped, metadata, observed);
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

async fn map_response(
    metadata: Result<CallbackInvocationMetadata, CallbackError>,
    response: InvokeResponse,
    mode: ResponseMode,
    request_id: String,
    core: &mut watch::Receiver<bool>,
    manager: &mut watch::Receiver<bool>,
    relay: &watch::Sender<bool>,
) -> (Mapped, Option<CallbackInvocationMetadata>) {
    let mut workers = JoinSet::new();
    workers.spawn_blocking(move || match mode {
        ResponseMode::ObservedCallback => mapping::observed_response(&response, metadata),
        ResponseMode::Direct | ResponseMode::Optimized => mapping::response(
            &response,
            matches!(mode, ResponseMode::Optimized),
            &request_id,
        )
        .map(|mapped| (mapped, None)),
    });
    match observe(workers.join_next(), core, manager, relay).await {
        Some(Ok(Ok(mapped))) => mapped,
        Some(Ok(Err(error))) => (checked(Err(error)), None),
        Some(Err(error)) => (checked(Err(BridgeError::Worker(error))), None),
        None => (checked(Err(BridgeError::Unavailable)), None),
    }
}

pub(super) fn report(
    reporter: &mut Option<CallbackInvocationReporter>,
    mapped: &mut Mapped,
    metadata: Option<CallbackInvocationMetadata>,
    observed: Instant,
) {
    let Some(reporter) = reporter.as_mut() else {
        return;
    };
    let result = match &mapped.result {
        Err(failure) => Err(failure.clone()),
        Ok(_) => metadata.ok_or_else(TaskFailure::service_exception),
    };
    if let Err(error) = reporter.complete_at(result, observed) {
        mapped.result = Err(TaskFailure::service_exception());
        mapped.diagnostic = Some(BridgeError::Callback(Arc::new(error)));
    }
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

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;

    #[tokio::test]
    async fn test_should_relay_caller_and_manager_cancellation_without_dropping_mapping_join() {
        for manager_cancel in [false, true] {
            let (caller, mut core) = watch::channel(false);
            let (stop, mut manager) = watch::channel(false);
            let (relay, mut relayed) = watch::channel(false);
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, held) = mpsc::sync_channel(1);
            let mut mapping = JoinSet::new();
            mapping.spawn_blocking(move || {
                let _ = entered.send(());
                held.recv_timeout(Duration::from_secs(5)).unwrap();
                42
            });
            let mut owners = JoinSet::new();
            owners.spawn(async move {
                observe(mapping.join_next(), &mut core, &mut manager, &relay).await
            });
            tokio::time::timeout(Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap();
            if manager_cancel {
                stop.send_replace(true);
            } else {
                caller.send_replace(true);
            }
            tokio::time::timeout(Duration::from_secs(5), relayed.wait_for(|v| *v))
                .await
                .unwrap()
                .unwrap();
            assert!(
                owners.try_join_next().is_none(),
                "mapping ownership ended before its CPU job"
            );
            release.send(()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), owners.join_next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .unwrap(),
                42
            );
        }
    }
}
