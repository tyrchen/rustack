//! Lazy immutable structured effects; the resource owner schedules and joins them.

use std::{fmt, num::NonZeroUsize, sync::Arc, time::Instant};

use serde_json::{Value, json};
use thiserror::Error;

use super::{
    AttemptId, DriverStep, ExecutionDriver, ExecutionFailure, ExecutionSource, Phase,
    types::FailureOrigin,
};
use crate::{
    AslError, CompiledMachine, EvaluationContext, MapConcurrency, Payload, StateId, StateView,
    TaskFailure, UuidSource, payload::encode,
};

const MAX_ITEMS: usize = 10_000;
const INLINE_LIMIT: u8 = 40;

#[derive(Debug, Clone)]
enum Inputs {
    Parallel,
    Map(Payload),
}

/// Captured round input, context and lazy indexed branch descriptors.
///
/// This value does not own asynchronous calls or certify cleanup. The resource
/// owner must enforce frame/transition/result-retention budgets independently.
#[derive(Clone)]
#[non_exhaustive]
pub struct FanoutPlan {
    machine: Arc<CompiledMachine>,
    state: StateId,
    attempt: AttemptId,
    effective: Payload,
    context: EvaluationContext,
    base: EvaluationContext,
    inputs: Inputs,
    count: usize,
    concurrency: NonZeroUsize,
    deadline: Instant,
}
impl fmt::Debug for FanoutPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FanoutPlan(<redacted>)")
    }
}

/// One lazy child's immutable machine, raw input and inherited base context.
#[derive(Clone)]
#[non_exhaustive]
pub struct FanoutBranch {
    machine: Arc<CompiledMachine>,
    input: Payload,
    context: EvaluationContext,
}
impl fmt::Debug for FanoutBranch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FanoutBranch(<redacted>)")
    }
}
impl FanoutBranch {
    /// Read this child's lexical machine; it is not a registrable root revision.
    ///
    /// ```
    /// # fn inspect(branch: &rustack_stepfunctions_core::FanoutBranch) { let machine = branch.machine(); }
    /// ```
    #[must_use]
    pub fn machine(&self) -> &Arc<CompiledMachine> {
        &self.machine
    }

    /// Read the exact raw input selected for this branch or iteration.
    ///
    /// ```
    /// # fn inspect(branch: &rustack_stepfunctions_core::FanoutBranch) { let input = branch.input(); }
    /// ```
    #[must_use]
    pub fn input(&self) -> &Payload {
        &self.input
    }

    /// Read inherited execution metadata, with no implicit Map.Item context.
    ///
    /// ```
    /// # fn inspect(branch: &rustack_stepfunctions_core::FanoutBranch) { let context = branch.context(); }
    /// ```
    #[must_use]
    pub fn context(&self) -> &EvaluationContext {
        &self.context
    }
}

/// A classified child terminal failure retaining its original typed source.
#[derive(Error)]
#[error("child workflow evaluation failed")]
#[non_exhaustive]
pub struct PropagatedFailure {
    pub(super) failure: ExecutionFailure,
    #[source]
    pub(super) source: Arc<AslError>,
}
impl fmt::Debug for PropagatedFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PropagatedFailure(<redacted>)")
    }
}
impl PropagatedFailure {
    /// Read the child's original bounded terminal classification and fields.
    ///
    /// ```
    /// # fn inspect(failure:&rustack_stepfunctions_core::PropagatedFailure) { let details=failure.details(); }
    /// ```
    #[must_use]
    pub fn details(&self) -> &ExecutionFailure {
        &self.failure
    }
}

/// A failed round, preserving typed interpreter errors separately from branch failures.
#[derive(Error)]
#[non_exhaustive]
pub enum FanoutFailure {
    /// An uncaught branch failure after the owner joins cancelled siblings.
    #[error("structured branch failed")]
    Branch(ExecutionFailure),
    /// A selector or aggregate failure after the same cleanup boundary.
    #[error("structured evaluation failed")]
    Evaluation(#[source] Arc<AslError>),
    /// A child interpreter failure whose classification must not be recomputed.
    #[error("structured child evaluation failed")]
    Propagated(#[source] PropagatedFailure),
    /// A local safety failure that bypasses business recovery.
    #[error("structured safety limit exceeded")]
    Unrecoverable(ExecutionFailure),
}
impl fmt::Debug for FanoutFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FanoutFailure(<redacted>)")
    }
}
impl From<AslError> for FanoutFailure {
    fn from(error: AslError) -> Self {
        Self::Evaluation(Arc::new(error))
    }
}
impl From<TaskFailure> for FanoutFailure {
    fn from(failure: TaskFailure) -> Self {
        failure.0.into()
    }
}
impl From<ExecutionFailure> for FanoutFailure {
    fn from(failure: ExecutionFailure) -> Self {
        match &failure.origin {
            FailureOrigin::Business => Self::Branch(failure),
            FailureOrigin::Evaluation(source) => {
                let source = source.clone();
                Self::Propagated(PropagatedFailure { failure, source })
            }
            FailureOrigin::Safety => Self::Unrecoverable(failure),
        }
    }
}

impl FanoutPlan {
    /// Read the round generation required by `complete_fanout`.
    ///
    /// ```
    /// # fn inspect(plan: &rustack_stepfunctions_core::FanoutPlan) { let attempt = plan.attempt(); }
    /// ```
    #[must_use]
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }

    /// Read the bounded total number of indexed children in this round.
    ///
    /// ```
    /// # fn inspect(plan: &rustack_stepfunctions_core::FanoutPlan) { let count = plan.item_count(); }
    /// ```
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.count
    }

    /// Read the normalized concurrency ceiling, before shared frame/task limits.
    ///
    /// ```
    /// # fn inspect(plan: &rustack_stepfunctions_core::FanoutPlan) { let limit = plan.max_concurrency(); }
    /// ```
    #[must_use]
    pub fn max_concurrency(&self) -> NonZeroUsize {
        self.concurrency
    }

    /// Read the original absolute execution deadline; children must inherit it.
    ///
    /// ```
    /// # fn inspect(plan: &rustack_stepfunctions_core::FanoutPlan) { let deadline = plan.deadline(); }
    /// ```
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Prepare exactly one child by definition/item index, with lazy UUID sourcing.
    ///
    /// Async owners run this potentially entropy-bearing evaluation in a bounded
    /// retained worker. Child execution must share the root deadline and budget.
    ///
    /// # Errors
    /// Rejects invalid indices, selection/context failures and payload limits.
    ///
    /// ```
    /// # fn child(plan: &rustack_stepfunctions_core::FanoutPlan) {
    /// let child = plan.branch(0, &mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// # }
    /// ```
    pub fn branch<S: UuidSource + ?Sized>(
        &self,
        index: usize,
        source: &mut S,
    ) -> Result<FanoutBranch, AslError> {
        if index >= self.count {
            return Err(AslError::InvalidFanoutResult);
        }
        let (machine, input) = match (self.machine.state(self.state)?, &self.inputs) {
            (StateView::Parallel { branches, .. }, Inputs::Parallel) => (
                branches.get(index).ok_or(AslError::InvalidStateId)?.clone(),
                self.effective.clone(),
            ),
            (
                StateView::Map {
                    processor,
                    selector,
                    ..
                },
                Inputs::Map(items),
            ) => {
                let item = items
                    .value()
                    .as_array()
                    .and_then(|values| values.get(index))
                    .ok_or(AslError::InvalidStateId)?;
                let input = if let Some(selector) = selector {
                    let mut context = self.context.value().clone();
                    context
                        .as_object_mut()
                        .ok_or(AslError::InvalidExecution)?
                        .insert(
                            "Map".into(),
                            json!({"Item":{"Index":index,"Value":item,"Source":"STATE_DATA"}}),
                        );
                    selector.evaluate_with_source(
                        &self.effective,
                        &EvaluationContext::from_value(&context)?,
                        source,
                    )?
                } else {
                    Payload::from_value(item)?
                };
                (processor.clone(), input)
            }
            _ => return Err(AslError::InvalidStateId),
        };
        Ok(FanoutBranch {
            machine,
            input,
            context: self.base.clone(),
        })
    }

    /// Encode exactly one output per child, ordering independently of completion.
    ///
    /// Input outputs must already have been charged to the owner's shared retained
    /// result budget. The bounded writer rejects oversized aggregate materialization.
    ///
    /// # Errors
    /// Rejects missing/duplicate/out-of-range indices and aggregate payload limits.
    ///
    /// ```
    /// # fn collect(plan: &rustack_stepfunctions_core::FanoutPlan, outputs: &[(usize, rustack_stepfunctions_core::Payload)]) {
    /// let result = plan.collect(outputs);
    /// # }
    /// ```
    pub fn collect(&self, outputs: &[(usize, Payload)]) -> Result<Payload, AslError> {
        if outputs.len() != self.count {
            return Err(AslError::InvalidFanoutResult);
        }
        let mut ordered = vec![None; self.count];
        for (index, output) in outputs {
            let slot = ordered
                .get_mut(*index)
                .ok_or(AslError::InvalidFanoutResult)?;
            if slot.is_some() {
                return Err(AslError::InvalidFanoutResult);
            }
            *slot = Some(output.value());
        }
        let values = ordered
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(AslError::InvalidFanoutResult)?;
        Payload::from_bytes(encode(&values)?)
    }
}

impl ExecutionDriver {
    pub(super) fn fanout<S: ExecutionSource + ?Sized>(
        &mut self,
        source: &mut S,
    ) -> Result<DriverStep, AslError> {
        let machine = self.machine.clone();
        let (effective, inputs, count, concurrency) = match machine.state(self.state)? {
            StateView::Parallel { flow, branches, .. } => (
                flow.prepare_with_source(&self.input, &self.context, source)?,
                Inputs::Parallel,
                branches.len(),
                branches.len(),
            ),
            StateView::Map {
                flow,
                items,
                concurrency,
                ..
            } => {
                let effective = flow.prepare_with_source(&self.input, &self.context, source)?;
                let selected = items
                    .lookup(&effective, &self.context)
                    .ok_or(AslError::Missing)?;
                let count = selected
                    .as_array()
                    .filter(|items| items.len() <= MAX_ITEMS)
                    .ok_or(AslError::InvalidMap)?
                    .len();
                let items = Payload::from_value(selected)?;
                let concurrency = match concurrency {
                    MapConcurrency::Limit(limit) => usize::from(limit.get()),
                    MapConcurrency::Path(path) => {
                        let number = path
                            .lookup(&effective, &self.context)
                            .and_then(Value::as_u64)
                            .filter(|n| *n <= u64::from(INLINE_LIMIT))
                            .ok_or(AslError::InvalidMap)?;
                        if number == 0 {
                            usize::from(INLINE_LIMIT)
                        } else {
                            usize::try_from(number).map_err(|_| AslError::InvalidMap)?
                        }
                    }
                };
                (effective, Inputs::Map(items), count, concurrency)
            }
            _ => return Err(AslError::InvalidStateId),
        };
        self.fanout_started = Some(count);
        if count == 0 {
            return self.finish_result(Payload::parse(b"[]"), source);
        }
        let base = if self.base.value().get("Map").is_some() {
            let mut base = self.base.value().clone();
            base.as_object_mut()
                .ok_or(AslError::InvalidExecution)?
                .remove("Map");
            EvaluationContext::from_value(&base)?
        } else {
            self.base.clone()
        };
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(AslError::InvalidExecution)?;
        let attempt = AttemptId(self.generation);
        let concurrency = NonZeroUsize::new(concurrency).ok_or(AslError::InvalidMap)?;
        let plan = FanoutPlan {
            machine,
            state: self.state,
            attempt,
            effective,
            context: self.context.clone(),
            base,
            inputs,
            count,
            concurrency,
            deadline: self.deadline,
        };
        self.phase = Phase::Joining { attempt, count };
        Ok(DriverStep::FanOut(plan))
    }
}

#[cfg(test)]
mod tests;
