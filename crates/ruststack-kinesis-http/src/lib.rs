//! Kinesis HTTP service layer for RustStack.
//!
//! This crate implements the `awsJson1_1` protocol for Kinesis, providing:
//!
//! - **Router**: Extracts the operation from `X-Amz-Target` header
//! - **Handler trait**: Defines the boundary between HTTP and business logic
//! - **Service**: Hyper `Service` implementation for the Kinesis protocol
//! - **Response helpers**: JSON success/error response formatting
#![allow(missing_docs)]

pub mod body;
pub mod dispatch;
pub mod response;
pub mod router;
pub mod service;

pub use body::KinesisResponseBody;
pub use dispatch::{KinesisHandler, NotImplementedHandler};
pub use service::{KinesisHttpConfig, KinesisHttpService};
