//! Auto-generated from AWS S3 Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

/// S3 ArchiveStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ArchiveStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "ARCHIVE_ACCESS")]
    ArchiveAccess,
    #[serde(rename = "DEEP_ARCHIVE_ACCESS")]
    DeepArchiveAccess,
}

impl ArchiveStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ArchiveAccess => "ARCHIVE_ACCESS",
            Self::DeepArchiveAccess => "DEEP_ARCHIVE_ACCESS",
        }
    }
}

impl std::fmt::Display for ArchiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ArchiveStatus {
    fn from(s: &str) -> Self {
        match s {
            "ARCHIVE_ACCESS" => Self::ArchiveAccess,
            "DEEP_ARCHIVE_ACCESS" => Self::DeepArchiveAccess,
            _ => Self::default(),
        }
    }
}

/// S3 BucketAccelerateStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketAccelerateStatus {
    /// Default variant.
    #[default]
    Enabled,
    Suspended,
}

impl BucketAccelerateStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Suspended => "Suspended",
        }
    }
}

impl std::fmt::Display for BucketAccelerateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketAccelerateStatus {
    fn from(s: &str) -> Self {
        match s {
            "Enabled" => Self::Enabled,
            "Suspended" => Self::Suspended,
            _ => Self::default(),
        }
    }
}

/// S3 BucketCannedACL enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketCannedACL {
    /// Default variant.
    #[default]
    #[serde(rename = "authenticated-read")]
    AuthenticatedRead,
    #[serde(rename = "private")]
    Private,
    #[serde(rename = "public-read")]
    PublicRead,
    #[serde(rename = "public-read-write")]
    PublicReadWrite,
}

impl BucketCannedACL {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthenticatedRead => "authenticated-read",
            Self::Private => "private",
            Self::PublicRead => "public-read",
            Self::PublicReadWrite => "public-read-write",
        }
    }
}

impl std::fmt::Display for BucketCannedACL {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketCannedACL {
    fn from(s: &str) -> Self {
        match s {
            "authenticated-read" => Self::AuthenticatedRead,
            "private" => Self::Private,
            "public-read" => Self::PublicRead,
            "public-read-write" => Self::PublicReadWrite,
            _ => Self::default(),
        }
    }
}

/// S3 BucketLocationConstraint enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketLocationConstraint {
    /// Default variant.
    #[default]
    #[serde(rename = "af-south-1")]
    AfSouth1,
    #[serde(rename = "ap-east-1")]
    ApEast1,
    #[serde(rename = "ap-northeast-1")]
    ApNortheast1,
    #[serde(rename = "ap-northeast-2")]
    ApNortheast2,
    #[serde(rename = "ap-northeast-3")]
    ApNortheast3,
    #[serde(rename = "ap-south-1")]
    ApSouth1,
    #[serde(rename = "ap-south-2")]
    ApSouth2,
    #[serde(rename = "ap-southeast-1")]
    ApSoutheast1,
    #[serde(rename = "ap-southeast-2")]
    ApSoutheast2,
    #[serde(rename = "ap-southeast-3")]
    ApSoutheast3,
    #[serde(rename = "ap-southeast-4")]
    ApSoutheast4,
    #[serde(rename = "ap-southeast-5")]
    ApSoutheast5,
    #[serde(rename = "ca-central-1")]
    CaCentral1,
    #[serde(rename = "cn-north-1")]
    CnNorth1,
    #[serde(rename = "cn-northwest-1")]
    CnNorthwest1,
    #[serde(rename = "EU")]
    Eu,
    #[serde(rename = "eu-central-1")]
    EuCentral1,
    #[serde(rename = "eu-central-2")]
    EuCentral2,
    #[serde(rename = "eu-north-1")]
    EuNorth1,
    #[serde(rename = "eu-south-1")]
    EuSouth1,
    #[serde(rename = "eu-south-2")]
    EuSouth2,
    #[serde(rename = "eu-west-1")]
    EuWest1,
    #[serde(rename = "eu-west-2")]
    EuWest2,
    #[serde(rename = "eu-west-3")]
    EuWest3,
    #[serde(rename = "il-central-1")]
    IlCentral1,
    #[serde(rename = "me-central-1")]
    MeCentral1,
    #[serde(rename = "me-south-1")]
    MeSouth1,
    #[serde(rename = "sa-east-1")]
    SaEast1,
    #[serde(rename = "us-east-2")]
    UsEast2,
    #[serde(rename = "us-gov-east-1")]
    UsGovEast1,
    #[serde(rename = "us-gov-west-1")]
    UsGovWest1,
    #[serde(rename = "us-west-1")]
    UsWest1,
    #[serde(rename = "us-west-2")]
    UsWest2,
}

impl BucketLocationConstraint {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfSouth1 => "af-south-1",
            Self::ApEast1 => "ap-east-1",
            Self::ApNortheast1 => "ap-northeast-1",
            Self::ApNortheast2 => "ap-northeast-2",
            Self::ApNortheast3 => "ap-northeast-3",
            Self::ApSouth1 => "ap-south-1",
            Self::ApSouth2 => "ap-south-2",
            Self::ApSoutheast1 => "ap-southeast-1",
            Self::ApSoutheast2 => "ap-southeast-2",
            Self::ApSoutheast3 => "ap-southeast-3",
            Self::ApSoutheast4 => "ap-southeast-4",
            Self::ApSoutheast5 => "ap-southeast-5",
            Self::CaCentral1 => "ca-central-1",
            Self::CnNorth1 => "cn-north-1",
            Self::CnNorthwest1 => "cn-northwest-1",
            Self::Eu => "EU",
            Self::EuCentral1 => "eu-central-1",
            Self::EuCentral2 => "eu-central-2",
            Self::EuNorth1 => "eu-north-1",
            Self::EuSouth1 => "eu-south-1",
            Self::EuSouth2 => "eu-south-2",
            Self::EuWest1 => "eu-west-1",
            Self::EuWest2 => "eu-west-2",
            Self::EuWest3 => "eu-west-3",
            Self::IlCentral1 => "il-central-1",
            Self::MeCentral1 => "me-central-1",
            Self::MeSouth1 => "me-south-1",
            Self::SaEast1 => "sa-east-1",
            Self::UsEast2 => "us-east-2",
            Self::UsGovEast1 => "us-gov-east-1",
            Self::UsGovWest1 => "us-gov-west-1",
            Self::UsWest1 => "us-west-1",
            Self::UsWest2 => "us-west-2",
        }
    }
}

impl std::fmt::Display for BucketLocationConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketLocationConstraint {
    fn from(s: &str) -> Self {
        match s {
            "af-south-1" => Self::AfSouth1,
            "ap-east-1" => Self::ApEast1,
            "ap-northeast-1" => Self::ApNortheast1,
            "ap-northeast-2" => Self::ApNortheast2,
            "ap-northeast-3" => Self::ApNortheast3,
            "ap-south-1" => Self::ApSouth1,
            "ap-south-2" => Self::ApSouth2,
            "ap-southeast-1" => Self::ApSoutheast1,
            "ap-southeast-2" => Self::ApSoutheast2,
            "ap-southeast-3" => Self::ApSoutheast3,
            "ap-southeast-4" => Self::ApSoutheast4,
            "ap-southeast-5" => Self::ApSoutheast5,
            "ca-central-1" => Self::CaCentral1,
            "cn-north-1" => Self::CnNorth1,
            "cn-northwest-1" => Self::CnNorthwest1,
            "EU" => Self::Eu,
            "eu-central-1" => Self::EuCentral1,
            "eu-central-2" => Self::EuCentral2,
            "eu-north-1" => Self::EuNorth1,
            "eu-south-1" => Self::EuSouth1,
            "eu-south-2" => Self::EuSouth2,
            "eu-west-1" => Self::EuWest1,
            "eu-west-2" => Self::EuWest2,
            "eu-west-3" => Self::EuWest3,
            "il-central-1" => Self::IlCentral1,
            "me-central-1" => Self::MeCentral1,
            "me-south-1" => Self::MeSouth1,
            "sa-east-1" => Self::SaEast1,
            "us-east-2" => Self::UsEast2,
            "us-gov-east-1" => Self::UsGovEast1,
            "us-gov-west-1" => Self::UsGovWest1,
            "us-west-1" => Self::UsWest1,
            "us-west-2" => Self::UsWest2,
            _ => Self::default(),
        }
    }
}

/// S3 BucketLogsPermission enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketLogsPermission {
    /// Default variant.
    #[default]
    #[serde(rename = "FULL_CONTROL")]
    FullControl,
    #[serde(rename = "READ")]
    Read,
    #[serde(rename = "WRITE")]
    Write,
}

impl BucketLogsPermission {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullControl => "FULL_CONTROL",
            Self::Read => "READ",
            Self::Write => "WRITE",
        }
    }
}

impl std::fmt::Display for BucketLogsPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketLogsPermission {
    fn from(s: &str) -> Self {
        match s {
            "FULL_CONTROL" => Self::FullControl,
            "READ" => Self::Read,
            "WRITE" => Self::Write,
            _ => Self::default(),
        }
    }
}

/// S3 BucketType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketType {
    /// Default variant.
    #[default]
    Directory,
}

impl BucketType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Directory => "Directory",
        }
    }
}

impl std::fmt::Display for BucketType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketType {
    fn from(s: &str) -> Self {
        match s {
            "Directory" => Self::Directory,
            _ => Self::default(),
        }
    }
}

/// S3 BucketVersioningStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BucketVersioningStatus {
    /// Default variant.
    #[default]
    Enabled,
    Suspended,
}

impl BucketVersioningStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Suspended => "Suspended",
        }
    }
}

impl std::fmt::Display for BucketVersioningStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for BucketVersioningStatus {
    fn from(s: &str) -> Self {
        match s {
            "Enabled" => Self::Enabled,
            "Suspended" => Self::Suspended,
            _ => Self::default(),
        }
    }
}

/// S3 ChecksumAlgorithm enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChecksumAlgorithm {
    /// Default variant.
    #[default]
    #[serde(rename = "CRC32")]
    Crc32,
    #[serde(rename = "CRC32C")]
    Crc32c,
    #[serde(rename = "CRC64NVME")]
    Crc64nvme,
    #[serde(rename = "SHA1")]
    Sha1,
    #[serde(rename = "SHA256")]
    Sha256,
}

impl ChecksumAlgorithm {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Crc32 => "CRC32",
            Self::Crc32c => "CRC32C",
            Self::Crc64nvme => "CRC64NVME",
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
        }
    }
}

impl std::fmt::Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ChecksumAlgorithm {
    fn from(s: &str) -> Self {
        match s {
            "CRC32" => Self::Crc32,
            "CRC32C" => Self::Crc32c,
            "CRC64NVME" => Self::Crc64nvme,
            "SHA1" => Self::Sha1,
            "SHA256" => Self::Sha256,
            _ => Self::default(),
        }
    }
}

/// S3 ChecksumMode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChecksumMode {
    /// Default variant.
    #[default]
    #[serde(rename = "ENABLED")]
    Enabled,
}

impl ChecksumMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "ENABLED",
        }
    }
}

impl std::fmt::Display for ChecksumMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ChecksumMode {
    fn from(s: &str) -> Self {
        match s {
            "ENABLED" => Self::Enabled,
            _ => Self::default(),
        }
    }
}

/// S3 ChecksumType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChecksumType {
    /// Default variant.
    #[default]
    #[serde(rename = "COMPOSITE")]
    Composite,
    #[serde(rename = "FULL_OBJECT")]
    FullObject,
}

impl ChecksumType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Composite => "COMPOSITE",
            Self::FullObject => "FULL_OBJECT",
        }
    }
}

impl std::fmt::Display for ChecksumType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ChecksumType {
    fn from(s: &str) -> Self {
        match s {
            "COMPOSITE" => Self::Composite,
            "FULL_OBJECT" => Self::FullObject,
            _ => Self::default(),
        }
    }
}

/// S3 DataRedundancy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataRedundancy {
    /// Default variant.
    #[default]
    SingleAvailabilityZone,
    SingleLocalZone,
}

impl DataRedundancy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SingleAvailabilityZone => "SingleAvailabilityZone",
            Self::SingleLocalZone => "SingleLocalZone",
        }
    }
}

impl std::fmt::Display for DataRedundancy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DataRedundancy {
    fn from(s: &str) -> Self {
        match s {
            "SingleAvailabilityZone" => Self::SingleAvailabilityZone,
            "SingleLocalZone" => Self::SingleLocalZone,
            _ => Self::default(),
        }
    }
}

/// S3 EncodingType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EncodingType {
    /// Default variant.
    #[default]
    #[serde(rename = "url")]
    Url,
}

impl EncodingType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Url => "url",
        }
    }
}

impl std::fmt::Display for EncodingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EncodingType {
    fn from(s: &str) -> Self {
        match s {
            "url" => Self::Url,
            _ => Self::default(),
        }
    }
}

/// S3 EncryptionType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EncryptionType {
    /// Default variant.
    #[default]
    #[serde(rename = "NONE")]
    None,
    #[serde(rename = "SSE-C")]
    SseC,
}

impl EncryptionType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::SseC => "SSE-C",
        }
    }
}

impl std::fmt::Display for EncryptionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EncryptionType {
    fn from(s: &str) -> Self {
        match s {
            "NONE" => Self::None,
            "SSE-C" => Self::SseC,
            _ => Self::default(),
        }
    }
}

/// S3 Event enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Event {
    /// Default variant.
    #[default]
    #[serde(rename = "s3:IntelligentTiering")]
    S3IntelligentTiering,
    #[serde(rename = "s3:LifecycleExpiration:*")]
    S3LifecycleExpiration,
    #[serde(rename = "s3:LifecycleExpiration:Delete")]
    S3LifecycleExpirationDelete,
    #[serde(rename = "s3:LifecycleExpiration:DeleteMarkerCreated")]
    S3LifecycleExpirationDeleteMarkerCreated,
    #[serde(rename = "s3:LifecycleTransition")]
    S3LifecycleTransition,
    #[serde(rename = "s3:ObjectAcl:Put")]
    S3ObjectAclPut,
    #[serde(rename = "s3:ObjectCreated:*")]
    S3ObjectCreated,
    #[serde(rename = "s3:ObjectCreated:CompleteMultipartUpload")]
    S3ObjectCreatedCompleteMultipartUpload,
    #[serde(rename = "s3:ObjectCreated:Copy")]
    S3ObjectCreatedCopy,
    #[serde(rename = "s3:ObjectCreated:Post")]
    S3ObjectCreatedPost,
    #[serde(rename = "s3:ObjectCreated:Put")]
    S3ObjectCreatedPut,
    #[serde(rename = "s3:ObjectRemoved:*")]
    S3ObjectRemoved,
    #[serde(rename = "s3:ObjectRemoved:Delete")]
    S3ObjectRemovedDelete,
    #[serde(rename = "s3:ObjectRemoved:DeleteMarkerCreated")]
    S3ObjectRemovedDeleteMarkerCreated,
    #[serde(rename = "s3:ObjectRestore:*")]
    S3ObjectRestore,
    #[serde(rename = "s3:ObjectRestore:Completed")]
    S3ObjectRestoreCompleted,
    #[serde(rename = "s3:ObjectRestore:Delete")]
    S3ObjectRestoreDelete,
    #[serde(rename = "s3:ObjectRestore:Post")]
    S3ObjectRestorePost,
    #[serde(rename = "s3:ObjectTagging:*")]
    S3ObjectTagging,
    #[serde(rename = "s3:ObjectTagging:Delete")]
    S3ObjectTaggingDelete,
    #[serde(rename = "s3:ObjectTagging:Put")]
    S3ObjectTaggingPut,
    #[serde(rename = "s3:ReducedRedundancyLostObject")]
    S3ReducedRedundancyLostObject,
    #[serde(rename = "s3:Replication:*")]
    S3Replication,
    #[serde(rename = "s3:Replication:OperationFailedReplication")]
    S3ReplicationOperationFailedReplication,
    #[serde(rename = "s3:Replication:OperationMissedThreshold")]
    S3ReplicationOperationMissedThreshold,
    #[serde(rename = "s3:Replication:OperationNotTracked")]
    S3ReplicationOperationNotTracked,
    #[serde(rename = "s3:Replication:OperationReplicatedAfterThreshold")]
    S3ReplicationOperationReplicatedAfterThreshold,
}

impl Event {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::S3IntelligentTiering => "s3:IntelligentTiering",
            Self::S3LifecycleExpiration => "s3:LifecycleExpiration:*",
            Self::S3LifecycleExpirationDelete => "s3:LifecycleExpiration:Delete",
            Self::S3LifecycleExpirationDeleteMarkerCreated => {
                "s3:LifecycleExpiration:DeleteMarkerCreated"
            }
            Self::S3LifecycleTransition => "s3:LifecycleTransition",
            Self::S3ObjectAclPut => "s3:ObjectAcl:Put",
            Self::S3ObjectCreated => "s3:ObjectCreated:*",
            Self::S3ObjectCreatedCompleteMultipartUpload => {
                "s3:ObjectCreated:CompleteMultipartUpload"
            }
            Self::S3ObjectCreatedCopy => "s3:ObjectCreated:Copy",
            Self::S3ObjectCreatedPost => "s3:ObjectCreated:Post",
            Self::S3ObjectCreatedPut => "s3:ObjectCreated:Put",
            Self::S3ObjectRemoved => "s3:ObjectRemoved:*",
            Self::S3ObjectRemovedDelete => "s3:ObjectRemoved:Delete",
            Self::S3ObjectRemovedDeleteMarkerCreated => "s3:ObjectRemoved:DeleteMarkerCreated",
            Self::S3ObjectRestore => "s3:ObjectRestore:*",
            Self::S3ObjectRestoreCompleted => "s3:ObjectRestore:Completed",
            Self::S3ObjectRestoreDelete => "s3:ObjectRestore:Delete",
            Self::S3ObjectRestorePost => "s3:ObjectRestore:Post",
            Self::S3ObjectTagging => "s3:ObjectTagging:*",
            Self::S3ObjectTaggingDelete => "s3:ObjectTagging:Delete",
            Self::S3ObjectTaggingPut => "s3:ObjectTagging:Put",
            Self::S3ReducedRedundancyLostObject => "s3:ReducedRedundancyLostObject",
            Self::S3Replication => "s3:Replication:*",
            Self::S3ReplicationOperationFailedReplication => {
                "s3:Replication:OperationFailedReplication"
            }
            Self::S3ReplicationOperationMissedThreshold => {
                "s3:Replication:OperationMissedThreshold"
            }
            Self::S3ReplicationOperationNotTracked => "s3:Replication:OperationNotTracked",
            Self::S3ReplicationOperationReplicatedAfterThreshold => {
                "s3:Replication:OperationReplicatedAfterThreshold"
            }
        }
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Event {
    fn from(s: &str) -> Self {
        match s {
            "s3:IntelligentTiering" => Self::S3IntelligentTiering,
            "s3:LifecycleExpiration:*" => Self::S3LifecycleExpiration,
            "s3:LifecycleExpiration:Delete" => Self::S3LifecycleExpirationDelete,
            "s3:LifecycleExpiration:DeleteMarkerCreated" => {
                Self::S3LifecycleExpirationDeleteMarkerCreated
            }
            "s3:LifecycleTransition" => Self::S3LifecycleTransition,
            "s3:ObjectAcl:Put" => Self::S3ObjectAclPut,
            "s3:ObjectCreated:*" => Self::S3ObjectCreated,
            "s3:ObjectCreated:CompleteMultipartUpload" => {
                Self::S3ObjectCreatedCompleteMultipartUpload
            }
            "s3:ObjectCreated:Copy" => Self::S3ObjectCreatedCopy,
            "s3:ObjectCreated:Post" => Self::S3ObjectCreatedPost,
            "s3:ObjectCreated:Put" => Self::S3ObjectCreatedPut,
            "s3:ObjectRemoved:*" => Self::S3ObjectRemoved,
            "s3:ObjectRemoved:Delete" => Self::S3ObjectRemovedDelete,
            "s3:ObjectRemoved:DeleteMarkerCreated" => Self::S3ObjectRemovedDeleteMarkerCreated,
            "s3:ObjectRestore:*" => Self::S3ObjectRestore,
            "s3:ObjectRestore:Completed" => Self::S3ObjectRestoreCompleted,
            "s3:ObjectRestore:Delete" => Self::S3ObjectRestoreDelete,
            "s3:ObjectRestore:Post" => Self::S3ObjectRestorePost,
            "s3:ObjectTagging:*" => Self::S3ObjectTagging,
            "s3:ObjectTagging:Delete" => Self::S3ObjectTaggingDelete,
            "s3:ObjectTagging:Put" => Self::S3ObjectTaggingPut,
            "s3:ReducedRedundancyLostObject" => Self::S3ReducedRedundancyLostObject,
            "s3:Replication:*" => Self::S3Replication,
            "s3:Replication:OperationFailedReplication" => {
                Self::S3ReplicationOperationFailedReplication
            }
            "s3:Replication:OperationMissedThreshold" => {
                Self::S3ReplicationOperationMissedThreshold
            }
            "s3:Replication:OperationNotTracked" => Self::S3ReplicationOperationNotTracked,
            "s3:Replication:OperationReplicatedAfterThreshold" => {
                Self::S3ReplicationOperationReplicatedAfterThreshold
            }
            _ => Self::default(),
        }
    }
}

/// S3 ExpirationStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExpirationStatus {
    /// Default variant.
    #[default]
    Disabled,
    Enabled,
}

impl ExpirationStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
        }
    }
}

impl std::fmt::Display for ExpirationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExpirationStatus {
    fn from(s: &str) -> Self {
        match s {
            "Disabled" => Self::Disabled,
            "Enabled" => Self::Enabled,
            _ => Self::default(),
        }
    }
}

/// S3 FilterRuleName enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FilterRuleName {
    /// Default variant.
    #[default]
    #[serde(rename = "prefix")]
    Prefix,
    #[serde(rename = "suffix")]
    Suffix,
}

impl FilterRuleName {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
        }
    }
}

impl std::fmt::Display for FilterRuleName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for FilterRuleName {
    fn from(s: &str) -> Self {
        match s {
            "prefix" => Self::Prefix,
            "suffix" => Self::Suffix,
            _ => Self::default(),
        }
    }
}

/// S3 LocationType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LocationType {
    /// Default variant.
    #[default]
    AvailabilityZone,
    LocalZone,
}

impl LocationType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AvailabilityZone => "AvailabilityZone",
            Self::LocalZone => "LocalZone",
        }
    }
}

impl std::fmt::Display for LocationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for LocationType {
    fn from(s: &str) -> Self {
        match s {
            "AvailabilityZone" => Self::AvailabilityZone,
            "LocalZone" => Self::LocalZone,
            _ => Self::default(),
        }
    }
}

/// S3 MFADelete enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MFADelete {
    /// Default variant.
    #[default]
    Disabled,
    Enabled,
}

impl MFADelete {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
        }
    }
}

impl std::fmt::Display for MFADelete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MFADelete {
    fn from(s: &str) -> Self {
        match s {
            "Disabled" => Self::Disabled,
            "Enabled" => Self::Enabled,
            _ => Self::default(),
        }
    }
}

/// S3 MFADeleteStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MFADeleteStatus {
    /// Default variant.
    #[default]
    Disabled,
    Enabled,
}

impl MFADeleteStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
        }
    }
}

impl std::fmt::Display for MFADeleteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MFADeleteStatus {
    fn from(s: &str) -> Self {
        match s {
            "Disabled" => Self::Disabled,
            "Enabled" => Self::Enabled,
            _ => Self::default(),
        }
    }
}

/// S3 MetadataDirective enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MetadataDirective {
    /// Default variant.
    #[default]
    #[serde(rename = "COPY")]
    Copy,
    #[serde(rename = "REPLACE")]
    Replace,
}

impl MetadataDirective {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Copy => "COPY",
            Self::Replace => "REPLACE",
        }
    }
}

impl std::fmt::Display for MetadataDirective {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MetadataDirective {
    fn from(s: &str) -> Self {
        match s {
            "COPY" => Self::Copy,
            "REPLACE" => Self::Replace,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectAttributes enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectAttributes {
    /// Default variant.
    #[default]
    Checksum,
    #[serde(rename = "ETag")]
    Etag,
    ObjectParts,
    ObjectSize,
    StorageClass,
}

impl ObjectAttributes {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Checksum => "Checksum",
            Self::Etag => "ETag",
            Self::ObjectParts => "ObjectParts",
            Self::ObjectSize => "ObjectSize",
            Self::StorageClass => "StorageClass",
        }
    }
}

impl std::fmt::Display for ObjectAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectAttributes {
    fn from(s: &str) -> Self {
        match s {
            "Checksum" => Self::Checksum,
            "ETag" => Self::Etag,
            "ObjectParts" => Self::ObjectParts,
            "ObjectSize" => Self::ObjectSize,
            "StorageClass" => Self::StorageClass,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectCannedACL enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectCannedACL {
    /// Default variant.
    #[default]
    #[serde(rename = "authenticated-read")]
    AuthenticatedRead,
    #[serde(rename = "aws-exec-read")]
    AwsExecRead,
    #[serde(rename = "bucket-owner-full-control")]
    BucketOwnerFullControl,
    #[serde(rename = "bucket-owner-read")]
    BucketOwnerRead,
    #[serde(rename = "private")]
    Private,
    #[serde(rename = "public-read")]
    PublicRead,
    #[serde(rename = "public-read-write")]
    PublicReadWrite,
}

impl ObjectCannedACL {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthenticatedRead => "authenticated-read",
            Self::AwsExecRead => "aws-exec-read",
            Self::BucketOwnerFullControl => "bucket-owner-full-control",
            Self::BucketOwnerRead => "bucket-owner-read",
            Self::Private => "private",
            Self::PublicRead => "public-read",
            Self::PublicReadWrite => "public-read-write",
        }
    }
}

impl std::fmt::Display for ObjectCannedACL {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectCannedACL {
    fn from(s: &str) -> Self {
        match s {
            "authenticated-read" => Self::AuthenticatedRead,
            "aws-exec-read" => Self::AwsExecRead,
            "bucket-owner-full-control" => Self::BucketOwnerFullControl,
            "bucket-owner-read" => Self::BucketOwnerRead,
            "private" => Self::Private,
            "public-read" => Self::PublicRead,
            "public-read-write" => Self::PublicReadWrite,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectLockEnabled enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectLockEnabled {
    /// Default variant.
    #[default]
    Enabled,
}

impl ObjectLockEnabled {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
        }
    }
}

impl std::fmt::Display for ObjectLockEnabled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectLockEnabled {
    fn from(s: &str) -> Self {
        match s {
            "Enabled" => Self::Enabled,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectLockLegalHoldStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectLockLegalHoldStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "OFF")]
    Off,
    #[serde(rename = "ON")]
    On,
}

impl ObjectLockLegalHoldStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
        }
    }
}

impl std::fmt::Display for ObjectLockLegalHoldStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectLockLegalHoldStatus {
    fn from(s: &str) -> Self {
        match s {
            "OFF" => Self::Off,
            "ON" => Self::On,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectLockMode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectLockMode {
    /// Default variant.
    #[default]
    #[serde(rename = "COMPLIANCE")]
    Compliance,
    #[serde(rename = "GOVERNANCE")]
    Governance,
}

impl ObjectLockMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compliance => "COMPLIANCE",
            Self::Governance => "GOVERNANCE",
        }
    }
}

impl std::fmt::Display for ObjectLockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectLockMode {
    fn from(s: &str) -> Self {
        match s {
            "COMPLIANCE" => Self::Compliance,
            "GOVERNANCE" => Self::Governance,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectLockRetentionMode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectLockRetentionMode {
    /// Default variant.
    #[default]
    #[serde(rename = "COMPLIANCE")]
    Compliance,
    #[serde(rename = "GOVERNANCE")]
    Governance,
}

impl ObjectLockRetentionMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compliance => "COMPLIANCE",
            Self::Governance => "GOVERNANCE",
        }
    }
}

impl std::fmt::Display for ObjectLockRetentionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectLockRetentionMode {
    fn from(s: &str) -> Self {
        match s {
            "COMPLIANCE" => Self::Compliance,
            "GOVERNANCE" => Self::Governance,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectOwnership enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectOwnership {
    /// Default variant.
    #[default]
    BucketOwnerEnforced,
    BucketOwnerPreferred,
    ObjectWriter,
}

impl ObjectOwnership {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BucketOwnerEnforced => "BucketOwnerEnforced",
            Self::BucketOwnerPreferred => "BucketOwnerPreferred",
            Self::ObjectWriter => "ObjectWriter",
        }
    }
}

impl std::fmt::Display for ObjectOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectOwnership {
    fn from(s: &str) -> Self {
        match s {
            "BucketOwnerEnforced" => Self::BucketOwnerEnforced,
            "BucketOwnerPreferred" => Self::BucketOwnerPreferred,
            "ObjectWriter" => Self::ObjectWriter,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectStorageClass enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectStorageClass {
    /// Default variant.
    #[default]
    #[serde(rename = "DEEP_ARCHIVE")]
    DeepArchive,
    #[serde(rename = "EXPRESS_ONEZONE")]
    ExpressOnezone,
    #[serde(rename = "FSX_ONTAP")]
    FsxOntap,
    #[serde(rename = "FSX_OPENZFS")]
    FsxOpenzfs,
    #[serde(rename = "GLACIER")]
    Glacier,
    #[serde(rename = "GLACIER_IR")]
    GlacierIr,
    #[serde(rename = "INTELLIGENT_TIERING")]
    IntelligentTiering,
    #[serde(rename = "ONEZONE_IA")]
    OnezoneIa,
    #[serde(rename = "OUTPOSTS")]
    Outposts,
    #[serde(rename = "REDUCED_REDUNDANCY")]
    ReducedRedundancy,
    #[serde(rename = "SNOW")]
    Snow,
    #[serde(rename = "STANDARD")]
    Standard,
    #[serde(rename = "STANDARD_IA")]
    StandardIa,
}

impl ObjectStorageClass {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepArchive => "DEEP_ARCHIVE",
            Self::ExpressOnezone => "EXPRESS_ONEZONE",
            Self::FsxOntap => "FSX_ONTAP",
            Self::FsxOpenzfs => "FSX_OPENZFS",
            Self::Glacier => "GLACIER",
            Self::GlacierIr => "GLACIER_IR",
            Self::IntelligentTiering => "INTELLIGENT_TIERING",
            Self::OnezoneIa => "ONEZONE_IA",
            Self::Outposts => "OUTPOSTS",
            Self::ReducedRedundancy => "REDUCED_REDUNDANCY",
            Self::Snow => "SNOW",
            Self::Standard => "STANDARD",
            Self::StandardIa => "STANDARD_IA",
        }
    }
}

impl std::fmt::Display for ObjectStorageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectStorageClass {
    fn from(s: &str) -> Self {
        match s {
            "DEEP_ARCHIVE" => Self::DeepArchive,
            "EXPRESS_ONEZONE" => Self::ExpressOnezone,
            "FSX_ONTAP" => Self::FsxOntap,
            "FSX_OPENZFS" => Self::FsxOpenzfs,
            "GLACIER" => Self::Glacier,
            "GLACIER_IR" => Self::GlacierIr,
            "INTELLIGENT_TIERING" => Self::IntelligentTiering,
            "ONEZONE_IA" => Self::OnezoneIa,
            "OUTPOSTS" => Self::Outposts,
            "REDUCED_REDUNDANCY" => Self::ReducedRedundancy,
            "SNOW" => Self::Snow,
            "STANDARD" => Self::Standard,
            "STANDARD_IA" => Self::StandardIa,
            _ => Self::default(),
        }
    }
}

/// S3 ObjectVersionStorageClass enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ObjectVersionStorageClass {
    /// Default variant.
    #[default]
    #[serde(rename = "STANDARD")]
    Standard,
}

impl ObjectVersionStorageClass {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "STANDARD",
        }
    }
}

impl std::fmt::Display for ObjectVersionStorageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObjectVersionStorageClass {
    fn from(s: &str) -> Self {
        match s {
            "STANDARD" => Self::Standard,
            _ => Self::default(),
        }
    }
}

/// S3 OptionalObjectAttributes enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OptionalObjectAttributes {
    /// Default variant.
    #[default]
    RestoreStatus,
}

impl OptionalObjectAttributes {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RestoreStatus => "RestoreStatus",
        }
    }
}

impl std::fmt::Display for OptionalObjectAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for OptionalObjectAttributes {
    fn from(s: &str) -> Self {
        match s {
            "RestoreStatus" => Self::RestoreStatus,
            _ => Self::default(),
        }
    }
}

/// S3 PartitionDateSource enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PartitionDateSource {
    /// Default variant.
    #[default]
    DeliveryTime,
    EventTime,
}

impl PartitionDateSource {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeliveryTime => "DeliveryTime",
            Self::EventTime => "EventTime",
        }
    }
}

impl std::fmt::Display for PartitionDateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for PartitionDateSource {
    fn from(s: &str) -> Self {
        match s {
            "DeliveryTime" => Self::DeliveryTime,
            "EventTime" => Self::EventTime,
            _ => Self::default(),
        }
    }
}

/// S3 Payer enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Payer {
    /// Default variant.
    #[default]
    BucketOwner,
    Requester,
}

impl Payer {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BucketOwner => "BucketOwner",
            Self::Requester => "Requester",
        }
    }
}

impl std::fmt::Display for Payer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Payer {
    fn from(s: &str) -> Self {
        match s {
            "BucketOwner" => Self::BucketOwner,
            "Requester" => Self::Requester,
            _ => Self::default(),
        }
    }
}

/// S3 Permission enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Permission {
    /// Default variant.
    #[default]
    #[serde(rename = "FULL_CONTROL")]
    FullControl,
    #[serde(rename = "READ")]
    Read,
    #[serde(rename = "READ_ACP")]
    ReadAcp,
    #[serde(rename = "WRITE")]
    Write,
    #[serde(rename = "WRITE_ACP")]
    WriteAcp,
}

impl Permission {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullControl => "FULL_CONTROL",
            Self::Read => "READ",
            Self::ReadAcp => "READ_ACP",
            Self::Write => "WRITE",
            Self::WriteAcp => "WRITE_ACP",
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Permission {
    fn from(s: &str) -> Self {
        match s {
            "FULL_CONTROL" => Self::FullControl,
            "READ" => Self::Read,
            "READ_ACP" => Self::ReadAcp,
            "WRITE" => Self::Write,
            "WRITE_ACP" => Self::WriteAcp,
            _ => Self::default(),
        }
    }
}

/// S3 Protocol enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Protocol {
    /// Default variant.
    #[default]
    #[serde(rename = "http")]
    Http,
    #[serde(rename = "https")]
    Https,
}

impl Protocol {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Protocol {
    fn from(s: &str) -> Self {
        match s {
            "http" => Self::Http,
            "https" => Self::Https,
            _ => Self::default(),
        }
    }
}

/// S3 ReplicationStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ReplicationStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "COMPLETE")]
    Complete,
    #[serde(rename = "COMPLETED")]
    Completed,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "REPLICA")]
    Replica,
}

impl ReplicationStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "COMPLETE",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Pending => "PENDING",
            Self::Replica => "REPLICA",
        }
    }
}

impl std::fmt::Display for ReplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ReplicationStatus {
    fn from(s: &str) -> Self {
        match s {
            "COMPLETE" => Self::Complete,
            "COMPLETED" => Self::Completed,
            "FAILED" => Self::Failed,
            "PENDING" => Self::Pending,
            "REPLICA" => Self::Replica,
            _ => Self::default(),
        }
    }
}

/// S3 RequestCharged enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RequestCharged {
    /// Default variant.
    #[default]
    #[serde(rename = "requester")]
    Requester,
}

impl RequestCharged {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Requester => "requester",
        }
    }
}

impl std::fmt::Display for RequestCharged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RequestCharged {
    fn from(s: &str) -> Self {
        match s {
            "requester" => Self::Requester,
            _ => Self::default(),
        }
    }
}

/// S3 RequestPayer enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RequestPayer {
    /// Default variant.
    #[default]
    #[serde(rename = "requester")]
    Requester,
}

impl RequestPayer {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Requester => "requester",
        }
    }
}

impl std::fmt::Display for RequestPayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RequestPayer {
    fn from(s: &str) -> Self {
        match s {
            "requester" => Self::Requester,
            _ => Self::default(),
        }
    }
}

/// S3 ServerSideEncryption enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ServerSideEncryption {
    /// Default variant.
    #[default]
    #[serde(rename = "AES256")]
    Aes256,
    #[serde(rename = "aws:fsx")]
    AwsFsx,
    #[serde(rename = "aws:kms")]
    AwsKms,
    #[serde(rename = "aws:kms:dsse")]
    AwsKmsDsse,
}

impl ServerSideEncryption {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aes256 => "AES256",
            Self::AwsFsx => "aws:fsx",
            Self::AwsKms => "aws:kms",
            Self::AwsKmsDsse => "aws:kms:dsse",
        }
    }
}

impl std::fmt::Display for ServerSideEncryption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ServerSideEncryption {
    fn from(s: &str) -> Self {
        match s {
            "AES256" => Self::Aes256,
            "aws:fsx" => Self::AwsFsx,
            "aws:kms" => Self::AwsKms,
            "aws:kms:dsse" => Self::AwsKmsDsse,
            _ => Self::default(),
        }
    }
}

/// S3 StorageClass enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StorageClass {
    /// Default variant.
    #[default]
    #[serde(rename = "DEEP_ARCHIVE")]
    DeepArchive,
    #[serde(rename = "EXPRESS_ONEZONE")]
    ExpressOnezone,
    #[serde(rename = "FSX_ONTAP")]
    FsxOntap,
    #[serde(rename = "FSX_OPENZFS")]
    FsxOpenzfs,
    #[serde(rename = "GLACIER")]
    Glacier,
    #[serde(rename = "GLACIER_IR")]
    GlacierIr,
    #[serde(rename = "INTELLIGENT_TIERING")]
    IntelligentTiering,
    #[serde(rename = "ONEZONE_IA")]
    OnezoneIa,
    #[serde(rename = "OUTPOSTS")]
    Outposts,
    #[serde(rename = "REDUCED_REDUNDANCY")]
    ReducedRedundancy,
    #[serde(rename = "SNOW")]
    Snow,
    #[serde(rename = "STANDARD")]
    Standard,
    #[serde(rename = "STANDARD_IA")]
    StandardIa,
}

impl StorageClass {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepArchive => "DEEP_ARCHIVE",
            Self::ExpressOnezone => "EXPRESS_ONEZONE",
            Self::FsxOntap => "FSX_ONTAP",
            Self::FsxOpenzfs => "FSX_OPENZFS",
            Self::Glacier => "GLACIER",
            Self::GlacierIr => "GLACIER_IR",
            Self::IntelligentTiering => "INTELLIGENT_TIERING",
            Self::OnezoneIa => "ONEZONE_IA",
            Self::Outposts => "OUTPOSTS",
            Self::ReducedRedundancy => "REDUCED_REDUNDANCY",
            Self::Snow => "SNOW",
            Self::Standard => "STANDARD",
            Self::StandardIa => "STANDARD_IA",
        }
    }
}

impl std::fmt::Display for StorageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StorageClass {
    fn from(s: &str) -> Self {
        match s {
            "DEEP_ARCHIVE" => Self::DeepArchive,
            "EXPRESS_ONEZONE" => Self::ExpressOnezone,
            "FSX_ONTAP" => Self::FsxOntap,
            "FSX_OPENZFS" => Self::FsxOpenzfs,
            "GLACIER" => Self::Glacier,
            "GLACIER_IR" => Self::GlacierIr,
            "INTELLIGENT_TIERING" => Self::IntelligentTiering,
            "ONEZONE_IA" => Self::OnezoneIa,
            "OUTPOSTS" => Self::Outposts,
            "REDUCED_REDUNDANCY" => Self::ReducedRedundancy,
            "SNOW" => Self::Snow,
            "STANDARD" => Self::Standard,
            "STANDARD_IA" => Self::StandardIa,
            _ => Self::default(),
        }
    }
}

/// S3 TaggingDirective enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TaggingDirective {
    /// Default variant.
    #[default]
    #[serde(rename = "COPY")]
    Copy,
    #[serde(rename = "REPLACE")]
    Replace,
}

impl TaggingDirective {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Copy => "COPY",
            Self::Replace => "REPLACE",
        }
    }
}

impl std::fmt::Display for TaggingDirective {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for TaggingDirective {
    fn from(s: &str) -> Self {
        match s {
            "COPY" => Self::Copy,
            "REPLACE" => Self::Replace,
            _ => Self::default(),
        }
    }
}

/// S3 TransitionDefaultMinimumObjectSize enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TransitionDefaultMinimumObjectSize {
    /// Default variant.
    #[default]
    #[serde(rename = "all_storage_classes_128K")]
    AllStorageClasses128k,
    #[serde(rename = "varies_by_storage_class")]
    VariesByStorageClass,
}

impl TransitionDefaultMinimumObjectSize {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AllStorageClasses128k => "all_storage_classes_128K",
            Self::VariesByStorageClass => "varies_by_storage_class",
        }
    }
}

impl std::fmt::Display for TransitionDefaultMinimumObjectSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for TransitionDefaultMinimumObjectSize {
    fn from(s: &str) -> Self {
        match s {
            "all_storage_classes_128K" => Self::AllStorageClasses128k,
            "varies_by_storage_class" => Self::VariesByStorageClass,
            _ => Self::default(),
        }
    }
}

/// S3 TransitionStorageClass enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TransitionStorageClass {
    /// Default variant.
    #[default]
    #[serde(rename = "DEEP_ARCHIVE")]
    DeepArchive,
    #[serde(rename = "GLACIER")]
    Glacier,
    #[serde(rename = "GLACIER_IR")]
    GlacierIr,
    #[serde(rename = "INTELLIGENT_TIERING")]
    IntelligentTiering,
    #[serde(rename = "ONEZONE_IA")]
    OnezoneIa,
    #[serde(rename = "STANDARD_IA")]
    StandardIa,
}

impl TransitionStorageClass {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepArchive => "DEEP_ARCHIVE",
            Self::Glacier => "GLACIER",
            Self::GlacierIr => "GLACIER_IR",
            Self::IntelligentTiering => "INTELLIGENT_TIERING",
            Self::OnezoneIa => "ONEZONE_IA",
            Self::StandardIa => "STANDARD_IA",
        }
    }
}

impl std::fmt::Display for TransitionStorageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for TransitionStorageClass {
    fn from(s: &str) -> Self {
        match s {
            "DEEP_ARCHIVE" => Self::DeepArchive,
            "GLACIER" => Self::Glacier,
            "GLACIER_IR" => Self::GlacierIr,
            "INTELLIGENT_TIERING" => Self::IntelligentTiering,
            "ONEZONE_IA" => Self::OnezoneIa,
            "STANDARD_IA" => Self::StandardIa,
            _ => Self::default(),
        }
    }
}

/// S3 Type enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Type {
    /// Default variant.
    #[default]
    AmazonCustomerByEmail,
    CanonicalUser,
    Group,
}

impl Type {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AmazonCustomerByEmail => "AmazonCustomerByEmail",
            Self::CanonicalUser => "CanonicalUser",
            Self::Group => "Group",
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Type {
    fn from(s: &str) -> Self {
        match s {
            "AmazonCustomerByEmail" => Self::AmazonCustomerByEmail,
            "CanonicalUser" => Self::CanonicalUser,
            "Group" => Self::Group,
            _ => Self::default(),
        }
    }
}

/// S3 AbortIncompleteMultipartUpload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AbortIncompleteMultipartUpload {
    pub days_after_initiation: Option<i32>,
}

/// S3 AccelerateConfiguration.
#[derive(Debug, Clone, Default)]
pub struct AccelerateConfiguration {
    pub status: Option<BucketAccelerateStatus>,
}

/// S3 AccessControlPolicy.
#[derive(Debug, Clone, Default)]
pub struct AccessControlPolicy {
    pub grants: Vec<Grant>,
    pub owner: Option<Owner>,
}

/// S3 BlockedEncryptionTypes.
#[derive(Debug, Clone, Default)]
pub struct BlockedEncryptionTypes {
    pub encryption_type: Vec<EncryptionType>,
}

/// S3 Bucket.
#[derive(Debug, Clone, Default)]
pub struct Bucket {
    pub bucket_arn: Option<String>,
    pub bucket_region: Option<String>,
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    pub name: Option<String>,
}

/// S3 BucketInfo.
#[derive(Debug, Clone, Default)]
pub struct BucketInfo {
    pub data_redundancy: Option<DataRedundancy>,
    pub r#type: Option<BucketType>,
}

/// S3 BucketLifecycleConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BucketLifecycleConfiguration {
    pub rules: Vec<LifecycleRule>,
}

/// S3 BucketLoggingStatus.
#[derive(Debug, Clone, Default)]
pub struct BucketLoggingStatus {
    pub logging_enabled: Option<LoggingEnabled>,
}

/// S3 CORSConfiguration.
#[derive(Debug, Clone, Default)]
pub struct CORSConfiguration {
    pub cors_rules: Vec<CORSRule>,
}

/// S3 CORSRule.
#[derive(Debug, Clone, Default)]
pub struct CORSRule {
    pub allowed_headers: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub expose_headers: Vec<String>,
    pub id: Option<String>,
    pub max_age_seconds: Option<i32>,
}

/// S3 Checksum.
#[derive(Debug, Clone, Default)]
pub struct Checksum {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub checksum_type: Option<ChecksumType>,
}

/// S3 CommonPrefix.
#[derive(Debug, Clone, Default)]
pub struct CommonPrefix {
    pub prefix: Option<String>,
}

/// S3 CompletedMultipartUpload.
#[derive(Debug, Clone, Default)]
pub struct CompletedMultipartUpload {
    pub parts: Vec<CompletedPart>,
}

/// S3 CompletedPart.
#[derive(Debug, Clone, Default)]
pub struct CompletedPart {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub e_tag: Option<String>,
    pub part_number: Option<i32>,
}

/// S3 Condition.
#[derive(Debug, Clone, Default)]
pub struct Condition {
    pub http_error_code_returned_equals: Option<String>,
    pub key_prefix_equals: Option<String>,
}

/// S3 CopyObjectResult.
#[derive(Debug, Clone, Default)]
pub struct CopyObjectResult {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub checksum_type: Option<ChecksumType>,
    pub e_tag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// S3 CopyPartResult.
#[derive(Debug, Clone, Default)]
pub struct CopyPartResult {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub e_tag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// S3 CreateBucketConfiguration.
#[derive(Debug, Clone, Default)]
pub struct CreateBucketConfiguration {
    pub bucket: Option<BucketInfo>,
    pub location: Option<LocationInfo>,
    pub location_constraint: Option<BucketLocationConstraint>,
    pub tags: Vec<Tag>,
}

/// S3 DefaultRetention.
#[derive(Debug, Clone, Default)]
pub struct DefaultRetention {
    pub days: Option<i32>,
    pub mode: Option<ObjectLockRetentionMode>,
    pub years: Option<i32>,
}

/// S3 Delete.
#[derive(Debug, Clone, Default)]
pub struct Delete {
    pub objects: Vec<ObjectIdentifier>,
    pub quiet: Option<bool>,
}

/// S3 DeleteMarkerEntry.
#[derive(Debug, Clone, Default)]
pub struct DeleteMarkerEntry {
    pub is_latest: Option<bool>,
    pub key: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub owner: Option<Owner>,
    pub version_id: Option<String>,
}

/// S3 DeletedObject.
#[derive(Debug, Clone, Default)]
pub struct DeletedObject {
    pub delete_marker: Option<bool>,
    pub delete_marker_version_id: Option<String>,
    pub key: Option<String>,
    pub version_id: Option<String>,
}

/// S3 Error.
#[derive(Debug, Clone, Default)]
pub struct Error {
    pub code: Option<String>,
    pub key: Option<String>,
    pub message: Option<String>,
    pub version_id: Option<String>,
}

/// S3 ErrorDocument.
#[derive(Debug, Clone, Default)]
pub struct ErrorDocument {
    pub key: String,
}

/// S3 EventBridgeConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventBridgeConfiguration {}

/// S3 FilterRule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterRule {
    pub name: Option<FilterRuleName>,
    pub value: Option<String>,
}

/// S3 GetObjectAttributesParts.
#[derive(Debug, Clone, Default)]
pub struct GetObjectAttributesParts {
    pub is_truncated: Option<bool>,
    pub max_parts: Option<i32>,
    pub next_part_number_marker: Option<String>,
    pub part_number_marker: Option<String>,
    pub parts: Vec<ObjectPart>,
    pub total_parts_count: Option<i32>,
}

/// S3 Grant.
#[derive(Debug, Clone, Default)]
pub struct Grant {
    pub grantee: Option<Grantee>,
    pub permission: Option<Permission>,
}

/// S3 Grantee.
#[derive(Debug, Clone, Default)]
pub struct Grantee {
    pub display_name: Option<String>,
    pub email_address: Option<String>,
    pub id: Option<String>,
    pub r#type: Type,
    pub uri: Option<String>,
}

/// S3 IndexDocument.
#[derive(Debug, Clone, Default)]
pub struct IndexDocument {
    pub suffix: String,
}

/// S3 Initiator.
#[derive(Debug, Clone, Default)]
pub struct Initiator {
    pub display_name: Option<String>,
    pub id: Option<String>,
}

/// S3 LambdaFunctionConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LambdaFunctionConfiguration {
    pub events: Vec<Event>,
    pub filter: Option<NotificationConfigurationFilter>,
    pub id: Option<String>,
    pub lambda_function_arn: String,
}

/// S3 LifecycleExpiration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleExpiration {
    pub date: Option<chrono::DateTime<chrono::Utc>>,
    pub days: Option<i32>,
    pub expired_object_delete_marker: Option<bool>,
}

/// S3 LifecycleRule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleRule {
    pub abort_incomplete_multipart_upload: Option<AbortIncompleteMultipartUpload>,
    pub expiration: Option<LifecycleExpiration>,
    pub filter: Option<LifecycleRuleFilter>,
    pub id: Option<String>,
    pub noncurrent_version_expiration: Option<NoncurrentVersionExpiration>,
    pub noncurrent_version_transitions: Vec<NoncurrentVersionTransition>,
    pub prefix: Option<String>,
    pub status: ExpirationStatus,
    pub transitions: Vec<Transition>,
}

/// S3 LifecycleRuleAndOperator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleRuleAndOperator {
    pub object_size_greater_than: Option<i64>,
    pub object_size_less_than: Option<i64>,
    pub prefix: Option<String>,
    pub tags: Vec<Tag>,
}

/// S3 LifecycleRuleFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleRuleFilter {
    pub and: Option<LifecycleRuleAndOperator>,
    pub object_size_greater_than: Option<i64>,
    pub object_size_less_than: Option<i64>,
    pub prefix: Option<String>,
    pub tag: Option<Tag>,
}

/// S3 LocationInfo.
#[derive(Debug, Clone, Default)]
pub struct LocationInfo {
    pub name: Option<String>,
    pub r#type: Option<LocationType>,
}

/// S3 LoggingEnabled.
#[derive(Debug, Clone, Default)]
pub struct LoggingEnabled {
    pub target_bucket: String,
    pub target_grants: Vec<TargetGrant>,
    pub target_object_key_format: Option<TargetObjectKeyFormat>,
    pub target_prefix: String,
}

/// S3 MultipartUpload.
#[derive(Debug, Clone, Default)]
pub struct MultipartUpload {
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    pub checksum_type: Option<ChecksumType>,
    pub initiated: Option<chrono::DateTime<chrono::Utc>>,
    pub initiator: Option<Initiator>,
    pub key: Option<String>,
    pub owner: Option<Owner>,
    pub storage_class: Option<StorageClass>,
    pub upload_id: Option<String>,
}

/// S3 NoncurrentVersionExpiration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoncurrentVersionExpiration {
    pub newer_noncurrent_versions: Option<i32>,
    pub noncurrent_days: Option<i32>,
}

/// S3 NoncurrentVersionTransition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoncurrentVersionTransition {
    pub newer_noncurrent_versions: Option<i32>,
    pub noncurrent_days: Option<i32>,
    pub storage_class: Option<TransitionStorageClass>,
}

/// S3 NotificationConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfiguration {
    pub event_bridge_configuration: Option<EventBridgeConfiguration>,
    pub lambda_function_configurations: Vec<LambdaFunctionConfiguration>,
    pub queue_configurations: Vec<QueueConfiguration>,
    pub topic_configurations: Vec<TopicConfiguration>,
}

/// S3 NotificationConfigurationFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfigurationFilter {
    pub key: Option<S3KeyFilter>,
}

/// S3 Object.
#[derive(Debug, Clone, Default)]
pub struct Object {
    pub checksum_algorithm: Vec<ChecksumAlgorithm>,
    pub checksum_type: Option<ChecksumType>,
    pub e_tag: Option<String>,
    pub key: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub owner: Option<Owner>,
    pub restore_status: Option<RestoreStatus>,
    pub size: Option<i64>,
    pub storage_class: Option<ObjectStorageClass>,
}

/// S3 ObjectIdentifier.
#[derive(Debug, Clone, Default)]
pub struct ObjectIdentifier {
    pub e_tag: Option<String>,
    pub key: String,
    pub last_modified_time: Option<chrono::DateTime<chrono::Utc>>,
    pub size: Option<i64>,
    pub version_id: Option<String>,
}

/// S3 ObjectLockConfiguration.
#[derive(Debug, Clone, Default)]
pub struct ObjectLockConfiguration {
    pub object_lock_enabled: Option<ObjectLockEnabled>,
    pub rule: Option<ObjectLockRule>,
}

/// S3 ObjectLockLegalHold.
#[derive(Debug, Clone, Default)]
pub struct ObjectLockLegalHold {
    pub status: Option<ObjectLockLegalHoldStatus>,
}

/// S3 ObjectLockRetention.
#[derive(Debug, Clone, Default)]
pub struct ObjectLockRetention {
    pub mode: Option<ObjectLockRetentionMode>,
    pub retain_until_date: Option<chrono::DateTime<chrono::Utc>>,
}

/// S3 ObjectLockRule.
#[derive(Debug, Clone, Default)]
pub struct ObjectLockRule {
    pub default_retention: Option<DefaultRetention>,
}

/// S3 ObjectPart.
#[derive(Debug, Clone, Default)]
pub struct ObjectPart {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub part_number: Option<i32>,
    pub size: Option<i64>,
}

/// S3 ObjectVersion.
#[derive(Debug, Clone, Default)]
pub struct ObjectVersion {
    pub checksum_algorithm: Vec<ChecksumAlgorithm>,
    pub checksum_type: Option<ChecksumType>,
    pub e_tag: Option<String>,
    pub is_latest: Option<bool>,
    pub key: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub owner: Option<Owner>,
    pub restore_status: Option<RestoreStatus>,
    pub size: Option<i64>,
    pub storage_class: Option<ObjectVersionStorageClass>,
    pub version_id: Option<String>,
}

/// S3 Owner.
#[derive(Debug, Clone, Default)]
pub struct Owner {
    pub display_name: Option<String>,
    pub id: Option<String>,
}

/// S3 OwnershipControls.
#[derive(Debug, Clone, Default)]
pub struct OwnershipControls {
    pub rules: Vec<OwnershipControlsRule>,
}

/// S3 OwnershipControlsRule.
#[derive(Debug, Clone, Default)]
pub struct OwnershipControlsRule {
    pub object_ownership: ObjectOwnership,
}

/// S3 Part.
#[derive(Debug, Clone, Default)]
pub struct Part {
    pub checksum_crc32: Option<String>,
    pub checksum_crc32c: Option<String>,
    pub checksum_crc64nvme: Option<String>,
    pub checksum_sha1: Option<String>,
    pub checksum_sha256: Option<String>,
    pub e_tag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub part_number: Option<i32>,
    pub size: Option<i64>,
}

/// S3 PartitionedPrefix.
#[derive(Debug, Clone, Default)]
pub struct PartitionedPrefix {
    pub partition_date_source: Option<PartitionDateSource>,
}

/// S3 PolicyStatus.
#[derive(Debug, Clone, Default)]
pub struct PolicyStatus {
    pub is_public: Option<bool>,
}

/// S3 PublicAccessBlockConfiguration.
#[derive(Debug, Clone, Default)]
pub struct PublicAccessBlockConfiguration {
    pub block_public_acls: Option<bool>,
    pub block_public_policy: Option<bool>,
    pub ignore_public_acls: Option<bool>,
    pub restrict_public_buckets: Option<bool>,
}

/// S3 QueueConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueConfiguration {
    pub events: Vec<Event>,
    pub filter: Option<NotificationConfigurationFilter>,
    pub id: Option<String>,
    pub queue_arn: String,
}

/// S3 Redirect.
#[derive(Debug, Clone, Default)]
pub struct Redirect {
    pub host_name: Option<String>,
    pub http_redirect_code: Option<String>,
    pub protocol: Option<Protocol>,
    pub replace_key_prefix_with: Option<String>,
    pub replace_key_with: Option<String>,
}

/// S3 RedirectAllRequestsTo.
#[derive(Debug, Clone, Default)]
pub struct RedirectAllRequestsTo {
    pub host_name: String,
    pub protocol: Option<Protocol>,
}

/// S3 RequestPaymentConfiguration.
#[derive(Debug, Clone, Default)]
pub struct RequestPaymentConfiguration {
    pub payer: Payer,
}

/// S3 RestoreStatus.
#[derive(Debug, Clone, Default)]
pub struct RestoreStatus {
    pub is_restore_in_progress: Option<bool>,
    pub restore_expiry_date: Option<chrono::DateTime<chrono::Utc>>,
}

/// S3 RoutingRule.
#[derive(Debug, Clone, Default)]
pub struct RoutingRule {
    pub condition: Option<Condition>,
    pub redirect: Redirect,
}

/// S3 S3KeyFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct S3KeyFilter {
    pub filter_rules: Vec<FilterRule>,
}

/// S3 ServerSideEncryptionByDefault.
#[derive(Debug, Clone, Default)]
pub struct ServerSideEncryptionByDefault {
    pub kms_master_key_id: Option<String>,
    pub sse_algorithm: ServerSideEncryption,
}

/// S3 ServerSideEncryptionConfiguration.
#[derive(Debug, Clone, Default)]
pub struct ServerSideEncryptionConfiguration {
    pub rules: Vec<ServerSideEncryptionRule>,
}

/// S3 ServerSideEncryptionRule.
#[derive(Debug, Clone, Default)]
pub struct ServerSideEncryptionRule {
    pub apply_server_side_encryption_by_default: Option<ServerSideEncryptionByDefault>,
    pub blocked_encryption_types: Option<BlockedEncryptionTypes>,
    pub bucket_key_enabled: Option<bool>,
}

/// S3 SimplePrefix.
#[derive(Debug, Clone, Default)]
pub struct SimplePrefix {}

/// S3 Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tag {
    pub key: String,
    pub value: String,
}

/// S3 Tagging.
#[derive(Debug, Clone, Default)]
pub struct Tagging {
    pub tag_set: Vec<Tag>,
}

/// S3 TargetGrant.
#[derive(Debug, Clone, Default)]
pub struct TargetGrant {
    pub grantee: Option<Grantee>,
    pub permission: Option<BucketLogsPermission>,
}

/// S3 TargetObjectKeyFormat.
#[derive(Debug, Clone, Default)]
pub struct TargetObjectKeyFormat {
    pub partitioned_prefix: Option<PartitionedPrefix>,
    pub simple_prefix: Option<SimplePrefix>,
}

/// S3 TopicConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TopicConfiguration {
    pub events: Vec<Event>,
    pub filter: Option<NotificationConfigurationFilter>,
    pub id: Option<String>,
    pub topic_arn: String,
}

/// S3 Transition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transition {
    pub date: Option<chrono::DateTime<chrono::Utc>>,
    pub days: Option<i32>,
    pub storage_class: Option<TransitionStorageClass>,
}

/// S3 VersioningConfiguration.
#[derive(Debug, Clone, Default)]
pub struct VersioningConfiguration {
    pub mfa_delete: Option<MFADelete>,
    pub status: Option<BucketVersioningStatus>,
}

/// S3 WebsiteConfiguration.
#[derive(Debug, Clone, Default)]
pub struct WebsiteConfiguration {
    pub error_document: Option<ErrorDocument>,
    pub index_document: Option<IndexDocument>,
    pub redirect_all_requests_to: Option<RedirectAllRequestsTo>,
    pub routing_rules: Vec<RoutingRule>,
}
