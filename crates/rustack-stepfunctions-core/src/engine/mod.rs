//! Deterministic frame transitions with explicit external effects.
//!
//! No resource is spawned or dropped here. The owning actor must retain accepted
//! invocations, process cancellation obligations and journal AWS history before
//! publishing read views. This driver alone does not install an execution API.

use std::{
    fmt, mem,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, Utc};
use rustack_core::json::JsonError;
use serde_json::{Value, json};
use types::FailureOrigin;

use crate::{
    AslError, CompiledMachine, EvaluationContext, Payload, StateId, StateView, Transition,
};

mod fanout;
mod history;
mod recovery;
mod runtime;
mod scheduler;
mod states;
#[cfg(test)]
mod tests;
mod types;

pub use fanout::{FanoutBranch, FanoutFailure, FanoutPlan, PropagatedFailure};
pub use scheduler::{
    FrameCallId, FrameCapacity, FrameError, FrameId, FrameLease, FrameLimits, FrameParent,
    FrameScheduler, FrameTransition, FrameUpdate,
};
pub use types::{
    AttemptId, ClockSample, DriverStep, DriverUpdate, ExecutionFailure, ExecutionLimits,
    ExecutionOutcome, ExecutionSource, TaskDispatch, TaskFailure,
};

#[derive(Debug, Clone)]
enum Phase {
    Ready {
        entry: bool,
    },
    Calling {
        attempt: AttemptId,
        deadline: Instant,
    },
    Joining {
        attempt: AttemptId,
        count: usize,
    },
    Waiting {
        until: Instant,
        continuation: Continuation,
    },
    Terminal(Arc<ExecutionOutcome>),
}
#[derive(Debug, Clone)]
enum Continuation {
    Retry,
    Wait { output: Payload, next: Transition },
}

/// One captured lexical machine's frame interpreter, with no resource owner.
#[non_exhaustive]
#[derive(Clone)]
pub struct ExecutionDriver {
    machine: Arc<CompiledMachine>,
    started: bool,
    input: Payload,
    base: EvaluationContext,
    context: EvaluationContext,
    state: StateId,
    phase: Phase,
    deadline: Instant,
    observed: Instant,
    entered: DateTime<Utc>,
    generation: u64,
    retries: Vec<u32>,
    retry_count: u32,
    remaining: u32,
    diagnostic: Option<Arc<AslError>>,
    fanout_started: Option<usize>,
}
impl fmt::Debug for ExecutionDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionDriver")
            .field("phase", &self.phase)
            .field("retry_count", &self.retry_count)
            .finish_non_exhaustive()
    }
}
impl ExecutionDriver {
    /// Capture immutable machine/input/context and compute the absolute deadline.
    ///
    /// The base context must be an object. Registry-owned Execution and
    /// `StateMachine` fields remain unchanged; this driver owns its State fields.
    ///
    /// # Errors
    /// Rejects structured artifacts, nonobject context or an unrepresentable deadline.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{AslCompiler,Payload,EvaluationContext,ClockSample,ExecutionDriver,ExecutionLimits};
    /// use std::time::Instant;
    /// let artifact=AslCompiler::default().compile(br#"{"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}"#)?;
    /// let input=Payload::parse(b"{}")?;
    /// let context=EvaluationContext::from(&input);
    /// let now=ClockSample::new(Instant::now(),chrono::Utc::now());
    /// let driver=ExecutionDriver::new(artifact.machine().clone(),input,context,now,ExecutionLimits::default())?;
    /// # Ok::<(),Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(
        machine: Arc<CompiledMachine>,
        input: Payload,
        base: EvaluationContext,
        started: ClockSample,
        limits: ExecutionLimits,
    ) -> Result<Self, AslError> {
        if machine.requires_structured_execution() {
            return Err(AslError::StructuredExecutionRequired);
        }
        Self::for_structured_owner(machine, input, base, started, limits)
    }

    /// Build a pure frame interpreter for an owner implementing `FanOut` effects.
    ///
    /// This constructor performs no scheduling or IO. The current sequential
    /// supervisor rejects structured artifacts; enabling these effects alone
    /// cannot enable the service's structured execution profile.
    ///
    /// # Errors
    /// Rejects invalid context or an unrepresentable absolute deadline.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{AslCompiler, ClockSample, EvaluationContext, ExecutionDriver, ExecutionLimits, Payload};
    /// let compiled = AslCompiler::default().compile(br#"{"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}"#)?;
    /// let input = Payload::parse(b"{}")?;
    /// let driver = ExecutionDriver::for_structured_owner(compiled.machine().clone(), input.clone(), EvaluationContext::from(&input), ClockSample::new(std::time::Instant::now(), chrono::Utc::now()), ExecutionLimits::default())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn for_structured_owner(
        machine: Arc<CompiledMachine>,
        input: Payload,
        base: EvaluationContext,
        started: ClockSample,
        limits: ExecutionLimits,
    ) -> Result<Self, AslError> {
        if !base.value().is_object() {
            return Err(AslError::InvalidExecution);
        }
        let duration = machine.timeout_seconds().map_or(limits.duration, |s| {
            limits.duration.min(Duration::from_secs(u64::from(s.get())))
        });
        let deadline = started
            .monotonic
            .checked_add(duration)
            .ok_or(AslError::InvalidExecution)?;
        Self::frame(machine, input, base, started, deadline, limits.transitions)
    }
    fn frame(
        machine: Arc<CompiledMachine>,
        input: Payload,
        base: EvaluationContext,
        started: ClockSample,
        deadline: Instant,
        remaining: u32,
    ) -> Result<Self, AslError> {
        if !base.value().is_object() || started.monotonic >= deadline {
            return Err(AslError::InvalidExecution);
        }
        Ok(Self {
            state: machine.start(),
            started: false,
            machine,
            input,
            context: base.clone(),
            base,
            phase: Phase::Ready { entry: true },
            deadline,
            observed: started.monotonic,
            entered: started.wall,
            generation: 0,
            retries: Vec::new(),
            retry_count: 0,
            remaining,
            diagnostic: None,
            fanout_started: None,
        })
    }

    /// Drive at most one state/attempt transition, without reading clocks or IO.
    ///
    /// ```
    /// # fn tick(driver:&mut rustack_stepfunctions_core::ExecutionDriver, now:rustack_stepfunctions_core::ClockSample) {
    /// let update=driver.advance(now,&mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// let next=update.step();
    /// # }
    /// ```
    pub fn advance<S: ExecutionSource + ?Sized>(
        &mut self,
        now: ClockSample,
        source: &mut S,
    ) -> DriverUpdate {
        self.started = true;
        if let Some(update) = self.guard(now) {
            return update;
        }
        match mem::replace(&mut self.phase, Phase::Ready { entry: false }) {
            Phase::Ready { entry } => self.ready(entry, now, source),
            Phase::Calling { attempt, deadline } => {
                if now.monotonic >= deadline {
                    let step = self.recover(
                        ExecutionFailure::named("States.Timeout", "Task deadline exceeded"),
                        now,
                        source,
                    );
                    DriverUpdate {
                        step,
                        cancel: Some(attempt),
                    }
                } else {
                    self.phase = Phase::Calling { attempt, deadline };
                    update(DriverStep::Park(deadline))
                }
            }
            Phase::Joining { attempt, count } => {
                self.phase = Phase::Joining { attempt, count };
                update(DriverStep::Park(self.deadline))
            }
            Phase::Waiting {
                until,
                continuation,
            } => {
                if now.monotonic < until {
                    self.phase = Phase::Waiting {
                        until,
                        continuation,
                    };
                    update(DriverStep::Park(until.min(self.deadline)))
                } else {
                    match continuation {
                        Continuation::Retry => update(DriverStep::Continue),
                        Continuation::Wait { output, next } => update(self.transfer(output, next)),
                    }
                }
            }
            Phase::Terminal(outcome) => {
                self.phase = Phase::Terminal(outcome.clone());
                update(DriverStep::Settled(outcome))
            }
        }
    }

    /// Apply only the current attempt's result; stale/duplicate results are inert.
    ///
    /// At/after a deadline, timeout wins even when this result was ready earlier.
    /// A cancellation obligation is not proof that external cleanup has settled.
    ///
    /// ```
    /// # fn finish(driver:&mut rustack_stepfunctions_core::ExecutionDriver, call:&rustack_stepfunctions_core::TaskDispatch, now:rustack_stepfunctions_core::ClockSample, output:rustack_stepfunctions_core::Payload) {
    /// let update=driver.complete(call.attempt(),Ok(output),now,&mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// # }
    /// ```
    pub fn complete<S: ExecutionSource + ?Sized>(
        &mut self,
        attempt: AttemptId,
        result: Result<Payload, TaskFailure>,
        now: ClockSample,
        source: &mut S,
    ) -> DriverUpdate {
        self.started = true;
        if let Some(update) = self.guard(now) {
            return update;
        }
        let Phase::Calling {
            attempt: current,
            deadline,
        } = &self.phase
        else {
            return self.current_update();
        };
        if *current != attempt {
            return self.current_update();
        }
        if now.monotonic >= *deadline {
            return self.advance(now, source);
        }
        self.phase = Phase::Ready { entry: false };
        update(match result {
            Err(failure) => self.recover(failure.0, now, source),
            Ok(result) => match self.finish_result(Ok(result), source) {
                Ok(step) => step,
                Err(error) => self.evaluation_failure(error, now, source, true),
            },
        })
    }

    /// Join the current round only after its owner has joined cancelled siblings.
    ///
    /// Stale, duplicate and Lambda-attempt results cannot settle this round.
    /// Deadline/Stop always preserve the independent cancellation obligation.
    ///
    /// ```
    /// # fn join(driver: &mut rustack_stepfunctions_core::ExecutionDriver, plan: &rustack_stepfunctions_core::FanoutPlan, result: Result<rustack_stepfunctions_core::Payload, rustack_stepfunctions_core::FanoutFailure>, now: rustack_stepfunctions_core::ClockSample) {
    /// let update = driver.complete_fanout(plan.attempt(), result, now, &mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// # }
    /// ```
    pub fn complete_fanout<S: ExecutionSource + ?Sized>(
        &mut self,
        attempt: AttemptId,
        result: Result<Payload, FanoutFailure>,
        now: ClockSample,
        source: &mut S,
    ) -> DriverUpdate {
        self.started = true;
        if let Some(update) = self.guard(now) {
            return update;
        }
        let Phase::Joining {
            attempt: current,
            count,
        } = self.phase
        else {
            return self.current_update();
        };
        if current != attempt {
            return self.current_update();
        }
        self.phase = Phase::Ready { entry: false };
        let step = match result {
            Err(FanoutFailure::Branch(failure)) => match &failure.origin {
                FailureOrigin::Business => self.recover(failure, now, source),
                FailureOrigin::Evaluation(_) => self.propagate_failure(failure, now, source),
                FailureOrigin::Safety => self.settle(ExecutionOutcome::Failed(failure)),
            },
            Err(FanoutFailure::Propagated(failure)) => {
                self.propagate_failure(failure.failure, now, source)
            }
            Err(FanoutFailure::Evaluation(error)) => {
                self.evaluation_failure_shared(error, now, source, true)
            }
            Err(FanoutFailure::Unrecoverable(failure)) => {
                self.settle(ExecutionOutcome::Failed(failure))
            }
            Ok(result) => {
                let result = if result
                    .value()
                    .as_array()
                    .is_some_and(|values| values.len() == count)
                {
                    Ok(result)
                } else {
                    Err(AslError::InvalidFanoutResult)
                };
                match self.finish_result(result, source) {
                    Ok(step) => step,
                    Err(error) => self.evaluation_failure(error, now, source, true),
                }
            }
        };
        update(step)
    }

    fn finish_result<S: ExecutionSource + ?Sized>(
        &mut self,
        result: Result<Payload, AslError>,
        source: &mut S,
    ) -> Result<DriverStep, AslError> {
        let result = result?;
        let machine = self.machine.clone();
        let (StateView::Task { flow, next, .. }
        | StateView::Parallel { flow, next, .. }
        | StateView::Map { flow, next, .. }) = machine.state(self.state)?
        else {
            return Err(AslError::InvalidStateId);
        };
        let output = flow.finish_with_source(&self.input, &result, &self.context, source)?;
        Ok(self.transfer(output, next))
    }

    /// Abort once, bypassing user recovery; elapsed overall deadlines still win.
    ///
    /// ```
    /// # fn stop(driver:&mut rustack_stepfunctions_core::ExecutionDriver, now:rustack_stepfunctions_core::ClockSample) -> Result<(),rustack_stepfunctions_core::AslError> {
    /// let reason=rustack_stepfunctions_core::TaskFailure::new("Rustack.Shutdown",None)?;
    /// let update=driver.stop(reason,now);
    /// # Ok(()) }
    /// ```
    pub fn stop(&mut self, reason: TaskFailure, now: ClockSample) -> DriverUpdate {
        self.started = true;
        if let Some(update) = self.guard(now) {
            return update;
        }
        let cancel = self.active();
        let step = self.settle(ExecutionOutcome::Aborted(reason));
        DriverUpdate { step, cancel }
    }

    /// Read the captured machine; dispatched state IDs refer only to this artifact.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let machine=d.machine(); }
    /// ```
    #[must_use]
    pub fn machine(&self) -> &Arc<CompiledMachine> {
        &self.machine
    }
    /// Read the current state's original input, retained across all its retries.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let raw=d.input(); }
    /// ```
    #[must_use]
    pub fn input(&self) -> &Payload {
        &self.input
    }
    /// Read the current state index for observation/history projection.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let state=d.state(); }
    /// ```
    #[must_use]
    pub fn state(&self) -> StateId {
        self.state
    }
    /// Read the total retries scheduled in the current state visit.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let retries=d.retry_count(); }
    /// ```
    #[must_use]
    pub fn retry_count(&self) -> u32 {
        self.retry_count
    }
    /// Read the immutable execution deadline.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let deadline=d.deadline(); }
    /// ```
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    /// Read the most recent typed interpreter/source failure without public data.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let diagnostic=d.diagnostic(); }
    /// ```
    #[must_use]
    pub fn diagnostic(&self) -> Option<&AslError> {
        self.diagnostic.as_deref()
    }
    /// Read the stable terminal outcome, if any; cleanup is independently owned.
    ///
    /// ```
    /// # fn inspect(d:&rustack_stepfunctions_core::ExecutionDriver) { let outcome=d.outcome(); }
    /// ```
    #[must_use]
    pub fn outcome(&self) -> Option<&Arc<ExecutionOutcome>> {
        match &self.phase {
            Phase::Terminal(outcome) => Some(outcome),
            _ => None,
        }
    }

    fn guard(&mut self, now: ClockSample) -> Option<DriverUpdate> {
        if let Phase::Terminal(outcome) = &self.phase {
            return Some(update(DriverStep::Settled(outcome.clone())));
        }
        if now.monotonic < self.observed {
            let cancel = self.active();
            let error = Arc::new(AslError::InvalidExecution);
            self.diagnostic = Some(error.clone());
            let mut failure =
                ExecutionFailure::named("Rustack.InternalError", "Monotonic clock regressed");
            failure.origin = FailureOrigin::Evaluation(error);
            let step = self.settle(ExecutionOutcome::Failed(failure));
            return Some(DriverUpdate { step, cancel });
        }
        self.observed = now.monotonic;
        if now.monotonic >= self.deadline {
            let cancel = self.active();
            let step = self.settle(ExecutionOutcome::TimedOut);
            return Some(DriverUpdate { step, cancel });
        }
        None
    }
    fn current_update(&self) -> DriverUpdate {
        update(match &self.phase {
            Phase::Calling { deadline, .. } => DriverStep::Park(*deadline),
            Phase::Joining { .. } => DriverStep::Park(self.deadline),
            Phase::Waiting { until, .. } => DriverStep::Park((*until).min(self.deadline)),
            Phase::Ready { .. } => DriverStep::Continue,
            Phase::Terminal(outcome) => DriverStep::Settled(outcome.clone()),
        })
    }
    fn active(&self) -> Option<AttemptId> {
        match self.phase {
            Phase::Calling { attempt, .. } | Phase::Joining { attempt, .. } => Some(attempt),
            _ => None,
        }
    }
    fn settle(&mut self, outcome: ExecutionOutcome) -> DriverStep {
        let outcome = Arc::new(outcome);
        self.phase = Phase::Terminal(outcome.clone());
        DriverStep::Settled(outcome)
    }
    fn transfer(&mut self, output: Payload, next: Transition) -> DriverStep {
        match next {
            Transition::End => self.settle(ExecutionOutcome::Succeeded(output)),
            Transition::Next(state) => {
                self.input = output;
                self.state = state;
                self.phase = Phase::Ready { entry: true };
                DriverStep::Continue
            }
        }
    }
    fn refresh_context(&mut self) -> Result<(), AslError> {
        let mut value: Value = self.base.value().clone();
        let object = value.as_object_mut().ok_or(AslError::InvalidExecution)?;
        object.insert("State".into(),json!({"Name":self.machine.state_name(self.state)?,
            "EnteredTime":self.entered.to_rfc3339_opts(SecondsFormat::Nanos,true),"RetryCount":self.retry_count}));
        self.context = EvaluationContext::from_value(&value)?;
        Ok(())
    }
    fn ready<S: ExecutionSource + ?Sized>(
        &mut self,
        entry: bool,
        now: ClockSample,
        source: &mut S,
    ) -> DriverUpdate {
        if self.remaining == 0 {
            return update(self.transition_limit());
        }
        self.remaining = self.remaining.saturating_sub(1);
        if entry {
            self.entered = now.wall;
            self.retry_count = 0;
            self.retries.clear();
        }
        let step = match self
            .refresh_context()
            .and_then(|()| self.execute(now, source))
        {
            Ok(step) => step,
            Err(error) => {
                let task = matches!(
                    self.machine.state(self.state),
                    Ok(StateView::Task { .. } | StateView::Parallel { .. } | StateView::Map { .. })
                );
                self.evaluation_failure(error, now, source, task)
            }
        };
        update(step)
    }
    fn transition_limit(&mut self) -> DriverStep {
        self.settle(ExecutionOutcome::Failed(ExecutionFailure::safety(
            "Rustack.TransitionLimitExceeded",
            "Execution transition limit exceeded",
        )))
    }
    fn evaluation_failure<S: ExecutionSource + ?Sized>(
        &mut self,
        error: AslError,
        now: ClockSample,
        source: &mut S,
        recover: bool,
    ) -> DriverStep {
        self.evaluation_failure_shared(Arc::new(error), now, source, recover)
    }
    fn propagate_failure<S: ExecutionSource + ?Sized>(
        &mut self,
        failure: ExecutionFailure,
        now: ClockSample,
        source: &mut S,
    ) -> DriverStep {
        if let FailureOrigin::Evaluation(error) = &failure.origin {
            self.diagnostic = Some(error.clone());
        }
        if failure.error() == Some("States.DataLimitExceeded") {
            self.recover(failure, now, source)
        } else {
            self.settle(ExecutionOutcome::Failed(failure))
        }
    }
    fn evaluation_failure_shared<S: ExecutionSource + ?Sized>(
        &mut self,
        error: Arc<AslError>,
        now: ClockSample,
        source: &mut S,
        recover: bool,
    ) -> DriverStep {
        let name = match error.as_ref() {
            AslError::DataLimit
            | AslError::Json(JsonError::Bytes | JsonError::String | JsonError::Key) => {
                "States.DataLimitExceeded"
            }
            AslError::Entropy(_)
            | AslError::Encoding(_)
            | AslError::Allocation(_)
            | AslError::InvalidStateId
            | AslError::InvalidFanoutResult
            | AslError::InvalidExecution => "Rustack.InternalError",
            _ => "States.Runtime",
        };
        let mut failure = ExecutionFailure::named(name, "State evaluation failed");
        failure.origin = FailureOrigin::Evaluation(error.clone());
        self.diagnostic = Some(error);
        if recover && name == "States.DataLimitExceeded" {
            self.recover(failure, now, source)
        } else {
            self.settle(ExecutionOutcome::Failed(failure))
        }
    }
}
fn update(step: DriverStep) -> DriverUpdate {
    DriverUpdate { step, cancel: None }
}

pub use history::{ExecutionHistory, HistoryLimits, HistoryPage};
pub use runtime::{
    CleanupState, ExecutionError, ExecutionHandle, ExecutionSupervisor, ExecutionView,
    RuntimeLimits,
};
