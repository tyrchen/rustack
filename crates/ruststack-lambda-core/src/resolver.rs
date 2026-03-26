//! Function name/ARN resolution and ARN construction.
//!
//! Lambda allows referencing functions by:
//! - Simple name: `my-function`
//! - Qualified name: `my-function:qualifier`
//! - Full ARN: `arn:aws:lambda:us-east-1:123456789012:function:my-function`
//! - Qualified ARN: `arn:aws:lambda:us-east-1:123456789012:function:my-function:qualifier`

use crate::{
    error::LambdaServiceError,
    storage::{FunctionRecord, VersionRecord},
};

/// Parse a function reference into `(function_name, optional_qualifier)`.
///
/// Handles:
/// - `my-function` -> `("my-function", None)`
/// - `my-function:prod` -> `("my-function", Some("prod"))`
/// - `arn:aws:lambda:...:function:my-function` -> `("my-function", None)`
/// - `arn:aws:lambda:...:function:my-function:prod` -> `("my-function", Some("prod"))`
///
/// # Errors
///
/// Returns `InvalidArn` if the input looks like an ARN but cannot be parsed.
pub fn resolve_function_ref(
    function_ref: &str,
) -> Result<(String, Option<String>), LambdaServiceError> {
    if function_ref.starts_with("arn:") {
        parse_arn(function_ref)
    } else if let Some((left, right)) = function_ref.split_once(':') {
        // Handle partial ARN: `{account}:function:{name}[:{qualifier}]`
        if let Some(rest) = right.strip_prefix("function:") {
            if let Some((name, qualifier)) = rest.split_once(':') {
                Ok((name.to_owned(), Some(qualifier.to_owned())))
            } else {
                Ok((rest.to_owned(), None))
            }
        } else {
            // Simple qualified name: `my-function:qualifier`
            Ok((left.to_owned(), Some(right.to_owned())))
        }
    } else {
        Ok((function_ref.to_owned(), None))
    }
}

/// Parse a Lambda function ARN.
///
/// Format: `arn:aws:lambda:{region}:{account}:function:{name}[:{qualifier}]`
fn parse_arn(arn: &str) -> Result<(String, Option<String>), LambdaServiceError> {
    let parts: Vec<&str> = arn.split(':').collect();
    // Unqualified ARN: arn:aws:lambda:region:account:function:name = 7 parts
    // Qualified ARN:   arn:aws:lambda:region:account:function:name:qualifier = 8 parts
    if parts.len() < 7 || parts[0] != "arn" || parts[2] != "lambda" || parts[5] != "function" {
        return Err(LambdaServiceError::InvalidArn {
            arn: arn.to_owned(),
        });
    }

    let name = parts[6].to_owned();
    let qualifier = if parts.len() >= 8 && !parts[7].is_empty() {
        Some(parts[7].to_owned())
    } else {
        None
    };

    Ok((name, qualifier))
}

/// Resolve a qualifier to a specific `VersionRecord`.
///
/// Qualifiers can be:
/// - `$LATEST` -> the latest version record
/// - A numeric version string -> a published version
/// - An alias name -> the version the alias points to
///
/// If `qualifier` is `None`, returns `$LATEST`.
///
/// # Errors
///
/// Returns `VersionNotFound` or `AliasNotFound` if the qualifier cannot
/// be resolved to an existing version.
pub fn resolve_version<'a>(
    function: &'a FunctionRecord,
    qualifier: Option<&str>,
) -> Result<&'a VersionRecord, LambdaServiceError> {
    match qualifier {
        None | Some("$LATEST") => Ok(&function.latest),
        Some(q) => {
            // Try numeric version first.
            if let Ok(version_num) = q.parse::<u64>() {
                return function.versions.get(&version_num).ok_or(
                    LambdaServiceError::VersionNotFound {
                        function_name: function.name.clone(),
                        version: q.to_owned(),
                    },
                );
            }
            // Try alias.
            if let Some(alias) = function.aliases.get(q) {
                let version_str = &alias.function_version;
                if version_str == "$LATEST" {
                    return Ok(&function.latest);
                }
                if let Ok(version_num) = version_str.parse::<u64>() {
                    return function.versions.get(&version_num).ok_or(
                        LambdaServiceError::VersionNotFound {
                            function_name: function.name.clone(),
                            version: version_str.clone(),
                        },
                    );
                }
            }
            Err(LambdaServiceError::AliasNotFound {
                function_name: function.name.clone(),
                alias: q.to_owned(),
            })
        }
    }
}

/// Construct the unqualified function ARN.
///
/// Format: `arn:aws:lambda:{region}:{account_id}:function:{function_name}`
#[must_use]
pub fn function_arn(region: &str, account_id: &str, function_name: &str) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}")
}

/// Construct a version-qualified function ARN.
///
/// Format: `arn:aws:lambda:{region}:{account_id}:function:{function_name}:{version}`
#[must_use]
pub fn function_version_arn(
    region: &str,
    account_id: &str,
    function_name: &str,
    version: &str,
) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}:{version}")
}

/// Construct an alias ARN.
///
/// Format: `arn:aws:lambda:{region}:{account_id}:function:{function_name}:{alias}`
#[must_use]
pub fn alias_arn(region: &str, account_id: &str, function_name: &str, alias: &str) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}:{alias}")
}

/// Construct a layer ARN (without version).
///
/// Format: `arn:aws:lambda:{region}:{account_id}:layer:{layer_name}`
#[must_use]
pub fn layer_arn(region: &str, account_id: &str, layer_name: &str) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:layer:{layer_name}")
}

/// Construct a layer version ARN.
///
/// Format: `arn:aws:lambda:{region}:{account_id}:layer:{layer_name}:{version}`
#[must_use]
pub fn layer_version_arn(region: &str, account_id: &str, layer_name: &str, version: u64) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:layer:{layer_name}:{version}")
}

/// Parse a layer version ARN into `(layer_name, version)`.
///
/// Format: `arn:aws:lambda:{region}:{account}:layer:{name}:{version}`
///
/// # Errors
///
/// Returns `InvalidArn` if the ARN cannot be parsed.
pub fn parse_layer_version_arn(arn: &str) -> Result<(String, u64), LambdaServiceError> {
    let parts: Vec<&str> = arn.split(':').collect();
    // Expected: arn:aws:lambda:region:account:layer:name:version = 8 parts
    if parts.len() < 8 || parts[0] != "arn" || parts[2] != "lambda" || parts[5] != "layer" {
        return Err(LambdaServiceError::InvalidArn {
            arn: arn.to_owned(),
        });
    }

    let name = parts[6].to_owned();
    let version: u64 = parts[7]
        .parse()
        .map_err(|_| LambdaServiceError::InvalidArn {
            arn: arn.to_owned(),
        })?;

    Ok((name, version))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::storage::{AliasRecord, PolicyDocument, VersionRecord};

    fn make_version(ver: &str) -> VersionRecord {
        VersionRecord {
            version: ver.to_owned(),
            runtime: Some("python3.12".to_owned()),
            handler: Some("index.handler".to_owned()),
            role: "arn:aws:iam::000000000000:role/test".to_owned(),
            description: String::new(),
            timeout: 3,
            memory_size: 128,
            environment: HashMap::new(),
            package_type: "Zip".to_owned(),
            code_path: None,
            image_uri: None,
            zip_bytes: None,
            state: "Active".to_owned(),
            last_modified: "2024-01-01T00:00:00.000+0000".to_owned(),
            architectures: vec!["x86_64".to_owned()],
            ephemeral_storage_size: 512,
            code_sha256: "abc".to_owned(),
            code_size: 100,
            revision_id: "rev-1".to_owned(),
            layers: Vec::new(),
            vpc_config: None,
            dead_letter_config: None,
            tracing_config: None,
            image_config: None,
            logging_config: None,
            snap_start: None,
        }
    }

    fn make_function() -> FunctionRecord {
        let mut versions = BTreeMap::new();
        versions.insert(1, make_version("1"));

        let mut aliases = HashMap::new();
        aliases.insert(
            "prod".to_owned(),
            AliasRecord {
                name: "prod".to_owned(),
                function_version: "1".to_owned(),
                description: String::new(),
                routing_config: None,
                revision_id: "rev-a".to_owned(),
            },
        );

        FunctionRecord {
            name: "my-func".to_owned(),
            arn: "arn:aws:lambda:us-east-1:000000000000:function:my-func".to_owned(),
            latest: make_version("$LATEST"),
            versions,
            next_version: 2,
            aliases,
            policy: PolicyDocument::default(),
            tags: HashMap::new(),
            url_config: None,
            reserved_concurrent_executions: None,
            event_invoke_configs: HashMap::new(),
            created_at: "2024-01-01T00:00:00.000+0000".to_owned(),
        }
    }

    // ---- resolve_function_ref tests ----

    #[test]
    fn test_should_resolve_simple_name() {
        let (name, qual) = resolve_function_ref("my-func").unwrap();
        assert_eq!(name, "my-func");
        assert!(qual.is_none());
    }

    #[test]
    fn test_should_resolve_qualified_name() {
        let (name, qual) = resolve_function_ref("my-func:prod").unwrap();
        assert_eq!(name, "my-func");
        assert_eq!(qual.unwrap(), "prod");
    }

    #[test]
    fn test_should_resolve_unqualified_arn() {
        let (name, qual) =
            resolve_function_ref("arn:aws:lambda:us-east-1:123456789012:function:my-func").unwrap();
        assert_eq!(name, "my-func");
        assert!(qual.is_none());
    }

    #[test]
    fn test_should_resolve_qualified_arn() {
        let (name, qual) =
            resolve_function_ref("arn:aws:lambda:us-east-1:123456789012:function:my-func:prod")
                .unwrap();
        assert_eq!(name, "my-func");
        assert_eq!(qual.unwrap(), "prod");
    }

    #[test]
    fn test_should_resolve_partial_arn() {
        let (name, qual) = resolve_function_ref("000000000000:function:my-func").unwrap();
        assert_eq!(name, "my-func");
        assert!(qual.is_none());
    }

    #[test]
    fn test_should_resolve_partial_arn_with_qualifier() {
        let (name, qual) = resolve_function_ref("000000000000:function:my-func:prod").unwrap();
        assert_eq!(name, "my-func");
        assert_eq!(qual.unwrap(), "prod");
    }

    #[test]
    fn test_should_error_on_invalid_arn() {
        let err = resolve_function_ref("arn:invalid").unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidArn { .. }));
    }

    // ---- resolve_version tests ----

    #[test]
    fn test_should_resolve_latest_by_default() {
        let func = make_function();
        let ver = resolve_version(&func, None).unwrap();
        assert_eq!(ver.version, "$LATEST");
    }

    #[test]
    fn test_should_resolve_explicit_latest() {
        let func = make_function();
        let ver = resolve_version(&func, Some("$LATEST")).unwrap();
        assert_eq!(ver.version, "$LATEST");
    }

    #[test]
    fn test_should_resolve_numeric_version() {
        let func = make_function();
        let ver = resolve_version(&func, Some("1")).unwrap();
        assert_eq!(ver.version, "1");
    }

    #[test]
    fn test_should_resolve_alias() {
        let func = make_function();
        let ver = resolve_version(&func, Some("prod")).unwrap();
        assert_eq!(ver.version, "1");
    }

    #[test]
    fn test_should_error_on_unknown_version() {
        let func = make_function();
        let err = resolve_version(&func, Some("99")).unwrap_err();
        assert!(matches!(err, LambdaServiceError::VersionNotFound { .. }));
    }

    #[test]
    fn test_should_error_on_unknown_alias() {
        let func = make_function();
        let err = resolve_version(&func, Some("staging")).unwrap_err();
        assert!(matches!(err, LambdaServiceError::AliasNotFound { .. }));
    }

    // ---- ARN construction tests ----

    #[test]
    fn test_should_build_function_arn() {
        assert_eq!(
            function_arn("us-east-1", "123456789012", "my-func"),
            "arn:aws:lambda:us-east-1:123456789012:function:my-func"
        );
    }

    #[test]
    fn test_should_build_function_version_arn() {
        assert_eq!(
            function_version_arn("us-east-1", "123456789012", "my-func", "1"),
            "arn:aws:lambda:us-east-1:123456789012:function:my-func:1"
        );
    }

    #[test]
    fn test_should_build_alias_arn() {
        assert_eq!(
            alias_arn("us-east-1", "123456789012", "my-func", "prod"),
            "arn:aws:lambda:us-east-1:123456789012:function:my-func:prod"
        );
    }

    // ---- Layer ARN tests ----

    #[test]
    fn test_should_build_layer_arn() {
        assert_eq!(
            layer_arn("us-east-1", "123456789012", "my-layer"),
            "arn:aws:lambda:us-east-1:123456789012:layer:my-layer"
        );
    }

    #[test]
    fn test_should_build_layer_version_arn() {
        assert_eq!(
            layer_version_arn("us-east-1", "123456789012", "my-layer", 3),
            "arn:aws:lambda:us-east-1:123456789012:layer:my-layer:3"
        );
    }

    #[test]
    fn test_should_parse_layer_version_arn_valid() {
        let (name, version) =
            parse_layer_version_arn("arn:aws:lambda:us-east-1:123456789012:layer:my-layer:5")
                .unwrap();
        assert_eq!(name, "my-layer");
        assert_eq!(version, 5);
    }

    #[test]
    fn test_should_error_on_invalid_layer_arn() {
        let err = parse_layer_version_arn("arn:invalid:stuff").unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidArn { .. }));
    }

    #[test]
    fn test_should_error_on_non_numeric_layer_version() {
        let err =
            parse_layer_version_arn("arn:aws:lambda:us-east-1:123456789012:layer:my-layer:abc")
                .unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidArn { .. }));
    }
}
