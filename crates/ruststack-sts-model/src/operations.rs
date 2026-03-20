//! Auto-generated from AWS STS Smithy model. DO NOT EDIT.

/// All supported Sts operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StsOperation {
    /// The GetCallerIdentity operation.
    GetCallerIdentity,
    /// The AssumeRole operation.
    AssumeRole,
    /// The GetSessionToken operation.
    GetSessionToken,
    /// The GetAccessKeyInfo operation.
    GetAccessKeyInfo,
    /// The AssumeRoleWithSAML operation.
    AssumeRoleWithSAML,
    /// The AssumeRoleWithWebIdentity operation.
    AssumeRoleWithWebIdentity,
    /// The DecodeAuthorizationMessage operation.
    DecodeAuthorizationMessage,
    /// The GetFederationToken operation.
    GetFederationToken,
}

impl StsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GetCallerIdentity => "GetCallerIdentity",
            Self::AssumeRole => "AssumeRole",
            Self::GetSessionToken => "GetSessionToken",
            Self::GetAccessKeyInfo => "GetAccessKeyInfo",
            Self::AssumeRoleWithSAML => "AssumeRoleWithSAML",
            Self::AssumeRoleWithWebIdentity => "AssumeRoleWithWebIdentity",
            Self::DecodeAuthorizationMessage => "DecodeAuthorizationMessage",
            Self::GetFederationToken => "GetFederationToken",
        }
    }

    /// Parse an operation name string into an StsOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "GetCallerIdentity" => Some(Self::GetCallerIdentity),
            "AssumeRole" => Some(Self::AssumeRole),
            "GetSessionToken" => Some(Self::GetSessionToken),
            "GetAccessKeyInfo" => Some(Self::GetAccessKeyInfo),
            "AssumeRoleWithSAML" => Some(Self::AssumeRoleWithSAML),
            "AssumeRoleWithWebIdentity" => Some(Self::AssumeRoleWithWebIdentity),
            "DecodeAuthorizationMessage" => Some(Self::DecodeAuthorizationMessage),
            "GetFederationToken" => Some(Self::GetFederationToken),
            _ => None,
        }
    }
}

impl std::fmt::Display for StsOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
