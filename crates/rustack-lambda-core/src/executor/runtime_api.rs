//! Shared Lambda Runtime API reexports for existing executor consumers.
//!
//! Native and relay adapters use the same bounded runtime session implementation.

pub use rustack_lambda_runtime::{
    http::{ApiError, RuntimeApiHandle, start, start_at},
    session::{Invocation, InvocationTicket, RuntimeResult, SessionError},
};
