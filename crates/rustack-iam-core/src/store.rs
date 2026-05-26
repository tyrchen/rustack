//! In-memory storage for all IAM entities.
//!
//! Uses [`DashMap`] for concurrent access to each entity collection.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

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

    /// Export IAM state for runtime snapshots.
    #[must_use]
    pub fn export_snapshot(&self) -> IamStoreSnapshot {
        IamStoreSnapshot {
            users: sorted_values(&self.users, |record| record.user_name.clone()),
            roles: sorted_values(&self.roles, |record| record.role_name.clone()),
            groups: sorted_values(&self.groups, |record| record.group_name.clone()),
            policies: sorted_values(&self.policies, |record| record.arn.clone()),
            instance_profiles: sorted_values(&self.instance_profiles, |record| {
                record.instance_profile_name.clone()
            }),
            access_keys: sorted_values(&self.access_keys, |record| record.access_key_id.clone()),
            oidc_providers: sorted_values(&self.oidc_providers, |record| record.arn.clone()),
        }
    }

    /// Import IAM state from a runtime snapshot.
    pub fn import_snapshot(&self, snapshot: IamStoreSnapshot) {
        self.users.clear();
        self.roles.clear();
        self.groups.clear();
        self.policies.clear();
        self.instance_profiles.clear();
        self.access_keys.clear();
        self.oidc_providers.clear();

        for record in snapshot.users {
            self.users.insert(record.user_name.clone(), record);
        }
        for record in snapshot.roles {
            self.roles.insert(record.role_name.clone(), record);
        }
        for record in snapshot.groups {
            self.groups.insert(record.group_name.clone(), record);
        }
        for record in snapshot.policies {
            self.policies.insert(record.arn.clone(), record);
        }
        for record in snapshot.instance_profiles {
            self.instance_profiles
                .insert(record.instance_profile_name.clone(), record);
        }
        for record in snapshot.access_keys {
            self.access_keys
                .insert(record.access_key_id.clone(), record);
        }
        for record in snapshot.oidc_providers {
            self.oidc_providers.insert(record.arn.clone(), record);
        }
    }
}

impl Default for IamStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable IAM store snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamStoreSnapshot {
    /// Users.
    pub users: Vec<UserRecord>,
    /// Roles.
    pub roles: Vec<RoleRecord>,
    /// Groups.
    pub groups: Vec<GroupRecord>,
    /// Managed policies.
    pub policies: Vec<ManagedPolicyRecord>,
    /// Instance profiles.
    pub instance_profiles: Vec<InstanceProfileRecord>,
    /// Access keys.
    pub access_keys: Vec<AccessKeyRecord>,
    /// OIDC providers.
    pub oidc_providers: Vec<OidcProviderRecord>,
}

fn sorted_values<T, F>(map: &DashMap<String, T>, key_fn: F) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    let mut values: Vec<T> = map.iter().map(|entry| entry.value().clone()).collect();
    values.sort_by_key(key_fn);
    values
}
