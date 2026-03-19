//! Auto-generated from AWS Secrets Manager Smithy model. DO NOT EDIT.

/// All supported SecretsManager operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretsManagerOperation {
    /// The CreateSecret operation.
    CreateSecret,
    /// The DescribeSecret operation.
    DescribeSecret,
    /// The GetSecretValue operation.
    GetSecretValue,
    /// The PutSecretValue operation.
    PutSecretValue,
    /// The UpdateSecret operation.
    UpdateSecret,
    /// The DeleteSecret operation.
    DeleteSecret,
    /// The RestoreSecret operation.
    RestoreSecret,
    /// The ListSecrets operation.
    ListSecrets,
    /// The ListSecretVersionIds operation.
    ListSecretVersionIds,
    /// The GetRandomPassword operation.
    GetRandomPassword,
    /// The TagResource operation.
    TagResource,
    /// The UntagResource operation.
    UntagResource,
    /// The UpdateSecretVersionStage operation.
    UpdateSecretVersionStage,
    /// The RotateSecret operation.
    RotateSecret,
    /// The CancelRotateSecret operation.
    CancelRotateSecret,
    /// The BatchGetSecretValue operation.
    BatchGetSecretValue,
    /// The GetResourcePolicy operation.
    GetResourcePolicy,
    /// The PutResourcePolicy operation.
    PutResourcePolicy,
    /// The DeleteResourcePolicy operation.
    DeleteResourcePolicy,
    /// The ValidateResourcePolicy operation.
    ValidateResourcePolicy,
    /// The ReplicateSecretToRegions operation.
    ReplicateSecretToRegions,
    /// The RemoveRegionsFromReplication operation.
    RemoveRegionsFromReplication,
    /// The StopReplicationToReplica operation.
    StopReplicationToReplica,
}

impl SecretsManagerOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateSecret => "CreateSecret",
            Self::DescribeSecret => "DescribeSecret",
            Self::GetSecretValue => "GetSecretValue",
            Self::PutSecretValue => "PutSecretValue",
            Self::UpdateSecret => "UpdateSecret",
            Self::DeleteSecret => "DeleteSecret",
            Self::RestoreSecret => "RestoreSecret",
            Self::ListSecrets => "ListSecrets",
            Self::ListSecretVersionIds => "ListSecretVersionIds",
            Self::GetRandomPassword => "GetRandomPassword",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::UpdateSecretVersionStage => "UpdateSecretVersionStage",
            Self::RotateSecret => "RotateSecret",
            Self::CancelRotateSecret => "CancelRotateSecret",
            Self::BatchGetSecretValue => "BatchGetSecretValue",
            Self::GetResourcePolicy => "GetResourcePolicy",
            Self::PutResourcePolicy => "PutResourcePolicy",
            Self::DeleteResourcePolicy => "DeleteResourcePolicy",
            Self::ValidateResourcePolicy => "ValidateResourcePolicy",
            Self::ReplicateSecretToRegions => "ReplicateSecretToRegions",
            Self::RemoveRegionsFromReplication => "RemoveRegionsFromReplication",
            Self::StopReplicationToReplica => "StopReplicationToReplica",
        }
    }

    /// Parse an operation name string into an SecretsManagerOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateSecret" => Some(Self::CreateSecret),
            "DescribeSecret" => Some(Self::DescribeSecret),
            "GetSecretValue" => Some(Self::GetSecretValue),
            "PutSecretValue" => Some(Self::PutSecretValue),
            "UpdateSecret" => Some(Self::UpdateSecret),
            "DeleteSecret" => Some(Self::DeleteSecret),
            "RestoreSecret" => Some(Self::RestoreSecret),
            "ListSecrets" => Some(Self::ListSecrets),
            "ListSecretVersionIds" => Some(Self::ListSecretVersionIds),
            "GetRandomPassword" => Some(Self::GetRandomPassword),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "UpdateSecretVersionStage" => Some(Self::UpdateSecretVersionStage),
            "RotateSecret" => Some(Self::RotateSecret),
            "CancelRotateSecret" => Some(Self::CancelRotateSecret),
            "BatchGetSecretValue" => Some(Self::BatchGetSecretValue),
            "GetResourcePolicy" => Some(Self::GetResourcePolicy),
            "PutResourcePolicy" => Some(Self::PutResourcePolicy),
            "DeleteResourcePolicy" => Some(Self::DeleteResourcePolicy),
            "ValidateResourcePolicy" => Some(Self::ValidateResourcePolicy),
            "ReplicateSecretToRegions" => Some(Self::ReplicateSecretToRegions),
            "RemoveRegionsFromReplication" => Some(Self::RemoveRegionsFromReplication),
            "StopReplicationToReplica" => Some(Self::StopReplicationToReplica),
            _ => None,
        }
    }
}

impl std::fmt::Display for SecretsManagerOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
