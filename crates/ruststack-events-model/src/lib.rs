//! EventBridge model types for RustStack.
//!
//! This crate provides all EventBridge API types needed for the
//! RustStack Events implementation. Types are hand-written since the
//! `awsJson1_1` protocol makes serde derives trivial.
#![allow(clippy::doc_markdown)]
#![allow(missing_docs)]

pub mod error;
pub mod input;
pub mod operations;
pub mod output;
pub mod types;

pub use error::{EventsError, EventsErrorCode};
pub use operations::EventsOperation;
