//! Shared SSM types used across input, output, and internal representations.
//!
//! All types follow the SSM JSON wire format with `PascalCase` field names.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The type of a parameter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterType {
    /// A plain string value.
    #[default]
    String,
    /// A comma-separated list of strings.
    StringList,
    /// An encrypted string value.
    SecureString,
}

impl ParameterType {
    /// Returns the wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "String",
            Self::StringList => "StringList",
            Self::SecureString => "SecureString",
        }
    }
}

impl std::fmt::Display for ParameterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The tier of a parameter, which affects storage limits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterTier {
    /// Standard tier (4 KB value limit).
    #[default]
    Standard,
    /// Advanced tier (8 KB value limit, additional features).
    Advanced,
    /// Intelligent-Tiering (automatically selects tier).
    #[serde(rename = "Intelligent-Tiering")]
    IntelligentTiering,
}

impl ParameterTier {
    /// Returns the wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Advanced => "Advanced",
            Self::IntelligentTiering => "Intelligent-Tiering",
        }
    }
}

impl std::fmt::Display for ParameterTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// A tag associated with a resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    /// The tag key.
    pub key: String,
    /// The tag value.
    pub value: String,
}

/// A parameter returned in API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Parameter {
    /// The parameter name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The parameter type.
    #[serde(rename = "Type", skip_serializing_if = "Option::is_none")]
    pub parameter_type: Option<String>,

    /// The parameter value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// The parameter version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,

    /// The date the parameter was last changed or updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified_date: Option<f64>,

    /// The Amazon Resource Name (ARN) of the parameter.
    #[serde(rename = "ARN", skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,

    /// The data type of the parameter (e.g., `"text"`, `"aws:ec2:image"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
}

/// A filter for `GetParametersByPath`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParameterStringFilter {
    /// The filter key (e.g., `"Type"`, `"KeyId"`, `"Path"`).
    pub key: String,

    /// The filter comparison option (e.g., `"Equals"`, `"BeginsWith"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,

    /// The filter values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}
