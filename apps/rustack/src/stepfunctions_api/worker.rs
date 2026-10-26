//! Retained CPU preparation, registry commands and immutable response projection.

use std::{future::Future, sync::Arc};

use rustack_stepfunctions_core::{AslCompiler, ExecutionError, RegistryCommand, RegistryResult};
use rustack_stepfunctions_http::{Request, Response};
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode as Code, StepFunctionsOperation as Op,
};
use tokio::{
    sync::oneshot,
    task::JoinSet,
    time::{Instant, timeout_at},
};

use super::{
    Context,
    error::{check, error, registry, source},
    pages,
    prepare::{self, Prepared},
    render,
};

pub(super) async fn run(
    context: Arc<Context>,
    request: Request,
    deadline: Instant,
) -> Result<Response, StepFunctionsError> {
    let scope = context.scope.clone();
    let tasks = context.tasks;
    let states = context.registry.state_support();
    let prepared = cpu(&context, deadline, move || {
        prepare::prepare(
            request,
            &AslCompiler::new(scope.clone(), tasks).with_state_support(states),
            &scope,
        )
    })
    .await?;
    check(deadline)?;
    match prepared {
        Prepared::Ready(response) => Ok(response),
        Prepared::Command(command, op) => {
            let mut result = submit_registry(&context, command, op, deadline).await?;
            if op == Op::StopExecution {
                let RegistryResult::Execution(snapshot) = result else {
                    return Err(error(Code::InternalFailure));
                };
                result = RegistryResult::Execution(snapshot.wait(deadline).await.map_err(|e| {
                    source(
                        if matches!(e, ExecutionError::Deadline) {
                            Code::RequestTimeoutException
                        } else {
                            Code::InternalFailure
                        },
                        e,
                    )
                })?);
            }
            cpu(&context, deadline, move || render::render(op, result)).await
        }
        Prepared::Page(binding, token, initial) => {
            let cursor = if let Some(token) = token {
                let (reply, receiver) = oneshot::channel();
                page_command(
                    &context,
                    deadline,
                    pages::Request::Read(token, binding, reply),
                    receiver,
                )
                .await?
            } else {
                let result = submit_registry(&context, initial, binding.op, deadline).await?;
                let captured = binding.clone();
                let data = cpu(&context, deadline, move || {
                    pages::Data::capture(&captured, result)
                })
                .await?;
                let (reply, receiver) = oneshot::channel();
                let snapshot = page_command(
                    &context,
                    deadline,
                    pages::Request::Reserve(data, reply),
                    receiver,
                )
                .await?;
                pages::first(binding, snapshot)
            };
            let mut page = cpu(&context, deadline, move || pages::page(cursor)).await?;
            if let Some(next) = page.next {
                let (reply, receiver) = oneshot::channel();
                page_command(
                    &context,
                    deadline,
                    pages::Request::Save(page.token.clone(), next, reply),
                    receiver,
                )
                .await?;
                pages::attach(&mut page.response, page.token)?;
            }
            Ok(page.response)
        }
    }
}
async fn submit_registry(
    context: &Context,
    command: RegistryCommand,
    op: Op,
    deadline: Instant,
) -> Result<RegistryResult, StepFunctionsError> {
    check(deadline)?;
    let mut receipt = context
        .registry
        .submit(command)
        .map_err(|e| registry(e, op))?;
    receipt.wait(deadline).await.map_err(|e| registry(e, op))
}
async fn page_command<T>(
    context: &Context,
    deadline: Instant,
    request: pages::Request,
    receiver: oneshot::Receiver<Result<T, StepFunctionsError>>,
) -> Result<T, StepFunctionsError> {
    let operation = async {
        context
            .pages
            .send(pages::Command { deadline, request })
            .await
            .map_err(|_| error(Code::ServiceUnavailable))?;
        receiver
            .await
            .map_err(|_| error(Code::ServiceUnavailable))?
    };
    observe(deadline, operation).await
}
async fn observe<T>(
    deadline: Instant,
    future: impl Future<Output = Result<T, StepFunctionsError>>,
) -> Result<T, StepFunctionsError> {
    timeout_at(deadline, future)
        .await
        .map_err(|_| error(Code::RequestTimeoutException))?
}
pub(super) async fn cpu<T: Send + 'static>(
    context: &Context,
    deadline: Instant,
    work: impl FnOnce() -> Result<T, StepFunctionsError> + Send + 'static,
) -> Result<T, StepFunctionsError> {
    check(deadline)?;
    let permit = timeout_at(deadline, context.cpu.clone().acquire_owned())
        .await
        .map_err(|_| error(Code::RequestTimeoutException))?
        .map_err(|_| error(Code::ServiceUnavailable))?;
    check(deadline)?;
    let mut jobs = JoinSet::new();
    jobs.spawn_blocking(move || (work(), permit));
    match jobs.join_next().await {
        Some(Ok((result, permit))) => {
            drop(permit);
            check(deadline)?;
            result
        }
        Some(Err(e)) => Err(source(Code::InternalFailure, e)),
        None => Err(error(Code::InternalFailure)),
    }
}
