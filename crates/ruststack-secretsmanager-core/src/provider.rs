//! Secrets Manager provider implementing all 23 operations.

use std::collections::HashMap;

use chrono::Utc;

use ruststack_secretsmanager_model::error::{SecretsManagerError, SecretsManagerErrorCode};
use ruststack_secretsmanager_model::input::{
    BatchGetSecretValueInput, CancelRotateSecretInput, CreateSecretInput,
    DeleteResourcePolicyInput, DeleteSecretInput, DescribeSecretInput, GetRandomPasswordInput,
    GetResourcePolicyInput, GetSecretValueInput, ListSecretVersionIdsInput, ListSecretsInput,
    PutResourcePolicyInput, PutSecretValueInput, RemoveRegionsFromReplicationInput,
    ReplicateSecretToRegionsInput, RestoreSecretInput, RotateSecretInput,
    StopReplicationToReplicaInput, TagResourceInput, UntagResourceInput, UpdateSecretInput,
    UpdateSecretVersionStageInput, ValidateResourcePolicyInput,
};
use ruststack_secretsmanager_model::output::{
    BatchGetSecretValueResponse, CancelRotateSecretResponse, CreateSecretResponse,
    DeleteResourcePolicyResponse, DeleteSecretResponse, DescribeSecretResponse,
    GetRandomPasswordResponse, GetResourcePolicyResponse, GetSecretValueResponse,
    ListSecretVersionIdsResponse, ListSecretsResponse, PutResourcePolicyResponse,
    PutSecretValueResponse, RemoveRegionsFromReplicationResponse, ReplicateSecretToRegionsResponse,
    RestoreSecretResponse, RotateSecretResponse, StopReplicationToReplicaResponse,
    UpdateSecretResponse, UpdateSecretVersionStageResponse, ValidateResourcePolicyResponse,
};
use ruststack_secretsmanager_model::types::{
    APIErrorType, SecretListEntry, SecretValueEntry, SecretVersionsListEntry, SortByType,
    SortOrderType,
};

use crate::config::SecretsManagerConfig;
use crate::filter::matches_filters;
use crate::password::generate_random_password;
use crate::storage::{SecretRecord, SecretStore, SecretVersion};
use crate::validation::{
    MAX_TAGS, validate_client_request_token, validate_description, validate_recovery_window,
    validate_secret_name, validate_secret_value, validate_tags,
};
use crate::version::AWSCURRENT;

/// Default max results for `ListSecrets`.
const DEFAULT_LIST_MAX_RESULTS: i32 = 100;

/// Maximum max results for `ListSecrets`.
const MAX_LIST_MAX_RESULTS: i32 = 100;

/// Default recovery window in days.
const DEFAULT_RECOVERY_WINDOW_DAYS: i64 = 30;

/// The Secrets Manager provider.
#[derive(Debug)]
pub struct RustStackSecretsManager {
    config: SecretsManagerConfig,
    store: SecretStore,
}

impl RustStackSecretsManager {
    /// Create a new Secrets Manager provider with the given configuration.
    #[must_use]
    pub fn new(config: SecretsManagerConfig) -> Self {
        Self {
            config,
            store: SecretStore::new(),
        }
    }

    // =========================================================================
    // Phase 0: Core CRUD
    // =========================================================================

    /// Handle `CreateSecret`.
    pub fn handle_create_secret(
        &self,
        input: CreateSecretInput,
    ) -> Result<CreateSecretResponse, SecretsManagerError> {
        validate_secret_name(&input.name)?;

        if let Some(ref desc) = input.description {
            validate_description(desc)?;
        }
        validate_tags(&input.tags)?;

        // Check if secret with same name exists (including pending deletion).
        if self.store.contains_key(&input.name) {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceExistsException,
                format!(
                    "The operation failed because the secret {} already exists.",
                    input.name
                ),
            ));
        }

        let now = Utc::now();
        let arn = SecretStore::secret_arn(
            &self.config.default_region,
            &self.config.default_account_id,
            &input.name,
        );

        let mut record = SecretRecord {
            name: input.name.clone(),
            arn: arn.clone(),
            description: input.description,
            kms_key_id: input.kms_key_id,
            tags: input.tags,
            resource_policy: None,
            versions: HashMap::new(),
            staging_labels: HashMap::new(),
            rotation_enabled: false,
            rotation_lambda_arn: None,
            rotation_rules: None,
            last_rotated_date: None,
            deleted_date: None,
            recovery_window_in_days: None,
            created_date: now,
            last_changed_date: now,
            last_accessed_date: None,
            owning_service: None,
            primary_region: Some(self.config.default_region.clone()),
        };

        // Create a version if a value is provided.
        let version_id = if input.secret_string.is_some() || input.secret_binary.is_some() {
            let vid = input
                .client_request_token
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            record.add_version(
                vid.clone(),
                input.secret_string,
                input.secret_binary.map(|b| b.to_vec()),
                vec![AWSCURRENT.to_owned()],
                now,
            )?;

            Some(vid)
        } else {
            None
        };

        self.store.insert(input.name.clone(), record);

        Ok(CreateSecretResponse {
            arn: Some(arn),
            name: Some(input.name),
            version_id,
            replication_status: Vec::new(),
        })
    }

    /// Handle `GetSecretValue`.
    pub fn handle_get_secret_value(
        &self,
        input: &GetSecretValueInput,
    ) -> Result<GetSecretValueResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if record.is_pending_deletion() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on the secret because it was marked for deletion.",
            ));
        }

        // Determine which version to return.
        let version = resolve_version_from_record(
            &record,
            input.version_id.as_ref(),
            input.version_stage.as_ref(),
        )?;

        let response = GetSecretValueResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id: Some(version.version_id.clone()),
            secret_string: version.secret_string.clone(),
            secret_binary: version
                .secret_binary
                .as_ref()
                .map(|b| bytes::Bytes::from(b.clone())),
            version_stages: version.version_stages.clone(),
            created_date: Some(version.created_date),
        };

        // Update LastAccessedDate (date-only granularity).
        let today = Utc::now().date_naive().and_hms_opt(0, 0, 0);
        if let Some(midnight) = today {
            record.last_accessed_date = Some(chrono::DateTime::from_naive_utc_and_offset(
                midnight,
                chrono::Utc,
            ));
        }

        Ok(response)
    }

    /// Handle `PutSecretValue`.
    pub fn handle_put_secret_value(
        &self,
        input: PutSecretValueInput,
    ) -> Result<PutSecretValueResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        validate_secret_value(input.secret_string.as_deref(), input.secret_binary.as_ref())?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if record.is_pending_deletion() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on the secret because it was marked for deletion.",
            ));
        }

        let version_id = if let Some(ref token) = input.client_request_token {
            validate_client_request_token(token)?;
            token.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        let stages = if input.version_stages.is_empty() {
            vec![AWSCURRENT.to_owned()]
        } else {
            input.version_stages
        };

        let now = Utc::now();

        record.add_version(
            version_id.clone(),
            input.secret_string,
            input.secret_binary.map(|b| b.to_vec()),
            stages.clone(),
            now,
        )?;

        // Rebuild to get the actual stages after promotion.
        let actual_stages = record
            .versions
            .get(&version_id)
            .map(|v| v.version_stages.clone())
            .unwrap_or(stages);

        Ok(PutSecretValueResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id: Some(version_id),
            version_stages: actual_stages,
        })
    }

    /// Handle `DescribeSecret`.
    pub fn handle_describe_secret(
        &self,
        input: &DescribeSecretInput,
    ) -> Result<DescribeSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        Ok(build_describe_response(&record))
    }

    /// Handle `DeleteSecret`.
    pub fn handle_delete_secret(
        &self,
        input: &DeleteSecretInput,
    ) -> Result<DeleteSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let force = input.force_delete_without_recovery.unwrap_or(false);
        let recovery_days = input.recovery_window_in_days;

        // Cannot specify both force and recovery window.
        if force && recovery_days.is_some() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidParameterException,
                "You can't use ForceDeleteWithoutRecovery in conjunction with RecoveryWindowInDays.",
            ));
        }

        if force {
            // Immediately remove the secret.
            let (_, removed) = self.store.remove(&name).ok_or_else(|| {
                SecretsManagerError::with_message(
                    SecretsManagerErrorCode::ResourceNotFoundException,
                    "Secrets Manager can't find the specified secret.",
                )
            })?;

            Ok(DeleteSecretResponse {
                arn: Some(removed.arn),
                name: Some(removed.name),
                deletion_date: Some(Utc::now()),
            })
        } else {
            let days = recovery_days.unwrap_or(DEFAULT_RECOVERY_WINDOW_DAYS);
            validate_recovery_window(days)?;

            let mut record = self.store.get_mut(&name).ok_or_else(|| {
                SecretsManagerError::with_message(
                    SecretsManagerErrorCode::ResourceNotFoundException,
                    "Secrets Manager can't find the specified secret.",
                )
            })?;

            let now = Utc::now();
            record.schedule_deletion(days, now);

            let deletion_date = now + chrono::Duration::days(days);

            Ok(DeleteSecretResponse {
                arn: Some(record.arn.clone()),
                name: Some(record.name.clone()),
                deletion_date: Some(deletion_date),
            })
        }
    }

    /// Handle `RestoreSecret`.
    pub fn handle_restore_secret(
        &self,
        input: &RestoreSecretInput,
    ) -> Result<RestoreSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        record.restore()?;

        Ok(RestoreSecretResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
        })
    }

    /// Handle `UpdateSecret`.
    pub fn handle_update_secret(
        &self,
        input: UpdateSecretInput,
    ) -> Result<UpdateSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if record.is_pending_deletion() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on the secret because it was marked for deletion.",
            ));
        }

        // Update metadata.
        if let Some(desc) = input.description {
            validate_description(&desc)?;
            record.description = Some(desc);
        }
        if let Some(kms) = input.kms_key_id {
            record.kms_key_id = Some(kms);
        }

        // If a new value is provided, create a new version.
        let version_id = if input.secret_string.is_some() || input.secret_binary.is_some() {
            let vid = if let Some(ref token) = input.client_request_token {
                validate_client_request_token(token)?;
                token.clone()
            } else {
                uuid::Uuid::new_v4().to_string()
            };

            let now = Utc::now();
            record.add_version(
                vid.clone(),
                input.secret_string,
                input.secret_binary.map(|b| b.to_vec()),
                vec![AWSCURRENT.to_owned()],
                now,
            )?;

            Some(vid)
        } else {
            record.last_changed_date = Utc::now();
            None
        };

        Ok(UpdateSecretResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id,
        })
    }

    /// Handle `ListSecrets`.
    pub fn handle_list_secrets(
        &self,
        input: &ListSecretsInput,
    ) -> Result<ListSecretsResponse, SecretsManagerError> {
        let include_planned_deletion = input.include_planned_deletion.unwrap_or(false);

        #[allow(clippy::cast_sign_loss)]
        let max_results = input
            .max_results
            .unwrap_or(DEFAULT_LIST_MAX_RESULTS)
            .clamp(1, MAX_LIST_MAX_RESULTS) as usize;

        // Collect matching secrets.
        let mut entries: Vec<SecretListEntry> = Vec::new();
        for entry in self.store.secrets() {
            let record = entry.value();

            // Skip deleted unless requested.
            if record.is_pending_deletion() && !include_planned_deletion {
                continue;
            }

            // Apply filters.
            if !matches_filters(record, &input.filters) {
                continue;
            }

            entries.push(build_secret_list_entry(record));
        }

        // Sort.
        let sort_by = input.sort_by.as_ref().unwrap_or(&SortByType::CreatedDate);
        let sort_order = input.sort_order.as_ref().unwrap_or(&SortOrderType::Asc);
        sort_entries(&mut entries, sort_by, sort_order);

        // Paginate.
        let start_idx = decode_offset(input.next_token.as_deref())?.unwrap_or(0);

        if start_idx >= entries.len() {
            return Ok(ListSecretsResponse {
                secret_list: Vec::new(),
                next_token: None,
            });
        }

        let page = &entries[start_idx..];
        let take = page.len().min(max_results);
        let result = page[..take].to_vec();

        let next_token = if take < page.len() {
            Some(encode_offset(start_idx + take))
        } else {
            None
        };

        Ok(ListSecretsResponse {
            secret_list: result,
            next_token,
        })
    }

    /// Handle `ListSecretVersionIds`.
    pub fn handle_list_secret_version_ids(
        &self,
        input: &ListSecretVersionIdsInput,
    ) -> Result<ListSecretVersionIdsResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        let include_deprecated = input.include_deprecated.unwrap_or(false);

        #[allow(clippy::cast_sign_loss)]
        let max_results = input.max_results.unwrap_or(100).clamp(1, 100) as usize;

        // Collect versions.
        let mut version_entries: Vec<SecretVersionsListEntry> = record
            .versions
            .values()
            .filter(|v| include_deprecated || !v.version_stages.is_empty())
            .map(|v| SecretVersionsListEntry {
                version_id: Some(v.version_id.clone()),
                version_stages: v.version_stages.clone(),
                created_date: Some(v.created_date),
                last_accessed_date: record.last_accessed_date,
                kms_key_ids: record
                    .kms_key_id
                    .as_ref()
                    .map(|k| vec![k.clone()])
                    .unwrap_or_default(),
            })
            .collect();

        // Sort by created_date for deterministic output.
        version_entries.sort_by_key(|a| a.created_date);

        // Paginate.
        let start_idx = decode_offset(input.next_token.as_deref())?.unwrap_or(0);

        if start_idx >= version_entries.len() {
            return Ok(ListSecretVersionIdsResponse {
                arn: Some(record.arn.clone()),
                name: Some(record.name.clone()),
                versions: Vec::new(),
                next_token: None,
            });
        }

        let page = &version_entries[start_idx..];
        let take = page.len().min(max_results);
        let result = page[..take].to_vec();

        let next_token = if take < page.len() {
            Some(encode_offset(start_idx + take))
        } else {
            None
        };

        Ok(ListSecretVersionIdsResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            versions: result,
            next_token,
        })
    }

    /// Handle `GetRandomPassword`.
    pub fn handle_get_random_password(
        &self,
        input: &GetRandomPasswordInput,
    ) -> Result<GetRandomPasswordResponse, SecretsManagerError> {
        let password = generate_random_password(
            input.password_length,
            input.exclude_characters.as_deref(),
            input.exclude_lowercase.unwrap_or(false),
            input.exclude_uppercase.unwrap_or(false),
            input.exclude_numbers.unwrap_or(false),
            input.exclude_punctuation.unwrap_or(false),
            input.include_space.unwrap_or(false),
            input.require_each_included_type.unwrap_or(true),
        )?;

        Ok(GetRandomPasswordResponse {
            random_password: Some(password),
        })
    }

    // =========================================================================
    // Phase 1: Tags and Resource Policies
    // =========================================================================

    /// Handle `TagResource`.
    pub fn handle_tag_resource(&self, input: &TagResourceInput) -> Result<(), SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        validate_tags(&input.tags)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        // Count new keys (not already present).
        let new_key_count = input
            .tags
            .iter()
            .filter(|t| {
                let key = t.key.as_deref().unwrap_or("");
                !record
                    .tags
                    .iter()
                    .any(|existing| existing.key.as_deref() == Some(key))
            })
            .count();

        if record.tags.len() + new_key_count > MAX_TAGS {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidParameterException,
                format!(
                    "Adding {new_key_count} tag(s) would exceed the maximum of {MAX_TAGS} tags."
                ),
            ));
        }

        // Merge tags (overwrite existing keys).
        for new_tag in &input.tags {
            let key = new_tag.key.as_deref().unwrap_or("");
            if let Some(existing) = record
                .tags
                .iter_mut()
                .find(|t| t.key.as_deref() == Some(key))
            {
                existing.value.clone_from(&new_tag.value);
            } else {
                record.tags.push(new_tag.clone());
            }
        }

        Ok(())
    }

    /// Handle `UntagResource`.
    pub fn handle_untag_resource(
        &self,
        input: &UntagResourceInput,
    ) -> Result<(), SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        record.tags.retain(|t| {
            !input
                .tag_keys
                .iter()
                .any(|k| t.key.as_deref() == Some(k.as_str()))
        });

        Ok(())
    }

    /// Handle `UpdateSecretVersionStage`.
    pub fn handle_update_secret_version_stage(
        &self,
        input: &UpdateSecretVersionStageInput,
    ) -> Result<UpdateSecretVersionStageResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if let Some(ref to_vid) = input.move_to_version_id {
            record.move_staging_label(
                &input.version_stage,
                to_vid,
                input.remove_from_version_id.as_deref(),
            )?;
        } else if let Some(ref from_vid) = input.remove_from_version_id {
            // Just remove the label.
            let current_holder = record.staging_labels.get(&input.version_stage);
            if current_holder.map(String::as_str) != Some(from_vid.as_str()) {
                return Err(SecretsManagerError::with_message(
                    SecretsManagerErrorCode::InvalidParameterException,
                    "The staging label is not currently attached to the specified version.",
                ));
            }
            record.staging_labels.remove(&input.version_stage);
            record.rebuild_version_stages();
        }

        Ok(UpdateSecretVersionStageResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
        })
    }

    /// Handle `RotateSecret`.
    pub fn handle_rotate_secret(
        &self,
        input: RotateSecretInput,
    ) -> Result<RotateSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if record.is_pending_deletion() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on the secret because it was marked for deletion.",
            ));
        }

        // Configure rotation if provided.
        record.configure_rotation(input.rotation_lambda_arn, input.rotation_rules);

        let version_id = if let Some(ref token) = input.client_request_token {
            validate_client_request_token(token)?;
            token.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        let rotate_immediately = input.rotate_immediately.unwrap_or(true);
        let now = Utc::now();

        record.start_rotation(version_id.clone(), now, rotate_immediately)?;

        Ok(RotateSecretResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id: Some(version_id),
        })
    }

    /// Handle `CancelRotateSecret`.
    pub fn handle_cancel_rotate_secret(
        &self,
        input: &CancelRotateSecretInput,
    ) -> Result<CancelRotateSecretResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        // Remove AWSPENDING label if it exists.
        record.staging_labels.remove("AWSPENDING");
        record.rotation_enabled = false;
        record.rebuild_version_stages();

        Ok(CancelRotateSecretResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id: None,
        })
    }

    /// Handle `BatchGetSecretValue`.
    pub fn handle_batch_get_secret_value(
        &self,
        input: &BatchGetSecretValueInput,
    ) -> Result<BatchGetSecretValueResponse, SecretsManagerError> {
        let mut secret_values: Vec<SecretValueEntry> = Vec::new();
        let mut errors: Vec<APIErrorType> = Vec::new();

        // If secret_id_list is provided, resolve each.
        if !input.secret_id_list.is_empty() {
            for secret_id in &input.secret_id_list {
                match self.get_secret_value_for_batch(secret_id) {
                    Ok(entry) => secret_values.push(entry),
                    Err(err) => {
                        errors.push(APIErrorType {
                            secret_id: Some(secret_id.clone()),
                            error_code: Some(err.code.as_str().to_owned()),
                            message: Some(err.message.clone()),
                        });
                    }
                }
            }
        } else if !input.filters.is_empty() {
            // Filter-based batch retrieval.
            for entry in self.store.secrets() {
                let record = entry.value();
                if record.is_pending_deletion() {
                    continue;
                }
                if matches_filters(record, &input.filters) {
                    match self.get_secret_value_for_batch(&record.name) {
                        Ok(sv) => secret_values.push(sv),
                        Err(err) => {
                            errors.push(APIErrorType {
                                secret_id: Some(record.name.clone()),
                                error_code: Some(err.code.as_str().to_owned()),
                                message: Some(err.message.clone()),
                            });
                        }
                    }
                }
            }
        }

        Ok(BatchGetSecretValueResponse {
            secret_values,
            errors,
            next_token: None,
        })
    }

    // =========================================================================
    // Phase 2: Resource Policies
    // =========================================================================

    /// Handle `GetResourcePolicy`.
    pub fn handle_get_resource_policy(
        &self,
        input: &GetResourcePolicyInput,
    ) -> Result<GetResourcePolicyResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        Ok(GetResourcePolicyResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            resource_policy: record.resource_policy.clone(),
        })
    }

    /// Handle `PutResourcePolicy`.
    pub fn handle_put_resource_policy(
        &self,
        input: &PutResourcePolicyInput,
    ) -> Result<PutResourcePolicyResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        record.resource_policy = Some(input.resource_policy.clone());

        Ok(PutResourcePolicyResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
        })
    }

    /// Handle `DeleteResourcePolicy`.
    pub fn handle_delete_resource_policy(
        &self,
        input: &DeleteResourcePolicyInput,
    ) -> Result<DeleteResourcePolicyResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let mut record = self.store.get_mut(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        record.resource_policy = None;

        Ok(DeleteResourcePolicyResponse {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
        })
    }

    /// Handle `ValidateResourcePolicy`.
    pub fn handle_validate_resource_policy(
        &self,
        _input: &ValidateResourcePolicyInput,
    ) -> Result<ValidateResourcePolicyResponse, SecretsManagerError> {
        // Always return valid (no actual policy validation engine).
        Ok(ValidateResourcePolicyResponse {
            policy_validation_passed: Some(true),
            validation_errors: Vec::new(),
        })
    }

    /// Handle `ReplicateSecretToRegions` (stub).
    pub fn handle_replicate_secret_to_regions(
        &self,
        input: &ReplicateSecretToRegionsInput,
    ) -> Result<ReplicateSecretToRegionsResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        Ok(ReplicateSecretToRegionsResponse {
            arn: Some(record.arn.clone()),
            replication_status: Vec::new(),
        })
    }

    /// Handle `RemoveRegionsFromReplication` (stub).
    pub fn handle_remove_regions_from_replication(
        &self,
        input: &RemoveRegionsFromReplicationInput,
    ) -> Result<RemoveRegionsFromReplicationResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        Ok(RemoveRegionsFromReplicationResponse {
            arn: Some(record.arn.clone()),
            replication_status: Vec::new(),
        })
    }

    /// Handle `StopReplicationToReplica` (stub).
    pub fn handle_stop_replication_to_replica(
        &self,
        input: &StopReplicationToReplicaInput,
    ) -> Result<StopReplicationToReplicaResponse, SecretsManagerError> {
        let name = self.store.resolve_secret_id(&input.secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        Ok(StopReplicationToReplicaResponse {
            arn: Some(record.arn.clone()),
        })
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Get a secret value for batch operations.
    fn get_secret_value_for_batch(
        &self,
        secret_id: &str,
    ) -> Result<SecretValueEntry, SecretsManagerError> {
        let name = self.store.resolve_secret_id(secret_id)?;

        let record = self.store.get(&name).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "Secrets Manager can't find the specified secret.",
            )
        })?;

        if record.is_pending_deletion() {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidRequestException,
                "You can't perform this operation on the secret because it was marked for deletion.",
            ));
        }

        let version = record.get_current_version().ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                "The secret has no current version.",
            )
        })?;

        Ok(SecretValueEntry {
            arn: Some(record.arn.clone()),
            name: Some(record.name.clone()),
            version_id: Some(version.version_id.clone()),
            secret_string: version.secret_string.clone(),
            secret_binary: version
                .secret_binary
                .as_ref()
                .map(|b| bytes::Bytes::from(b.clone())),
            version_stages: version.version_stages.clone(),
            created_date: Some(version.created_date),
        })
    }
}

/// Resolve a version from a record given optional version_id and version_stage.
fn resolve_version_from_record<'a>(
    record: &'a SecretRecord,
    version_id: Option<&String>,
    version_stage: Option<&String>,
) -> Result<&'a SecretVersion, SecretsManagerError> {
    if let Some(vid) = version_id {
        // Look up by version ID.
        return record.versions.get(vid).ok_or_else(|| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::ResourceNotFoundException,
                format!(
                    "Secrets Manager can't find the specified secret value for VersionId: {vid}"
                ),
            )
        });
    }

    // Look up by stage (default to AWSCURRENT).
    let stage = version_stage.map_or(AWSCURRENT, String::as_str);
    let vid = record.staging_labels.get(stage).ok_or_else(|| {
        SecretsManagerError::with_message(
            SecretsManagerErrorCode::ResourceNotFoundException,
            format!(
                "Secrets Manager can't find the specified secret value for staging label: {stage}"
            ),
        )
    })?;

    record.versions.get(vid).ok_or_else(|| {
        SecretsManagerError::with_message(
            SecretsManagerErrorCode::ResourceNotFoundException,
            "Secrets Manager can't find the specified secret value.",
        )
    })
}

/// Build a `DescribeSecretResponse` from a record.
fn build_describe_response(record: &SecretRecord) -> DescribeSecretResponse {
    DescribeSecretResponse {
        arn: Some(record.arn.clone()),
        name: Some(record.name.clone()),
        description: record.description.clone(),
        kms_key_id: record.kms_key_id.clone(),
        rotation_enabled: Some(record.rotation_enabled),
        rotation_lambda_arn: record.rotation_lambda_arn.clone(),
        rotation_rules: record.rotation_rules.clone(),
        last_rotated_date: record.last_rotated_date,
        last_changed_date: Some(record.last_changed_date),
        last_accessed_date: record.last_accessed_date,
        deleted_date: record.deleted_date,
        tags: record.tags.clone(),
        version_ids_to_stages: record.version_ids_to_stages(),
        created_date: Some(record.created_date),
        primary_region: record.primary_region.clone(),
        replication_status: Vec::new(),
        owning_service: record.owning_service.clone(),
        r#type: None,
        next_rotation_date: None,
        external_secret_rotation_metadata: Vec::new(),
        external_secret_rotation_role_arn: None,
    }
}

/// Build a `SecretListEntry` from a record.
fn build_secret_list_entry(record: &SecretRecord) -> SecretListEntry {
    SecretListEntry {
        arn: Some(record.arn.clone()),
        name: Some(record.name.clone()),
        description: record.description.clone(),
        kms_key_id: record.kms_key_id.clone(),
        rotation_enabled: Some(record.rotation_enabled),
        rotation_lambda_arn: record.rotation_lambda_arn.clone(),
        rotation_rules: record.rotation_rules.clone(),
        last_rotated_date: record.last_rotated_date,
        last_changed_date: Some(record.last_changed_date),
        last_accessed_date: record.last_accessed_date,
        deleted_date: record.deleted_date,
        tags: record.tags.clone(),
        secret_versions_to_stages: record.version_ids_to_stages(),
        created_date: Some(record.created_date),
        primary_region: record.primary_region.clone(),
        owning_service: record.owning_service.clone(),
        r#type: None,
        next_rotation_date: None,
        external_secret_rotation_metadata: Vec::new(),
        external_secret_rotation_role_arn: None,
    }
}

/// Sort `SecretListEntry` by the given field and order.
fn sort_entries(entries: &mut [SecretListEntry], sort_by: &SortByType, sort_order: &SortOrderType) {
    entries.sort_by(|a, b| {
        let cmp = match sort_by {
            SortByType::Name => a.name.cmp(&b.name),
            SortByType::CreatedDate => a.created_date.cmp(&b.created_date),
            SortByType::LastAccessedDate => a.last_accessed_date.cmp(&b.last_accessed_date),
            SortByType::LastChangedDate => a.last_changed_date.cmp(&b.last_changed_date),
        };
        match sort_order {
            SortOrderType::Asc => cmp,
            SortOrderType::Desc => cmp.reverse(),
        }
    });
}

/// Encode a pagination offset as a base64 next token.
fn encode_offset(offset: usize) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(offset.to_string())
}

/// Decode a base64 next token back to an offset.
fn decode_offset(token: Option<&str>) -> Result<Option<usize>, SecretsManagerError> {
    use base64::Engine;
    let Some(token) = token else {
        return Ok(None);
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .map_err(|_| {
            SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidNextTokenException,
                "The specified token is invalid.",
            )
        })?;
    let s = String::from_utf8(decoded).map_err(|_| {
        SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidNextTokenException,
            "The specified token is invalid.",
        )
    })?;
    let offset = s.parse::<usize>().map_err(|_| {
        SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidNextTokenException,
            "The specified token is invalid.",
        )
    })?;
    Ok(Some(offset))
}
