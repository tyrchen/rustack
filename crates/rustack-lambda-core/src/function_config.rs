//! Validate captured function settings before storage or execution admission.

use std::{collections::HashMap, fmt};

use rustack_lambda_model::{input::CreateFunctionInput, types::ImageConfig};
use typed_builder::TypedBuilder;

use crate::{
    artifact::{Architecture, ImageReference, ResolvedImage},
    error::LambdaServiceError,
    storage::{VersionRecord, VersionRecordSnapshot},
};

fn invalid(message: &'static str) -> LambdaServiceError {
    LambdaServiceError::InvalidParameter {
        message: message.into(),
    }
}

#[derive(TypedBuilder)]
struct Settings<'a> {
    package_type: &'a str,
    runtime: Option<&'a str>,
    handler: Option<&'a str>,
    architectures: Option<&'a [String]>,
    environment: Option<&'a HashMap<String, String>>,
    image_config: Option<&'a ImageConfig>,
    image_uri: Option<&'a str>,
    timeout: u32,
    memory: u32,
    ephemeral: u32,
}

impl fmt::Debug for Settings<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("package_type", &self.package_type)
            .field("environment", &"[REDACTED]")
            .field("image_config", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl Settings<'_> {
    fn validate(&self) -> Result<Architecture, LambdaServiceError> {
        if !(1..=900).contains(&self.timeout)
            || !(128..=10240).contains(&self.memory)
            || !(512..=10240).contains(&self.ephemeral)
        {
            return Err(invalid(
                "Invalid function timeout, memory or ephemeral storage",
            ));
        }
        let architecture = match self.architectures {
            None => Architecture::X86_64,
            Some([architecture]) => architecture
                .parse()
                .map_err(|_| invalid("Invalid Lambda architecture"))?,
            Some(_) => return Err(invalid("Exactly one Lambda architecture is required")),
        };
        match self.package_type {
            "Zip" => {
                if self.runtime.is_none_or(|runtime| {
                    runtime.is_empty() || runtime.len() > 128 || runtime.contains('\0')
                }) || self.handler.is_none_or(|handler| {
                    handler.is_empty() || handler.len() > 256 || handler.contains('\0')
                }) || self.image_uri.is_some()
                    || self.image_config.is_some()
                {
                    return Err(invalid("Invalid ZIP runtime, handler or image fields"));
                }
            }
            "Image" => {
                let reference = self
                    .image_uri
                    .ok_or_else(|| invalid("ImageUri is required"))?;
                reference
                    .parse::<ImageReference>()
                    .map_err(|_| invalid("Invalid ImageUri"))?;
                if self.runtime.is_some() || self.handler.is_some() {
                    return Err(invalid(
                        "Image functions cannot contain ZIP runtime or handler fields",
                    ));
                }
                if let Some(config) = self.image_config {
                    validate_image_config(config)?;
                }
            }
            _ => return Err(invalid("Unsupported deployment package type")),
        }
        if let Some(environment) = self.environment {
            validate_environment(environment)?;
        }
        Ok(architecture)
    }
}

pub(crate) fn validate_provided_settings(
    runtime: Option<&str>,
    handler: Option<&str>,
) -> Result<(), LambdaServiceError> {
    if runtime != Some("provided.al2023")
        || handler.is_none_or(|value| {
            value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-".contains(&byte))
        })
    {
        return Err(invalid(
            "Docker ZIP requires provided.al2023 and a supported handler",
        ));
    }
    Ok(())
}

pub(crate) fn validate_create_settings(
    input: &CreateFunctionInput,
) -> Result<Architecture, LambdaServiceError> {
    Settings::builder()
        .package_type(input.package_type.as_deref().unwrap_or("Zip"))
        .runtime(input.runtime.as_deref())
        .handler(input.handler.as_deref())
        .architectures(input.architectures.as_deref())
        .environment(
            input
                .environment
                .as_ref()
                .and_then(|environment| environment.variables.as_ref()),
        )
        .image_config(input.image_config.as_ref())
        .image_uri(input.code.image_uri.as_deref())
        .timeout(input.timeout.unwrap_or(3))
        .memory(input.memory_size.unwrap_or(128))
        .ephemeral(
            input
                .ephemeral_storage
                .as_ref()
                .map_or(512, |storage| storage.size),
        )
        .build()
        .validate()
}

pub(crate) fn validate_execution_settings(
    version: &VersionRecord,
) -> Result<Architecture, LambdaServiceError> {
    let architecture = Settings::builder()
        .package_type(&version.package_type)
        .runtime(version.runtime.as_deref())
        .handler(version.handler.as_deref())
        .architectures(Some(&version.architectures))
        .environment(Some(&version.environment))
        .image_config(version.image_config.as_ref())
        .image_uri(version.image_uri.as_deref())
        .timeout(version.timeout)
        .memory(version.memory_size)
        .ephemeral(version.ephemeral_storage_size)
        .build()
        .validate()?;
    validate_layout(
        &version.package_type,
        version.zip_bytes.is_some(),
        version.code_path.is_some(),
        version.image.resolved(),
        architecture,
        version.image_uri.as_deref(),
    )?;
    validate_image_fingerprint(
        version.image.resolved(),
        &version.code_sha256,
        version.code_size,
        version
            .image
            .pinned()
            .map(|image| image.unpacked_size().get()),
    )?;
    Ok(architecture)
}

pub(crate) fn validate_snapshot_settings(
    version: &VersionRecordSnapshot,
) -> Result<(), LambdaServiceError> {
    let architecture = Settings::builder()
        .package_type(&version.package_type)
        .runtime(version.runtime.as_deref())
        .handler(version.handler.as_deref())
        .architectures(Some(&version.architectures))
        .environment(Some(&version.environment))
        .image_config(version.image_config.as_ref())
        .image_uri(version.image_uri.as_deref())
        .timeout(version.timeout)
        .memory(version.memory_size)
        .ephemeral(version.ephemeral_storage_size)
        .build()
        .validate()?;
    validate_layout(
        &version.package_type,
        version.zip_bytes_base64.is_some(),
        false,
        version.resolved_image.as_ref(),
        architecture,
        version.image_uri.as_deref(),
    )?;
    validate_image_fingerprint(
        version.resolved_image.as_ref(),
        &version.code_sha256,
        version.code_size,
        None,
    )
}

fn validate_image_fingerprint(
    image: Option<&ResolvedImage>,
    hash: &str,
    size: u64,
    pinned_size: Option<u64>,
) -> Result<(), LambdaServiceError> {
    if let Some(image) = image {
        if hash
            != image
                .code_sha256()
                .map_err(|_| invalid("Invalid image fingerprint"))?
            || !(1..=crate::artifact::MAX_IMAGE_BYTES).contains(&size)
            || pinned_size.is_some_and(|proven| proven != size)
        {
            return Err(invalid("Image deployment metadata mismatch"));
        }
    }
    Ok(())
}

fn validate_layout(
    package_type: &str,
    has_zip: bool,
    has_extracted: bool,
    image: Option<&ResolvedImage>,
    architecture: Architecture,
    reference: Option<&str>,
) -> Result<(), LambdaServiceError> {
    if (package_type == "Zip" && image.is_some())
        || (package_type == "Image" && (has_extracted || has_zip))
    {
        return Err(invalid("Deployment package fields are inconsistent"));
    }
    if let Some(image) = image {
        if image.metadata().architecture != architecture
            || Some(image.metadata().requested_reference.as_str()) != reference
        {
            return Err(invalid(
                "Resolved image does not match function configuration",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_image_config(config: &ImageConfig) -> Result<(), LambdaServiceError> {
    let mut bytes = 0usize;
    for args in [config.entry_point.as_deref(), config.command.as_deref()]
        .into_iter()
        .flatten()
    {
        if args.len() > 1500 {
            return Err(invalid("Image argv count exceeds the local limit"));
        }
        for arg in args {
            if arg.len() > 4096 || arg.contains('\0') {
                return Err(invalid("Invalid image argument"));
            }
            bytes = bytes
                .checked_add(arg.len())
                .ok_or_else(|| invalid("Image argv size overflow"))?;
            if bytes > 64 * 1024 {
                return Err(invalid("Image argv exceeds the local byte limit"));
            }
        }
    }
    if config
        .working_directory
        .as_ref()
        .is_some_and(|path| !path.starts_with('/') || path.len() > 1000 || path.contains('\0'))
    {
        return Err(invalid(
            "Image working directory must be a bounded absolute path",
        ));
    }
    Ok(())
}

fn validate_environment(environment: &HashMap<String, String>) -> Result<(), LambdaServiceError> {
    validate_environment_entries(environment)
}

pub(crate) fn validate_environment_entries<'a>(
    environment: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Result<(), LambdaServiceError> {
    let mut count = 0_u16;
    let mut bytes = 0usize;
    for (key, value) in environment {
        count = count.saturating_add(1);
        if count > 128 {
            return Err(invalid("Environment has too many variables"));
        }
        if !(2..=64).contains(&key.len())
            || !key.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.len() > 4096
            || value.contains('\0')
            || is_reserved(key)
        {
            return Err(invalid("Invalid or reserved environment variable"));
        }
        bytes = bytes
            .checked_add(key.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or_else(|| invalid("Environment size overflow"))?;
        if bytes > 4096 {
            return Err(invalid("Environment exceeds 4 KiB"));
        }
    }
    Ok(())
}

fn is_reserved(key: &str) -> bool {
    matches!(
        key,
        "_HANDLER"
            | "_X_AMZN_TRACE_ID"
            | "AWS_DEFAULT_REGION"
            | "AWS_REGION"
            | "AWS_EXECUTION_ENV"
            | "AWS_LAMBDA_FUNCTION_NAME"
            | "AWS_LAMBDA_FUNCTION_MEMORY_SIZE"
            | "AWS_LAMBDA_FUNCTION_VERSION"
            | "AWS_LAMBDA_FUNCTION_TIMEOUT"
            | "AWS_LAMBDA_INITIALIZATION_TYPE"
            | "AWS_LAMBDA_LOG_GROUP_NAME"
            | "AWS_LAMBDA_LOG_STREAM_NAME"
            | "AWS_ACCESS_KEY"
            | "AWS_ACCESS_KEY_ID"
            | "AWS_SECRET_ACCESS_KEY"
            | "AWS_SESSION_TOKEN"
            | "AWS_LAMBDA_RUNTIME_API"
            | "LAMBDA_TASK_ROOT"
            | "LAMBDA_RUNTIME_DIR"
            | "AWS_LAMBDA_MAX_CONCURRENCY"
            | "AWS_LAMBDA_METADATA_API"
            | "AWS_LAMBDA_METADATA_TOKEN"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_bound_environment_bytes_and_reject_reserved_or_invalid_keys() {
        let at_limit = HashMap::from([("KEY".into(), "v".repeat(4093))]);
        assert!(validate_environment(&at_limit).is_ok());
        let over_limit = HashMap::from([("KEY".into(), "v".repeat(4094))]);
        assert!(validate_environment(&over_limit).is_err());
        for key in [
            "AWS_REGION",
            "AWS_LAMBDA_RUNTIME_API",
            "AWS_SECRET_ACCESS_KEY",
            "LAMBDA_TASK_ROOT",
            "A",
            "A-B",
            "ÄB",
        ] {
            let environment = HashMap::from([(key.into(), "value".into())]);
            assert!(validate_environment(&environment).is_err(), "{key}");
        }
        for key in ["AWS_ENDPOINT_URL", "TZ", "PATH"] {
            assert!(validate_environment(&HashMap::from([(key.into(), "value".into())])).is_ok());
        }
    }

    #[test]
    fn test_should_validate_image_argv_without_collapsing_empty_and_missing() {
        assert!(validate_image_config(&ImageConfig::default()).is_ok());
        assert!(
            validate_image_config(&ImageConfig {
                command: Some(vec![]),
                ..ImageConfig::default()
            })
            .is_ok()
        );
        for config in [
            ImageConfig {
                command: Some(vec!["x".repeat(4097)]),
                ..ImageConfig::default()
            },
            ImageConfig {
                command: Some(vec![String::new(); 1501]),
                ..ImageConfig::default()
            },
            ImageConfig {
                command: Some(vec!["x".repeat(4096); 17]),
                ..ImageConfig::default()
            },
            ImageConfig {
                entry_point: Some(vec!["x\0".into()]),
                ..ImageConfig::default()
            },
            ImageConfig {
                working_directory: Some("relative".into()),
                ..ImageConfig::default()
            },
        ] {
            assert!(validate_image_config(&config).is_err());
        }
    }
}
