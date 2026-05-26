//! S3 HTTP routing, request parsing, response serialization, and hyper service.
//!
//! This crate provides the HTTP layer for a LocalStack-compatible S3 server. It handles:
//!
//! - **Routing** ([`router`]): Maps HTTP requests to S3 operations by examining method, path, query
//!   parameters, and headers. Supports both path-style and virtual-hosted-style bucket addressing.
//!
//! - **Request deserialization** ([`request`]): Converts raw HTTP request parts into typed S3 Input
//!   structs from `rustack-s3-model`.
//!
//! - **Response serialization** ([`response`]): Converts typed S3 Output structs into HTTP
//!   responses with appropriate status codes, headers, and bodies.
//!
//! - **Dispatch** ([`dispatch`]): Routes identified S3 operations to the business logic handler via
//!   the [`S3Handler`] trait.
//!
//! - **Service** ([`service`]): The main [`S3HttpService`] that implements hyper's `Service` trait,
//!   tying routing, auth, dispatch, and middleware together.
//!
//! - **Body** ([`body`]): The [`S3ResponseBody`] type supporting buffered and empty response modes.
//!
//! # Architecture
//!
//! ```text
//! HTTP Request
//!   -> S3HttpService (hyper Service)
//!     -> Health check / CORS interception
//!     -> S3Router (virtual hosting + operation identification)
//!     -> Body collection
//!     -> SigV4 authentication (optional)
//!     -> dispatch_operation (S3Handler trait)
//!     -> Common response headers (x-amz-request-id, Server, etc.)
//!   <- HTTP Response
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use rustack_s3_http::service::{S3HttpConfig, S3HttpService};
//! use rustack_s3_http::dispatch::NotImplementedHandler;
//!
//! let config = S3HttpConfig::default();
//! let handler = NotImplementedHandler;
//! let service = S3HttpService::new(handler, config);
//! // Use `service` with hyper server.
//! ```

// S3Error is a fundamental domain error type used pervasively as Result<T, S3Error>.
// Its size (128 bytes) is inherent to its fields (code, message, resource, request_id,
// status_code, source, headers). Boxing S3Error in every Result would add indirection
// on the hot path for negligible benefit.
#![allow(clippy::result_large_err)]

pub mod body;
pub mod codec;
pub mod dispatch;
pub mod multipart;
pub mod request;
pub mod response;
pub mod router;
pub mod service;

// Re-export key types for convenience.
pub use body::S3ResponseBody;
pub use dispatch::{NotImplementedHandler, S3Handler};
pub use request::FromS3Request;
pub use response::IntoS3Response;
pub use router::{RoutingContext, S3Router};
pub use service::{S3HttpConfig, S3HttpService};
