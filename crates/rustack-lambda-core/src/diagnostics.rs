//! Bounded actual invocation ownership, sampled without business admission.
//!
//! Counters are independently sampled; they are not a transaction spanning two
//! semaphores. They contain no function identity, request data or credentials.

use serde::Serialize;

pub(crate) const ACCEPTED_LIMIT: usize = 128;
pub(crate) const EXECUTION_LIMIT: usize = 32;
pub(crate) const DEFAULT_FUNCTION_LIMIT: usize = 8;

/// Limits used by the real invocation supervisor and its admission semaphores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct InvocationWorkLimits {
    /// Maximum accepted calls, including asynchronous waiting and cleanup.
    pub accepted_invocations: usize,
    /// Maximum owned global execution permits, including backend pool waiting.
    pub active_executions: usize,
    /// Per-function execution limit when no reserved override is installed.
    pub default_function_executions: usize,
}

/// Actual independently sampled ownership counters from one Lambda provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct InvocationWorkStatus {
    /// Accepted-work permits retained until execution/cleanup settles.
    pub accepted_invocations: usize,
    /// Global execution permits retained through backend waiting and cleanup.
    pub active_executions: usize,
    /// Whether the supervisor still accepts ordinary invocation submissions.
    pub accepting: bool,
    /// The same limits used to construct/acquire actual admission capacity.
    pub limits: InvocationWorkLimits,
}
