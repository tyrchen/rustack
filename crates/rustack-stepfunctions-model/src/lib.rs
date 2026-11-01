//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.
#![forbid(unsafe_code)]
pub(crate) mod epoch_seconds;

pub mod error;
pub mod input;
pub mod operations;
pub mod output;
pub mod types;
/// JSON protocol target prefix.
pub const TARGET_PREFIX: &str = "AWSStepFunctions";
/// Whether the service config requests AWS Query-compatible errors.
pub const AWS_QUERY_COMPATIBLE: bool = false;

pub use error::{StepFunctionsError, StepFunctionsErrorCode};
pub use operations::StepFunctionsOperation;

// Generated: 37 operations, 16 enums, 63 shared structs, 37 input structs, 37 output structs
