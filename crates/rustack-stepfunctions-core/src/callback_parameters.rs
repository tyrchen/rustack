//! Distinct outbound/history projections for one callback parameter evaluation.

use std::fmt;

use crate::Payload;

/// Bounded history-only Parameters; no unchecked public constructor or serializer.
#[derive(Clone)]
#[non_exhaustive]
pub struct CallbackHistory {
    pub(crate) payload: Payload,
    pub(crate) redacted: bool,
}
impl fmt::Debug for CallbackHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackHistory(<redacted>)")
    }
}
impl CallbackHistory {
    /// Read the prepared history JSON, with sensitive expression values null.
    ///
    /// ```
    /// # fn journal(h:&rustack_stepfunctions_core::CallbackHistory){let safe=h.payload();}
    /// ```
    #[must_use]
    pub fn payload(&self) -> &Payload {
        &self.payload
    }
    /// Whether local callback provenance removed any sensitive values.
    ///
    /// `TaskScheduled` has no AWS truncated metadata field; do not invent one.
    ///
    /// ```
    /// # fn inspect(h:&rustack_stepfunctions_core::CallbackHistory){let redacted=h.redacted();}
    /// ```
    #[must_use]
    pub fn redacted(&self) -> bool {
        self.redacted
    }
}
/// Actual invocation Parameters and a separately typed safe history projection.
///
/// The combined object deliberately cannot be serialized into history:
///
/// ```compile_fail
/// # fn persist(value:rustack_stepfunctions_core::CallbackParameters) {
/// let unsafe_history = serde_json::to_string(&value);
/// # }
/// ```
#[derive(Clone)]
#[non_exhaustive]
pub struct CallbackParameters {
    pub(crate) outbound: Payload,
    pub(crate) history: CallbackHistory,
}
impl fmt::Debug for CallbackParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackParameters(<redacted>)")
    }
}
impl CallbackParameters {
    /// Read the actual invocation JSON, including explicitly selected credential.
    ///
    /// Do not pass this value to the callback history adapter.
    ///
    /// ```
    /// # fn invoke(p:&rustack_stepfunctions_core::CallbackParameters){let actual=p.outbound();}
    /// ```
    #[must_use]
    pub fn outbound(&self) -> &Payload {
        &self.outbound
    }
    /// Read the distinct projection reserved for journal/persistence consumers.
    ///
    /// ```
    /// # fn journal(p:&rustack_stepfunctions_core::CallbackParameters){let safe=p.history();}
    /// ```
    #[must_use]
    pub fn history(&self) -> &CallbackHistory {
        &self.history
    }
}
