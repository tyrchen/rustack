//! DynamoDB Streams HTTP service layer for RustStack.
//!
//! This crate implements the `awsJson1_0` protocol for DynamoDB Streams, providing:
//!
//! - **Router**: Extracts the operation from `X-Amz-Target` header
//! - **Handler trait**: Defines the boundary between HTTP and business logic
//! - **Service**: Hyper `Service` implementation for the DynamoDB Streams protocol
//! - **Response helpers**: JSON success/error response formatting
#![allow(missing_docs)]

pub mod body;
pub mod dispatch;
pub mod response;
pub mod router;
pub mod service;

pub use body::DynamoDBStreamsResponseBody;
pub use dispatch::{DynamoDBStreamsHandler, NotImplementedHandler};
pub use service::{DynamoDBStreamsHttpConfig, DynamoDBStreamsHttpService};
