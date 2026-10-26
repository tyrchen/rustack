//! Pure, bounded ASL payload and compiled-path semantics.
//!
//! This crate has no network, SDK, Lambda or HTTP dependency. Immutable payloads
//! preserve source bytes and compiled paths distinguish missing values from null.
//! No registry or execution-service capability is implied by these data APIs.

#![forbid(unsafe_code)]

mod choice;
mod error;
mod path;
mod payload;

pub use choice::Predicate;
pub use error::AslError;
pub use path::{MatchKind, Path, PathMatches, ReferencePath, ResultPath};
pub use payload::{EvaluationContext, MAX_PAYLOAD_BYTES, Payload};
