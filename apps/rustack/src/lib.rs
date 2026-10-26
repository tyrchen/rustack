//! Composition adapters shared by the Rustack binary and integration harnesses.
//!
//! Exporting an adapter does not register or advertise a gateway service.

#![forbid(unsafe_code)]

#[cfg(all(feature = "lambda", feature = "stepfunctions"))]
pub mod stepfunctions_bridge;

/// Real registry-backed Step Functions API and stable pagination ownership.
#[cfg(feature = "stepfunctions")]
pub mod stepfunctions_api;
