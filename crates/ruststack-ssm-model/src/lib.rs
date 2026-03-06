//! SSM Parameter Store model types for RustStack.
//!
//! This crate provides all SSM Parameter Store API types needed for the
//! RustStack SSM implementation. Types are hand-written since the SSM
//! `awsJson1_1` protocol makes serde derives trivial.
#![allow(clippy::doc_markdown)]
#![allow(missing_docs)]

pub mod error;
pub mod input;
pub mod operations;
pub mod output;
pub mod types;

pub use error::{SsmError, SsmErrorCode};
pub use operations::SsmOperation;
