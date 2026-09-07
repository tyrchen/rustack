//! Core types, configuration, and state management for Rustack.
//!
//! This crate provides the foundational building blocks shared across all
//! Rustack service implementations, including multi-account/multi-region
//! state management, configuration, and common AWS type definitions.

mod config;
mod error;
pub mod http;
pub mod settings;
mod state;
mod types;

pub use config::RustackConfig;
pub use error::{RustackError, RustackResult};
pub use state::AccountRegionStore;
pub use types::{AccountId, AwsRegion};
