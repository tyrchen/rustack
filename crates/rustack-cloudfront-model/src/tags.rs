//! Tag types.

use serde::{Deserialize, Serialize};

/// An AWS resource tag (key/value pair).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// Tag key (required).
    pub key: String,
    /// Tag value (may be empty).
    pub value: String,
}

/// Ordered collection of tags with unique keys.
pub type TagSet = Vec<Tag>;
