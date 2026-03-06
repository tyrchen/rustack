//! SSM provider implementing all Phase 0 operations.

use ruststack_ssm_model::error::SsmError;
use ruststack_ssm_model::input::{
    DeleteParameterInput, DeleteParametersInput, GetParameterInput, GetParametersByPathInput,
    GetParametersInput, PutParameterInput,
};
use ruststack_ssm_model::output::{
    DeleteParameterOutput, DeleteParametersOutput, GetParameterOutput, GetParametersByPathOutput,
    GetParametersOutput, PutParameterOutput,
};
use ruststack_ssm_model::types::ParameterTier;

use crate::config::SsmConfig;
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
        #[allow(clippy::cast_sign_loss)]
        let max_results = input
            .max_results
            .unwrap_or(DEFAULT_PATH_MAX_RESULTS)
            .clamp(0, MAX_PATH_MAX_RESULTS) as usize;

        let recursive = input.recursive.unwrap_or(false);

        let (parameters, next_token) = self.store.get_parameters_by_path(
            &input.path,
            recursive,
            max_results,
            input.next_token.as_deref(),
            &self.config.default_region,
            &self.config.default_account_id,
        );

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
}
