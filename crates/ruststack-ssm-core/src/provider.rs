//! SSM provider implementing Phase 0, Phase 1, and Phase 2 operations.

use ruststack_ssm_model::error::{SsmError, SsmErrorCode};
use ruststack_ssm_model::input::{
    AddTagsToResourceInput, DeleteParameterInput, DeleteParametersInput, DescribeParametersInput,
    GetParameterHistoryInput, GetParameterInput, GetParametersByPathInput, GetParametersInput,
    LabelParameterVersionInput, ListTagsForResourceInput, PutParameterInput,
    RemoveTagsFromResourceInput, UnlabelParameterVersionInput,
};
use ruststack_ssm_model::output::{
    AddTagsToResourceOutput, DeleteParameterOutput, DeleteParametersOutput,
    DescribeParametersOutput, GetParameterHistoryOutput, GetParameterOutput,
    GetParametersByPathOutput, GetParametersOutput, LabelParameterVersionOutput,
    ListTagsForResourceOutput, PutParameterOutput, RemoveTagsFromResourceOutput,
    UnlabelParameterVersionOutput,
};
use ruststack_ssm_model::types::ParameterTier;

use crate::config::SsmConfig;
use crate::filter::validate_filters;
use crate::selector::parse_name_with_selector;
use crate::storage::ParameterStore;
use crate::validation::{
    MAX_BATCH_SIZE, parse_parameter_type, parse_tier, validate_allowed_pattern,
    validate_description, validate_name, validate_tags, validate_value,
};

/// Default max results for `GetParametersByPath`.
const DEFAULT_PATH_MAX_RESULTS: i32 = 10;

/// Maximum max results for `GetParametersByPath`.
const MAX_PATH_MAX_RESULTS: i32 = 10;

/// Default max results for `DescribeParameters` and `GetParameterHistory`.
const DEFAULT_DESCRIBE_MAX_RESULTS: i32 = 50;

/// Maximum max results for `DescribeParameters` and `GetParameterHistory`.
const MAX_DESCRIBE_MAX_RESULTS: i32 = 50;

/// The only valid resource type for tag operations.
const RESOURCE_TYPE_PARAMETER: &str = "Parameter";

/// The SSM Parameter Store provider.
#[derive(Debug)]
pub struct RustStackSsm {
    config: SsmConfig,
    store: ParameterStore,
}

impl RustStackSsm {
    /// Create a new SSM provider with the given configuration.
    #[must_use]
    pub fn new(config: SsmConfig) -> Self {
        Self {
            config,
            store: ParameterStore::new(),
        }
    }

    /// Handle `PutParameter`.
    pub fn handle_put_parameter(
        &self,
        input: PutParameterInput,
    ) -> Result<PutParameterOutput, SsmError> {
        // Validate name.
        validate_name(&input.name)?;

        // Parse and validate type.
        let param_type = if let Some(ref type_str) = input.parameter_type {
            parse_parameter_type(type_str)?
        } else {
            ruststack_ssm_model::types::ParameterType::String
        };

        // Parse tier.
        let tier = if let Some(ref tier_str) = input.tier {
            parse_tier(tier_str)?
        } else {
            ParameterTier::Standard
        };

        // Validate value.
        validate_value(&input.value, &tier)?;

        // Validate description.
        if let Some(ref desc) = input.description {
            validate_description(desc)?;
        }

        // Validate tags.
        validate_tags(&input.tags)?;

        // Validate allowed pattern.
        if let Some(ref pattern) = input.allowed_pattern {
            validate_allowed_pattern(pattern, &input.value)?;
        }

        let overwrite = input.overwrite.unwrap_or(false);
        let data_type = input.data_type.unwrap_or_else(|| "text".to_owned());

        let policies: Vec<String> = if let Some(ref p) = input.policies {
            if p.is_empty() {
                vec![]
            } else {
                vec![p.clone()]
            }
        } else {
            vec![]
        };

        let (version, effective_tier) = self.store.put_parameter(
            &input.name,
            input.value,
            param_type,
            input.description,
            input.key_id,
            overwrite,
            input.allowed_pattern,
            &input.tags,
            &tier,
            data_type,
            policies,
            &self.config.default_account_id,
        )?;

        Ok(PutParameterOutput {
            version: version.cast_signed(),
            tier: effective_tier.as_str().to_owned(),
        })
    }

    /// Handle `GetParameter`.
    pub fn handle_get_parameter(
        &self,
        input: &GetParameterInput,
    ) -> Result<GetParameterOutput, SsmError> {
        let parsed = parse_name_with_selector(&input.name)?;

        let param = self.store.get_parameter(
            &parsed.name,
            parsed.selector.as_ref(),
            &self.config.default_region,
            &self.config.default_account_id,
        )?;

        Ok(GetParameterOutput {
            parameter: Some(param),
        })
    }

    /// Handle `GetParameters`.
    pub fn handle_get_parameters(
        &self,
        input: &GetParametersInput,
    ) -> Result<GetParametersOutput, SsmError> {
        if input.names.len() > MAX_BATCH_SIZE {
            return Err(SsmError::validation(format!(
                "GetParameters request exceeds the maximum batch size of {MAX_BATCH_SIZE}."
            )));
        }

        let (parameters, invalid_parameters) = self.store.get_parameters(
            &input.names,
            &self.config.default_region,
            &self.config.default_account_id,
        );

        Ok(GetParametersOutput {
            parameters,
            invalid_parameters,
        })
    }

    /// Handle `GetParametersByPath`.
    pub fn handle_get_parameters_by_path(
        &self,
        input: &GetParametersByPathInput,
    ) -> Result<GetParametersByPathOutput, SsmError> {
        // Validate filters.
        validate_filters(&input.parameter_filters)?;

        #[allow(clippy::cast_sign_loss)]
        let max_results = input
            .max_results
            .unwrap_or(DEFAULT_PATH_MAX_RESULTS)
            .clamp(0, MAX_PATH_MAX_RESULTS) as usize;

        let recursive = input.recursive.unwrap_or(false);

        let (parameters, next_token) = self.store.get_parameters_by_path(
            &input.path,
            recursive,
            &input.parameter_filters,
            max_results,
            input.next_token.as_deref(),
            &self.config.default_region,
            &self.config.default_account_id,
        )?;

        Ok(GetParametersByPathOutput {
            parameters,
            next_token,
        })
    }

    /// Handle `DeleteParameter`.
    pub fn handle_delete_parameter(
        &self,
        input: &DeleteParameterInput,
    ) -> Result<DeleteParameterOutput, SsmError> {
        self.store.delete_parameter(&input.name)?;
        Ok(DeleteParameterOutput {})
    }

    /// Handle `DeleteParameters`.
    pub fn handle_delete_parameters(
        &self,
        input: &DeleteParametersInput,
    ) -> Result<DeleteParametersOutput, SsmError> {
        if input.names.len() > MAX_BATCH_SIZE {
            return Err(SsmError::validation(format!(
                "DeleteParameters request exceeds the maximum batch size of {MAX_BATCH_SIZE}."
            )));
        }

        let (deleted_parameters, invalid_parameters) = self.store.delete_parameters(&input.names);

        Ok(DeleteParametersOutput {
            deleted_parameters,
            invalid_parameters,
        })
    }

    // ----- Phase 1 operations -----

    /// Handle `DescribeParameters`.
    pub fn handle_describe_parameters(
        &self,
        input: &DescribeParametersInput,
    ) -> Result<DescribeParametersOutput, SsmError> {
        // Validate filters.
        validate_filters(&input.parameter_filters)?;

        #[allow(clippy::cast_sign_loss)]
        let max_results = input
            .max_results
            .unwrap_or(DEFAULT_DESCRIBE_MAX_RESULTS)
            .clamp(1, MAX_DESCRIBE_MAX_RESULTS) as usize;

        let (parameters, next_token) = self.store.describe_parameters(
            &input.parameter_filters,
            max_results,
            input.next_token.as_deref(),
        )?;

        Ok(DescribeParametersOutput {
            parameters,
            next_token,
        })
    }

    /// Handle `GetParameterHistory`.
    pub fn handle_get_parameter_history(
        &self,
        input: &GetParameterHistoryInput,
    ) -> Result<GetParameterHistoryOutput, SsmError> {
        #[allow(clippy::cast_sign_loss)]
        let max_results = input
            .max_results
            .unwrap_or(DEFAULT_DESCRIBE_MAX_RESULTS)
            .clamp(1, MAX_DESCRIBE_MAX_RESULTS) as usize;

        let (parameters, next_token) = self.store.get_parameter_history(
            &input.name,
            max_results,
            input.next_token.as_deref(),
        )?;

        Ok(GetParameterHistoryOutput {
            parameters,
            next_token,
        })
    }

    /// Handle `AddTagsToResource`.
    pub fn handle_add_tags_to_resource(
        &self,
        input: &AddTagsToResourceInput,
    ) -> Result<AddTagsToResourceOutput, SsmError> {
        validate_resource_type(&input.resource_type)?;
        validate_tags(&input.tags)?;
        self.store.add_tags(&input.resource_id, &input.tags)?;
        Ok(AddTagsToResourceOutput {})
    }

    /// Handle `RemoveTagsFromResource`.
    pub fn handle_remove_tags_from_resource(
        &self,
        input: &RemoveTagsFromResourceInput,
    ) -> Result<RemoveTagsFromResourceOutput, SsmError> {
        validate_resource_type(&input.resource_type)?;
        self.store
            .remove_tags(&input.resource_id, &input.tag_keys)?;
        Ok(RemoveTagsFromResourceOutput {})
    }

    /// Handle `ListTagsForResource`.
    pub fn handle_list_tags_for_resource(
        &self,
        input: &ListTagsForResourceInput,
    ) -> Result<ListTagsForResourceOutput, SsmError> {
        validate_resource_type(&input.resource_type)?;
        let tag_list = self.store.list_tags(&input.resource_id)?;
        Ok(ListTagsForResourceOutput { tag_list })
    }

    // ----- Phase 2 operations -----

    /// Handle `LabelParameterVersion`.
    pub fn handle_label_parameter_version(
        &self,
        input: &LabelParameterVersionInput,
    ) -> Result<LabelParameterVersionOutput, SsmError> {
        #[allow(clippy::cast_sign_loss)]
        let version = input.parameter_version.map(|v| v as u64);

        let (invalid_labels, parameter_version) =
            self.store
                .label_parameter_version(&input.name, version, &input.labels)?;

        Ok(LabelParameterVersionOutput {
            invalid_labels,
            parameter_version: parameter_version.cast_signed(),
        })
    }

    /// Handle `UnlabelParameterVersion`.
    pub fn handle_unlabel_parameter_version(
        &self,
        input: &UnlabelParameterVersionInput,
    ) -> Result<UnlabelParameterVersionOutput, SsmError> {
        #[allow(clippy::cast_sign_loss)]
        let version = input.parameter_version as u64;

        let (invalid_labels, removed_labels) =
            self.store
                .unlabel_parameter_version(&input.name, version, &input.labels)?;

        Ok(UnlabelParameterVersionOutput {
            invalid_labels,
            removed_labels,
        })
    }
}

/// Validate that the resource type is `"Parameter"`.
fn validate_resource_type(resource_type: &str) -> Result<(), SsmError> {
    if resource_type != RESOURCE_TYPE_PARAMETER {
        return Err(SsmError::with_message(
            SsmErrorCode::InvalidResourceType,
            format!(
                "The resource type '{resource_type}' is not valid. \
                 Valid resource types: Parameter."
            ),
        ));
    }
    Ok(())
}
