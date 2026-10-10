//! Identity store for verified email addresses and domains.
//!
//! In local development mode, all identities are auto-verified on creation.
//! The store supports email and domain identity types, with domain fallback
//! verification (emails are verified if their domain is verified).

use std::collections::HashMap;

use dashmap::DashMap;
use rustack_ses_model::types::{
    BehaviorOnMXFailure, CustomMailFromStatus, IdentityDkimAttributes,
    IdentityMailFromDomainAttributes, IdentityNotificationAttributes, IdentityType,
    IdentityVerificationAttributes, NotificationType, VerificationStatus,
};

/// A single identity record (email address or domain).
#[derive(Debug, Clone)]
pub struct IdentityRecord {
    /// The identity string (email address or domain name).
    pub identity: String,
    /// Type of identity.
    pub identity_type: IdentityType,
    /// Verification status (always `Success` in local dev).
    pub verification_status: VerificationStatus,
    /// Verification token (for domain identities).
    pub verification_token: Option<String>,
    /// DKIM enabled flag.
    pub dkim_enabled: bool,
    /// DKIM tokens (stub tokens for domain identities).
    pub dkim_tokens: Vec<String>,
    /// Notification topic ARNs keyed by notification type.
    pub notification_topics: HashMap<String, Option<String>>,
    /// Feedback forwarding enabled.
    pub feedback_forwarding_enabled: bool,
    /// Mail-from domain.
    pub mail_from_domain: Option<String>,
    /// Behavior on MX failure.
    pub behavior_on_mx_failure: BehaviorOnMXFailure,
    /// Sending authorization policies keyed by policy name.
    pub policies: HashMap<String, String>,
}

/// Store for verified email addresses and domains.
///
/// In local development mode, all identities are auto-verified on creation.
/// In strict mode (configurable), identities must be explicitly verified first
/// before they can be used as a source address in `SendEmail`.
#[derive(Debug)]
pub struct IdentityStore {
    /// All identities keyed by identity string (email address or domain).
    identities: DashMap<String, IdentityRecord>,
}

impl Default for IdentityStore {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentityStore {
    /// Create a new empty identity store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            identities: DashMap::new(),
        }
    }

    /// Add an email identity. Auto-verifies in local dev mode.
    #[must_use]
    pub fn verify_email(&self, email: &str) -> IdentityRecord {
        let record = IdentityRecord {
            identity: email.to_owned(),
            identity_type: IdentityType::EmailAddress,
            verification_status: VerificationStatus::Success,
            verification_token: None,
            dkim_enabled: false,
            dkim_tokens: Vec::new(),
            notification_topics: HashMap::new(),
            feedback_forwarding_enabled: true,
            mail_from_domain: None,
            behavior_on_mx_failure: BehaviorOnMXFailure::UseDefaultValue,
            policies: HashMap::new(),
        };
        self.identities.insert(email.to_owned(), record.clone());
        record
    }

    /// Add a domain identity. Auto-verifies in local dev mode.
    #[must_use]
    pub fn verify_domain(&self, domain: &str) -> (IdentityRecord, String) {
        let token = uuid::Uuid::new_v4().to_string();
        let dkim_tokens = vec![
            format!("{:x}", md5_stub(domain, 1)),
            format!("{:x}", md5_stub(domain, 2)),
            format!("{:x}", md5_stub(domain, 3)),
        ];
        let record = IdentityRecord {
            identity: domain.to_owned(),
            identity_type: IdentityType::Domain,
            verification_status: VerificationStatus::Success,
            verification_token: Some(token.clone()),
            dkim_enabled: false,
            dkim_tokens,
            notification_topics: HashMap::new(),
            feedback_forwarding_enabled: true,
            mail_from_domain: None,
            behavior_on_mx_failure: BehaviorOnMXFailure::UseDefaultValue,
            policies: HashMap::new(),
        };
        self.identities.insert(domain.to_owned(), record.clone());
        (record, token)
    }

    /// Check if an email address is verified (either directly or via domain).
    #[must_use]
    pub fn is_verified(&self, email: &str) -> bool {
        // Direct email match
        if let Some(record) = self.identities.get(email) {
            return matches!(record.verification_status, VerificationStatus::Success);
        }
        // Domain match: extract domain from email and check
        if let Some(domain) = email.split('@').nth(1) {
            if let Some(record) = self.identities.get(domain) {
                return matches!(record.verification_status, VerificationStatus::Success);
            }
        }
        false
    }

    /// List all identity strings, optionally filtered by type.
    #[must_use]
    pub fn list(&self, identity_type: Option<&IdentityType>) -> Vec<String> {
        self.identities
            .iter()
            .filter(|entry| identity_type.is_none_or(|t| entry.identity_type == *t))
            .map(|entry| entry.identity.clone())
            .collect()
    }

    /// Delete an identity by its string.
    pub fn delete(&self, identity: &str) {
        self.identities.remove(identity);
    }

    /// Get a reference to an identity record.
    #[must_use]
    pub fn get(&self, identity: &str) -> Option<IdentityRecord> {
        self.identities.get(identity).map(|r| r.value().clone())
    }

    /// Get verification attributes for a list of identities.
    #[must_use]
    pub fn get_verification_attributes(
        &self,
        identities: &[String],
    ) -> HashMap<String, IdentityVerificationAttributes> {
        let mut result = HashMap::new();
        for identity in identities {
            let attrs = if let Some(record) = self.identities.get(identity) {
                IdentityVerificationAttributes {
                    verification_status: record.verification_status.clone(),
                    verification_token: record.verification_token.clone(),
                }
            } else {
                // Unknown identities still return Success in local dev
                IdentityVerificationAttributes {
                    verification_status: VerificationStatus::Success,
                    verification_token: if identity.contains('@') {
                        None
                    } else {
                        Some(uuid::Uuid::new_v4().to_string())
                    },
                }
            };
            result.insert(identity.clone(), attrs);
        }
        result
    }

    /// Get notification attributes for a list of identities.
    #[must_use]
    pub fn get_notification_attributes(
        &self,
        identities: &[String],
    ) -> HashMap<String, IdentityNotificationAttributes> {
        let mut result = HashMap::new();
        for identity in identities {
            let attrs = if let Some(record) = self.identities.get(identity) {
                let bounce_topic = record
                    .notification_topics
                    .get(NotificationType::Bounce.as_str())
                    .and_then(Clone::clone)
                    .unwrap_or_default();
                let complaint_topic = record
                    .notification_topics
                    .get(NotificationType::Complaint.as_str())
                    .and_then(Clone::clone)
                    .unwrap_or_default();
                let delivery_topic = record
                    .notification_topics
                    .get(NotificationType::Delivery.as_str())
                    .and_then(Clone::clone)
                    .unwrap_or_default();
                IdentityNotificationAttributes {
                    bounce_topic,
                    complaint_topic,
                    delivery_topic,
                    forwarding_enabled: record.feedback_forwarding_enabled,
                    headers_in_bounce_notifications_enabled: Some(false),
                    headers_in_complaint_notifications_enabled: Some(false),
                    headers_in_delivery_notifications_enabled: Some(false),
                }
            } else {
                IdentityNotificationAttributes {
                    bounce_topic: String::new(),
                    complaint_topic: String::new(),
                    delivery_topic: String::new(),
                    forwarding_enabled: true,
                    headers_in_bounce_notifications_enabled: Some(false),
                    headers_in_complaint_notifications_enabled: Some(false),
                    headers_in_delivery_notifications_enabled: Some(false),
                }
            };
            result.insert(identity.clone(), attrs);
        }
        result
    }

    /// Set the notification topic for an identity.
    pub fn set_notification_topic(
        &self,
        identity: &str,
        notification_type: &NotificationType,
        sns_topic: Option<String>,
    ) {
        if let Some(mut record) = self.identities.get_mut(identity) {
            record
                .notification_topics
                .insert(notification_type.as_str().to_owned(), sns_topic);
        }
    }

    /// Set feedback forwarding enabled for an identity.
    pub fn set_feedback_forwarding_enabled(&self, identity: &str, enabled: bool) {
        if let Some(mut record) = self.identities.get_mut(identity) {
            record.feedback_forwarding_enabled = enabled;
        }
    }

    /// Get DKIM attributes for a list of identities.
    #[must_use]
    pub fn get_dkim_attributes(
        &self,
        identities: &[String],
    ) -> HashMap<String, IdentityDkimAttributes> {
        let mut result = HashMap::new();
        for identity in identities {
            let attrs = if let Some(record) = self.identities.get(identity) {
                IdentityDkimAttributes {
                    dkim_enabled: record.dkim_enabled,
                    dkim_tokens: record.dkim_tokens.clone(),
                    dkim_verification_status: VerificationStatus::Success,
                }
            } else {
                IdentityDkimAttributes {
                    dkim_enabled: false,
                    dkim_tokens: Vec::new(),
                    dkim_verification_status: VerificationStatus::NotStarted,
                }
            };
            result.insert(identity.clone(), attrs);
        }
        result
    }

    /// Verify domain DKIM and return DKIM tokens.
    #[must_use]
    pub fn verify_domain_dkim(&self, domain: &str) -> Vec<String> {
        if let Some(mut record) = self.identities.get_mut(domain) {
            record.dkim_enabled = true;
            record.dkim_tokens.clone()
        } else {
            // Auto-create domain identity if not exists
            let (_, _) = self.verify_domain(domain);
            if let Some(mut record) = self.identities.get_mut(domain) {
                record.dkim_enabled = true;
                record.dkim_tokens.clone()
            } else {
                Vec::new()
            }
        }
    }

    /// Get mail-from domain attributes for a list of identities.
    #[must_use]
    pub fn get_mail_from_domain_attributes(
        &self,
        identities: &[String],
    ) -> HashMap<String, IdentityMailFromDomainAttributes> {
        let mut result = HashMap::new();
        for identity in identities {
            let attrs = if let Some(record) = self.identities.get(identity) {
                IdentityMailFromDomainAttributes {
                    mail_from_domain: record.mail_from_domain.clone().unwrap_or_default(),
                    mail_from_domain_status: CustomMailFromStatus::Success,
                    behavior_on_mx_failure: record.behavior_on_mx_failure.clone(),
                }
            } else {
                IdentityMailFromDomainAttributes {
                    mail_from_domain: String::new(),
                    mail_from_domain_status: CustomMailFromStatus::Success,
                    behavior_on_mx_failure: BehaviorOnMXFailure::UseDefaultValue,
                }
            };
            result.insert(identity.clone(), attrs);
        }
        result
    }

    /// Set the mail-from domain for an identity.
    pub fn set_mail_from_domain(
        &self,
        identity: &str,
        mail_from_domain: Option<String>,
        behavior_on_mx_failure: Option<BehaviorOnMXFailure>,
    ) {
        if let Some(mut record) = self.identities.get_mut(identity) {
            record.mail_from_domain = mail_from_domain;
            if let Some(behavior) = behavior_on_mx_failure {
                record.behavior_on_mx_failure = behavior;
            }
        }
    }

    /// Get policies for an identity.
    #[must_use]
    pub fn get_policies(&self, identity: &str, policy_names: &[String]) -> HashMap<String, String> {
        let mut result = HashMap::new();
        if let Some(record) = self.identities.get(identity) {
            for name in policy_names {
                if let Some(policy) = record.policies.get(name) {
                    result.insert(name.clone(), policy.clone());
                }
            }
        }
        result
    }

    /// Put (create or update) a policy for an identity.
    pub fn put_policy(&self, identity: &str, policy_name: &str, policy: &str) {
        if let Some(mut record) = self.identities.get_mut(identity) {
            record
                .policies
                .insert(policy_name.to_owned(), policy.to_owned());
        }
    }

    /// Delete a policy from an identity.
    pub fn delete_policy(&self, identity: &str, policy_name: &str) {
        if let Some(mut record) = self.identities.get_mut(identity) {
            record.policies.remove(policy_name);
        }
    }

    /// List policy names for an identity.
    #[must_use]
    pub fn list_policy_names(&self, identity: &str) -> Vec<String> {
        self.identities
            .get(identity)
            .map(|record| record.policies.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// Stub hash function for generating deterministic DKIM-like tokens.
fn md5_stub(domain: &str, index: u32) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in domain.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash ^= u64::from(index);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_verify_email() {
        let store = IdentityStore::new();
        let record = store.verify_email("test@example.com");
        assert_eq!(record.identity, "test@example.com");
        assert_eq!(record.identity_type, IdentityType::EmailAddress);
        assert_eq!(record.verification_status, VerificationStatus::Success);
        assert!(record.verification_token.is_none());
    }

    #[test]
    fn test_should_verify_domain() {
        let store = IdentityStore::new();
        let (record, token) = store.verify_domain("example.com");
        assert_eq!(record.identity, "example.com");
        assert_eq!(record.identity_type, IdentityType::Domain);
        assert_eq!(record.verification_status, VerificationStatus::Success);
        assert_ne!(token, "");
    }

    #[test]
    fn test_should_check_direct_email_verification() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        assert!(store.is_verified("test@example.com"));
        assert!(!store.is_verified("other@example.com"));
    }

    #[test]
    fn test_should_check_domain_fallback_verification() {
        let store = IdentityStore::new();
        let _ = store.verify_domain("example.com");
        assert!(store.is_verified("any@example.com"));
        assert!(!store.is_verified("any@other.com"));
    }

    #[test]
    fn test_should_list_all_identities() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        let _ = store.verify_domain("example.com");
        let all = store.list(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_should_list_by_type() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        let _ = store.verify_domain("example.com");
        let emails = store.list(Some(&IdentityType::EmailAddress));
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0], "test@example.com");
        let domains = store.list(Some(&IdentityType::Domain));
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0], "example.com");
    }

    #[test]
    fn test_should_delete_identity() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        assert!(store.is_verified("test@example.com"));
        store.delete("test@example.com");
        assert!(!store.is_verified("test@example.com"));
    }

    #[test]
    fn test_should_get_verification_attributes() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        let _ = store.verify_domain("example.com");
        let attrs = store.get_verification_attributes(&[
            "test@example.com".to_owned(),
            "example.com".to_owned(),
            "unknown@other.com".to_owned(),
        ]);
        assert_eq!(attrs.len(), 3);
        assert_eq!(
            attrs["test@example.com"].verification_status,
            VerificationStatus::Success
        );
        assert_eq!(
            attrs["example.com"].verification_status,
            VerificationStatus::Success
        );
        // Unknown identities return Success in local dev
        assert_eq!(
            attrs["unknown@other.com"].verification_status,
            VerificationStatus::Success
        );
    }

    #[test]
    fn test_should_set_and_get_notification_topics() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        store.set_notification_topic(
            "test@example.com",
            &NotificationType::Bounce,
            Some("arn:aws:sns:us-east-1:000:bounce-topic".to_owned()),
        );
        let attrs = store.get_notification_attributes(&["test@example.com".to_owned()]);
        assert_eq!(
            attrs["test@example.com"].bounce_topic,
            "arn:aws:sns:us-east-1:000:bounce-topic"
        );
    }

    #[test]
    fn test_should_manage_policies() {
        let store = IdentityStore::new();
        let _ = store.verify_email("test@example.com");
        store.put_policy(
            "test@example.com",
            "my-policy",
            r#"{"Version":"2012-10-17"}"#,
        );
        let names = store.list_policy_names("test@example.com");
        assert_eq!(names, vec!["my-policy"]);
        let policies = store.get_policies("test@example.com", &["my-policy".to_owned()]);
        assert_eq!(policies["my-policy"], r#"{"Version":"2012-10-17"}"#);
        store.delete_policy("test@example.com", "my-policy");
        let names = store.list_policy_names("test@example.com");
        assert_eq!(names.len(), 0);
    }
}
