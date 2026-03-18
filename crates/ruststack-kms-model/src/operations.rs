//! Auto-generated from AWS KMS Smithy model. DO NOT EDIT.

/// All supported Kms operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KmsOperation {
    /// The CreateKey operation.
    CreateKey,
    /// The DescribeKey operation.
    DescribeKey,
    /// The ListKeys operation.
    ListKeys,
    /// The EnableKey operation.
    EnableKey,
    /// The DisableKey operation.
    DisableKey,
    /// The ScheduleKeyDeletion operation.
    ScheduleKeyDeletion,
    /// The CancelKeyDeletion operation.
    CancelKeyDeletion,
    /// The UpdateKeyDescription operation.
    UpdateKeyDescription,
    /// The Encrypt operation.
    Encrypt,
    /// The Decrypt operation.
    Decrypt,
    /// The ReEncrypt operation.
    ReEncrypt,
    /// The GenerateDataKey operation.
    GenerateDataKey,
    /// The GenerateDataKeyWithoutPlaintext operation.
    GenerateDataKeyWithoutPlaintext,
    /// The GenerateDataKeyPair operation.
    GenerateDataKeyPair,
    /// The GenerateDataKeyPairWithoutPlaintext operation.
    GenerateDataKeyPairWithoutPlaintext,
    /// The Sign operation.
    Sign,
    /// The Verify operation.
    Verify,
    /// The GetPublicKey operation.
    GetPublicKey,
    /// The GenerateMac operation.
    GenerateMac,
    /// The VerifyMac operation.
    VerifyMac,
    /// The GenerateRandom operation.
    GenerateRandom,
    /// The CreateAlias operation.
    CreateAlias,
    /// The DeleteAlias operation.
    DeleteAlias,
    /// The ListAliases operation.
    ListAliases,
    /// The UpdateAlias operation.
    UpdateAlias,
    /// The TagResource operation.
    TagResource,
    /// The UntagResource operation.
    UntagResource,
    /// The ListResourceTags operation.
    ListResourceTags,
    /// The GetKeyPolicy operation.
    GetKeyPolicy,
    /// The PutKeyPolicy operation.
    PutKeyPolicy,
    /// The ListKeyPolicies operation.
    ListKeyPolicies,
    /// The CreateGrant operation.
    CreateGrant,
    /// The ListGrants operation.
    ListGrants,
    /// The RetireGrant operation.
    RetireGrant,
    /// The RevokeGrant operation.
    RevokeGrant,
    /// The ListRetirableGrants operation.
    ListRetirableGrants,
    /// The EnableKeyRotation operation.
    EnableKeyRotation,
    /// The DisableKeyRotation operation.
    DisableKeyRotation,
    /// The GetKeyRotationStatus operation.
    GetKeyRotationStatus,
}

impl KmsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateKey => "CreateKey",
            Self::DescribeKey => "DescribeKey",
            Self::ListKeys => "ListKeys",
            Self::EnableKey => "EnableKey",
            Self::DisableKey => "DisableKey",
            Self::ScheduleKeyDeletion => "ScheduleKeyDeletion",
            Self::CancelKeyDeletion => "CancelKeyDeletion",
            Self::UpdateKeyDescription => "UpdateKeyDescription",
            Self::Encrypt => "Encrypt",
            Self::Decrypt => "Decrypt",
            Self::ReEncrypt => "ReEncrypt",
            Self::GenerateDataKey => "GenerateDataKey",
            Self::GenerateDataKeyWithoutPlaintext => "GenerateDataKeyWithoutPlaintext",
            Self::GenerateDataKeyPair => "GenerateDataKeyPair",
            Self::GenerateDataKeyPairWithoutPlaintext => "GenerateDataKeyPairWithoutPlaintext",
            Self::Sign => "Sign",
            Self::Verify => "Verify",
            Self::GetPublicKey => "GetPublicKey",
            Self::GenerateMac => "GenerateMac",
            Self::VerifyMac => "VerifyMac",
            Self::GenerateRandom => "GenerateRandom",
            Self::CreateAlias => "CreateAlias",
            Self::DeleteAlias => "DeleteAlias",
            Self::ListAliases => "ListAliases",
            Self::UpdateAlias => "UpdateAlias",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListResourceTags => "ListResourceTags",
            Self::GetKeyPolicy => "GetKeyPolicy",
            Self::PutKeyPolicy => "PutKeyPolicy",
            Self::ListKeyPolicies => "ListKeyPolicies",
            Self::CreateGrant => "CreateGrant",
            Self::ListGrants => "ListGrants",
            Self::RetireGrant => "RetireGrant",
            Self::RevokeGrant => "RevokeGrant",
            Self::ListRetirableGrants => "ListRetirableGrants",
            Self::EnableKeyRotation => "EnableKeyRotation",
            Self::DisableKeyRotation => "DisableKeyRotation",
            Self::GetKeyRotationStatus => "GetKeyRotationStatus",
        }
    }

    /// Parse an operation name string into an KmsOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateKey" => Some(Self::CreateKey),
            "DescribeKey" => Some(Self::DescribeKey),
            "ListKeys" => Some(Self::ListKeys),
            "EnableKey" => Some(Self::EnableKey),
            "DisableKey" => Some(Self::DisableKey),
            "ScheduleKeyDeletion" => Some(Self::ScheduleKeyDeletion),
            "CancelKeyDeletion" => Some(Self::CancelKeyDeletion),
            "UpdateKeyDescription" => Some(Self::UpdateKeyDescription),
            "Encrypt" => Some(Self::Encrypt),
            "Decrypt" => Some(Self::Decrypt),
            "ReEncrypt" => Some(Self::ReEncrypt),
            "GenerateDataKey" => Some(Self::GenerateDataKey),
            "GenerateDataKeyWithoutPlaintext" => Some(Self::GenerateDataKeyWithoutPlaintext),
            "GenerateDataKeyPair" => Some(Self::GenerateDataKeyPair),
            "GenerateDataKeyPairWithoutPlaintext" => {
                Some(Self::GenerateDataKeyPairWithoutPlaintext)
            }
            "Sign" => Some(Self::Sign),
            "Verify" => Some(Self::Verify),
            "GetPublicKey" => Some(Self::GetPublicKey),
            "GenerateMac" => Some(Self::GenerateMac),
            "VerifyMac" => Some(Self::VerifyMac),
            "GenerateRandom" => Some(Self::GenerateRandom),
            "CreateAlias" => Some(Self::CreateAlias),
            "DeleteAlias" => Some(Self::DeleteAlias),
            "ListAliases" => Some(Self::ListAliases),
            "UpdateAlias" => Some(Self::UpdateAlias),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "ListResourceTags" => Some(Self::ListResourceTags),
            "GetKeyPolicy" => Some(Self::GetKeyPolicy),
            "PutKeyPolicy" => Some(Self::PutKeyPolicy),
            "ListKeyPolicies" => Some(Self::ListKeyPolicies),
            "CreateGrant" => Some(Self::CreateGrant),
            "ListGrants" => Some(Self::ListGrants),
            "RetireGrant" => Some(Self::RetireGrant),
            "RevokeGrant" => Some(Self::RevokeGrant),
            "ListRetirableGrants" => Some(Self::ListRetirableGrants),
            "EnableKeyRotation" => Some(Self::EnableKeyRotation),
            "DisableKeyRotation" => Some(Self::DisableKeyRotation),
            "GetKeyRotationStatus" => Some(Self::GetKeyRotationStatus),
            _ => None,
        }
    }
}

impl std::fmt::Display for KmsOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
