//! KMS in-memory state store using DashMap.

use std::collections::HashMap;

use dashmap::DashMap;

use ruststack_kms_model::types::{GrantConstraints, GrantOperation};

use crate::key::KmsKey;

/// Grant entry stored in the grant store.
#[derive(Debug, Clone)]
pub struct GrantEntry {
    /// The grant ID.
    pub grant_id: String,
    /// The key ID the grant is for.
    pub key_id: String,
    /// The grantee principal.
    pub grantee_principal: String,
    /// The retiring principal (optional).
    pub retiring_principal: Option<String>,
    /// The operations allowed by this grant.
    pub operations: Vec<GrantOperation>,
    /// Grant constraints.
    pub constraints: Option<GrantConstraints>,
    /// Grant name (optional).
    pub name: Option<String>,
    /// Creation timestamp.
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// Whether this grant has been retired.
    pub retired: bool,
    /// Issuing account.
    pub issuing_account: String,
}

/// Alias entry stored in the alias store.
#[derive(Debug, Clone)]
pub struct AliasEntry {
    /// The alias name (e.g., "alias/my-key").
    pub alias_name: String,
    /// The alias ARN.
    pub alias_arn: String,
    /// The target key ID.
    pub target_key_id: String,
    /// Creation timestamp.
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// Last updated timestamp.
    pub last_updated_date: chrono::DateTime<chrono::Utc>,
}

/// In-memory KMS state store.
#[derive(Debug)]
pub struct KmsStore {
    /// Keys indexed by key ID.
    pub keys: DashMap<String, KmsKey>,
    /// Aliases indexed by alias name.
    pub aliases: DashMap<String, AliasEntry>,
    /// Grants indexed by grant ID.
    pub grants: DashMap<String, GrantEntry>,
    /// Index: key ID -> set of grant IDs for that key.
    pub key_grants: DashMap<String, Vec<String>>,
    /// Default account ID.
    pub account_id: String,
    /// Default region.
    pub region: String,
}

impl KmsStore {
    /// Create a new empty store.
    pub fn new(account_id: String, region: String) -> Self {
        Self {
            keys: DashMap::new(),
            aliases: DashMap::new(),
            grants: DashMap::new(),
            key_grants: DashMap::new(),
            account_id,
            region,
        }
    }

    /// Get a key by ID.
    pub fn get_key(&self, key_id: &str) -> Option<KmsKey> {
        self.keys.get(key_id).map(|r| r.value().clone())
    }

    /// Insert or update a key.
    pub fn put_key(&self, key: KmsKey) {
        self.keys.insert(key.key_id.clone(), key);
    }

    /// Get an alias by name.
    pub fn get_alias(&self, alias_name: &str) -> Option<AliasEntry> {
        self.aliases.get(alias_name).map(|r| r.value().clone())
    }

    /// Insert or update an alias.
    pub fn put_alias(&self, alias: AliasEntry) {
        self.aliases.insert(alias.alias_name.clone(), alias);
    }

    /// Remove an alias by name.
    pub fn remove_alias(&self, alias_name: &str) -> Option<AliasEntry> {
        self.aliases.remove(alias_name).map(|(_, v)| v)
    }

    /// List all aliases, optionally filtered by key ID.
    pub fn list_aliases(&self, key_id: Option<&str>) -> Vec<AliasEntry> {
        self.aliases
            .iter()
            .filter(|entry| key_id.is_none_or(|kid| entry.value().target_key_id == kid))
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Insert a grant.
    pub fn put_grant(&self, grant: GrantEntry) {
        let key_id = grant.key_id.clone();
        let grant_id = grant.grant_id.clone();
        self.grants.insert(grant.grant_id.clone(), grant);
        self.key_grants.entry(key_id).or_default().push(grant_id);
    }

    /// Get a grant by ID.
    pub fn get_grant(&self, grant_id: &str) -> Option<GrantEntry> {
        self.grants.get(grant_id).map(|r| r.value().clone())
    }

    /// List grants for a key.
    pub fn list_grants_for_key(&self, key_id: &str) -> Vec<GrantEntry> {
        self.key_grants
            .get(key_id)
            .map(|grant_ids| {
                grant_ids
                    .iter()
                    .filter_map(|gid| self.grants.get(gid).map(|r| r.value().clone()))
                    .filter(|g| !g.retired)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all grants that have the given retiring principal.
    pub fn list_retirable_grants(&self, retiring_principal: &str) -> Vec<GrantEntry> {
        self.grants
            .iter()
            .filter(|entry| {
                let g = entry.value();
                !g.retired
                    && g.retiring_principal
                        .as_deref()
                        .is_some_and(|rp| rp == retiring_principal)
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Retire a grant by setting its retired flag.
    pub fn retire_grant(&self, grant_id: &str) -> bool {
        if let Some(mut grant) = self.grants.get_mut(grant_id) {
            grant.retired = true;
            true
        } else {
            false
        }
    }

    /// Remove a grant entirely (revoke).
    pub fn remove_grant(&self, grant_id: &str) -> Option<GrantEntry> {
        if let Some((_, grant)) = self.grants.remove(grant_id) {
            // Also remove from key_grants index.
            if let Some(mut grant_ids) = self.key_grants.get_mut(&grant.key_id) {
                grant_ids.retain(|id| id != grant_id);
            }
            Some(grant)
        } else {
            None
        }
    }

    /// Get tags for a key as a HashMap.
    pub fn get_tags(&self, key_id: &str) -> HashMap<String, String> {
        self.keys
            .get(key_id)
            .map(|r| r.value().tags.clone())
            .unwrap_or_default()
    }
}
