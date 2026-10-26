//! Validated image defaults retained through capture and snapshot restoration.

use std::{fmt, sync::Arc};

use rustack_lambda_model::types::ImageConfig;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::ArtifactError;
use crate::function_config::validate_image_config;

/// Primitive image execution metadata; validate with `ImageExecutionDefaults`.
#[derive(Clone, Serialize, Deserialize, TypedBuilder)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct ImageExecutionMetadata {
    /// Inherited entrypoint, before function overrides.
    pub entrypoint: Vec<String>,
    /// Inherited command, before function overrides.
    pub command: Vec<String>,
    /// Image-internal working directory; empty normalizes to root.
    pub working_directory: String,
    /// Ordered inherited environment, including Docker deletion markers.
    pub environment: Vec<String>,
    /// Image user declaration; container security policy is applied separately.
    pub user: String,
}
impl fmt::Debug for ImageExecutionMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImageExecutionMetadata(<redacted>)")
    }
}

/// Immutable normalized image defaults, revalidated on deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "ImageExecutionMetadata", into = "ImageExecutionMetadata")]
#[non_exhaustive]
pub struct ImageExecutionDefaults(Arc<ImageExecutionMetadata>);

impl TryFrom<ImageExecutionMetadata> for ImageExecutionDefaults {
    type Error = ArtifactError;
    fn try_from(mut metadata: ImageExecutionMetadata) -> Result<Self, Self::Error> {
        if metadata.entrypoint.len() == 1
            && metadata.entrypoint.first().is_some_and(String::is_empty)
        {
            metadata.entrypoint.clear();
        }
        if metadata.working_directory.is_empty() {
            metadata.working_directory = "/".into();
        }
        validate_image_config(&ImageConfig {
            entry_point: Some(metadata.entrypoint.clone()),
            command: Some(metadata.command.clone()),
            working_directory: Some(metadata.working_directory.clone()),
        })
        .map_err(|_| ArtifactError::Inspection)?;
        validate_environment(&metadata.environment)?;
        if metadata.user.len() > 256
            || !metadata.user.bytes().all(|byte| (32..=126).contains(&byte))
        {
            return Err(ArtifactError::Inspection);
        }
        Ok(Self(Arc::new(metadata)))
    }
}
impl From<ImageExecutionDefaults> for ImageExecutionMetadata {
    fn from(defaults: ImageExecutionDefaults) -> Self {
        Arc::unwrap_or_clone(defaults.0)
    }
}

impl ImageExecutionDefaults {
    /// Borrow normalized defaults; diagnostics must not print their values.
    ///
    /// ```no_run
    /// # fn defaults(defaults: &rustack_lambda_core::artifact::ImageExecutionDefaults) {
    /// let environment = &defaults.metadata().environment;
    /// # }
    /// ```
    #[must_use]
    pub fn metadata(&self) -> &ImageExecutionMetadata {
        &self.0
    }

    /// Materialize the exact Docker argv/workdir, preserving empty overrides.
    ///
    /// # Errors
    /// Rejects invalid overrides, an empty executable or composed argv bounds.
    ///
    /// ```no_run
    /// # fn plan(defaults: &rustack_lambda_core::artifact::ImageExecutionDefaults) -> Result<(), rustack_lambda_core::artifact::ArtifactError> {
    /// let plan = defaults.plan(None)?;
    /// assert!(!plan.entrypoint().is_empty()); // Includes Docker's reset sentinel.
    /// # Ok(()) }
    /// ```
    pub fn plan(
        &self,
        overrides: Option<&ImageConfig>,
    ) -> Result<ImageExecutionPlan, ArtifactError> {
        if let Some(config) = overrides {
            validate_image_config(config).map_err(|_| ArtifactError::Inspection)?;
        }
        let mut entrypoint = overrides
            .and_then(|config| config.entry_point.as_ref())
            .unwrap_or(&self.0.entrypoint)
            .clone();
        let command = overrides
            .and_then(|config| config.command.as_ref())
            .unwrap_or(&self.0.command)
            .clone();
        let workdir = overrides
            .and_then(|config| config.working_directory.as_ref())
            .unwrap_or(&self.0.working_directory)
            .clone();
        if entrypoint.len() == 1 && entrypoint.first().is_some_and(String::is_empty) {
            entrypoint.clear();
        }
        validate_image_config(&ImageConfig {
            entry_point: Some(entrypoint.clone()),
            command: Some(command.clone()),
            working_directory: Some(workdir.clone()),
        })
        .map_err(|_| ArtifactError::Inspection)?;
        if entrypoint
            .iter()
            .chain(&command)
            .next()
            .is_none_or(String::is_empty)
        {
            return Err(ArtifactError::Inspection);
        }
        if entrypoint.is_empty() {
            entrypoint.push(String::new());
        }
        Ok(ImageExecutionPlan {
            entrypoint,
            command,
            workdir,
        })
    }
}

/// Validated Docker-create fields; no daemon inheritance is needed.
#[derive(Clone)]
#[non_exhaustive]
pub struct ImageExecutionPlan {
    entrypoint: Vec<String>,
    command: Vec<String>,
    workdir: String,
}
impl fmt::Debug for ImageExecutionPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImageExecutionPlan(<redacted>)")
    }
}
impl ImageExecutionPlan {
    /// Exact create entrypoint; semantic emptiness uses the `[""]` reset sentinel.
    ///
    /// ```no_run
    /// # fn entry(plan: &rustack_lambda_core::artifact::ImageExecutionPlan) {
    /// let entrypoint = plan.entrypoint().to_vec();
    /// # }
    /// ```
    #[must_use]
    pub fn entrypoint(&self) -> &[String] {
        &self.entrypoint
    }
    /// Exact create command, including an explicitly empty array.
    ///
    /// ```no_run
    /// # fn command(plan: &rustack_lambda_core::artifact::ImageExecutionPlan) {
    /// let command = plan.command().to_vec();
    /// # }
    /// ```
    #[must_use]
    pub fn command(&self) -> &[String] {
        &self.command
    }
    /// Exact absolute working directory inside the image.
    ///
    /// ```no_run
    /// # fn workdir(plan: &rustack_lambda_core::artifact::ImageExecutionPlan) {
    /// let workdir = plan.working_directory();
    /// # }
    /// ```
    #[must_use]
    pub fn working_directory(&self) -> &str {
        &self.workdir
    }
}

fn validate_environment(environment: &[String]) -> Result<(), ArtifactError> {
    if environment.len() > 128 {
        return Err(ArtifactError::Inspection);
    }
    let mut total = 0_usize;
    for entry in environment {
        let key = entry.split_once('=').map_or(entry.as_str(), |(key, _)| key);
        if entry.len() > 4096
            || entry.contains('\0')
            || key.len() > 256
            || !key
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(ArtifactError::Inspection);
        }
        total = total
            .checked_add(entry.len())
            .filter(|total| *total <= 64 * 1024)
            .ok_or(ArtifactError::Inspection)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> ImageExecutionMetadata {
        ImageExecutionMetadata::builder()
            .entrypoint(vec!["/entry".into()])
            .command(vec!["handler".into()])
            .working_directory("/task".into())
            .environment(vec!["PATH=/bin".into(), "REMOVED".into()])
            .user("1000:1000".into())
            .build()
    }

    #[test]
    fn test_should_materialize_inherited_and_explicit_empty_docker_argv() {
        let defaults = ImageExecutionDefaults::try_from(metadata()).unwrap();
        let inherited = defaults.plan(None).unwrap();
        assert_eq!(inherited.entrypoint(), ["/entry"]);
        assert_eq!(inherited.command(), ["handler"]);
        assert_eq!(inherited.working_directory(), "/task");
        let clear_command = defaults
            .plan(Some(&ImageConfig {
                command: Some(vec![]),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(clear_command.command(), [] as [String; 0]);
        assert_eq!(clear_command.entrypoint(), ["/entry"]);
        for reset in [vec![], vec![String::new()]] {
            let override_config = ImageConfig {
                entry_point: Some(reset),
                command: Some(vec!["/other".into()]),
                working_directory: Some("/tmp".into()),
            };
            let plan = defaults.plan(Some(&override_config)).unwrap();
            assert_eq!(plan.entrypoint(), [""]);
            assert_eq!(plan.command(), ["/other"]);
            assert_eq!(plan.working_directory(), "/tmp");
        }
        assert!(
            defaults
                .plan(Some(&ImageConfig {
                    entry_point: Some(vec![]),
                    command: Some(vec![]),
                    ..Default::default()
                }))
                .is_err()
        );
    }

    #[test]
    fn test_should_allow_empty_defaults_with_an_executable_override_and_normalize_root() {
        let mut input = metadata();
        input.entrypoint = vec![String::new()];
        input.command.clear();
        input.working_directory.clear();
        let defaults = ImageExecutionDefaults::try_from(input).unwrap();
        assert_eq!(defaults.metadata().entrypoint, [] as [String; 0]);
        assert_eq!(defaults.metadata().working_directory, "/");
        assert!(defaults.plan(None).is_err());
        let plan = defaults
            .plan(Some(&ImageConfig {
                command: Some(vec!["/start".into()]),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(plan.entrypoint(), [""]);
        assert_eq!(plan.command(), ["/start"]);
        let mut invalid = metadata();
        invalid.entrypoint.clear();
        invalid.command.clear();
        invalid.working_directory = "relative".into();
        assert!(ImageExecutionDefaults::try_from(invalid).is_err());
    }

    #[rstest::rstest]
    #[case("argv_count")]
    #[case("argv_item")]
    #[case("argv_total")]
    #[case("argv_nul")]
    #[case("directory")]
    #[case("directory_size")]
    #[case("env_count")]
    #[case("env_item")]
    #[case("env_total")]
    #[case("env_key")]
    #[case("env_key_size")]
    #[case("env_nul")]
    #[case("user_size")]
    #[case("user_control")]
    fn test_should_reject_invalid_defaults_before_capture(#[case] field: &str) {
        let mut input = metadata();
        match field {
            "argv_count" => input.command = vec!["x".into(); 1501],
            "argv_item" => input.command = vec!["x".repeat(4097)],
            "argv_total" => input.command = vec!["x".repeat(4096); 17],
            "argv_nul" => input.entrypoint = vec!["/bin/x\0".into()],
            "directory" => input.working_directory = "relative".into(),
            "directory_size" => input.working_directory = format!("/{}", "x".repeat(1000)),
            "env_count" => input.environment = vec!["X=1".into(); 129],
            "env_item" => input.environment = vec![format!("X={}", "x".repeat(4095))],
            "env_total" => input.environment = vec![format!("X={}", "x".repeat(4094)); 17],
            "env_key" => input.environment = vec!["1KEY=value".into()],
            "env_key_size" => input.environment = vec![format!("{}=x", "K".repeat(257))],
            "env_nul" => input.environment = vec!["KEY=\0".into()],
            "user_size" => input.user = "x".repeat(257),
            "user_control" => input.user = "1000\n".into(),
            _ => panic!("unknown test field"),
        }
        assert!(matches!(
            ImageExecutionDefaults::try_from(input),
            Err(ArtifactError::Inspection)
        ));
    }

    #[test]
    fn test_should_check_composed_argv_and_environment_exact_byte_boundaries() {
        let mut input = metadata();
        input.entrypoint = vec!["e".repeat(4096); 16];
        input.command.clear();
        input.environment = vec![format!("X={}", "x".repeat(4094)); 16];
        let defaults = ImageExecutionDefaults::try_from(input).unwrap();
        defaults.plan(None).unwrap();
        assert!(
            defaults
                .plan(Some(&ImageConfig {
                    command: Some(vec!["x".into()]),
                    ..Default::default()
                }))
                .is_err()
        );
        assert!(
            defaults
                .plan(Some(&ImageConfig {
                    working_directory: Some(String::new()),
                    ..Default::default()
                }))
                .is_err()
        );
    }

    #[test]
    fn test_should_revalidate_saved_defaults_and_redact_all_debug_surfaces() {
        let mut input = metadata();
        input.entrypoint = vec!["secret_argv".into()];
        input.environment = vec!["SECRET=secret_value".into()];
        input.working_directory = "/secret_path".into();
        input.user = "secret_user".into();
        let defaults = ImageExecutionDefaults::try_from(input.clone()).unwrap();
        let saved = serde_json::to_value(&defaults).unwrap();
        let restored: ImageExecutionDefaults = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(restored.plan(None).unwrap().entrypoint(), ["secret_argv"]);
        for shown in [
            format!("{input:?}"),
            format!("{defaults:?}"),
            format!("{:?}", defaults.plan(None).unwrap()),
        ] {
            assert!(!shown.contains("secret_"));
        }
        let mut invalid = saved;
        invalid["environment"] = serde_json::json!(["BAD-KEY=value"]);
        assert!(serde_json::from_value::<ImageExecutionDefaults>(invalid).is_err());
    }
}
