#![allow(clippy::struct_excessive_bools)]
//! CloudFront type model for Rustack.
//!
//! Defines the domain types for the CloudFront management API (distributions,
//! invalidations, origin access controls, cache/origin/response-header policies,
//! key groups, public keys, functions, field-level encryption, realtime log
//! configs, tags) and the error type used across the HTTP and provider layers.
//!
//! Wire-format XML (request parsing and response rendering) lives in the
//! companion `rustack-cloudfront-http` crate. This crate is dependency-free of
//! HTTP plumbing so it can be reused from both management and data-plane
//! code.

pub mod error;
pub mod ids;
pub mod tags;
pub mod types;

pub use error::CloudFrontError;
pub use ids::{DistributionId, InvalidationId, ResourceId};
pub use tags::{Tag, TagSet};
pub use types::*;

/// XML namespace for the 2020-05-31 CloudFront API.
pub const CLOUDFRONT_XML_NAMESPACE: &str = "http://cloudfront.amazonaws.com/doc/2020-05-31/";

/// API version string used in URL paths.
pub const CLOUDFRONT_API_VERSION: &str = "2020-05-31";
