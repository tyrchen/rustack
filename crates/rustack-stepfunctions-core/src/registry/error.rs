//! Typed static registry failures with retained redacted sources.

use std::{fmt, sync::Arc};

use thiserror::Error;
use tokio::task::JoinError;

use crate::{AslError, ExecutionError};

/// Domain, admission and lifecycle failures; no payload-bearing public text.
#[derive(Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// Invalid scope, metadata, identifier, arguments or trusted limits.
    #[error("invalid workflow registry input")]
    Validation,
    /// Atomic tag mutation would exceed the fifty-tag quota.
    #[error("too many workflow resource tags")]
    TagLimit,
    /// No such current mutable state machine.
    #[error("state machine does not exist")]
    MachineMissing,
    /// A conflicting same-name state machine already exists.
    #[error("state machine already exists")]
    MachineExists,
    /// Starts/updates are forbidden during deletion.
    #[error("state machine is deleting")]
    Deleting,
    /// No retained execution record is available.
    #[error("execution does not exist")]
    ExecutionMissing,
    /// Execution ARN is already reserved with incompatible input or closed.
    #[error("execution already exists")]
    ExecutionExists,
    /// A bounded allocation cannot be reserved without evicting live ownership.
    #[error("workflow registry capacity exhausted")]
    Capacity,
    /// No accepting/live registry owner is available.
    #[error("workflow registry unavailable")]
    Unavailable,
    /// Observer deadline expired; accepted ownership is unchanged.
    #[error("workflow registry observation expired")]
    Deadline,
    /// Interpreter/context/entropy conversion failure.
    #[error("workflow registry preparation failed")]
    Evaluation(
        #[from]
        #[source]
        AslError,
    ),
    /// Execution registration or cleanup failure.
    #[error("workflow execution owner failed")]
    Execution(
        #[from]
        #[source]
        ExecutionError,
    ),
    /// Static Unicode tag parser failed to initialize.
    #[error("workflow tag validator failed")]
    Regex(#[source] regex::Error),
    /// Unique actor failed; unresolved allocations remain retained.
    #[error("workflow registry worker failed")]
    Worker(#[source] JoinError),
    /// Cached unique-owner failure.
    #[error("workflow registry shutdown failed")]
    Owner(#[source] Arc<RegistryError>),
}
impl fmt::Debug for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
