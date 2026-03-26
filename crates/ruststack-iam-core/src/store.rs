//! In-memory storage for all IAM entities.
//!
//! Uses [`DashMap`] for concurrent access to each entity collection.

use dashmap::DashMap;

use crate::types::{
    AccessKeyRecord, GroupRecord, InstanceProfileRecord, ManagedPolicyRecord, OidcProviderRecord,
    RoleRecord, UserRecord,
};

/// Concurrent in-memory store holding all IAM entity collections.
#[derive(Debug)]
pub struct IamStore {
    /// Users keyed by user name.
    pub users: DashMap<String, UserRecord>,
    /// Roles keyed by role name.
    pub roles: DashMap<String, RoleRecord>,
    /// Groups keyed by group name.
    pub groups: DashMap<String, GroupRecord>,
    /// Managed policies keyed by policy ARN.
    pub policies: DashMap<String, ManagedPolicyRecord>,
    /// Instance profiles keyed by instance profile name.
    pub instance_profiles: DashMap<String, InstanceProfileRecord>,
    /// Access keys keyed by access key ID.
    pub access_keys: DashMap<String, AccessKeyRecord>,
    /// OIDC providers keyed by provider ARN.
    pub oidc_providers: DashMap<String, OidcProviderRecord>,
}

impl IamStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            users: DashMap::new(),
            roles: DashMap::new(),
            groups: DashMap::new(),
            policies: DashMap::new(),
            instance_profiles: DashMap::new(),
            access_keys: DashMap::new(),
            oidc_providers: DashMap::new(),
        }
    }
}

impl Default for IamStore {
    fn default() -> Self {
        Self::new()
    }
}
