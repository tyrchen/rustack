//! In-memory storage engine for SSM Parameter Store.
//!
//! Parameters are stored in a `DashMap<String, ParameterRecord>` keyed by
//! the fully-qualified parameter name. Each record tracks the current version,
//! all version snapshots (up to 100), tags, and metadata.

use std::collections::{BTreeMap, HashMap, HashSet};

use dashmap::DashMap;

use ruststack_ssm_model::error::SsmError;
use ruststack_ssm_model::types::{Parameter, ParameterTier, ParameterType, Tag};

use crate::selector::ParameterSelector;
use crate::validation::MAX_VERSIONS;

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

            // Check version limit.
            if record.versions.len() >= MAX_VERSIONS {
                return Err(SsmError::with_message(
                    ruststack_ssm_model::error::SsmErrorCode::ParameterMaxVersionLimitExceeded,
                    format!(
                        "Parameter {name} has reached the maximum number of \
                         {MAX_VERSIONS} versions."
                    ),
                ));
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
    #[must_use]
    pub fn get_parameters(
        &self,
        names: &[String],
        region: &str,
        account_id: &str,
    ) -> (Vec<Parameter>, Vec<String>) {
        let mut found = Vec::new();
        let mut invalid = Vec::new();

        for name in names {
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
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn get_parameters_by_path(
        &self,
        path: &str,
        recursive: bool,
        max_results: usize,
        next_token: Option<&str>,
        region: &str,
        account_id: &str,
    ) -> (Vec<Parameter>, Option<String>) {
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

            if recursive {
                // Include all descendants.
                if !remainder.is_empty() {
                    matching_names.push(param_name.clone());
                }
            } else {
                // Only direct children (no further `/` in remainder).
                if !remainder.is_empty() && !remainder.contains('/') {
                    matching_names.push(param_name.clone());
                }
            }
        }

        // Sort for deterministic pagination.
        matching_names.sort();

        // Apply next_token (skip entries up to and including the token value).
        let start_idx = if let Some(token) = next_token {
            matching_names
                .iter()
                .position(|n| n.as_str() > token)
                .unwrap_or(matching_names.len())
        } else {
            0
        };

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
            page_names.last().cloned()
        } else {
            None
        };

        (parameters, new_next_token)
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
                format!("Version {} not found for parameter {}", v, record.name,),
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
                        format!("Label '{}' not found for parameter {}", label, record.name,),
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
        let (params, token) =
            store.get_parameters_by_path("/app/db", false, 10, None, "us-east-1", "123456789012");
        assert_eq!(params.len(), 2);
        assert!(token.is_none());

        // Recursive: all descendants of /app.
        let (params, _) =
            store.get_parameters_by_path("/app", true, 10, None, "us-east-1", "123456789012");
        assert_eq!(params.len(), 3);
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

        let (page1, token1) =
            store.get_parameters_by_path("/page", false, 2, None, "us-east-1", "123456789012");
        assert_eq!(page1.len(), 2);
        assert!(token1.is_some());

        let (page2, token2) = store.get_parameters_by_path(
            "/page",
            false,
            2,
            token1.as_deref(),
            "us-east-1",
            "123456789012",
        );
        assert_eq!(page2.len(), 2);
        assert!(token2.is_some());

        let (page3, token3) = store.get_parameters_by_path(
            "/page",
            false,
            2,
            token2.as_deref(),
            "us-east-1",
            "123456789012",
        );
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
}
