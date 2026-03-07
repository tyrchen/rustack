//! Lambda API model types for RustStack.
//!
//! This crate defines the data types, operations, and error types for the
//! AWS Lambda `restJson1` protocol. Unlike other RustStack services that use
//! `awsJson1.0`/`1.1`, Lambda dispatches by HTTP method + URL path rather
//! than `X-Amz-Target` header.

pub mod error;
pub mod input;
pub mod operations;
pub mod output;
pub mod types;

pub use error::{LambdaError, LambdaErrorCode};
pub use operations::{LAMBDA_ROUTES, LambdaOperation, LambdaRoute};
pub use types::*;
