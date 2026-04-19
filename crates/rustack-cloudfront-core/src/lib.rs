#![allow(clippy::must_use_candidate)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::items_after_statements)]
//! CloudFront business logic for Rustack.
//!
//! Owns the in-memory resource store, ETag model, distribution / invalidation
//! lifecycle simulation, and managed-policy seeding. Exposes a single
//! `RustackCloudFront` provider that the HTTP crate and the data-plane crate
//! consume.

pub mod arn;
pub mod config;
pub mod id_gen;
pub mod managed;
pub mod provider;
pub mod store;

pub use config::CloudFrontConfig;
pub use provider::RustackCloudFront;
pub use store::CloudFrontStore;
