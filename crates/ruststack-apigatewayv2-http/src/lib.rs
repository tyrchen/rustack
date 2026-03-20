//! API Gateway v2 HTTP service layer for RustStack.
//!
//! This crate implements the `restJson1` protocol for API Gateway v2, providing:
//!
//! - **Router**: Dispatches by HTTP method + URL path
//! - **Handler trait**: Defines the boundary between HTTP and business logic
//! - **Service**: Hyper `Service` implementation for the API Gateway v2 protocol
//! - **Response helpers**: JSON success/error response formatting
//!
//! API Gateway v2 uses REST-style routing where each operation has a unique
//! method + path combination. Path parameters (e.g., `{apiId}`) are extracted
//! and passed to the handler.
//!
//! # Error format
//!
//! API Gateway v2 errors use a JSON body with a lowercase `message` field:
//! `{"message": "..."}`

pub mod body;
pub mod dispatch;
pub mod response;
pub mod router;
pub mod service;

pub use body::ApiGatewayV2ResponseBody;
pub use dispatch::{ApiGatewayV2Handler, NotImplementedHandler};
pub use router::PathParams;
pub use service::{ApiGatewayV2HttpConfig, ApiGatewayV2HttpService};
