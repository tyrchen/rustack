//! Service configuration for config-driven code generation.
//!
//! This module defines the TOML configuration structures and the derived
//! runtime `ServiceConfig` used throughout the codegen pipeline.
//!
//! Many fields are defined here for future phases of the universal codegen
//! and are not yet read by the current pipeline.
use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

/// Top-level TOML configuration file structure.
#[derive(Debug, Deserialize)]
pub struct ServiceConfigFile {
    /// Service identification and protocol info.
    pub service: ServiceSection,
    /// Protocol-specific settings.
    #[serde(default)]
    pub protocol: ProtocolSection,
    /// Operations to generate code for.
    #[serde(default)]
    pub operations: OperationsSection,
    /// Custom error definitions.
    #[serde(default)]
    pub errors: ErrorsSection,
    /// Output settings.
    #[serde(default)]
    pub output: OutputSection,
    /// Overlay settings for preserving manual code.
    #[serde(default)]
    pub overlay: OverlaySection,
}

/// Service identification section.
#[derive(Debug, Deserialize)]
pub struct ServiceSection {
    /// Short service name (e.g., "s3").
    pub name: String,
    /// Display name for doc comments (e.g., "S3").
    pub display_name: String,
    /// Rust type prefix (e.g., "S3" for S3Operation, S3Error).
    pub rust_prefix: String,
    /// Smithy namespace (e.g., "com.amazonaws.s3").
    pub namespace: String,
    /// Protocol identifier (e.g., "restXml").
    pub protocol: String,
}

/// Protocol-specific settings.
#[derive(Debug, Default, Deserialize)]
pub struct ProtocolSection {
    /// Serde rename strategy (e.g., "PascalCase", "none").
    #[serde(default = "default_serde_rename")]
    pub serde_rename: String,
    /// Target prefix for JSON protocols.
    #[serde(default)]
    pub target_prefix: Option<String>,
    /// Whether to emit HTTP binding comments on struct fields.
    #[serde(default)]
    pub emit_http_bindings: bool,
    /// Whether to derive Serialize/Deserialize on generated types.
    #[serde(default = "default_true")]
    pub emit_serde_derives: bool,
    /// Whether to emit the request wrapper (S3Request, StreamingBlob, etc.).
    #[serde(default)]
    pub emit_request_wrapper: bool,
    /// Custom error type format string.
    #[serde(default)]
    pub error_type_format: Option<String>,
    /// Whether this service uses AWS Query compatible error mode.
    #[serde(default)]
    pub aws_query_compatible: bool,
}

/// Operations to generate.
#[derive(Debug, Default, Deserialize)]
pub struct OperationsSection {
    /// Explicit ordered list of all operations (determines enum variant order).
    /// When present, this takes precedence over phase/category ordering.
    #[serde(default)]
    pub all: Vec<String>,
    /// Phase 0 operations.
    #[serde(default)]
    pub phase0: Vec<String>,
    /// Phase 1 operations.
    #[serde(default)]
    pub phase1: Vec<String>,
    /// Phase 2 operations.
    #[serde(default)]
    pub phase2: Vec<String>,
    /// Phase 3 operations.
    #[serde(default)]
    pub phase3: Vec<String>,
    /// Categorized operations (e.g., for S3's file-per-category layout).
    #[serde(default)]
    pub categories: BTreeMap<String, Vec<String>>,
}

/// Custom error definitions.
#[derive(Debug, Default, Deserialize)]
pub struct ErrorsSection {
    /// Custom error codes with status and message.
    #[serde(default)]
    pub custom: BTreeMap<String, CustomError>,
}

/// A custom error definition.
#[derive(Debug, Deserialize)]
pub struct CustomError {
    /// HTTP status code.
    pub status: u16,
    /// Default error message.
    pub message: String,
}

/// Output settings.
#[derive(Debug, Default, Deserialize)]
pub struct OutputSection {
    /// Output directory override.
    #[serde(default)]
    pub dir: Option<String>,
    /// File layout strategy: "flat" or "categorized".
    #[serde(default = "default_file_layout")]
    pub file_layout: String,
    /// Whether to always serialize arrays (even when empty).
    #[serde(default)]
    pub always_serialize_arrays: bool,
}

/// Overlay settings for preserving manual code.
#[derive(Debug, Default, Deserialize)]
pub struct OverlaySection {
    /// Files to preserve during regeneration.
    #[serde(default)]
    pub preserve: Vec<String>,
    /// Extra modules to include in lib.rs.
    #[serde(default)]
    pub extra_modules: Vec<String>,
}

fn default_serde_rename() -> String {
    "PascalCase".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_file_layout() -> String {
    "flat".to_owned()
}

/// Derived runtime configuration used by the codegen pipeline.
#[derive(Debug)]
pub struct ServiceConfig {
    /// Short service name.
    pub name: String,
    /// Display name for doc comments.
    pub display_name: String,
    /// Rust type prefix.
    pub rust_prefix: String,
    /// Smithy namespace.
    pub namespace: String,
    /// Protocol variant.
    pub protocol: Protocol,
    /// Serde rename strategy (None means no rename).
    pub serde_rename: Option<String>,
    /// Whether to emit HTTP binding comments.
    pub emit_http_bindings: bool,
    /// Whether to derive Serialize/Deserialize.
    pub emit_serde_derives: bool,
    /// Whether to generate request wrapper types.
    pub emit_request_wrapper: bool,
    /// Ordered list of all operations to generate.
    pub all_operations: Vec<String>,
    /// Map from operation name to phase number.
    pub operation_phases: BTreeMap<String, usize>,
    /// Operation categories for file organization (if categorized layout).
    pub categories: Option<BTreeMap<String, Vec<String>>>,
    /// File layout strategy.
    pub file_layout: String,
    /// Custom error definitions.
    pub custom_errors: BTreeMap<String, CustomError>,
    /// Files to preserve during regeneration.
    pub overlay_preserve: Vec<String>,
    /// Extra modules for lib.rs.
    pub overlay_extra_modules: Vec<String>,
    /// Whether to always serialize arrays.
    pub always_serialize_arrays: bool,
}

/// Supported protocol variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// REST XML protocol (used by S3).
    RestXml,
    /// AWS JSON 1.0 protocol.
    AwsJson1_0,
    /// AWS JSON 1.1 protocol.
    AwsJson1_1,
    /// AWS Query protocol.
    AwsQuery,
    /// REST JSON 1 protocol.
    RestJson1,
}

impl ServiceConfig {
    /// Build a `ServiceConfig` from a parsed TOML configuration file.
    pub fn from_file(config: ServiceConfigFile) -> Result<Self> {
        let protocol = match config.service.protocol.as_str() {
            "restXml" => Protocol::RestXml,
            "awsJson1_0" => Protocol::AwsJson1_0,
            "awsJson1_1" => Protocol::AwsJson1_1,
            "awsQuery" => Protocol::AwsQuery,
            "restJson1" => Protocol::RestJson1,
            other => bail!("Unknown protocol: {other}"),
        };

        let serde_rename = match config.protocol.serde_rename.as_str() {
            "none" | "" => None,
            s => Some(s.to_owned()),
        };

        // Build all_operations list.
        // Priority: explicit `all` list > phases > categories
        let mut all_operations = Vec::new();
        let mut operation_phases = BTreeMap::new();

        if !config.operations.all.is_empty() {
            // Use the explicit ordered list
            all_operations = config.operations.all.clone();
        } else {
            // Build from phases
            for (phase_num, phase_ops) in [
                (0, &config.operations.phase0),
                (1, &config.operations.phase1),
                (2, &config.operations.phase2),
                (3, &config.operations.phase3),
            ] {
                for op in phase_ops {
                    all_operations.push(op.clone());
                    operation_phases.insert(op.clone(), phase_num);
                }
            }

            // If still empty, build from categories
            if all_operations.is_empty() {
                for ops in config.operations.categories.values() {
                    for op in ops {
                        if !all_operations.contains(op) {
                            all_operations.push(op.clone());
                        }
                    }
                }
            }
        }

        // Build categories map
        let categories = if config.operations.categories.is_empty() {
            None
        } else {
            Some(config.operations.categories.clone())
        };

        Ok(Self {
            name: config.service.name,
            display_name: config.service.display_name,
            rust_prefix: config.service.rust_prefix,
            namespace: config.service.namespace,
            protocol,
            serde_rename,
            emit_http_bindings: config.protocol.emit_http_bindings,
            emit_serde_derives: config.protocol.emit_serde_derives,
            emit_request_wrapper: config.protocol.emit_request_wrapper,
            all_operations,
            operation_phases,
            categories,
            file_layout: config.output.file_layout,
            custom_errors: config.errors.custom,
            overlay_preserve: config.overlay.preserve,
            overlay_extra_modules: config.overlay.extra_modules,
            always_serialize_arrays: config.output.always_serialize_arrays,
        })
    }

    /// Whether this service uses serde derives.
    pub fn uses_serde(&self) -> bool {
        self.emit_serde_derives
    }

    /// Returns the namespace prefix with trailing `#`.
    pub fn namespace_prefix(&self) -> String {
        format!("{}#", self.namespace)
    }
}
