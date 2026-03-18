//! Rust code generation from resolved Smithy shapes.
//!
//! This module takes a `ResolvedModel` and produces Rust source code strings
//! for types, input/output structs, operations, errors, and request types.

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::Result;

use heck::{ToLowerCamelCase, ToPascalCase};

use crate::config::ServiceConfig;
use crate::shapes::{EnumVariantInfo, FieldInfo, HttpBinding, OperationCategories, ResolvedModel};

/// Header comment placed at the top of every generated file.
fn file_header(config: &ServiceConfig) -> String {
    format!(
        "//! Auto-generated from AWS {} Smithy model. DO NOT EDIT.",
        config.display_name
    )
}

/// Generate all source files and return them as a map of path -> content.
pub fn generate_all(
    resolved: &ResolvedModel,
    config: &ServiceConfig,
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();

    files.insert("types.rs".to_owned(), generate_types(resolved, config)?);
    files.insert("operations.rs".to_owned(), generate_operations(config)?);
    files.insert("error.rs".to_owned(), generate_error(resolved, config)?);
    if config.emit_request_wrapper {
        files.insert("request.rs".to_owned(), generate_request(config)?);
    }
    files.insert("lib.rs".to_owned(), generate_lib(resolved, config)?);

    if config.file_layout == "flat" {
        // Flat layout: single input.rs and output.rs files
        generate_flat_io(
            &mut files,
            "input",
            &resolved.input_structs,
            resolved,
            config,
        )?;
        generate_flat_io(
            &mut files,
            "output",
            &resolved.output_structs,
            resolved,
            config,
        )?;
    } else {
        // Categorized layout: input/mod.rs + input/{cat}.rs
        generate_io_modules(
            &mut files,
            "input",
            &resolved.input_categories,
            &resolved.input_structs,
            resolved,
            config,
        )?;
        generate_io_modules(
            &mut files,
            "output",
            &resolved.output_categories,
            &resolved.output_structs,
            resolved,
            config,
        )?;
    }

    Ok(files)
}

/// Generate types.rs with all shared enums and structs.
fn generate_types(resolved: &ResolvedModel, config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let display_name = &config.display_name;
    let mut out = String::with_capacity(64 * 1024);
    writeln!(out, "{header}")?;
    writeln!(out)?;

    // Check if HashMap is needed in shared structs
    let needs_hashmap = resolved
        .shared_structs
        .values()
        .any(|fields| fields.iter().any(|f| f.rust_type.contains("HashMap")));
    if needs_hashmap {
        writeln!(out, "use std::collections::HashMap;")?;
        writeln!(out)?;
    }

    // Serde is always needed for enums (they always derive Serialize, Deserialize).
    // When config.uses_serde() is true, structs also need it.
    let needs_serde = !resolved.enums.is_empty() || config.uses_serde();
    if needs_serde {
        writeln!(out, "use serde::{{Deserialize, Serialize}};")?;
        writeln!(out)?;
    }

    // Check if StreamingBlob is needed in shared structs
    let needs_streaming_blob = resolved
        .shared_structs
        .values()
        .any(|fields| fields.iter().any(|f| f.rust_type.contains("StreamingBlob")));
    if needs_streaming_blob {
        writeln!(out, "use crate::request::StreamingBlob;")?;
        writeln!(out)?;
    }

    // Generate enums
    for (name, variants) in &resolved.enums {
        write_enum(&mut out, name, variants, display_name)?;
    }

    // Generate shared structs
    for (name, fields) in &resolved.shared_structs {
        write_struct(&mut out, name, fields, false, display_name, config)?;
    }

    Ok(out)
}

/// Generate a single Rust enum.
fn write_enum(
    out: &mut String,
    name: &str,
    variants: &[EnumVariantInfo],
    display_name: &str,
) -> Result<()> {
    writeln!(out, "/// {display_name} {name} enum.")?;
    writeln!(
        out,
        "#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]"
    )?;
    writeln!(out, "pub enum {name} {{")?;

    for (i, variant) in variants.iter().enumerate() {
        if i == 0 {
            writeln!(out, "    /// Default variant.")?;
            writeln!(out, "    #[default]")?;
        }
        if variant.rust_name != variant.string_value {
            writeln!(out, "    #[serde(rename = \"{}\")]", variant.string_value)?;
        }
        writeln!(out, "    {},", variant.rust_name)?;
    }

    writeln!(out, "}}")?;
    writeln!(out)?;

    // Generate as_str()
    writeln!(out, "impl {name} {{")?;
    writeln!(
        out,
        "    /// Returns the string value of this enum variant."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn as_str(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for variant in variants {
        writeln!(
            out,
            "            Self::{} => \"{}\",",
            variant.rust_name, variant.string_value
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Generate Display
    writeln!(out, "impl std::fmt::Display for {name} {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )?;
    writeln!(out, "        f.write_str(self.as_str())")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Generate From<&str>
    writeln!(out, "impl From<&str> for {name} {{")?;
    writeln!(out, "    fn from(s: &str) -> Self {{")?;
    writeln!(out, "        match s {{")?;
    for variant in variants {
        writeln!(
            out,
            "            \"{}\" => Self::{},",
            variant.string_value, variant.rust_name
        )?;
    }
    writeln!(out, "            _ => Self::default(),")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    Ok(())
}

/// Check whether an explicit `#[serde(rename = "...")]` is needed for a field.
///
/// This compares what `rename_all` would produce from the Rust field name
/// against the actual Smithy name.  When they differ, an explicit rename is
/// required so the serialised form matches the wire protocol.
fn needs_explicit_rename(rust_name: &str, smithy_name: &str, rename_strategy: &str) -> bool {
    // Strip raw-identifier prefix before applying case conversion.
    let clean = rust_name.strip_prefix("r#").unwrap_or(rust_name);
    let expected = match rename_strategy {
        "PascalCase" => clean.to_pascal_case(),
        "camelCase" => clean.to_lower_camel_case(),
        _ => clean.to_owned(),
    };
    expected != smithy_name
}

/// Generate a Rust struct with all fields.
fn write_struct(
    out: &mut String,
    name: &str,
    fields: &[FieldInfo],
    include_http_comments: bool,
    display_name: &str,
    config: &ServiceConfig,
) -> Result<()> {
    writeln!(out, "/// {display_name} {name}.")?;

    if config.uses_serde() {
        writeln!(
            out,
            "#[derive(Debug, Clone, Default, Serialize, Deserialize)]"
        )?;
        if let Some(ref rename) = config.serde_rename {
            writeln!(out, "#[serde(rename_all = \"{rename}\")]")?;
        }
    } else {
        writeln!(out, "#[derive(Debug, Clone, Default)]")?;
    }

    writeln!(out, "pub struct {name} {{")?;

    for field in fields {
        // Write HTTP binding comment if requested
        if include_http_comments && let Some(ref binding) = field.http_binding {
            let comment = match binding {
                HttpBinding::Label => "    /// HTTP label (URI path).".to_owned(),
                HttpBinding::Query(q) => format!("    /// HTTP query: `{q}`."),
                HttpBinding::Header(h) => format!("    /// HTTP header: `{h}`."),
                HttpBinding::Payload => "    /// HTTP payload body.".to_owned(),
                HttpBinding::PrefixHeaders(p) => {
                    format!("    /// HTTP prefix headers: `{p}`.")
                }
            };
            writeln!(out, "{comment}")?;
        }

        // Write serde field attributes when serde is enabled
        if config.uses_serde() {
            // Explicit rename if rename_all would not produce the correct Smithy name
            if let Some(ref strategy) = config.serde_rename
                && needs_explicit_rename(&field.rust_name, &field.smithy_name, strategy)
            {
                writeln!(out, "    #[serde(rename = \"{}\")]", field.smithy_name)?;
            }

            if field.rust_type.starts_with("Option<") {
                writeln!(
                    out,
                    "    #[serde(skip_serializing_if = \"Option::is_none\")]"
                )?;
            } else if field.rust_type.starts_with("Vec<") {
                if config.always_serialize_arrays {
                    writeln!(out, "    #[serde(default)]")?;
                } else {
                    writeln!(
                        out,
                        "    #[serde(default, skip_serializing_if = \"Vec::is_empty\")]"
                    )?;
                }
            } else if field.rust_type.starts_with("HashMap<") {
                writeln!(
                    out,
                    "    #[serde(default, skip_serializing_if = \"HashMap::is_empty\")]"
                )?;
            }
        }

        writeln!(out, "    pub {}: {},", field.rust_name, field.rust_type)?;
    }

    writeln!(out, "}}")?;
    writeln!(out)?;
    Ok(())
}

/// Generate input or output module files.
fn generate_io_modules(
    files: &mut BTreeMap<String, String>,
    kind: &str, // "input" or "output"
    categories: &OperationCategories,
    structs: &BTreeMap<String, Vec<FieldInfo>>,
    resolved: &ResolvedModel,
    config: &ServiceConfig,
) -> Result<()> {
    let header = file_header(config);
    let display_name = &config.display_name;

    // Generate mod.rs
    let mut mod_out = String::with_capacity(4096);
    writeln!(mod_out, "{header}")?;
    writeln!(mod_out)?;

    for cat_name in categories.keys() {
        writeln!(mod_out, "mod {cat_name};")?;
    }
    writeln!(mod_out)?;

    for cat_name in categories.keys() {
        writeln!(mod_out, "pub use {cat_name}::*;")?;
    }

    files.insert(format!("{kind}/mod.rs"), mod_out);

    // Generate each category file
    for (cat_name, ops) in categories {
        let mut out = String::with_capacity(16 * 1024);
        writeln!(out, "{header}")?;
        writeln!(out)?;

        // Collect which structs belong to this category (owned names for lifetime safety)
        let mut category_struct_names: Vec<String> = Vec::new();

        for op_name in ops {
            let op_info = resolved.operations.iter().find(|o| &o.name == op_name);

            if let Some(info) = op_info {
                let shape_id = if kind == "input" {
                    &info.input_shape
                } else {
                    &info.output_shape
                };

                if let Some(sid) = shape_id {
                    let short = crate::model::SmithyModel::short_name(sid);
                    let struct_name = if kind == "input" {
                        short
                            .strip_suffix("Request")
                            .map_or_else(|| short.to_owned(), |base| format!("{base}Input"))
                    } else {
                        short.to_owned()
                    };

                    if structs.contains_key(&struct_name) {
                        category_struct_names.push(struct_name);
                    }
                }
            }
        }

        // Deduplicate and sort
        category_struct_names.sort();
        category_struct_names.dedup();

        // Build references for type checking
        let category_structs: Vec<(&String, &Vec<FieldInfo>)> = category_struct_names
            .iter()
            .filter_map(|name| structs.get(name).map(|fields| (name, fields)))
            .collect();

        // Determine imports needed
        let needs_hashmap = category_structs
            .iter()
            .any(|(_, fields)| fields.iter().any(|f| f.rust_type.contains("HashMap")));
        let needs_streaming_blob = category_structs
            .iter()
            .any(|(_, fields)| fields.iter().any(|f| f.rust_type.contains("StreamingBlob")));
        let needs_types = category_needs_types(&category_structs, resolved);

        if needs_hashmap {
            writeln!(out, "use std::collections::HashMap;")?;
            writeln!(out)?;
        }

        if config.uses_serde() {
            writeln!(out, "use serde::{{Deserialize, Serialize}};")?;
            writeln!(out)?;
        }

        if needs_streaming_blob {
            writeln!(out, "use crate::request::StreamingBlob;")?;
            writeln!(out)?;
        }

        if !needs_types.is_empty() {
            let types_list = needs_types.join(", ");
            writeln!(out, "use crate::types::{{{types_list}}};")?;
            writeln!(out)?;
        }

        // Write structs
        for (name, fields) in &category_structs {
            write_struct(&mut out, name, fields, true, display_name, config)?;
        }

        files.insert(format!("{kind}/{cat_name}.rs"), out);
    }

    Ok(())
}

/// Generate a flat input.rs or output.rs file containing all structs.
fn generate_flat_io(
    files: &mut BTreeMap<String, String>,
    kind: &str, // "input" or "output"
    structs: &BTreeMap<String, Vec<FieldInfo>>,
    resolved: &ResolvedModel,
    config: &ServiceConfig,
) -> Result<()> {
    let header = file_header(config);
    let display_name = &config.display_name;

    let mut out = String::with_capacity(32 * 1024);
    writeln!(out, "{header}")?;
    writeln!(out)?;

    // Determine imports needed across all structs
    let all_struct_refs: Vec<(&String, &Vec<FieldInfo>)> = structs.iter().collect();

    let needs_hashmap = all_struct_refs
        .iter()
        .any(|(_, fields)| fields.iter().any(|f| f.rust_type.contains("HashMap")));
    let needs_streaming_blob = all_struct_refs
        .iter()
        .any(|(_, fields)| fields.iter().any(|f| f.rust_type.contains("StreamingBlob")));
    let needs_types = category_needs_types(&all_struct_refs, resolved);

    if needs_hashmap {
        writeln!(out, "use std::collections::HashMap;")?;
        writeln!(out)?;
    }

    if config.uses_serde() {
        writeln!(out, "use serde::{{Deserialize, Serialize}};")?;
        writeln!(out)?;
    }

    if needs_streaming_blob {
        writeln!(out, "use crate::request::StreamingBlob;")?;
        writeln!(out)?;
    }

    if !needs_types.is_empty() {
        let types_list = needs_types.join(", ");
        writeln!(out, "use crate::types::{{{types_list}}};")?;
        writeln!(out)?;
    }

    // Write all structs
    for (name, fields) in structs {
        write_struct(
            &mut out,
            name,
            fields,
            config.emit_http_bindings,
            display_name,
            config,
        )?;
    }

    files.insert(format!("{kind}.rs"), out);
    Ok(())
}

/// Determine which types from `types.rs` are referenced in a category's structs.
fn category_needs_types(
    structs: &[(&String, &Vec<FieldInfo>)],
    resolved: &ResolvedModel,
) -> Vec<String> {
    let mut needed = BTreeMap::new();

    for (_, fields) in structs {
        for field in *fields {
            collect_type_references(&field.rust_type, resolved, &mut needed);
        }
    }

    needed.into_keys().collect()
}

/// Collect all type references from a Rust type string that exist in types.rs.
fn collect_type_references(
    rust_type: &str,
    resolved: &ResolvedModel,
    refs: &mut BTreeMap<String, ()>,
) {
    // Extract type names from the type expression
    // Handle Option<T>, Vec<T>, HashMap<K, V>, and plain types
    let cleaned = rust_type
        .replace("Option<", " ")
        .replace("Vec<", " ")
        .replace("HashMap<", " ")
        .replace("Box<", " ")
        .replace(['>', ','], " ");

    for token in cleaned.split_whitespace() {
        // Skip primitive types and known external types
        if matches!(
            token,
            "String"
                | "bool"
                | "i32"
                | "i64"
                | "f32"
                | "f64"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "bytes::Bytes"
                | "chrono::DateTime<chrono::Utc"
                | "chrono::DateTime"
                | "chrono::Utc"
                | "serde_json::Value"
                | "StreamingBlob"
        ) {
            continue;
        }

        // Check if this type is an enum or shared struct
        if resolved.enums.contains_key(token) || resolved.shared_structs.contains_key(token) {
            refs.insert(token.to_owned(), ());
        }
    }
}

/// Generate operations.rs with the operation enum.
fn generate_operations(config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let prefix = &config.rust_prefix;
    let all_operations = &config.all_operations;

    let mut out = String::with_capacity(8192);
    writeln!(out, "{header}")?;
    writeln!(out)?;

    writeln!(out, "/// All supported {prefix} operations.")?;
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum {prefix}Operation {{")?;

    for op in all_operations {
        writeln!(out, "    /// The {op} operation.")?;
        writeln!(out, "    {op},")?;
    }

    writeln!(out, "}}")?;
    writeln!(out)?;

    // Generate as_str()
    writeln!(out, "impl {prefix}Operation {{")?;
    writeln!(out, "    /// Returns the AWS operation name string.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn as_str(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for op in all_operations {
        writeln!(out, "            Self::{op} => \"{op}\",")?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    // Generate from_str
    writeln!(
        out,
        "    /// Parse an operation name string into an {prefix}Operation."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn from_name(name: &str) -> Option<Self> {{")?;
    writeln!(out, "        match name {{")?;
    for op in all_operations {
        writeln!(out, "            \"{op}\" => Some(Self::{op}),")?;
    }
    writeln!(out, "            _ => None,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Display
    writeln!(out, "impl std::fmt::Display for {prefix}Operation {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )?;
    writeln!(out, "        f.write_str(self.as_str())")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;

    Ok(out)
}

/// Generate error.rs with error code enum and error struct.
fn generate_error(resolved: &ResolvedModel, config: &ServiceConfig) -> Result<String> {
    if config.emit_request_wrapper {
        return generate_error_s3(config);
    }
    generate_error_json(resolved, config)
}

/// Generate error.rs for JSON protocol services (SSM, EventBridge, etc.).
///
/// Uses the resolved error types from the Smithy model + custom errors from config.
fn generate_error_json(resolved: &ResolvedModel, config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let prefix = &config.rust_prefix;
    let name = &config.name;
    let display_name = &config.display_name;

    let mut out = String::with_capacity(16 * 1024);
    writeln!(out, "{header}")?;
    writeln!(out, "//!")?;
    writeln!(
        out,
        "//! {display_name} errors use JSON format with a `__type` field containing the"
    )?;
    writeln!(
        out,
        "//! short error type name (e.g., `ResourceNotFoundException`)."
    )?;
    writeln!(out)?;
    writeln!(out, "use std::fmt;")?;
    writeln!(out)?;

    let errors = &resolved.errors;

    // Generate error code enum
    writeln!(out, "/// Well-known {display_name} error codes.")?;
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]"
    )?;
    writeln!(out, "#[non_exhaustive]")?;
    writeln!(out, "pub enum {prefix}ErrorCode {{")?;

    // Find the default variant (ValidationException if present, else first).
    // If no errors exist, add an InternalException fallback so the enum is not empty.
    let default_name = errors
        .iter()
        .find(|e| e.name == "ValidationException")
        .map_or_else(
            || {
                errors
                    .first()
                    .map_or("InternalException", |e| e.name.as_str())
            },
            |e| e.name.as_str(),
        );

    if errors.is_empty() {
        writeln!(out, "    /// Internal error.")?;
        writeln!(out, "    #[default]")?;
        writeln!(out, "    InternalException,")?;
    }

    for err in errors {
        writeln!(out, "    /// {} error.", err.name)?;
        if err.name == default_name {
            writeln!(out, "    #[default]")?;
        }
        writeln!(out, "    {},", err.name)?;
    }
    writeln!(out, "}}")?;
    writeln!(out)?;

    // error_type() and as_str()
    writeln!(out, "impl {prefix}ErrorCode {{")?;
    writeln!(
        out,
        "    /// Returns the short error type string for the JSON `__type` field."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn error_type(&self) -> &'static str {{")?;
    writeln!(out, "        self.as_str()")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Returns the short error code string.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn as_str(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for err in errors {
        writeln!(
            out,
            "            Self::{} => \"{}\",",
            err.name, err.wire_name
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    // default_status_code()
    writeln!(
        out,
        "    /// Returns the default HTTP status code for this error."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn default_status_code(&self) -> http::StatusCode {{"
    )?;
    writeln!(out, "        match self {{")?;

    // Group by status code to produce clean match arms
    let mut status_groups: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    for err in errors {
        status_groups
            .entry(err.status_code)
            .or_default()
            .push(&err.name);
    }
    for (status, codes) in &status_groups {
        let status_const = status_code_const(*status);
        let patterns: Vec<String> = codes.iter().map(|c| format!("Self::{c}")).collect();
        let joined = patterns.join(" | ");
        writeln!(out, "            {joined} => {status_const},")?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Display impl
    writeln!(out, "impl fmt::Display for {prefix}ErrorCode {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{"
    )?;
    writeln!(out, "        f.write_str(self.as_str())")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error struct (no resource/request_id for JSON services)
    writeln!(out, "/// An {display_name} error response.")?;
    writeln!(out, "#[derive(Debug)]")?;
    writeln!(out, "pub struct {prefix}Error {{")?;
    writeln!(out, "    /// The error code.")?;
    writeln!(out, "    pub code: {prefix}ErrorCode,")?;
    writeln!(out, "    /// A human-readable error message.")?;
    writeln!(out, "    pub message: String,")?;
    writeln!(out, "    /// The HTTP status code.")?;
    writeln!(out, "    pub status_code: http::StatusCode,")?;
    writeln!(out, "    /// The underlying source error, if any.")?;
    writeln!(
        out,
        "    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,"
    )?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Display impl
    writeln!(out, "impl fmt::Display for {prefix}Error {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{"
    )?;
    writeln!(
        out,
        "        write!(f, \"{prefix}Error({{}}): {{}}\", self.code, self.message)"
    )?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // std::error::Error impl
    writeln!(out, "impl std::error::Error for {prefix}Error {{")?;
    writeln!(
        out,
        "    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {{"
    )?;
    writeln!(
        out,
        "        self.source.as_ref().map(|e| e.as_ref() as &(dyn std::error::Error + 'static))"
    )?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Constructors
    writeln!(out, "impl {prefix}Error {{")?;
    writeln!(
        out,
        "    /// Create a new `{prefix}Error` from an error code."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn new(code: {prefix}ErrorCode) -> Self {{")?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            status_code: code.default_status_code(),")?;
    writeln!(out, "            message: code.as_str().to_owned(),")?;
    writeln!(out, "            code,")?;
    writeln!(out, "            source: None,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(
        out,
        "    /// Create a new `{prefix}Error` with a custom message."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_message(code: {prefix}ErrorCode, message: impl Into<String>) -> Self {{"
    )?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            status_code: code.default_status_code(),")?;
    writeln!(out, "            message: message.into(),")?;
    writeln!(out, "            code,")?;
    writeln!(out, "            source: None,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(
        out,
        "    /// Returns the `__type` string for the JSON error response."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn error_type(&self) -> &'static str {{")?;
    writeln!(out, "        self.code.error_type()")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    // Convenience constructors for common errors
    // Generate validation(), internal_error(), missing_action(), unknown_operation(), not_implemented()
    // based on which errors are present

    let has_error = |name: &str| errors.iter().any(|e| e.name == name);

    // Find the "validation" error (ValidationException)
    if has_error("ValidationException") {
        writeln!(out, "    /// Validation error.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(
            out,
            "    pub fn validation(message: impl Into<String>) -> Self {{"
        )?;
        writeln!(
            out,
            "        Self::with_message({prefix}ErrorCode::ValidationException, message)"
        )?;
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    // Find the "internal" error
    let internal_variant = errors
        .iter()
        .find(|e| e.fault == "server")
        .map(|e| e.name.as_str());

    if let Some(internal_name) = internal_variant {
        writeln!(out, "    /// Internal error.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(
            out,
            "    pub fn internal_error(message: impl Into<String>) -> Self {{"
        )?;
        writeln!(
            out,
            "        Self::with_message({prefix}ErrorCode::{internal_name}, message)"
        )?;
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    if has_error("MissingAction") {
        writeln!(out, "    /// Missing action header.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(out, "    pub fn missing_action() -> Self {{")?;
        writeln!(out, "        Self::with_message(")?;
        writeln!(out, "            {prefix}ErrorCode::MissingAction,")?;
        writeln!(
            out,
            "            \"Missing required header: X-Amz-Target\","
        )?;
        writeln!(out, "        )")?;
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    if has_error("InvalidAction") {
        writeln!(out, "    /// Unknown operation.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(out, "    pub fn unknown_operation(target: &str) -> Self {{")?;
        writeln!(out, "        Self::with_message(")?;
        writeln!(out, "            {prefix}ErrorCode::InvalidAction,")?;
        writeln!(
            out,
            "            format!(\"Operation {{target}} is not supported.\"),"
        )?;
        writeln!(out, "        )")?;
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    if let Some(internal_name) = internal_variant {
        writeln!(out, "    /// Not implemented.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(
            out,
            "    pub fn not_implemented(operation: &str) -> Self {{"
        )?;
        writeln!(out, "        Self::with_message(")?;
        writeln!(out, "            {prefix}ErrorCode::{internal_name},")?;
        writeln!(
            out,
            "            format!(\"Operation {{operation}} is not yet implemented\"),"
        )?;
        writeln!(out, "        )")?;
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    writeln!(out, "}}")?;
    writeln!(out)?;

    // Generate {name}_error! macro
    writeln!(out, "/// Create an `{prefix}Error` from an error code.")?;
    writeln!(out, "///")?;
    writeln!(out, "/// # Examples")?;
    writeln!(out, "///")?;
    writeln!(out, "/// ```ignore")?;
    writeln!(out, "/// let err = {name}_error!({default_name});")?;
    writeln!(
        out,
        "/// assert_eq!(err.code, {prefix}ErrorCode::{default_name});"
    )?;
    writeln!(out, "/// ```")?;
    writeln!(out, "#[macro_export]")?;
    writeln!(out, "macro_rules! {name}_error {{")?;
    writeln!(out, "    ($code:ident) => {{")?;
    writeln!(
        out,
        "        $crate::error::{prefix}Error::new($crate::error::{prefix}ErrorCode::$code)"
    )?;
    writeln!(out, "    }};")?;
    writeln!(out, "    ($code:ident, $msg:expr) => {{")?;
    writeln!(
        out,
        "        $crate::error::{prefix}Error::with_message($crate::error::{prefix}ErrorCode::$code, $msg)"
    )?;
    writeln!(out, "    }};")?;
    writeln!(out, "}}")?;

    Ok(out)
}

/// Map an HTTP status code to a Rust `http::StatusCode` constant name.
fn status_code_const(status: u16) -> &'static str {
    match status {
        400 => "http::StatusCode::BAD_REQUEST",
        401 => "http::StatusCode::UNAUTHORIZED",
        403 => "http::StatusCode::FORBIDDEN",
        404 => "http::StatusCode::NOT_FOUND",
        405 => "http::StatusCode::METHOD_NOT_ALLOWED",
        409 => "http::StatusCode::CONFLICT",
        411 => "http::StatusCode::LENGTH_REQUIRED",
        412 => "http::StatusCode::PRECONDITION_FAILED",
        416 => "http::StatusCode::RANGE_NOT_SATISFIABLE",
        500 => "http::StatusCode::INTERNAL_SERVER_ERROR",
        501 => "http::StatusCode::NOT_IMPLEMENTED",
        503 => "http::StatusCode::SERVICE_UNAVAILABLE",
        _ => "http::StatusCode::INTERNAL_SERVER_ERROR",
    }
}

/// Generate error.rs with S3-specific hardcoded error codes.
///
/// Preserves exact byte-identical output for S3 codegen.
fn generate_error_s3(config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let prefix = &config.rust_prefix;

    let mut out = String::with_capacity(16 * 1024);
    writeln!(out, "{header}")?;
    writeln!(out)?;
    writeln!(out, "use std::fmt;")?;
    writeln!(out)?;

    let error_codes: &[(&str, &str, u16)] = &[
        ("AccessDenied", "Access Denied", 403),
        ("AccountProblem", "There is a problem with the account", 403),
        (
            "BucketAlreadyExists",
            "The requested bucket name is not available",
            409,
        ),
        (
            "BucketAlreadyOwnedByYou",
            "The bucket is already owned by you",
            409,
        ),
        (
            "BucketNotEmpty",
            "The bucket you tried to delete is not empty",
            409,
        ),
        (
            "EntityTooLarge",
            "Your proposed upload exceeds the maximum allowed size",
            400,
        ),
        (
            "EntityTooSmall",
            "Your proposed upload is smaller than the minimum allowed size",
            400,
        ),
        ("InternalError", "Internal server error", 500),
        ("InvalidArgument", "Invalid Argument", 400),
        (
            "InvalidBucketName",
            "The specified bucket is not valid",
            400,
        ),
        (
            "InvalidBucketState",
            "The request is not valid with the current state of the bucket",
            409,
        ),
        (
            "InvalidDigest",
            "The Content-MD5 you specified is not valid",
            400,
        ),
        (
            "InvalidLocationConstraint",
            "The specified location constraint is not valid",
            400,
        ),
        (
            "InvalidObjectState",
            "The operation is not valid for the current state of the object",
            403,
        ),
        (
            "InvalidPart",
            "One or more of the specified parts could not be found",
            400,
        ),
        (
            "InvalidPartOrder",
            "The list of parts was not in ascending order",
            400,
        ),
        (
            "InvalidRange",
            "The requested range cannot be satisfied",
            416,
        ),
        ("InvalidRequest", "Invalid Request", 400),
        (
            "InvalidStorageClass",
            "The storage class you specified is not valid",
            400,
        ),
        ("KeyTooLongError", "Your key is too long", 400),
        (
            "MalformedXML",
            "The XML you provided was not well-formed",
            400,
        ),
        (
            "MetadataTooLarge",
            "Your metadata headers exceed the maximum allowed metadata size",
            400,
        ),
        (
            "MethodNotAllowed",
            "The specified method is not allowed against this resource",
            405,
        ),
        (
            "MissingContentLength",
            "You must provide the Content-Length HTTP header",
            411,
        ),
        ("NoSuchBucket", "The specified bucket does not exist", 404),
        (
            "NoSuchBucketPolicy",
            "The specified bucket does not have a bucket policy",
            404,
        ),
        (
            "NoSuchCORSConfiguration",
            "The CORS configuration does not exist",
            404,
        ),
        ("NoSuchKey", "The specified key does not exist", 404),
        (
            "NoSuchLifecycleConfiguration",
            "The lifecycle configuration does not exist",
            404,
        ),
        (
            "NoSuchUpload",
            "The specified multipart upload does not exist",
            404,
        ),
        ("NoSuchVersion", "The specified version does not exist", 404),
        ("NoSuchTagSet", "The TagSet does not exist", 404),
        (
            "NoSuchWebsiteConfiguration",
            "The website configuration does not exist",
            404,
        ),
        (
            "NotImplemented",
            "The functionality is not implemented",
            501,
        ),
        (
            "ObjectNotInActiveTierError",
            "The source object of the COPY operation is not in the active tier",
            403,
        ),
        (
            "PreconditionFailed",
            "At least one of the preconditions you specified did not hold",
            412,
        ),
        (
            "SignatureDoesNotMatch",
            "The request signature does not match",
            403,
        ),
        (
            "TooManyBuckets",
            "You have attempted to create more buckets than allowed",
            400,
        ),
        (
            "XAmzContentSHA256Mismatch",
            "The provided x-amz-content-sha256 header does not match",
            400,
        ),
    ];

    // Generate error code enum
    writeln!(out, "/// Well-known {prefix} error codes.")?;
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]"
    )?;
    writeln!(out, "#[non_exhaustive]")?;
    writeln!(out, "pub enum {prefix}ErrorCode {{")?;
    for (i, (code, _msg, _status)) in error_codes.iter().enumerate() {
        if i == 0 {
            writeln!(out, "    /// Default error code.")?;
            writeln!(out, "    #[default]")?;
        }
        writeln!(out, "    /// {code} error.")?;
        writeln!(out, "    {code},")?;
    }
    writeln!(out, "    /// A custom error code not in the standard set.")?;
    writeln!(out, "    Custom(&'static str),")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error code as_str
    writeln!(out, "impl {prefix}ErrorCode {{")?;
    writeln!(out, "    /// Returns the error code as a string.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn as_str(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for (code, _msg, _status) in error_codes {
        writeln!(out, "            Self::{code} => \"{code}\",")?;
    }
    writeln!(out, "            Self::Custom(s) => s,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(
        out,
        "    /// Returns the default HTTP status code for this error."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    #[allow(clippy::match_same_arms)]")?;
    writeln!(
        out,
        "    pub fn default_status_code(&self) -> http::StatusCode {{"
    )?;
    writeln!(out, "        match self {{")?;
    // Group error codes by status code to avoid match_same_arms lint
    let mut status_groups: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    for (code, _msg, status) in error_codes {
        status_groups.entry(*status).or_default().push(code);
    }
    for (status, codes) in &status_groups {
        let status_const = match status {
            400 => "http::StatusCode::BAD_REQUEST",
            403 => "http::StatusCode::FORBIDDEN",
            404 => "http::StatusCode::NOT_FOUND",
            405 => "http::StatusCode::METHOD_NOT_ALLOWED",
            409 => "http::StatusCode::CONFLICT",
            411 => "http::StatusCode::LENGTH_REQUIRED",
            412 => "http::StatusCode::PRECONDITION_FAILED",
            416 => "http::StatusCode::RANGE_NOT_SATISFIABLE",
            500 => "http::StatusCode::INTERNAL_SERVER_ERROR",
            501 => "http::StatusCode::NOT_IMPLEMENTED",
            _ => "http::StatusCode::INTERNAL_SERVER_ERROR",
        };
        let patterns: Vec<String> = codes.iter().map(|c| format!("Self::{c}")).collect();
        let joined = patterns.join(" | ");
        writeln!(out, "            {joined} => {status_const},")?;
    }
    writeln!(
        out,
        "            Self::Custom(_) => http::StatusCode::INTERNAL_SERVER_ERROR,"
    )?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Returns the default message for this error.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn default_message(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for (code, msg, _status) in error_codes {
        writeln!(out, "            Self::{code} => \"{msg}\",")?;
    }
    writeln!(out, "            Self::Custom(s) => s,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error code Display
    writeln!(out, "impl fmt::Display for {prefix}ErrorCode {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{"
    )?;
    writeln!(out, "        f.write_str(self.as_str())")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error struct
    writeln!(out, "/// An {prefix} error response.")?;
    writeln!(out, "#[derive(Debug)]")?;
    writeln!(out, "pub struct {prefix}Error {{")?;
    writeln!(out, "    /// The error code.")?;
    writeln!(out, "    pub code: {prefix}ErrorCode,")?;
    writeln!(out, "    /// A human-readable error message.")?;
    writeln!(out, "    pub message: String,")?;
    writeln!(out, "    /// The resource that caused the error.")?;
    writeln!(out, "    pub resource: Option<String>,")?;
    writeln!(out, "    /// The request ID.")?;
    writeln!(out, "    pub request_id: Option<String>,")?;
    writeln!(out, "    /// The HTTP status code.")?;
    writeln!(out, "    pub status_code: http::StatusCode,")?;
    writeln!(out, "    /// The underlying source error, if any.")?;
    writeln!(
        out,
        "    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,"
    )?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error Display
    writeln!(out, "impl fmt::Display for {prefix}Error {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{"
    )?;
    writeln!(
        out,
        "        write!(f, \"{prefix}Error({{}}): {{}}\", self.code, self.message)"
    )?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error std::error::Error
    writeln!(out, "impl std::error::Error for {prefix}Error {{")?;
    writeln!(
        out,
        "    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {{"
    )?;
    writeln!(
        out,
        "        self.source.as_ref().map(|e| e.as_ref() as &(dyn std::error::Error + 'static))"
    )?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Error constructors
    writeln!(out, "impl {prefix}Error {{")?;
    writeln!(
        out,
        "    /// Create a new {prefix}Error from an error code."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn new(code: {prefix}ErrorCode) -> Self {{")?;
    writeln!(out, "        let status_code = code.default_status_code();")?;
    writeln!(
        out,
        "        let message = code.default_message().to_owned();"
    )?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            code,")?;
    writeln!(out, "            message,")?;
    writeln!(out, "            resource: None,")?;
    writeln!(out, "            request_id: None,")?;
    writeln!(out, "            status_code,")?;
    writeln!(out, "            source: None,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(
        out,
        "    /// Create a new {prefix}Error with a custom message."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_message(code: {prefix}ErrorCode, message: impl Into<String>) -> Self {{"
    )?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            status_code: code.default_status_code(),")?;
    writeln!(out, "            message: message.into(),")?;
    writeln!(out, "            code,")?;
    writeln!(out, "            resource: None,")?;
    writeln!(out, "            request_id: None,")?;
    writeln!(out, "            source: None,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Set the resource that caused this error.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {{"
    )?;
    writeln!(out, "        self.resource = Some(resource.into());")?;
    writeln!(out, "        self")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Set the request ID.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {{"
    )?;
    writeln!(out, "        self.request_id = Some(request_id.into());")?;
    writeln!(out, "        self")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Set the source error.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {{"
    )?;
    writeln!(out, "        self.source = Some(Box::new(source));")?;
    writeln!(out, "        self")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    // Convenience constructors
    let convenience: &[(&str, &str, &str)] = &[
        ("no_such_bucket", "NoSuchBucket", "bucket_name"),
        ("no_such_key", "NoSuchKey", "key"),
        ("no_such_upload", "NoSuchUpload", "upload_id"),
        ("no_such_version", "NoSuchVersion", "version_id"),
        (
            "bucket_already_exists",
            "BucketAlreadyExists",
            "bucket_name",
        ),
        (
            "bucket_already_owned_by_you",
            "BucketAlreadyOwnedByYou",
            "bucket_name",
        ),
        ("bucket_not_empty", "BucketNotEmpty", "bucket_name"),
        ("access_denied", "AccessDenied", "resource"),
        ("internal_error", "InternalError", "message"),
        ("invalid_argument", "InvalidArgument", "message"),
        ("invalid_bucket_name", "InvalidBucketName", "bucket_name"),
        ("invalid_range", "InvalidRange", "range"),
        ("invalid_part", "InvalidPart", "part_info"),
        ("invalid_part_order", "InvalidPartOrder", "detail"),
        ("malformed_xml", "MalformedXML", "detail"),
        ("method_not_allowed", "MethodNotAllowed", "method"),
        ("not_implemented", "NotImplemented", "detail"),
        ("precondition_failed", "PreconditionFailed", "condition"),
        (
            "signature_does_not_match",
            "SignatureDoesNotMatch",
            "detail",
        ),
    ];

    for (fn_name, code, param) in convenience {
        writeln!(out, "    /// Create a {code} error.")?;
        writeln!(out, "    #[must_use]")?;
        writeln!(
            out,
            "    pub fn {fn_name}({param}: impl Into<String>) -> Self {{"
        )?;
        if *param == "message" {
            writeln!(
                out,
                "        Self::with_message({prefix}ErrorCode::{code}, {param})"
            )?;
        } else {
            writeln!(
                out,
                "        Self::new({prefix}ErrorCode::{code}).with_resource({param})"
            )?;
        }
        writeln!(out, "    }}")?;
        writeln!(out)?;
    }

    writeln!(out, "}}")?;
    writeln!(out)?;

    // s3_error! macro
    writeln!(out, "/// Create an {prefix}Error from an error code.")?;
    writeln!(out, "///")?;
    writeln!(out, "/// # Examples")?;
    writeln!(out, "///")?;
    writeln!(out, "/// ```")?;
    writeln!(out, "/// use ruststack_s3_model::s3_error;")?;
    writeln!(out, "/// use ruststack_s3_model::error::S3ErrorCode;")?;
    writeln!(out, "///")?;
    writeln!(out, "/// let err = s3_error!(NoSuchBucket);")?;
    writeln!(out, "/// assert_eq!(err.code, S3ErrorCode::NoSuchBucket);")?;
    writeln!(out, "///")?;
    writeln!(
        out,
        "/// let err = s3_error!(NoSuchKey, \"The key does not exist\");"
    )?;
    writeln!(
        out,
        "/// assert_eq!(err.message, \"The key does not exist\");"
    )?;
    writeln!(out, "/// ```")?;
    writeln!(out, "#[macro_export]")?;
    writeln!(out, "macro_rules! s3_error {{")?;
    writeln!(out, "    ($code:ident) => {{")?;
    writeln!(
        out,
        "        $crate::error::S3Error::new($crate::error::S3ErrorCode::$code)"
    )?;
    writeln!(out, "    }};")?;
    writeln!(out, "    ($code:ident, $msg:expr) => {{")?;
    writeln!(
        out,
        "        $crate::error::S3Error::with_message($crate::error::S3ErrorCode::$code, $msg)"
    )?;
    writeln!(out, "    }};")?;
    writeln!(out, "}}")?;

    Ok(out)
}

/// Generate request.rs with request wrapper, StreamingBlob, and Credentials.
fn generate_request(config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let prefix = &config.rust_prefix;

    let mut out = String::with_capacity(4096);
    writeln!(out, "{header}")?;
    writeln!(out)?;

    // StreamingBlob
    writeln!(
        out,
        "/// A wrapper around `bytes::Bytes` for streaming blob data."
    )?;
    writeln!(out, "///")?;
    writeln!(
        out,
        "/// In the future, this may be replaced with an actual streaming type."
    )?;
    writeln!(out, "#[derive(Debug, Clone, Default)]")?;
    writeln!(out, "pub struct StreamingBlob {{")?;
    writeln!(out, "    /// The underlying bytes data.")?;
    writeln!(out, "    pub data: bytes::Bytes,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl StreamingBlob {{")?;
    writeln!(out, "    /// Create a new `StreamingBlob` from bytes.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn new(data: impl Into<bytes::Bytes>) -> Self {{"
    )?;
    writeln!(out, "        Self {{ data: data.into() }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;
    writeln!(out, "    /// Returns true if the blob is empty.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn is_empty(&self) -> bool {{")?;
    writeln!(out, "        self.data.is_empty()")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;
    writeln!(out, "    /// Returns the length of the blob.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn len(&self) -> usize {{")?;
    writeln!(out, "        self.data.len()")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl From<bytes::Bytes> for StreamingBlob {{")?;
    writeln!(out, "    fn from(data: bytes::Bytes) -> Self {{")?;
    writeln!(out, "        Self {{ data }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl From<Vec<u8>> for StreamingBlob {{")?;
    writeln!(out, "    fn from(data: Vec<u8>) -> Self {{")?;
    writeln!(out, "        Self {{ data: data.into() }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl From<&[u8]> for StreamingBlob {{")?;
    writeln!(out, "    fn from(data: &[u8]) -> Self {{")?;
    writeln!(out, "        Self {{")?;
    writeln!(
        out,
        "            data: bytes::Bytes::copy_from_slice(data),"
    )?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Credentials
    writeln!(out, "/// AWS credentials for request authentication.")?;
    writeln!(out, "#[derive(Clone, Default)]")?;
    writeln!(out, "pub struct Credentials {{")?;
    writeln!(out, "    /// The AWS access key ID.")?;
    writeln!(out, "    pub access_key_id: String,")?;
    writeln!(out, "    /// The AWS secret access key.")?;
    writeln!(out, "    pub secret_access_key: String,")?;
    writeln!(
        out,
        "    /// Optional session token for temporary credentials."
    )?;
    writeln!(out, "    pub session_token: Option<String>,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // Manual Debug impl for Credentials to avoid leaking secrets
    writeln!(out, "impl std::fmt::Debug for Credentials {{")?;
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )?;
    writeln!(out, "        f.debug_struct(\"Credentials\")")?;
    writeln!(
        out,
        "            .field(\"access_key_id\", &self.access_key_id)"
    )?;
    writeln!(
        out,
        "            .field(\"secret_access_key\", &\"[REDACTED]\")"
    )?;
    writeln!(
        out,
        "            .field(\"session_token\", &self.session_token.as_ref().map(|_| \"[REDACTED]\"))"
    )?;
    writeln!(out, "            .finish()")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    // S3Request
    writeln!(
        out,
        "/// An {prefix} request wrapping an input type with credentials and headers."
    )?;
    writeln!(out, "#[derive(Debug, Clone)]")?;
    writeln!(out, "pub struct {prefix}Request<T> {{")?;
    writeln!(out, "    /// The input payload.")?;
    writeln!(out, "    pub input: T,")?;
    writeln!(out, "    /// Optional credentials for the request.")?;
    writeln!(out, "    pub credentials: Option<Credentials>,")?;
    writeln!(out, "    /// Additional HTTP headers.")?;
    writeln!(out, "    pub headers: http::HeaderMap,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl<T: Default> Default for {prefix}Request<T> {{")?;
    writeln!(out, "    fn default() -> Self {{")?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            input: T::default(),")?;
    writeln!(out, "            credentials: None,")?;
    writeln!(out, "            headers: http::HeaderMap::new(),")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;

    writeln!(out, "impl<T> {prefix}Request<T> {{")?;
    writeln!(
        out,
        "    /// Create a new {prefix}Request with the given input."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(out, "    pub fn new(input: T) -> Self {{")?;
    writeln!(out, "        Self {{")?;
    writeln!(out, "            input,")?;
    writeln!(out, "            credentials: None,")?;
    writeln!(out, "            headers: http::HeaderMap::new(),")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Set the credentials for this request.")?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn with_credentials(mut self, credentials: Credentials) -> Self {{"
    )?;
    writeln!(out, "        self.credentials = Some(credentials);")?;
    writeln!(out, "        self")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;

    writeln!(out, "    /// Map the input type to a different type.")?;
    writeln!(
        out,
        "    pub fn map_input<U>(self, f: impl FnOnce(T) -> U) -> {prefix}Request<U> {{"
    )?;
    writeln!(out, "        {prefix}Request {{")?;
    writeln!(out, "            input: f(self.input),")?;
    writeln!(out, "            credentials: self.credentials,")?;
    writeln!(out, "            headers: self.headers,")?;
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;

    Ok(out)
}

/// Generate lib.rs that re-exports all modules.
fn generate_lib(resolved: &ResolvedModel, config: &ServiceConfig) -> Result<String> {
    let header = file_header(config);
    let prefix = &config.rust_prefix;

    let mut out = String::with_capacity(2048);
    writeln!(out, "{header}")?;
    writeln!(out, "#![allow(clippy::too_many_lines)]")?;
    writeln!(out, "#![allow(clippy::struct_excessive_bools)]")?;
    writeln!(out, "#![allow(missing_docs)]")?;
    writeln!(out)?;

    writeln!(out, "pub mod error;")?;
    writeln!(out, "pub mod input;")?;
    writeln!(out, "pub mod operations;")?;
    writeln!(out, "pub mod output;")?;
    if config.emit_request_wrapper {
        writeln!(out, "pub mod request;")?;
    }
    writeln!(out, "pub mod types;")?;
    writeln!(out)?;

    // Re-exports
    writeln!(out, "pub use error::{{{prefix}Error, {prefix}ErrorCode}};")?;
    writeln!(out, "pub use operations::{prefix}Operation;")?;
    if config.emit_request_wrapper {
        writeln!(
            out,
            "pub use request::{{Credentials, {prefix}Request, StreamingBlob}};"
        )?;
    }
    writeln!(out)?;

    // Count generated items
    let n_enums = resolved.enums.len();
    let n_shared = resolved.shared_structs.len();
    let n_inputs = resolved.input_structs.len();
    let n_outputs = resolved.output_structs.len();
    let n_ops = resolved.operations.len();

    writeln!(
        out,
        "// Generated: {n_ops} operations, {n_enums} enums, {n_shared} shared structs, {n_inputs} input structs, {n_outputs} output structs"
    )?;

    Ok(out)
}
