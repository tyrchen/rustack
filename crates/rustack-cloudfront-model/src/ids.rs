//! Resource ID and ETag types.
//!
//! CloudFront resource IDs are 14-character uppercase alphanumeric strings
//! prefixed with a type letter ('E' for most resources). ETags use the same
//! character shape — they are treated as opaque version tokens; they are
//! *not* MD5 hashes of the config.

/// Generic CloudFront resource ID (14 chars, `[A-Z0-9]`, leading `E`).
pub type ResourceId = String;

/// Distribution ID (alias for `ResourceId`, for documentation clarity).
pub type DistributionId = String;

/// Invalidation ID (`I` prefix, 14 chars).
pub type InvalidationId = String;
