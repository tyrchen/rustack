//! Lambda business logic for Rustack.
//!
//! This crate contains the core Lambda implementation including:
//!
//! - **Storage**: In-memory function store using `DashMap` for concurrent access
//! - **Provider**: CRUD operations for functions, versions, aliases, tags, and policies
//! - **Handler**: Bridges the HTTP layer to the provider via `LambdaHandler` trait
//! - **Resolver**: Function name/ARN parsing and version resolution
//! - **Config**: Service configuration from environment variables

#![forbid(unsafe_code)]

pub mod artifact;
pub mod code;
pub mod config;
pub mod error;
pub mod executor;
mod function_config;
pub mod handler;
pub mod identity;
pub mod invocation;
pub mod provider;
pub mod resolver;
pub mod storage;
#[cfg(test)]
#[path = "../../../tests/lambda-fixtures/minimal_zip.rs"]
mod test_zip;
mod work;
