//! Lambda HTTP service layer for RustStack.
//!
//! This crate implements the `restJson1` protocol for Lambda, providing:
//!
//! - **Router**: Dispatches by HTTP method + URL path (not `X-Amz-Target`)
//! - **Handler trait**: Defines the boundary between HTTP and business logic
//! - **Service**: Hyper `Service` implementation for the Lambda protocol
//! - **Response helpers**: JSON success/error response formatting
//!
//! Unlike `awsJson1.1` services (SSM, DynamoDB), Lambda uses REST-style
//! routing where each operation has a unique method + path combination.
//! Path parameters (e.g., `{FunctionName}`) are extracted and passed to
//! the handler.
//!
//! # Error format
//!
//! Lambda errors use the `restJson1` convention:
//! - `X-Amzn-Errortype` header carries the error code
//! - Response body contains `{"Type": "User"|"Service", "Message": "..."}`

pub mod body;
pub mod dispatch;
pub mod response;
pub mod router;
pub mod service;

pub use body::LambdaResponseBody;
pub use dispatch::{LambdaHandler, NotImplementedHandler};
pub use router::PathParams;
pub use service::{LambdaHttpConfig, LambdaHttpService};
