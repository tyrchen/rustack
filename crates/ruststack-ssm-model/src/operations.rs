//! SSM operation enum.

use std::fmt;

/// All supported SSM Parameter Store operations.
///
/// Phase 0 implements 6 core operations. The remaining operations are defined
/// here for forward compatibility but will return "not implemented" until their
/// respective phases are complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SsmOperation {
    // Phase 0: Core parameter CRUD
    /// Create or update a parameter.
    PutParameter,
    /// Get a single parameter by name (supports version/label selectors).
    GetParameter,
    /// Batch get up to 10 parameters by name.
    GetParameters,
    /// Get parameters under a path hierarchy.
    GetParametersByPath,
    /// Delete a single parameter.
    DeleteParameter,
    /// Batch delete up to 10 parameters.
    DeleteParameters,

    // Phase 1: Discovery and tagging
    /// Describe parameters with filtering.
    DescribeParameters,
    /// Get the version history of a parameter.
    GetParameterHistory,
    /// Add tags to a resource.
    AddTagsToResource,
    /// Remove tags from a resource.
    RemoveTagsFromResource,
    /// List tags for a resource.
    ListTagsForResource,

    // Phase 2: Labels
    /// Attach a label to a specific parameter version.
    LabelParameterVersion,
    /// Remove a label from a parameter version.
    UnlabelParameterVersion,
}

impl SsmOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PutParameter => "PutParameter",
            Self::GetParameter => "GetParameter",
            Self::GetParameters => "GetParameters",
            Self::GetParametersByPath => "GetParametersByPath",
            Self::DeleteParameter => "DeleteParameter",
            Self::DeleteParameters => "DeleteParameters",
            Self::DescribeParameters => "DescribeParameters",
            Self::GetParameterHistory => "GetParameterHistory",
            Self::AddTagsToResource => "AddTagsToResource",
            Self::RemoveTagsFromResource => "RemoveTagsFromResource",
            Self::ListTagsForResource => "ListTagsForResource",
            Self::LabelParameterVersion => "LabelParameterVersion",
            Self::UnlabelParameterVersion => "UnlabelParameterVersion",
        }
    }

    /// Parse an operation name string into an `SsmOperation`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "PutParameter" => Some(Self::PutParameter),
            "GetParameter" => Some(Self::GetParameter),
            "GetParameters" => Some(Self::GetParameters),
            "GetParametersByPath" => Some(Self::GetParametersByPath),
            "DeleteParameter" => Some(Self::DeleteParameter),
            "DeleteParameters" => Some(Self::DeleteParameters),
            "DescribeParameters" => Some(Self::DescribeParameters),
            "GetParameterHistory" => Some(Self::GetParameterHistory),
            "AddTagsToResource" => Some(Self::AddTagsToResource),
            "RemoveTagsFromResource" => Some(Self::RemoveTagsFromResource),
            "ListTagsForResource" => Some(Self::ListTagsForResource),
            "LabelParameterVersion" => Some(Self::LabelParameterVersion),
            "UnlabelParameterVersion" => Some(Self::UnlabelParameterVersion),
            _ => None,
        }
    }

    /// Returns `true` if this operation is implemented in Phase 0.
    #[must_use]
    pub fn is_phase0(&self) -> bool {
        matches!(
            self,
            Self::PutParameter
                | Self::GetParameter
                | Self::GetParameters
                | Self::GetParametersByPath
                | Self::DeleteParameter
                | Self::DeleteParameters
        )
    }
}

impl fmt::Display for SsmOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
