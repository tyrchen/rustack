//! Lambda business logic for RustStack.
//!
//! This crate contains the core Lambda implementation including:
//!
//! - **Storage**: In-memory function store using `DashMap` for concurrent access
//! - **Provider**: CRUD operations for functions, versions, aliases, tags, and policies
//! - **Handler**: Bridges the HTTP layer to the provider via `LambdaHandler` trait
//! - **Resolver**: Function name/ARN parsing and version resolution
//! - **Config**: Service configuration from environment variables

pub mod config;
pub mod error;
pub mod handler;
pub mod provider;
pub mod resolver;
pub mod storage;
