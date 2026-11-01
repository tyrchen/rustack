//! Bounded AWS JSON1.0 transport for typed Step Functions handlers.
//!
//! Core owns registry/domain semantics and accepted command lifetime. This crate
//! supplies no default handler and cannot advertise workflow execution by itself.
//! The async-trait handler is deliberately object-safe for trusted runtime DI.
//! The serving gateway owns bounded connection/request admission; this layer
//! does not spawn request jobs or own accepted workflow execution lifetime.
#![forbid(unsafe_code)]

mod body;
mod context;
mod error;
mod protocol;
mod response;
mod service;
mod transport;
mod validation;

pub use body::StepFunctionsBody;
pub use context::{RequestContext, RequestScope, StepFunctionsHttpConfig};
pub use error::HttpError;
pub use protocol::{CONTENT_TYPE, claims_request, resolve_operation};
pub use service::{StepFunctionsHandler, StepFunctionsHttpService};
pub use transport::{Request, Response};

#[cfg(test)]
mod tests;
