//! In-memory storage engine for SSM Parameter Store.
//!
//! Parameters are stored in a `DashMap<String, ParameterRecord>` keyed by
//! the fully-qualified parameter name. Each record tracks the current version,
//! all version snapshots (up to 100), tags, and metadata.

use std::collections::{BTreeMap, HashMap, HashSet};

use dashmap::DashMap;

use ruststack_ssm_model::error::{SsmError, SsmErrorCode};
use ruststack_ssm_model::types::{
    Parameter, ParameterHistory, ParameterInlinePolicy, ParameterMetadata, ParameterTier,
    ParameterType, Tag,
};

use crate::filter::matches_filters;

use crate::selector::ParameterSelector;
use crate::validation::{MAX_LABELS_PER_VERSION, MAX_VERSIONS, is_valid_label};

/// A snapshot of a single parameter version.
#[derive(Debug, Clone)]
pub struct ParameterVersion {
    /// The version number (1-indexed).
    pub version: u64,
    /// The parameter value.
    pub value: String,
    /// An optional description.
    pub description: Option<String>,
    /// An optional regex pattern for value validation.
    pub allowed_pattern: Option<String>,
    /// The data type (default `"text"`).
    pub data_type: String,
    /// The parameter tier.
    pub tier: ParameterTier,
    /// Labels attached to this version (max 10).
    pub labels: HashSet<String>,
    /// Parameter policies as JSON strings.
    pub policies: Vec<String>,
    /// Epoch seconds when this version was last modified.
    pub last_modified_date: f64,
    /// The ARN of the user who last modified this version.
    pub last_modified_user: String,
}

/// A parameter record containing all versions and metadata.
#[derive(Debug, Clone)]
pub struct ParameterRecord {
    /// The fully-qualified parameter name.
    pub name: String,
    /// The current (latest) version number.
    pub current_version: u64,
    /// All version snapshots keyed by version number.
    pub versions: BTreeMap<u64, ParameterVersion>,
    /// Tags associated with this parameter.
    pub tags: HashMap<String, String>,
    /// The parameter type.
    pub parameter_type: ParameterType,
    /// The KMS key ID for SecureString parameters.
    pub key_id: Option<String>,
}

/// In-memory parameter store.
#[derive(Debug)]
pub struct ParameterStore {
    /// All parameters keyed by name.
    parameters: DashMap<String, ParameterRecord>,
}

impl ParameterStore {
    /// Create a new empty parameter store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parameters: DashMap::new(),
        }
    }

    /// Put a parameter, creating or updating it.
    ///
    /// Returns the new version number and tier.
    #[allow(clippy::too_many_arguments)]
    pub fn put_parameter(
        &self,
        name: &str,
        value: String,
        parameter_type: ParameterType,
        description: Option<String>,
        key_id: Option<String>,
        overwrite: bool,
        allowed_pattern: Option<String>,
        tags: &[Tag],
        tier: &ParameterTier,
        data_type: String,
        policies: Vec<String>,
        account_id: &str,
    ) -> Result<(u64, ParameterTier), SsmError> {
        #[allow(clippy::cast_precision_loss)]
        let now = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
        let user_arn = format!("arn:aws:iam::{account_id}:root");

        // Check if parameter already exists.
        if let Some(mut record) = self.parameters.get_mut(name) {
            if !overwrite {
                return Err(SsmError::parameter_already_exists(name));
            }

            // Type cannot change on overwrite.
            if record.parameter_type != parameter_type {
                return Err(SsmError::with_message(
                    ruststack_ssm_model::error::SsmErrorCode::HierarchyTypeMismatch,
                    format!(
                        "The parameter type '{}' does not match the existing type '{}'.",
                        parameter_type, record.parameter_type,
                    ),
                ));
            }

            // Enforce 100-version cap: delete oldest unlabeled version.
            if record.versions.len() >= MAX_VERSIONS {
                // Find the oldest version without labels.
                let oldest_unlabeled = record
                    .versions
                    .iter()
                    .find(|(_, v)| v.labels.is_empty())
                    .map(|(k, _)| *k);

                if let Some(oldest_key) = oldest_unlabeled {
                    record.versions.remove(&oldest_key);
                } else {
                    // All versions have labels — cannot make room.
                    return Err(SsmError::with_message(
                        ruststack_ssm_model::error::SsmErrorCode::ParameterMaxVersionLimitExceeded,
                        format!(
                            "Parameter {name} has reached the maximum number of \
                             {MAX_VERSIONS} versions."
                        ),
                    ));
                }
            }

            let new_version = record.current_version + 1;
            let effective_tier = effective_tier(tier, &value);

            let version_snapshot = ParameterVersion {
                version: new_version,
                value,
                description,
                allowed_pattern,
                data_type,
                tier: effective_tier.clone(),
                labels: HashSet::new(),
                policies,
                last_modified_date: now,
                last_modified_user: user_arn,
            };

            record.current_version = new_version;
            if let Some(kid) = key_id {
                record.key_id = Some(kid);
            }
            record.versions.insert(new_version, version_snapshot);

            Ok((new_version, effective_tier))
        } else {
            // New parameter.
            let effective_tier = effective_tier(tier, &value);

            let version_snapshot = ParameterVersion {
                version: 1,
                value,
                description,
                allowed_pattern,
                data_type,
                tier: effective_tier.clone(),
                labels: HashSet::new(),
                policies,
                last_modified_date: now,
                last_modified_user: user_arn,
            };

            let mut tag_map = HashMap::new();
            for tag in tags {
                tag_map.insert(tag.key.clone(), tag.value.clone());
            }

            let mut versions = BTreeMap::new();
            versions.insert(1, version_snapshot);

            let record = ParameterRecord {
                name: name.to_owned(),
                current_version: 1,
                versions,
                tags: tag_map,
                parameter_type,
                key_id,
            };

            self.parameters.insert(name.to_owned(), record);

            Ok((1, effective_tier))
        }
    }

    /// Get a parameter by name with an optional selector.
    pub fn get_parameter(
        &self,
        name: &str,
        selector: Option<&ParameterSelector>,
        region: &str,
        account_id: &str,
    ) -> Result<Parameter, SsmError> {
        let record = self
            .parameters
            .get(name)
            .ok_or_else(|| SsmError::parameter_not_found(name))?;

        let version = resolve_version(&record, selector)?;

        Ok(build_parameter(&record, version, region, account_id))
    }

    /// Get parameters by a list of names (batch).
    ///
    /// Duplicate names are deduplicated (only one result per unique name).
    #[must_use]
    pub fn get_parameters(
        &self,
        names: &[String],
        region: &str,
        account_id: &str,
    ) -> (Vec<Parameter>, Vec<String>) {
        let mut found = Vec::new();
        let mut invalid = Vec::new();
        let mut seen = HashSet::new();

        for name in names {
            if !seen.insert(name.clone()) {
                continue; // Skip duplicate names.
            }

            let Ok(parsed) = crate::selector::parse_name_with_selector(name) else {
                invalid.push(name.clone());
                continue;
            };

            match self.get_parameter(&parsed.name, parsed.selector.as_ref(), region, account_id) {
                Ok(param) => found.push(param),
                Err(_) => invalid.push(name.clone()),
            }
        }

        (found, invalid)
    }

    /// Get parameters by path prefix.
    #[allow(clippy::too_many_arguments)]
    pub fn get_parameters_by_path(
        &self,
        path: &str,
        recursive: bool,
        filters: &[ruststack_ssm_model::types::ParameterStringFilter],
        max_results: usize,
        next_token: Option<&str>,
        region: &str,
        account_id: &str,
    ) -> Result<(Vec<Parameter>, Option<String>), SsmError> {
        // Normalize path to ensure trailing `/`.
        let normalized_path = if path.ends_with('/') {
            path.to_owned()
        } else {
            format!("{path}/")
        };

        // Collect all matching parameter names.
        let mut matching_names: Vec<String> = Vec::new();

        for entry in &self.parameters {
            let param_name = entry.key();

            // Parameter must start with the path prefix.
            if !param_name.starts_with(&normalized_path) {
                continue;
            }

            let remainder = &param_name[normalized_path.len()..];

            let path_match = if recursive {
                !remainder.is_empty()
            } else {
                !remainder.is_empty() && !remainder.contains('/')
            };

            if path_match && matches_filters(entry.value(), filters) {
                matching_names.push(param_name.clone());
            }
        }

        // Sort for deterministic pagination.
        matching_names.sort();

        // Decode offset from next_token.
        let start_idx = decode_offset(next_token)?.unwrap_or(0);

        if start_idx >= matching_names.len() {
            return Ok((Vec::new(), None));
        }

        let page = &matching_names[start_idx..];
        let take = page.len().min(max_results);
        let page_names = &page[..take];

        let mut parameters = Vec::with_capacity(take);
        for name in page_names {
            if let Some(record) = self.parameters.get(name) {
                if let Some(version) = record.versions.get(&record.current_version) {
                    parameters.push(build_parameter(&record, version, region, account_id));
                }
            }
        }

        let new_next_token = if take < page.len() {
            Some(encode_offset(start_idx + take))
        } else {
            None
        };

        Ok((parameters, new_next_token))
    }

    /// Delete a parameter by name.
    pub fn delete_parameter(&self, name: &str) -> Result<(), SsmError> {
        self.parameters
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| SsmError::parameter_not_found(name))
    }

    /// Delete multiple parameters by name (batch).
    #[must_use]
    pub fn delete_parameters(&self, names: &[String]) -> (Vec<String>, Vec<String>) {
        let mut deleted = Vec::new();
        let mut invalid = Vec::new();

        for name in names {
            if self.parameters.remove(name).is_some() {
                deleted.push(name.clone());
            } else {
                invalid.push(name.clone());
            }
        }

        (deleted, invalid)
    }

    /// Describe parameters with optional filtering and pagination.
    ///
    /// Returns parameter metadata (without values) matching the given filters.
    pub fn describe_parameters(
        &self,
        filters: &[ruststack_ssm_model::types::ParameterStringFilter],
        max_results: usize,
        next_token: Option<&str>,
    ) -> Result<(Vec<ParameterMetadata>, Option<String>), SsmError> {
        // Collect all matching parameter names.
        let mut matching_names: Vec<String> = Vec::new();
        for entry in &self.parameters {
            if matches_filters(entry.value(), filters) {
                matching_names.push(entry.key().clone());
            }
        }

        // Sort for deterministic pagination.
        matching_names.sort();

        // Decode offset from next_token.
        let start_idx = decode_offset(next_token)?.unwrap_or(0);

        if start_idx >= matching_names.len() {
            return Ok((Vec::new(), None));
        }

        let page = &matching_names[start_idx..];
        let take = page.len().min(max_results);
        let page_names = &page[..take];

        let mut result = Vec::with_capacity(take);
        for name in page_names {
            if let Some(record) = self.parameters.get(name) {
                result.push(build_parameter_metadata(&record));
            }
        }

        let new_next_token = if take < page.len() {
            Some(encode_offset(start_idx + take))
        } else {
            None
        };

        Ok((result, new_next_token))
    }

    /// Get the version history for a parameter.
    ///
    /// Returns all version entries ordered by version number, with pagination.
    pub fn get_parameter_history(
        &self,
        name: &str,
        max_results: usize,
        next_token: Option<&str>,
    ) -> Result<(Vec<ParameterHistory>, Option<String>), SsmError> {
        let record = self
            .parameters
            .get(name)
            .ok_or_else(|| SsmError::parameter_not_found(name))?;

        // All versions sorted by version number (BTreeMap is already sorted).
        let all_versions: Vec<&ParameterVersion> = record.versions.values().collect();
        let total = all_versions.len();

        let start_idx = decode_offset(next_token)?.unwrap_or(0);

        if start_idx >= total {
            return Ok((Vec::new(), None));
        }

        let page = &all_versions[start_idx..];
        let take = page.len().min(max_results);
        let page_versions = &page[..take];

        let entries: Vec<ParameterHistory> = page_versions
            .iter()
            .map(|ver| build_parameter_history(&record, ver))
            .collect();

        let new_next_token = if take < page.len() {
            Some(encode_offset(start_idx + take))
        } else {
            None
        };

        Ok((entries, new_next_token))
    }

    /// Add tags to a parameter, merging with existing tags.
    ///
    /// Overwrites existing tag values if the key already exists.
    /// Enforces the 50-tag limit.
    pub fn add_tags(&self, name: &str, tags: &[Tag]) -> Result<(), SsmError> {
        let mut record = self.parameters.get_mut(name).ok_or_else(|| {
            SsmError::with_message(
                SsmErrorCode::InvalidResourceId,
                format!("Parameter {name} not found."),
            )
        })?;

        // Calculate the resulting tag count (new keys only).
        let new_key_count = tags
            .iter()
            .filter(|t| !record.tags.contains_key(&t.key))
            .count();
        let total = record.tags.len() + new_key_count;

        if total > MAX_TAGS {
            return Err(SsmError::with_message(
                SsmErrorCode::TooManyTagsError,
                format!(
                    "Adding {new_key_count} tag(s) would exceed the maximum of {MAX_TAGS} tags."
                ),
            ));
        }

        for tag in tags {
            record.tags.insert(tag.key.clone(), tag.value.clone());
        }

        Ok(())
    }

    /// Remove tags from a parameter by key.
    pub fn remove_tags(&self, name: &str, tag_keys: &[String]) -> Result<(), SsmError> {
        let mut record = self.parameters.get_mut(name).ok_or_else(|| {
            SsmError::with_message(
                SsmErrorCode::InvalidResourceId,
                format!("Parameter {name} not found."),
            )
        })?;

        for key in tag_keys {
            record.tags.remove(key);
        }

        Ok(())
    }

    /// List tags for a parameter.
    pub fn list_tags(&self, name: &str) -> Result<Vec<Tag>, SsmError> {
        let record = self.parameters.get(name).ok_or_else(|| {
            SsmError::with_message(
                SsmErrorCode::InvalidResourceId,
                format!("Parameter {name} not found."),
            )
        })?;

        let mut tags: Vec<Tag> = record
            .tags
            .iter()
            .map(|(k, v)| Tag {
                key: k.clone(),
                value: v.clone(),
            })
            .collect();

        // Sort by key for deterministic output.
        tags.sort_by(|a, b| a.key.cmp(&b.key));

        Ok(tags)
    }

    /// Attach labels to a specific parameter version.
    ///
    /// If `version` is `None`, the latest version is used.
    /// Invalid-format labels are returned in the first element of the tuple.
    /// Labels are unique per parameter across versions -- attaching a label to
    /// one version removes it from any other version that had it.
    ///
    /// Returns `(invalid_labels, labeled_version)`.
    pub fn label_parameter_version(
        &self,
        name: &str,
        version: Option<u64>,
        labels: &[String],
    ) -> Result<(Vec<String>, u64), SsmError> {
        let mut record = self
            .parameters
            .get_mut(name)
            .ok_or_else(|| SsmError::parameter_not_found(name))?;

        // Resolve the target version number.
        let target_version = version.unwrap_or(record.current_version);

        // Verify the target version exists.
        if !record.versions.contains_key(&target_version) {
            return Err(SsmError::with_message(
                SsmErrorCode::ParameterVersionNotFound,
                format!(
                    "Systems Manager could not find version {target_version} of {name}. \
                     Verify the version and try again."
                ),
            ));
        }

        // Partition labels into valid and invalid.
        let mut valid_labels = Vec::new();
        let mut invalid_labels = Vec::new();
        for label in labels {
            if is_valid_label(label) {
                valid_labels.push(label.clone());
            } else {
                invalid_labels.push(label.clone());
            }
        }

        // Check label limit on the target version BEFORE modifying anything.
        // Count labels that would be newly added (not already on the target).
        {
            let target = record
                .versions
                .get(&target_version)
                .ok_or_else(|| SsmError::parameter_not_found(name))?;
            let new_label_count = valid_labels
                .iter()
                .filter(|l| !target.labels.contains(l.as_str()))
                .count();
            if target.labels.len() + new_label_count > MAX_LABELS_PER_VERSION {
                return Err(SsmError::with_message(
                    SsmErrorCode::ParameterVersionLabelLimitExceeded,
                    format!(
                        "A parameter version can have a maximum of {MAX_LABELS_PER_VERSION} labels. \
                         Move one or more labels to a different version and try again."
                    ),
                ));
            }
        }

        // Remove valid labels from any other version they might be on.
        for ver in record.versions.values_mut() {
            if ver.version != target_version {
                for label in &valid_labels {
                    ver.labels.remove(label);
                }
            }
        }

        // Attach labels to the target version.
        let target = record
            .versions
            .get_mut(&target_version)
            .ok_or_else(|| SsmError::parameter_not_found(name))?;
        for label in valid_labels {
            target.labels.insert(label);
        }

        Ok((invalid_labels, target_version))
    }

    /// Remove labels from a specific parameter version.
    ///
    /// Labels that exist on the version are removed and returned in `removed_labels`.
    /// Labels that do not exist on the version are returned in `invalid_labels`.
    ///
    /// Returns `(invalid_labels, removed_labels)`.
    pub fn unlabel_parameter_version(
        &self,
        name: &str,
        version: u64,
        labels: &[String],
    ) -> Result<(Vec<String>, Vec<String>), SsmError> {
        let mut record = self
            .parameters
            .get_mut(name)
            .ok_or_else(|| SsmError::parameter_not_found(name))?;

        // Verify the version exists.
        let target = record.versions.get_mut(&version).ok_or_else(|| {
            SsmError::with_message(
                SsmErrorCode::ParameterVersionNotFound,
                format!(
                    "Systems Manager could not find version {version} of {name}. \
                     Verify the version and try again."
                ),
            )
        })?;

        let mut invalid_labels = Vec::new();
        let mut removed_labels = Vec::new();

        for label in labels {
            if target.labels.remove(label) {
                removed_labels.push(label.clone());
            } else {
                invalid_labels.push(label.clone());
            }
        }

        Ok((invalid_labels, removed_labels))
    }
}

impl Default for ParameterStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve a specific version from a record given an optional selector.
fn resolve_version<'a>(
    record: &'a ParameterRecord,
    selector: Option<&ParameterSelector>,
) -> Result<&'a ParameterVersion, SsmError> {
    match selector {
        None => {
            // Latest version.
            record
                .versions
                .get(&record.current_version)
                .ok_or_else(|| SsmError::parameter_not_found(&record.name))
        }
        Some(ParameterSelector::Version(v)) => record.versions.get(v).ok_or_else(|| {
            SsmError::with_message(
                ruststack_ssm_model::error::SsmErrorCode::ParameterVersionNotFound,
                format!("Version {} not found for parameter {}", v, record.name),
            )
        }),
        Some(ParameterSelector::Label(label)) => {
            // Search all versions for the matching label.
            record
                .versions
                .values()
                .find(|v| v.labels.contains(label.as_str()))
                .ok_or_else(|| {
                    SsmError::with_message(
                        ruststack_ssm_model::error::SsmErrorCode::ParameterVersionNotFound,
                        format!("Label '{}' not found for parameter {}", label, record.name),
                    )
                })
        }
    }
}

/// Build a `Parameter` response object from a record and version snapshot.
fn build_parameter(
    record: &ParameterRecord,
    version: &ParameterVersion,
    region: &str,
    account_id: &str,
) -> Parameter {
    let arn = build_arn(&record.name, region, account_id);

    Parameter {
        name: Some(record.name.clone()),
        parameter_type: Some(record.parameter_type.as_str().to_owned()),
        value: Some(version.value.clone()),
        version: Some(version.version.cast_signed()),
        last_modified_date: Some(version.last_modified_date),
        arn: Some(arn),
        data_type: Some(version.data_type.clone()),
    }
}

/// Build the ARN for a parameter.
///
/// ```text
/// arn:aws:ssm:{region}:{account_id}:parameter{name}     // if name starts with /
/// arn:aws:ssm:{region}:{account_id}:parameter/{name}     // if name doesn't start with /
/// ```
fn build_arn(name: &str, region: &str, account_id: &str) -> String {
    if name.starts_with('/') {
        format!("arn:aws:ssm:{region}:{account_id}:parameter{name}")
    } else {
        format!("arn:aws:ssm:{region}:{account_id}:parameter/{name}")
    }
}

/// Determine the effective tier based on intelligent tiering.
fn effective_tier(requested: &ParameterTier, value: &str) -> ParameterTier {
    match requested {
        ParameterTier::IntelligentTiering => {
            if value.len() > 4096 {
                ParameterTier::Advanced
            } else {
                ParameterTier::Standard
            }
        }
        other => other.clone(),
    }
}

/// Maximum number of tags per resource.
const MAX_TAGS: usize = 50;

/// Build a `ParameterMetadata` from a record (no value included).
fn build_parameter_metadata(record: &ParameterRecord) -> ParameterMetadata {
    let version = record.versions.get(&record.current_version);

    ParameterMetadata {
        name: Some(record.name.clone()),
        parameter_type: Some(record.parameter_type.as_str().to_owned()),
        key_id: record.key_id.clone(),
        last_modified_date: version.map(|v| v.last_modified_date),
        last_modified_user: version.map(|v| v.last_modified_user.clone()),
        description: version.and_then(|v| v.description.clone()),
        allowed_pattern: version.and_then(|v| v.allowed_pattern.clone()),
        version: version.map(|v| v.version.cast_signed()),
        tier: version.map(|v| v.tier.as_str().to_owned()),
        policies: version
            .map(|v| build_inline_policies(&v.policies))
            .unwrap_or_default(),
        data_type: version.map(|v| v.data_type.clone()),
    }
}

/// Build a `ParameterHistory` entry from a record and version snapshot.
fn build_parameter_history(
    record: &ParameterRecord,
    version: &ParameterVersion,
) -> ParameterHistory {
    ParameterHistory {
        name: Some(record.name.clone()),
        parameter_type: Some(record.parameter_type.as_str().to_owned()),
        key_id: record.key_id.clone(),
        last_modified_date: Some(version.last_modified_date),
        last_modified_user: Some(version.last_modified_user.clone()),
        description: version.description.clone(),
        value: Some(version.value.clone()),
        allowed_pattern: version.allowed_pattern.clone(),
        version: Some(version.version.cast_signed()),
        labels: version.labels.iter().cloned().collect(),
        tier: Some(version.tier.as_str().to_owned()),
        policies: build_inline_policies(&version.policies),
        data_type: Some(version.data_type.clone()),
    }
}

/// Convert policy JSON strings into `ParameterInlinePolicy` structs.
fn build_inline_policies(policies: &[String]) -> Vec<ParameterInlinePolicy> {
    policies
        .iter()
        .map(|p| ParameterInlinePolicy {
            policy_text: Some(p.clone()),
            policy_type: None,
            policy_status: None,
        })
        .collect()
}

/// Encode a pagination offset as a base64 next token.
fn encode_offset(offset: usize) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(offset.to_string())
}

/// Decode a base64 next token back to an offset.
///
/// Returns `Ok(None)` if no token is provided, `Ok(Some(offset))` on success,
/// or `Err` if the token is present but invalid.
fn decode_offset(token: Option<&str>) -> Result<Option<usize>, SsmError> {
    use base64::Engine;
    let Some(token) = token else {
        return Ok(None);
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .map_err(|_| {
            SsmError::with_message(
                SsmErrorCode::InvalidNextToken,
                "The specified token is invalid.",
            )
        })?;
    let s = String::from_utf8(decoded).map_err(|_| {
        SsmError::with_message(
            SsmErrorCode::InvalidNextToken,
            "The specified token is invalid.",
        )
    })?;
    let offset = s.parse::<usize>().map_err(|_| {
        SsmError::with_message(
            SsmErrorCode::InvalidNextToken,
            "The specified token is invalid.",
        )
    })?;
    Ok(Some(offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> ParameterStore {
        ParameterStore::new()
    }

    #[test]
    fn test_should_put_and_get_parameter() {
        let store = test_store();
        let (version, tier) = store
            .put_parameter(
                "/test/param",
                "value1".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put");
        assert_eq!(version, 1);
        assert_eq!(tier, ParameterTier::Standard);

        let param = store
            .get_parameter("/test/param", None, "us-east-1", "123456789012")
            .expect("should get");
        assert_eq!(param.name.as_deref(), Some("/test/param"));
        assert_eq!(param.value.as_deref(), Some("value1"));
        assert_eq!(param.version, Some(1));
    }

    #[test]
    fn test_should_increment_version_on_overwrite() {
        let store = test_store();
        store
            .put_parameter(
                "/test/param",
                "v1".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put v1");

        let (version, _) = store
            .put_parameter(
                "/test/param",
                "v2".to_owned(),
                ParameterType::String,
                None,
                None,
                true,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put v2");
        assert_eq!(version, 2);
    }

    #[test]
    fn test_should_reject_duplicate_without_overwrite() {
        let store = test_store();
        store
            .put_parameter(
                "/test/param",
                "v1".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put");

        let err = store
            .put_parameter(
                "/test/param",
                "v2".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterAlreadyExists,
        );
    }

    #[test]
    fn test_should_delete_parameter() {
        let store = test_store();
        store
            .put_parameter(
                "/test/param",
                "val".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put");

        store
            .delete_parameter("/test/param")
            .expect("should delete");

        let err = store
            .get_parameter("/test/param", None, "us-east-1", "123456789012")
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterNotFound,
        );
    }

    #[test]
    fn test_should_get_parameter_by_version() {
        let store = test_store();
        store
            .put_parameter(
                "/test/param",
                "v1".to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("v1");
        store
            .put_parameter(
                "/test/param",
                "v2".to_owned(),
                ParameterType::String,
                None,
                None,
                true,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("v2");

        let param = store
            .get_parameter(
                "/test/param",
                Some(&ParameterSelector::Version(1)),
                "us-east-1",
                "123456789012",
            )
            .expect("should get v1");
        assert_eq!(param.value.as_deref(), Some("v1"));
        assert_eq!(param.version, Some(1));
    }

    #[test]
    fn test_should_get_parameters_by_path() {
        let store = test_store();
        let names = [
            "/app/db/host",
            "/app/db/port",
            "/app/cache/host",
            "/other/param",
        ];
        for name in &names {
            store
                .put_parameter(
                    name,
                    "val".to_owned(),
                    ParameterType::String,
                    None,
                    None,
                    false,
                    None,
                    &[],
                    &ParameterTier::Standard,
                    "text".to_owned(),
                    vec![],
                    "123456789012",
                )
                .expect("should put");
        }

        // Non-recursive: only direct children of /app/db.
        let (params, token) = store
            .get_parameters_by_path("/app/db", false, &[], 10, None, "us-east-1", "123456789012")
            .expect("should get by path");
        assert_eq!(params.len(), 2);
        assert!(token.is_none());

        // Recursive: all descendants of /app.
        let (recursive_params, _) = store
            .get_parameters_by_path("/app", true, &[], 10, None, "us-east-1", "123456789012")
            .expect("should get by path");
        assert_eq!(recursive_params.len(), 3);
    }

    #[test]
    fn test_should_paginate_by_path() {
        let store = test_store();
        for i in 0..5 {
            store
                .put_parameter(
                    &format!("/page/param{i}"),
                    "val".to_owned(),
                    ParameterType::String,
                    None,
                    None,
                    false,
                    None,
                    &[],
                    &ParameterTier::Standard,
                    "text".to_owned(),
                    vec![],
                    "123456789012",
                )
                .expect("should put");
        }

        let (page1, token1) = store
            .get_parameters_by_path("/page", false, &[], 2, None, "us-east-1", "123456789012")
            .expect("should get by path");
        assert_eq!(page1.len(), 2);
        assert!(token1.is_some());

        let (page2, token2) = store
            .get_parameters_by_path(
                "/page",
                false,
                &[],
                2,
                token1.as_deref(),
                "us-east-1",
                "123456789012",
            )
            .expect("should get by path");
        assert_eq!(page2.len(), 2);
        assert!(token2.is_some());

        let (page3, token3) = store
            .get_parameters_by_path(
                "/page",
                false,
                &[],
                2,
                token2.as_deref(),
                "us-east-1",
                "123456789012",
            )
            .expect("should get by path");
        assert_eq!(page3.len(), 1);
        assert!(token3.is_none());
    }

    #[test]
    fn test_should_build_arn_with_leading_slash() {
        let arn = build_arn("/my/param", "us-east-1", "123456789012");
        assert_eq!(arn, "arn:aws:ssm:us-east-1:123456789012:parameter/my/param");
    }

    #[test]
    fn test_should_build_arn_without_leading_slash() {
        let arn = build_arn("my-param", "us-east-1", "123456789012");
        assert_eq!(arn, "arn:aws:ssm:us-east-1:123456789012:parameter/my-param");
    }

    #[test]
    fn test_should_batch_delete() {
        let store = test_store();
        for name in &["/del/a", "/del/b"] {
            store
                .put_parameter(
                    name,
                    "val".to_owned(),
                    ParameterType::String,
                    None,
                    None,
                    false,
                    None,
                    &[],
                    &ParameterTier::Standard,
                    "text".to_owned(),
                    vec![],
                    "123456789012",
                )
                .expect("should put");
        }

        let names = vec![
            "/del/a".to_owned(),
            "/del/b".to_owned(),
            "/del/nonexistent".to_owned(),
        ];
        let (deleted, invalid) = store.delete_parameters(&names);
        assert_eq!(deleted.len(), 2);
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0], "/del/nonexistent");
    }

    // ----- Phase 1 tests -----

    fn put_simple(store: &ParameterStore, name: &str, value: &str) {
        store
            .put_parameter(
                name,
                value.to_owned(),
                ParameterType::String,
                None,
                None,
                false,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put");
    }

    #[test]
    fn test_should_describe_parameters_all() {
        let store = test_store();
        put_simple(&store, "/app/db/host", "localhost");
        put_simple(&store, "/app/db/port", "5432");
        put_simple(&store, "/app/cache/host", "redis");

        let (params, token) = store
            .describe_parameters(&[], 50, None)
            .expect("should describe");
        assert_eq!(params.len(), 3);
        assert!(token.is_none());

        // Verify metadata has no value field by type (ParameterMetadata doesn't have value).
        assert!(params[0].name.is_some());
        assert!(params[0].parameter_type.is_some());
    }

    #[test]
    fn test_should_describe_parameters_with_pagination() {
        let store = test_store();
        for i in 0..5 {
            put_simple(&store, &format!("/desc/p{i}"), "val");
        }

        let (page1, token1) = store
            .describe_parameters(&[], 2, None)
            .expect("should describe");
        assert_eq!(page1.len(), 2);
        assert!(token1.is_some());

        let (page2, token2) = store
            .describe_parameters(&[], 2, token1.as_deref())
            .expect("should describe");
        assert_eq!(page2.len(), 2);
        assert!(token2.is_some());

        let (page3, token3) = store
            .describe_parameters(&[], 2, token2.as_deref())
            .expect("should describe");
        assert_eq!(page3.len(), 1);
        assert!(token3.is_none());
    }

    #[test]
    fn test_should_get_parameter_history() {
        let store = test_store();
        put_simple(&store, "/hist/param", "v1");
        store
            .put_parameter(
                "/hist/param",
                "v2".to_owned(),
                ParameterType::String,
                Some("updated".to_owned()),
                None,
                true,
                None,
                &[],
                &ParameterTier::Standard,
                "text".to_owned(),
                vec![],
                "123456789012",
            )
            .expect("should put v2");

        let (history, token) = store
            .get_parameter_history("/hist/param", 50, None)
            .expect("should get history");
        assert_eq!(history.len(), 2);
        assert!(token.is_none());

        // First entry should be version 1.
        assert_eq!(history[0].version, Some(1));
        assert_eq!(history[0].value.as_deref(), Some("v1"));

        // Second entry should be version 2.
        assert_eq!(history[1].version, Some(2));
        assert_eq!(history[1].value.as_deref(), Some("v2"));
        assert_eq!(history[1].description.as_deref(), Some("updated"));
    }

    #[test]
    fn test_should_get_parameter_history_not_found() {
        let store = test_store();
        let err = store
            .get_parameter_history("/nonexistent", 50, None)
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterNotFound,
        );
    }

    #[test]
    fn test_should_add_and_list_tags() {
        let store = test_store();
        put_simple(&store, "/tag/param", "val");

        let tags = vec![
            Tag {
                key: "env".to_owned(),
                value: "prod".to_owned(),
            },
            Tag {
                key: "team".to_owned(),
                value: "backend".to_owned(),
            },
        ];
        store
            .add_tags("/tag/param", &tags)
            .expect("should add tags");

        let result = store.list_tags("/tag/param").expect("should list tags");
        assert_eq!(result.len(), 2);
        // Tags are sorted by key.
        assert_eq!(result[0].key, "env");
        assert_eq!(result[0].value, "prod");
        assert_eq!(result[1].key, "team");
        assert_eq!(result[1].value, "backend");
    }

    #[test]
    fn test_should_overwrite_existing_tags() {
        let store = test_store();
        put_simple(&store, "/tag/param", "val");

        let tags1 = vec![Tag {
            key: "env".to_owned(),
            value: "dev".to_owned(),
        }];
        store.add_tags("/tag/param", &tags1).expect("add");

        let tags2 = vec![Tag {
            key: "env".to_owned(),
            value: "prod".to_owned(),
        }];
        store.add_tags("/tag/param", &tags2).expect("overwrite");

        let result = store.list_tags("/tag/param").expect("list");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, "prod");
    }

    #[test]
    fn test_should_remove_tags() {
        let store = test_store();
        put_simple(&store, "/tag/param", "val");

        let tags = vec![
            Tag {
                key: "a".to_owned(),
                value: "1".to_owned(),
            },
            Tag {
                key: "b".to_owned(),
                value: "2".to_owned(),
            },
        ];
        store.add_tags("/tag/param", &tags).expect("add");
        store
            .remove_tags("/tag/param", &["a".to_owned()])
            .expect("remove");

        let result = store.list_tags("/tag/param").expect("list");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "b");
    }

    #[test]
    fn test_should_reject_tags_on_nonexistent_parameter() {
        let store = test_store();
        let err = store
            .add_tags(
                "/nonexistent",
                &[Tag {
                    key: "k".to_owned(),
                    value: "v".to_owned(),
                }],
            )
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::InvalidResourceId,
        );
    }

    #[test]
    fn test_should_enforce_tag_limit() {
        let store = test_store();
        put_simple(&store, "/tag/param", "val");

        // Add 50 tags (the max).
        let tags: Vec<Tag> = (0..50)
            .map(|i| Tag {
                key: format!("key{i}"),
                value: format!("val{i}"),
            })
            .collect();
        store.add_tags("/tag/param", &tags).expect("add 50");

        // Adding one more should fail.
        let err = store
            .add_tags(
                "/tag/param",
                &[Tag {
                    key: "extra".to_owned(),
                    value: "val".to_owned(),
                }],
            )
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::TooManyTagsError,
        );
    }

    // ----- Phase 2 tests -----

    fn put_versioned(store: &ParameterStore, name: &str, count: usize) {
        for i in 0..count {
            store
                .put_parameter(
                    name,
                    format!("v{}", i + 1),
                    ParameterType::String,
                    None,
                    None,
                    i > 0, // overwrite after first
                    None,
                    &[],
                    &ParameterTier::Standard,
                    "text".to_owned(),
                    vec![],
                    "123456789012",
                )
                .expect("should put");
        }
    }

    #[test]
    fn test_should_label_latest_version() {
        let store = test_store();
        put_versioned(&store, "/label/param", 2);

        let labels = vec!["release".to_owned()];
        let (invalid, version) = store
            .label_parameter_version("/label/param", None, &labels)
            .expect("should label");
        assert!(invalid.is_empty());
        assert_eq!(version, 2);

        // Verify label is on version 2.
        let record = store.parameters.get("/label/param").expect("exists");
        let v2 = record.versions.get(&2).expect("v2");
        assert!(v2.labels.contains("release"));
    }

    #[test]
    fn test_should_label_specific_version() {
        let store = test_store();
        put_versioned(&store, "/label/param", 3);

        let labels = vec!["staging".to_owned()];
        let (invalid, version) = store
            .label_parameter_version("/label/param", Some(1), &labels)
            .expect("should label");
        assert!(invalid.is_empty());
        assert_eq!(version, 1);

        let record = store.parameters.get("/label/param").expect("exists");
        let v1 = record.versions.get(&1).expect("v1");
        assert!(v1.labels.contains("staging"));
    }

    #[test]
    fn test_should_move_label_between_versions() {
        let store = test_store();
        put_versioned(&store, "/label/param", 2);

        // Label version 1.
        store
            .label_parameter_version("/label/param", Some(1), &["prod".to_owned()])
            .expect("label v1");

        // Move label to version 2.
        store
            .label_parameter_version("/label/param", Some(2), &["prod".to_owned()])
            .expect("label v2");

        let record = store.parameters.get("/label/param").expect("exists");
        let v1 = record.versions.get(&1).expect("v1");
        let v2 = record.versions.get(&2).expect("v2");
        assert!(!v1.labels.contains("prod"));
        assert!(v2.labels.contains("prod"));
    }

    #[test]
    fn test_should_reject_invalid_labels() {
        let store = test_store();
        put_simple(&store, "/label/param", "val");

        let labels = vec![
            "valid-label".to_owned(),
            "123invalid".to_owned(),   // starts with digit
            "aws-reserved".to_owned(), // starts with aws
            "ssm-reserved".to_owned(), // starts with ssm
            String::new(),             // empty
        ];
        let (invalid, version) = store
            .label_parameter_version("/label/param", None, &labels)
            .expect("should label");
        assert_eq!(version, 1);
        assert_eq!(invalid.len(), 4);
        assert!(invalid.contains(&"123invalid".to_owned()));
        assert!(invalid.contains(&"aws-reserved".to_owned()));
        assert!(invalid.contains(&"ssm-reserved".to_owned()));
        assert!(invalid.contains(&String::new()));

        // Valid label should be attached.
        let record = store.parameters.get("/label/param").expect("exists");
        let v1 = record.versions.get(&1).expect("v1");
        assert!(v1.labels.contains("valid-label"));
    }

    #[test]
    fn test_should_enforce_label_limit() {
        let store = test_store();
        put_simple(&store, "/label/param", "val");

        // Attach 10 labels (the max).
        let labels: Vec<String> = (0..10).map(|i| format!("label-{i}")).collect();
        store
            .label_parameter_version("/label/param", None, &labels)
            .expect("should label 10");

        // Adding one more should fail.
        let err = store
            .label_parameter_version("/label/param", None, &["overflow".to_owned()])
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterVersionLabelLimitExceeded,
        );
    }

    #[test]
    fn test_should_label_not_found_parameter() {
        let store = test_store();
        let err = store
            .label_parameter_version("/nonexistent", None, &["label".to_owned()])
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterNotFound,
        );
    }

    #[test]
    fn test_should_label_not_found_version() {
        let store = test_store();
        put_simple(&store, "/label/param", "val");

        let err = store
            .label_parameter_version("/label/param", Some(99), &["label".to_owned()])
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterVersionNotFound,
        );
    }

    #[test]
    fn test_should_unlabel_existing_labels() {
        let store = test_store();
        put_simple(&store, "/label/param", "val");

        store
            .label_parameter_version(
                "/label/param",
                Some(1),
                &["alpha".to_owned(), "beta".to_owned()],
            )
            .expect("label");

        let (invalid, removed) = store
            .unlabel_parameter_version("/label/param", 1, &["alpha".to_owned(), "gamma".to_owned()])
            .expect("unlabel");

        assert_eq!(removed, vec!["alpha".to_owned()]);
        assert_eq!(invalid, vec!["gamma".to_owned()]);

        // Verify beta is still there.
        let record = store.parameters.get("/label/param").expect("exists");
        let v1 = record.versions.get(&1).expect("v1");
        assert!(v1.labels.contains("beta"));
        assert!(!v1.labels.contains("alpha"));
    }

    #[test]
    fn test_should_unlabel_not_found_parameter() {
        let store = test_store();
        let err = store
            .unlabel_parameter_version("/nonexistent", 1, &["label".to_owned()])
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterNotFound,
        );
    }

    #[test]
    fn test_should_unlabel_not_found_version() {
        let store = test_store();
        put_simple(&store, "/label/param", "val");

        let err = store
            .unlabel_parameter_version("/label/param", 99, &["label".to_owned()])
            .unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ParameterVersionNotFound,
        );
    }
}
