//! S3 service implementation for Rustack.
//!
//! This crate implements the S3 business logic provider (`RustackS3`) that can
//! be plugged into the `rustack-s3-http` service layer via the `S3Handler` trait.
//! It supports bucket CRUD, object CRUD, multipart uploads, versioning, CORS,
//! tagging, ACLs, encryption metadata, checksums, object lock, and more.
//!
//! # Architecture
//!
//! ```text
//! rustack-s3-http (routing, XML, auth)
//!        |
//!        v
//! RustackS3 (S3Handler trait impl in server)
//!        |
//!        v
//!   S3ServiceState (buckets, global index)
//!        |
//!        v
//!   StorageBackend (in-memory + spillover)
//! ```

pub mod auth;
pub mod checksums;
pub mod config;
pub mod cors;
pub mod error;
pub mod ops;
pub mod provider;
pub mod snapshot;
pub mod state;
pub mod storage;
pub mod utils;
pub mod validation;

pub use config::S3Config;
pub use provider::RustackS3;
