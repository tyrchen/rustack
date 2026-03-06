//! SSM output types for Phase 0 operations.
//!
//! All output structs use `PascalCase` JSON field naming to match the SSM
//! wire protocol (`awsJson1_1`). Optional fields are omitted when `None`.

use serde::{Deserialize, Serialize};

use crate::types::Parameter;

// ---------------------------------------------------------------------------
// PutParameter
// ---------------------------------------------------------------------------

/// Output for the `PutParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutParameterOutput {
    /// The version of the parameter.
    pub version: i64,

    /// The tier of the parameter.
    pub tier: String,
}

// ---------------------------------------------------------------------------
// GetParameter
// ---------------------------------------------------------------------------

/// Output for the `GetParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParameterOutput {
    /// The parameter details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Parameter>,
}

// ---------------------------------------------------------------------------
// GetParameters
// ---------------------------------------------------------------------------

/// Output for the `GetParameters` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParametersOutput {
    /// The parameters that were found.
    #[serde(default)]
    pub parameters: Vec<Parameter>,

    /// The names of parameters that could not be found.
    #[serde(default)]
    pub invalid_parameters: Vec<String>,
}

// ---------------------------------------------------------------------------
// GetParametersByPath
// ---------------------------------------------------------------------------

/// Output for the `GetParametersByPath` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetParametersByPathOutput {
    /// The parameters that match the path.
    #[serde(default)]
    pub parameters: Vec<Parameter>,

    /// The token for the next page of results, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteParameter
// ---------------------------------------------------------------------------

/// Output for the `DeleteParameter` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteParameterOutput {}

// ---------------------------------------------------------------------------
// DeleteParameters
// ---------------------------------------------------------------------------

/// Output for the `DeleteParameters` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteParametersOutput {
    /// The names of parameters that were successfully deleted.
    #[serde(default)]
    pub deleted_parameters: Vec<String>,

    /// The names of parameters that could not be found.
    #[serde(default)]
    pub invalid_parameters: Vec<String>,
}
