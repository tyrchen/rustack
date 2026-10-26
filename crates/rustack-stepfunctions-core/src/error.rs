//! Redacted semantic failures with typed source ownership.

use std::{collections::TryReserveError, fmt};

use getrandom::Error as EntropyError;
use rustack_core::json::JsonError;
use thiserror::Error;

/// Bounded ASL data failures; messages never contain input or path text.
#[derive(Error)]
#[non_exhaustive]
pub enum AslError {
    /// Unsupported or invalid intrinsic syntax/arity.
    #[error("invalid ASL expression")]
    InvalidExpression,
    /// An intrinsic encountered an invalid value, type or format.
    #[error("invalid ASL intrinsic value")]
    IntrinsicValue,
    /// The operating system could not supply secure UUID entropy.
    #[error("ASL UUID entropy unavailable")]
    Entropy(#[source] EntropyError),
    /// Bounded storage could not be reserved.
    #[error("ASL allocation failed")]
    Allocation(#[source] TryReserveError),
    /// Unsupported or ambiguous static Wait source definition.
    #[error("invalid ASL Wait source")]
    InvalidWait,
    /// A dynamic Wait value has an invalid type, date or supported range.
    #[error("invalid ASL Wait value")]
    WaitValue,
    /// Unsupported, ambiguous or invalid Choice predicate definition.
    #[error("invalid ASL Choice predicate")]
    InvalidPredicate,
    /// Numeric comparison exceeds the local binary64 interoperability range.
    #[error("ASL numeric comparison outside supported range")]
    NumericRange,
    /// The path is invalid or exceeds the compiled-profile limits.
    #[error("invalid ASL path")]
    InvalidPath,
    /// A selection operator is not a reference path.
    #[error("invalid ASL reference path")]
    InvalidReference,
    /// Result paths cannot write into the context object.
    #[error("invalid ASL result target")]
    InvalidResultTarget,
    /// A singular path could not select a value.
    #[error("ASL path selected no value")]
    Missing,
    /// A merge traversed a scalar or an absent/out-of-range array element.
    #[error("ASL result path cannot be applied")]
    Merge,
    /// Expansion exceeds the allowed bounded evaluation work.
    #[error("ASL evaluation work limit exceeded")]
    WorkLimit,
    /// Materialized data exceeds the payload byte allowance.
    #[error("ASL payload byte limit exceeded")]
    DataLimit,
    /// Input or constructed data violates bounded JSON validation.
    #[error("invalid bounded ASL JSON")]
    Json(#[from] JsonError),
    /// A bounded serializer failed; retain its source without projecting it.
    #[error("ASL payload encoding failed")]
    Encoding(#[source] serde_json::Error),
}

impl fmt::Debug for AslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
