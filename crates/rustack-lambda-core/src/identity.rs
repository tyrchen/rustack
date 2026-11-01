//! Stable function resource identity, independent of mutable names and revisions.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

/// Identity of one function's lifetime, preserved across updates and snapshots.
///
/// Deleting and recreating a name must allocate a new identity.
///
/// ```
/// use rustack_lambda_core::identity::FunctionResourceId;
/// let identity = FunctionResourceId::default();
/// assert_eq!(identity.to_string().parse::<FunctionResourceId>()?, identity);
/// # Ok::<(), rustack_lambda_core::identity::ResourceIdError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FunctionResourceId(Uuid);

/// A saved resource identity is malformed or absent of meaningful identity.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResourceIdError {
    /// Only the 36-byte hyphenated UUID representation is accepted.
    #[error("function resource identity must be a 36-byte UUID")]
    Length,
    /// The input is not a UUID.
    #[error("invalid function resource UUID")]
    Invalid {
        /// UUID parser failure, without the original input.
        #[source]
        source: uuid::Error,
    },
    /// A nil UUID cannot identify a function lifetime.
    #[error("function resource UUID must not be nil")]
    Nil,
}

impl Default for FunctionResourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionResourceId {
    /// Allocate a random identity for a new function resource.
    ///
    /// ```
    /// use rustack_lambda_core::identity::FunctionResourceId;
    /// let first = FunctionResourceId::new();
    /// assert_ne!(first, FunctionResourceId::new());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for FunctionResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for FunctionResourceId {
    type Err = ResourceIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 36 {
            return Err(ResourceIdError::Length);
        }
        let uuid = Uuid::parse_str(value).map_err(|source| ResourceIdError::Invalid { source })?;
        if uuid.is_nil() {
            return Err(ResourceIdError::Nil);
        }
        Ok(Self(uuid))
    }
}

impl Serialize for FunctionResourceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FunctionResourceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = FunctionResourceId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a nonnil hyphenated function resource UUID")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_distinct_nonnil_serializable_identities()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = FunctionResourceId::default();
        let second = FunctionResourceId::default();
        assert_ne!(first, second);
        let saved = serde_json::to_string(&first)?;
        assert_eq!(serde_json::from_str::<FunctionResourceId>(&saved)?, first);
        Ok(())
    }

    #[test]
    fn test_should_reject_nil_malformed_and_oversized_resource_identity() {
        assert!(matches!(
            "00000000-0000-0000-0000-000000000000".parse::<FunctionResourceId>(),
            Err(ResourceIdError::Nil)
        ));
        for value in ["x".repeat(36), "x".repeat(4096), String::new()] {
            assert!(value.parse::<FunctionResourceId>().is_err());
            let saved = serde_json::Value::String(value);
            assert!(serde_json::from_value::<FunctionResourceId>(saved).is_err());
        }
    }
}
