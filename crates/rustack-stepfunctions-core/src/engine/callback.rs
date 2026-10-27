//! Credential-free callback instructions, materialized only after token registration.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rustack_stepfunctions_model::types::HistoryEvent;

use super::history::events;
use crate::{
    AslError, AttemptId, CallbackHistory, CallbackInvocationMetadata, CallbackToken,
    CompiledMachine, EvaluationContext, Integration, Payload, StateId, StateView, TaskDispatch,
    UuidSource,
};

#[derive(Debug, Clone)]
pub(super) struct CallbackIdentity {
    pub(super) attempt: AttemptId,
    pub(super) state: StateId,
    pub(super) deadline: Instant,
    pub(super) timeout: Option<u32>,
    pub(super) heartbeat: Option<Duration>,
}
/// One immutable callback attempt awaiting its exact registered token.
///
/// This effect contains no credential and performs no registration or invocation.
/// Its owner must check current frame/generation/control/deadline, register the
/// token, charge retained work, then commit typed history before business issuance.
#[derive(Clone)]
#[non_exhaustive]
pub struct CallbackDispatch {
    pub(super) identity: CallbackIdentity,
    pub(super) machine: Arc<CompiledMachine>,
    pub(super) input: Payload,
    pub(super) context: EvaluationContext,
}
impl fmt::Debug for CallbackDispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackDispatch(<redacted>)")
    }
}
macro_rules! read {
    ($name:ident,$out:ty,$s:ident=>$body:expr,$doc:literal) => {
        #[doc=$doc]
        #[doc=concat!("\n```\n# fn inspect(effect:&rustack_stepfunctions_core::CallbackDispatch){let value=effect.",stringify!($name),"();}\n```")]
        #[must_use] pub fn $name(&self)->$out {let $s=self;$body}
    };
}
impl CallbackDispatch {
    read!(attempt,AttemptId,s=>s.identity.attempt,"Read the assigned generation before token registration.");
    read!(state,StateId,s=>s.identity.state,"Read the state in the captured lexical machine.");
    read!(deadline,Instant,s=>s.identity.deadline,"Read the original absolute Task/execution minimum deadline.");
    read!(timeout_seconds,Option<u32>,s=>s.identity.timeout,"Read explicit resolved timeout seconds for history; omission stays omitted.");
    read!(heartbeat,Option<Duration>,s=>s.identity.heartbeat,"Read the optional validated heartbeat interval; no timer starts here.");
    read!(machine,&Arc<CompiledMachine>,s=>&s.machine,"Read the exact artifact captured before token registration.");
    /// Project required scheduled parameters exclusively from typed safe history.
    ///
    /// # Errors
    /// Returns typed encoding/instruction errors; no credential source is serialized.
    ///
    /// ```
    /// # fn event(e:&rustack_stepfunctions_core::CallbackDispatch,h:&rustack_stepfunctions_core::CallbackHistory){let event=e.scheduled_event(h,chrono::Utc::now());}
    /// ```
    pub fn scheduled_event(
        &self,
        history: &CallbackHistory,
        wall: DateTime<Utc>,
    ) -> Result<HistoryEvent, AslError> {
        events::scheduled_callback(
            &self.machine,
            self.state(),
            history,
            self.timeout_seconds().map(i64::from),
            self.heartbeat()
                .map(|value| i64::try_from(value.as_secs()).map_err(|_| AslError::InvalidExecution))
                .transpose()?,
            wall,
        )
    }
    /// Project actual invocation metadata, marking the omitted function body truncated.
    ///
    /// # Errors
    /// Returns typed bounded metadata/event encoding errors.
    ///
    /// ```
    /// # fn event(e:&rustack_stepfunctions_core::CallbackDispatch,m:&rustack_stepfunctions_core::CallbackInvocationMetadata){let event=e.submitted_event(m,chrono::Utc::now());}
    /// ```
    pub fn submitted_event(
        &self,
        metadata: &CallbackInvocationMetadata,
        wall: DateTime<Utc>,
    ) -> Result<HistoryEvent, AslError> {
        events::task_submitted(&self.machine, self.state(), &metadata.payload()?, wall)
    }
    /// Materialize one outbound invocation and separately typed history together.
    ///
    /// The caller supplies the exact token registered for this frame/generation.
    /// This pure method does not prove registration, current ownership or time;
    /// the async owner must fence these before/after bounded CPU work. Repeating
    /// preparation cannot authorize another invocation or history publication.
    ///
    /// # Errors
    /// Preserves typed template/context/quota/entropy failures with no partial
    /// output. The effect's captured original input/context remain unchanged.
    ///
    /// ```
    /// # fn prepare(e:&rustack_stepfunctions_core::CallbackDispatch,t:&rustack_stepfunctions_core::CallbackToken){let values=e.prepare_with_source(t,&mut rustack_stepfunctions_core::SystemUuidSource::default());}
    /// ```
    pub fn prepare_with_source<S: UuidSource + ?Sized>(
        &self,
        token: &CallbackToken,
        source: &mut S,
    ) -> Result<(TaskDispatch, CallbackHistory), AslError> {
        let StateView::Task {
            flow,
            resource: Integration::LambdaCallback(_),
            ..
        } = self.machine.state(self.identity.state)?
        else {
            return Err(AslError::InvalidStateId);
        };
        let prepared =
            flow.prepare_callback_with_source(&self.input, &self.context, token, source)?;
        Ok((
            TaskDispatch {
                attempt: self.identity.attempt,
                state: self.identity.state,
                parameters: prepared.outbound,
                deadline: self.identity.deadline,
            },
            prepared.history,
        ))
    }
}

#[cfg(test)]
mod tests;
