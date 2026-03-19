//! In-memory storage engine for Secrets Manager.
//!
//! Secrets are stored in a `DashMap<String, SecretRecord>` keyed by secret name.
//! Each record tracks versions identified by UUID, staging labels, tags, resource
//! policies, rotation configuration, and deletion state.

use std::collections::HashMap;

use dashmap::DashMap;
use rand::Rng;

use ruststack_secretsmanager_model::error::{SecretsManagerError, SecretsManagerErrorCode};
use ruststack_secretsmanager_model::types::{RotationRulesType, Tag};

use crate::version::{AWSCURRENT, AWSPENDING, AWSPREVIOUS, MAX_VERSIONS};

/// A single version of a secret.
#[derive(Debug, Clone)]
pub struct SecretVersion {
    /// Version ID (UUID string).
    pub version_id: String,
    /// Secret string value (mutually exclusive with `secret_binary`).
    pub secret_string: Option<String>,
    /// Secret binary value (mutually exclusive with `secret_string`).
    pub secret_binary: Option<Vec<u8>>,
    /// When this version was created.
    pub created_date: chrono::DateTime<chrono::Utc>,
    /// Staging labels currently attached to this version (derived from record).
    pub version_stages: Vec<String>,
}

/// A single secret with its version history and metadata.
#[derive(Debug, Clone)]
pub struct SecretRecord {
    /// Secret name.
    pub name: String,
    /// Secret ARN (includes 6-character random suffix).
    pub arn: String,
    /// Description.
    pub description: Option<String>,
    /// KMS key ID (stored but not used for encryption).
    pub kms_key_id: Option<String>,
    /// Tags on the secret resource.
    pub tags: Vec<Tag>,
    /// Resource policy JSON (stored but not enforced).
    pub resource_policy: Option<String>,

    /// All versions keyed by version ID (UUID string).
    pub versions: HashMap<String, SecretVersion>,
    /// Mapping from staging label to version ID.
    pub staging_labels: HashMap<String, String>,

    /// Whether rotation is enabled.
    pub rotation_enabled: bool,
    /// Lambda ARN for rotation.
    pub rotation_lambda_arn: Option<String>,
    /// Rotation rules configuration.
    pub rotation_rules: Option<RotationRulesType>,
    /// Timestamp of last rotation.
    pub last_rotated_date: Option<chrono::DateTime<chrono::Utc>>,

    /// If scheduled for deletion, the deletion date.
    pub deleted_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Recovery window in days (7-30, default 30).
    pub recovery_window_in_days: Option<i64>,

    /// When the secret was created.
    pub created_date: chrono::DateTime<chrono::Utc>,
    /// When the secret was last changed.
    pub last_changed_date: chrono::DateTime<chrono::Utc>,
    /// When the secret was last accessed (date-only granularity).
    pub last_accessed_date: Option<chrono::DateTime<chrono::Utc>>,

    /// Owning service (for managed secrets).
    pub owning_service: Option<String>,
    /// Primary region for replication.
    pub primary_region: Option<String>,
}

impl SecretRecord {
    /// Move a staging label from one version to another.
    ///
    /// If the label is `AWSCURRENT`, automatically move `AWSPREVIOUS`.
    pub fn move_staging_label(
        &mut self,
        label: &str,
        to_version_id: &str,
        from_version_id: Option<&str>,
    ) -> Result<(), SecretsManagerError> {
        // Validate that to_version exists.
        if !self.versions.contains_key(to_version_id) {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "The version ID specified doesn't exist for this secret.",
            ));
        }

        // If from_version_id specified, validate it currently holds the label.
        if let Some(from_id) = from_version_id {
            let current_holder = self.staging_labels.get(label);
            if current_holder.map(String::as_str) != Some(from_id) {
                return Err(SecretsManagerError::with_message(
                    SecretsManagerErrorCode::InvalidParameterException,
                    "The staging label is not currently attached to the specified version.",
                ));
            }
        }

        // Special handling for AWSCURRENT: auto-move AWSPREVIOUS.
        if label == AWSCURRENT {
            if let Some(old_current_id) = self.staging_labels.get(AWSCURRENT).cloned() {
                // Old AWSCURRENT becomes AWSPREVIOUS.
                self.staging_labels
                    .insert(AWSPREVIOUS.to_owned(), old_current_id);
            }
        }

        // Move the label.
        self.staging_labels
            .insert(label.to_owned(), to_version_id.to_owned());

        // Rebuild version_stages caches.
        self.rebuild_version_stages();

        Ok(())
    }

    /// Add a new version and optionally assign staging labels.
    ///
    /// If `version_stages` includes `AWSCURRENT`, automatically handles `AWSPREVIOUS`.
    /// If `version_stages` is empty, defaults to `["AWSCURRENT"]`.
    pub fn add_version(
        &mut self,
        version_id: String,
        secret_string: Option<String>,
        secret_binary: Option<Vec<u8>>,
        version_stages: Vec<String>,
        created_date: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), SecretsManagerError> {
        let stages = if version_stages.is_empty() {
            vec![AWSCURRENT.to_owned()]
        } else {
            version_stages
        };

        // Check for idempotent request (same version_id with same content).
        if let Some(existing) = self.versions.get(&version_id) {
            if existing.secret_string == secret_string && existing.secret_binary == secret_binary {
                return Ok(()); // Idempotent: same content, no-op.
            }
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceExistsException,
                "A resource with the ID you requested already exists.",
            ));
        }

        // Handle AWSCURRENT promotion.
        if stages.iter().any(|s| s == AWSCURRENT) {
            if let Some(old_current_id) = self.staging_labels.get(AWSCURRENT).cloned() {
                // Move AWSPREVIOUS to old AWSCURRENT holder.
                self.staging_labels
                    .insert(AWSPREVIOUS.to_owned(), old_current_id);
            }
        }

        // Move labels to new version.
        for stage in &stages {
            self.staging_labels
                .insert(stage.clone(), version_id.clone());
        }

        // Insert version.
        self.versions.insert(
            version_id.clone(),
            SecretVersion {
                version_id,
                secret_string,
                secret_binary,
                created_date,
                version_stages: Vec::new(), // rebuilt below
            },
        );

        // Enforce version limit.
        self.cleanup_deprecated_versions();

        // Rebuild cached stages.
        self.rebuild_version_stages();

        // Update last_changed_date.
        self.last_changed_date = created_date;

        Ok(())
    }

    /// Rebuild the `version_stages` field on each `SecretVersion` from
    /// the authoritative `staging_labels` map.
    pub fn rebuild_version_stages(&mut self) {
        // Clear all version stages.
        for version in self.versions.values_mut() {
            version.version_stages.clear();
        }
        // Rebuild from staging_labels.
        for (label, version_id) in &self.staging_labels {
            if let Some(version) = self.versions.get_mut(version_id) {
                version.version_stages.push(label.clone());
            }
        }
        // Sort for deterministic output.
        for version in self.versions.values_mut() {
            version.version_stages.sort();
        }
    }

    /// Remove deprecated (label-less) versions exceeding the version limit.
    pub fn cleanup_deprecated_versions(&mut self) {
        let total = self.versions.len();
        if total <= MAX_VERSIONS {
            return;
        }

        let labeled_version_ids: std::collections::HashSet<&String> =
            self.staging_labels.values().collect();

        let mut deprecated: Vec<(String, chrono::DateTime<chrono::Utc>)> = self
            .versions
            .iter()
            .filter(|(vid, _)| !labeled_version_ids.contains(vid))
            .map(|(vid, v)| (vid.clone(), v.created_date))
            .collect();

        // Sort by creation date ascending (oldest first).
        deprecated.sort_by_key(|a| a.1);

        let to_remove = total - MAX_VERSIONS;
        for (vid, _) in deprecated.iter().take(to_remove) {
            self.versions.remove(vid);
        }
    }

    /// Build the `VersionIdsToStages` map for API responses.
    #[must_use]
    pub fn version_ids_to_stages(&self) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (label, version_id) in &self.staging_labels {
            map.entry(version_id.clone())
                .or_default()
                .push(label.clone());
        }
        // Sort labels within each version for deterministic output.
        for labels in map.values_mut() {
            labels.sort();
        }
        map
    }

    /// Returns `true` if this secret is scheduled for deletion.
    #[must_use]
    pub fn is_pending_deletion(&self) -> bool {
        self.deleted_date.is_some()
    }

    /// Schedule this secret for deletion.
    pub fn schedule_deletion(
        &mut self,
        recovery_window_days: i64,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        self.deleted_date = Some(now);
        self.recovery_window_in_days = Some(recovery_window_days);
    }

    /// Restore a secret scheduled for deletion.
    pub fn restore(&mut self) -> Result<(), SecretsManagerError> {
        if self.deleted_date.is_none() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on a secret that's not scheduled for deletion.",
            ));
        }
        self.deleted_date = None;
        self.recovery_window_in_days = None;
        Ok(())
    }

    /// Configure rotation for this secret.
    pub fn configure_rotation(
        &mut self,
        lambda_arn: Option<String>,
        rules: Option<RotationRulesType>,
    ) {
        if let Some(arn) = lambda_arn {
            self.rotation_lambda_arn = Some(arn);
        }
        if let Some(r) = rules {
            self.rotation_rules = Some(r);
        }
        self.rotation_enabled = true;
    }

    /// Start a rotation by creating a pending version.
    ///
    /// Clones the current value into the AWSPENDING version since we cannot
    /// invoke a real Lambda function for local development.
    pub fn start_rotation(
        &mut self,
        version_id: String,
        now: chrono::DateTime<chrono::Utc>,
        rotate_immediately: bool,
    ) -> Result<(), SecretsManagerError> {
        // Get the current version's value.
        let current_value = self
            .staging_labels
            .get(AWSCURRENT)
            .and_then(|vid| self.versions.get(vid))
            .map(|v| (v.secret_string.clone(), v.secret_binary.clone()));

        let (ss, sb) = current_value.unwrap_or((None, None));

        if rotate_immediately {
            // Create version directly with AWSCURRENT (promotes immediately).
            self.add_version(version_id, ss, sb, vec![AWSCURRENT.to_owned()], now)?;
        } else {
            // Create version with AWSPENDING only.
            self.add_version(version_id, ss, sb, vec![AWSPENDING.to_owned()], now)?;
        }

        self.last_rotated_date = Some(now);
        Ok(())
    }

    /// Get the version labeled `AWSCURRENT`.
    #[must_use]
    pub fn get_current_version(&self) -> Option<&SecretVersion> {
        self.staging_labels
            .get(AWSCURRENT)
            .and_then(|vid| self.versions.get(vid))
    }
}

/// In-memory secret store.
#[derive(Debug)]
pub struct SecretStore {
    /// All secrets keyed by name.
    secrets: DashMap<String, SecretRecord>,
}

impl SecretStore {
    /// Create a new empty secret store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            secrets: DashMap::new(),
        }
    }

    /// Access the underlying `DashMap` for iteration.
    #[must_use]
    pub fn secrets(&self) -> &DashMap<String, SecretRecord> {
        &self.secrets
    }

    /// Generate a secret ARN with a 6-character random suffix.
    #[must_use]
    pub fn secret_arn(region: &str, account_id: &str, name: &str) -> String {
        let mut rng = rand::rng();
        let suffix: String = (0..6)
            .map(|_| {
                let idx: usize = rng.random_range(0..36);
                if idx < 10 {
                    char::from(b'0' + u8::try_from(idx).unwrap_or(0))
                } else {
                    char::from(b'a' + u8::try_from(idx - 10).unwrap_or(0))
                }
            })
            .collect();
        format!("arn:aws:secretsmanager:{region}:{account_id}:secret:{name}-{suffix}")
    }

    /// Resolve a `SecretId` to a secret name.
    ///
    /// Handles name, full ARN, and partial ARN lookups.
    pub fn resolve_secret_id(&self, secret_id: &str) -> Result<String, SecretsManagerError> {
        // Direct name match.
        if self.secrets.contains_key(secret_id) {
            return Ok(secret_id.to_owned());
        }

        // ARN match (full or partial).
        if secret_id.starts_with("arn:") {
            for entry in &self.secrets {
                let record = entry.value();
                if record.arn == secret_id || record.arn.starts_with(secret_id) {
                    return Ok(record.name.clone());
                }
            }
        }

        Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::ResourceNotFoundException,
            "Secrets Manager can't find the specified secret.",
        ))
    }

    /// Get a reference to a secret record by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, String, SecretRecord>> {
        self.secrets.get(name)
    }

    /// Get a mutable reference to a secret record by name.
    #[must_use]
    pub fn get_mut(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, SecretRecord>> {
        self.secrets.get_mut(name)
    }

    /// Insert a new secret record.
    pub fn insert(&self, name: String, record: SecretRecord) {
        self.secrets.insert(name, record);
    }

    /// Remove a secret by name. Returns the removed record if it existed.
    #[must_use]
    pub fn remove(&self, name: &str) -> Option<(String, SecretRecord)> {
        self.secrets.remove(name)
    }

    /// Check if a secret exists by name.
    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.secrets.contains_key(name)
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
