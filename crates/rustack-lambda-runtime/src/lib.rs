//! Shared, host-portable Lambda runtime contracts.
//!
//! The control protocol validates untrusted CBOR before deserialization and
//! preserves raw payload bytes without logging their contents. This crate has
//! no dependency on a function provider, VM implementation or Docker client.

#![forbid(unsafe_code)]

mod diagnostics;
pub mod http;
pub mod metadata;
pub mod protocol;
pub mod relay;
pub mod session;
