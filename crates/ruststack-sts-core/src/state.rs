//! STS state management.

use dashmap::DashMap;
use ruststack_sts_model::types::Tag;

use crate::config::StsConfig;
use crate::identity::CallerIdentity;
use crate::session::{SessionRecord, SessionTag};

/// A record associating an access key with its identity.
#[derive(Debug, Clone)]
pub struct CredentialRecord {
    /// The access key ID (AKIA* for permanent, ASIA* for temporary).
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: String,
    /// The session token (only for temporary credentials).
    pub session_token: Option<String>,
    /// The identity associated with this credential.
    pub identity: CallerIdentity,
    /// When these credentials expire (epoch seconds). Not enforced.
    pub expiration: Option<i64>,
}

/// Top-level STS state.
///
/// Global singleton (STS is a global service). Not scoped by region.
pub struct StsState {
    /// Maps access key ID -> caller identity info.
    pub credentials: DashMap<String, CredentialRecord>,
    /// Maps session token -> session record.
    pub sessions: DashMap<String, SessionRecord>,
    /// Default account ID for root credentials.
    pub default_account_id: String,
    /// Default access key ID that maps to the root account.
    pub default_access_key: String,
}

impl std::fmt::Debug for StsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StsState")
            .field("credentials_count", &self.credentials.len())
            .field("sessions_count", &self.sessions.len())
            .field("default_account_id", &self.default_account_id)
            .field("default_access_key", &self.default_access_key)
            .finish()
    }
}

impl StsState {
    /// Create a new STS state with default root credentials.
    #[must_use]
    pub fn new(config: &StsConfig) -> Self {
        let state = Self {
            credentials: DashMap::new(),
            sessions: DashMap::new(),
            default_account_id: config.default_account_id.clone(),
            default_access_key: config.default_access_key.clone(),
        };

        // Register default root credentials.
        state.credentials.insert(
            config.default_access_key.clone(),
            CredentialRecord {
                access_key_id: config.default_access_key.clone(),
                secret_access_key: config.default_secret_key.clone(),
                session_token: None,
                identity: CallerIdentity::Root {
                    account_id: config.default_account_id.clone(),
                },
                expiration: None,
            },
        );

        state
    }

    /// Resolve the caller identity from an access key ID.
    ///
    /// If the access key is found in the credential store, returns
    /// the stored identity. If not found, returns root identity for
    /// the default account (permissive mode for local development).
    #[must_use]
    pub fn resolve_identity(&self, access_key_id: &str) -> CallerIdentity {
        if let Some(record) = self.credentials.get(access_key_id) {
            return record.identity.clone();
        }

        // Permissive fallback: unknown access keys are treated as root.
        CallerIdentity::Root {
            account_id: self.default_account_id.clone(),
        }
    }

    /// Resolve the effective tags for a new AssumeRole call.
    ///
    /// Combines:
    /// 1. Tags explicitly provided in the AssumeRole request
    /// 2. Transitive tags inherited from the caller's session
    #[must_use]
    pub fn resolve_session_tags(
        &self,
        caller_access_key: &str,
        request_tags: &[Tag],
        request_transitive_keys: &[String],
    ) -> (Vec<SessionTag>, Vec<String>) {
        let mut effective_tags: Vec<SessionTag> = Vec::new();
        let mut effective_transitive_keys: Vec<String> = request_transitive_keys.to_vec();

        // Check if the caller has an existing session with transitive tags.
        if let Some(cred_record) = self.credentials.get(caller_access_key) {
            if let Some(ref token) = cred_record.session_token {
                if let Some(session) = self.sessions.get(token) {
                    // Inherit transitive tags from parent session.
                    for inherited_tag in &session.inherited_transitive_tags {
                        effective_tags.push(inherited_tag.clone());
                    }
                    for tag in &session.tags {
                        if session.transitive_tag_keys.contains(&tag.key) {
                            effective_tags.push(tag.clone());
                            if !effective_transitive_keys.contains(&tag.key) {
                                effective_transitive_keys.push(tag.key.clone());
                            }
                        }
                    }
                }
            }
        }

        // Request tags override inherited tags.
        let request_tag_keys: Vec<&str> = request_tags.iter().map(|t| t.key.as_str()).collect();
        effective_tags.retain(|t| !request_tag_keys.contains(&t.key.as_str()));
        for tag in request_tags {
            effective_tags.push(SessionTag {
                key: tag.key.clone(),
                value: tag.value.clone(),
            });
        }

        (effective_tags, effective_transitive_keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_resolve_default_root_identity() {
        let config = StsConfig::default();
        let state = StsState::new(&config);
        let identity = state.resolve_identity("test");
        assert_eq!(identity.account_id(), "000000000000");
        assert_eq!(identity.arn(), "arn:aws:iam::000000000000:root");
    }

    #[test]
    fn test_should_resolve_unknown_key_as_root() {
        let config = StsConfig::default();
        let state = StsState::new(&config);
        let identity = state.resolve_identity("AKIASOME_UNKNOWN_KEY");
        assert_eq!(identity.account_id(), "000000000000");
    }
}
