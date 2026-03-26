//! Internal storage types for IAM entities.
//!
//! These are distinct from the Smithy-generated model types and are
//! designed for efficient in-memory storage with relationship tracking
//! between entities.

use std::collections::{HashMap, HashSet};

/// Internal record for an IAM user.
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// The friendly name identifying the user.
    pub user_name: String,
    /// The stable and unique string identifying the user.
    pub user_id: String,
    /// The Amazon Resource Name (ARN) that identifies the user.
    pub arn: String,
    /// The path to the user.
    pub path: String,
    /// ISO 8601 date-time when the user was created.
    pub create_date: String,
    /// Tags attached to this user as `(key, value)` pairs.
    pub tags: Vec<(String, String)>,
    /// ARN of the permissions boundary policy, if set.
    pub permissions_boundary: Option<String>,
    /// ARNs of managed policies attached to this user.
    pub attached_policies: HashSet<String>,
    /// Inline policies: policy name to policy document JSON.
    pub inline_policies: HashMap<String, String>,
    /// Names of groups this user belongs to.
    pub groups: HashSet<String>,
}

/// Internal record for an IAM role.
#[derive(Debug, Clone)]
pub struct RoleRecord {
    /// The friendly name identifying the role.
    pub role_name: String,
    /// The stable and unique string identifying the role.
    pub role_id: String,
    /// The Amazon Resource Name (ARN) that identifies the role.
    pub arn: String,
    /// The path to the role.
    pub path: String,
    /// The trust policy (assume role policy document) as JSON.
    pub assume_role_policy_document: String,
    /// A description of the role.
    pub description: Option<String>,
    /// Maximum session duration in seconds (3600-43200).
    pub max_session_duration: i32,
    /// ISO 8601 date-time when the role was created.
    pub create_date: String,
    /// Tags attached to this role as `(key, value)` pairs.
    pub tags: Vec<(String, String)>,
    /// ARN of the permissions boundary policy, if set.
    pub permissions_boundary: Option<String>,
    /// ARNs of managed policies attached to this role.
    pub attached_policies: HashSet<String>,
    /// Inline policies: policy name to policy document JSON.
    pub inline_policies: HashMap<String, String>,
    /// Whether this is a service-linked role.
    pub is_service_linked: bool,
}

/// Internal record for an IAM group.
#[derive(Debug, Clone)]
pub struct GroupRecord {
    /// The friendly name identifying the group.
    pub group_name: String,
    /// The stable and unique string identifying the group.
    pub group_id: String,
    /// The Amazon Resource Name (ARN) that identifies the group.
    pub arn: String,
    /// The path to the group.
    pub path: String,
    /// ISO 8601 date-time when the group was created.
    pub create_date: String,
    /// ARNs of managed policies attached to this group.
    pub attached_policies: HashSet<String>,
    /// Inline policies: policy name to policy document JSON.
    pub inline_policies: HashMap<String, String>,
    /// User names of members in this group.
    pub members: HashSet<String>,
}

/// Internal record for a managed IAM policy.
#[derive(Debug, Clone)]
pub struct ManagedPolicyRecord {
    /// The friendly name identifying the policy.
    pub policy_name: String,
    /// The stable and unique string identifying the policy.
    pub policy_id: String,
    /// The Amazon Resource Name (ARN) that identifies the policy.
    pub arn: String,
    /// The path to the policy.
    pub path: String,
    /// A description of the policy.
    pub description: Option<String>,
    /// ISO 8601 date-time when the policy was created.
    pub create_date: String,
    /// ISO 8601 date-time when the policy was last updated.
    pub update_date: String,
    /// Whether the policy can be attached to entities.
    pub is_attachable: bool,
    /// Number of entities the policy is attached to.
    pub attachment_count: i32,
    /// Number of entities using this policy as a permissions boundary.
    pub permissions_boundary_usage_count: i32,
    /// All versions of this policy document.
    pub versions: Vec<PolicyVersionRecord>,
    /// The identifier of the default policy version (e.g., `v1`).
    pub default_version_id: String,
    /// Tags attached to this policy as `(key, value)` pairs.
    pub tags: Vec<(String, String)>,
}

/// A single version of a managed policy document.
#[derive(Debug, Clone)]
pub struct PolicyVersionRecord {
    /// The version identifier (e.g., `v1`, `v2`).
    pub version_id: String,
    /// The JSON policy document.
    pub document: String,
    /// Whether this is the default (active) version.
    pub is_default_version: bool,
    /// ISO 8601 date-time when this version was created.
    pub create_date: String,
}

/// Internal record for an IAM instance profile.
#[derive(Debug, Clone)]
pub struct InstanceProfileRecord {
    /// The friendly name identifying the instance profile.
    pub instance_profile_name: String,
    /// The stable and unique string identifying the instance profile.
    pub instance_profile_id: String,
    /// The Amazon Resource Name (ARN) that identifies the instance profile.
    pub arn: String,
    /// The path to the instance profile.
    pub path: String,
    /// ISO 8601 date-time when the instance profile was created.
    pub create_date: String,
    /// Tags attached to this instance profile as `(key, value)` pairs.
    pub tags: Vec<(String, String)>,
    /// Role names associated with this instance profile.
    pub roles: Vec<String>,
}

/// Internal record for an OIDC identity provider.
#[derive(Debug, Clone)]
pub struct OidcProviderRecord {
    /// The Amazon Resource Name (ARN) that identifies the provider.
    pub arn: String,
    /// The URL of the identity provider (e.g., `https://accounts.google.com`).
    pub url: String,
    /// A list of client IDs (audiences) registered with the provider.
    pub client_id_list: Vec<String>,
    /// A list of server certificate thumbprints for the provider.
    pub thumbprint_list: Vec<String>,
    /// Tags attached to this provider as `(key, value)` pairs.
    pub tags: Vec<(String, String)>,
    /// ISO 8601 date-time when the provider was created.
    pub create_date: String,
}

/// Internal record for an IAM access key.
#[derive(Debug, Clone)]
pub struct AccessKeyRecord {
    /// The access key ID (begins with `AKIA`).
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: String,
    /// The user name the access key belongs to.
    pub user_name: String,
    /// Status: `"Active"` or `"Inactive"`.
    pub status: String,
    /// ISO 8601 date-time when the access key was created.
    pub create_date: String,
}
