//! SSM input types for Phase 0 operations.
//!
//! All input structs use `PascalCase` JSON field naming to match the SSM
//! wire protocol (`awsJson1_1`). Optional fields are omitted when `None`.

use serde::{Deserialize, Serialize};

use crate::types::{ParameterStringFilter, Tag};

// ---------------------------------------------------------------------------
// PutParameter
// ---------------------------------------------------------------------------

/// Input for the `PutParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutParameterInput {
    /// The fully qualified name of the parameter.
    pub name: String,

    /// The parameter value.
    pub value: String,

    /// The type of parameter (`String`, `StringList`, or `SecureString`).
    #[serde(rename = "Type", skip_serializing_if = "Option::is_none")]
    pub parameter_type: Option<String>,

    /// A description of the parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The KMS key ID for `SecureString` parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,

    /// Whether to overwrite an existing parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,

    /// A regular expression used to validate the parameter value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_pattern: Option<String>,

    /// Tags to associate with the parameter (only on create).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,

    /// The parameter tier (`Standard`, `Advanced`, or `Intelligent-Tiering`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// The data type of the parameter (default `"text"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,

    /// Parameter policies in JSON format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policies: Option<String>,
}

// ---------------------------------------------------------------------------
// GetParameter
// ---------------------------------------------------------------------------

/// Input for the `GetParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParameterInput {
    /// The name of the parameter (supports `:version` and `:label` selectors).
    pub name: String,

    /// Whether to decrypt `SecureString` values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_decryption: Option<bool>,
}

// ---------------------------------------------------------------------------
// GetParameters
// ---------------------------------------------------------------------------

/// Input for the `GetParameters` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParametersInput {
    /// The names of the parameters to retrieve (max 10).
    #[serde(default)]
    pub names: Vec<String>,

    /// Whether to decrypt `SecureString` values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_decryption: Option<bool>,
}

// ---------------------------------------------------------------------------
// GetParametersByPath
// ---------------------------------------------------------------------------

/// Input for the `GetParametersByPath` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParametersByPathInput {
    /// The hierarchy path prefix.
    pub path: String,

    /// Whether to retrieve all parameters under the path recursively.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,

    /// Whether to decrypt `SecureString` values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_decryption: Option<bool>,

    /// Filters for the results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_filters: Vec<ParameterStringFilter>,

    /// The maximum number of results per page (default 10, max 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,

    /// The token for the next set of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteParameter
// ---------------------------------------------------------------------------

/// Input for the `DeleteParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteParameterInput {
    /// The name of the parameter to delete.
    pub name: String,
}

// ---------------------------------------------------------------------------
// DeleteParameters
// ---------------------------------------------------------------------------

/// Input for the `DeleteParameters` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteParametersInput {
    /// The names of the parameters to delete (max 10).
    #[serde(default)]
    pub names: Vec<String>,
}
