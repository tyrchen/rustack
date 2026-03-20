//! STS business logic for `RustStack`.
//!
//! Implements temporary credential management, identity resolution,
//! and all 8 STS operations.

pub mod config;
pub mod handler;
pub mod identity;
pub mod keygen;
pub mod provider;
pub mod session;
pub mod state;
pub mod validation;
