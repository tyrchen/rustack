//! Shape resolution and type mapping from Smithy shapes to Rust types.
//!
//! This module resolves Smithy shape references into concrete Rust type names
//! and collects the transitive closure of shapes needed by the target operations.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use crate::config::ServiceConfig;
use crate::model::{MemberShape, Shape, SmithyModel, StructureShape};

/// Maximum recursion depth when resolving shape references.
const MAX_RESOLVE_DEPTH: usize = 15;

/// Categorized operation info for code generation.
#[derive(Debug)]
pub struct OperationInfo {
    /// Short operation name (e.g., "PutObject").
    pub name: String,
    /// Fully qualified input shape ID.
    pub input_shape: Option<String>,
    /// Fully qualified output shape ID.
    pub output_shape: Option<String>,
}

/// Mapping from operation category name to a list of operation names.
pub type OperationCategories = BTreeMap<String, Vec<String>>;

/// Information about a struct member field for code generation.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Original Smithy member name (PascalCase).
    pub smithy_name: String,
    /// Rust field name (snake_case).
    pub rust_name: String,
    /// Rust type expression (e.g., `Option<String>`, `Vec<Tag>`).
    pub rust_type: String,
    /// Whether this field is required.
    pub required: bool,
    /// HTTP binding annotation if any.
    pub http_binding: Option<HttpBinding>,
}

/// HTTP binding information for a struct member.
#[derive(Debug, Clone)]
pub enum HttpBinding {
    /// Field comes from a URI path label.
    Label,
    /// Field comes from a query string parameter.
    Query(String),
    /// Field comes from an HTTP header.
    Header(String),
    /// Field is the HTTP payload body.
    Payload,
    /// Field comes from headers with a prefix.
    PrefixHeaders(String),
}

/// Enum variant information for code generation.
#[derive(Debug, Clone)]
pub struct EnumVariantInfo {
    /// Rust variant name (PascalCase).
    pub rust_name: String,
    /// The string value from `@enumValue`.
    pub string_value: String,
}

/// Information about a service error type extracted from the Smithy model.
#[derive(Debug, Clone)]
pub struct ErrorInfo {
    /// The Rust variant name (e.g., `ParameterNotFound`).
    pub name: String,
    /// The wire name for serialization (e.g., `ParameterNotFound` or `ResourceNotFoundException`).
    pub wire_name: String,
    /// Default HTTP status code (from `smithy.api#httpError`, or 400/500 based on fault).
    pub status_code: u16,
    /// Default human-readable message.
    pub message: String,
    /// Fault type: `"client"` or `"server"`.
    pub fault: String,
}

/// Resolved information about all shapes needed for code generation.
#[derive(Debug)]
pub struct ResolvedModel {
    /// All operations we are generating.
    pub operations: Vec<OperationInfo>,
    /// Operation categories for file organization.
    pub input_categories: OperationCategories,
    pub output_categories: OperationCategories,
    /// All enum shapes needed (short name -> variants).
    pub enums: BTreeMap<String, Vec<EnumVariantInfo>>,
    /// All shared struct shapes (short name -> fields).
    pub shared_structs: BTreeMap<String, Vec<FieldInfo>>,
    /// Input struct shapes (short name -> fields).
    pub input_structs: BTreeMap<String, Vec<FieldInfo>>,
    /// Output struct shapes (short name -> fields).
    pub output_structs: BTreeMap<String, Vec<FieldInfo>>,
    /// Error types referenced by the generated operations.
    pub errors: Vec<ErrorInfo>,
}

/// Build operation categories from the service config.
fn build_categories(config: &ServiceConfig) -> (OperationCategories, OperationCategories) {
    if let Some(ref cats) = config.categories {
        // Use config-defined categories
        let mut input_cats = BTreeMap::new();
        let mut output_cats = BTreeMap::new();
        for (cat, ops) in cats {
            input_cats.insert(cat.clone(), ops.clone());
            output_cats.insert(cat.clone(), ops.clone());
        }
        (input_cats, output_cats)
    } else {
        // Flat layout: all operations in a single "all" category
        let mut input_cats = BTreeMap::new();
        let mut output_cats = BTreeMap::new();
        input_cats.insert("all".to_owned(), config.all_operations.clone());
        output_cats.insert("all".to_owned(), config.all_operations.clone());
        (input_cats, output_cats)
    }
}

/// Resolve a Smithy shape target to a Rust type string.
fn resolve_rust_type(model: &SmithyModel, target: &str, needed_enums: &BTreeSet<String>) -> String {
    // Handle smithy.api primitive types
    if let Some(builtin) = resolve_builtin_type(target) {
        return builtin.to_owned();
    }

    let shape = match model.shapes.get(target) {
        Some(s) => s,
        None => return "String".to_owned(),
    };

    let short = SmithyModel::short_name(target);

    match shape {
        Shape::String(_)
        | Shape::Boolean(_)
        | Shape::Integer(_)
        | Shape::Long(_)
        | Shape::Timestamp(_)
        | Shape::Blob(_)
        | Shape::Double(_)
        | Shape::Float(_) => resolve_simple_rust_type(shape, target),
        Shape::Enum(_) => {
            if needed_enums.contains(short) {
                short.to_owned()
            } else {
                "String".to_owned()
            }
        }
        Shape::List(list_shape) => {
            let inner = resolve_rust_type(model, &list_shape.member.target, needed_enums);
            format!("Vec<{inner}>")
        }
        Shape::Map(map_shape) => {
            let key = resolve_rust_type(model, &map_shape.key.target, needed_enums);
            let val = resolve_rust_type(model, &map_shape.value.target, needed_enums);
            format!("HashMap<{key}, {val}>")
        }
        Shape::Structure(_) => short.to_owned(),
        Shape::Union(_) => short.to_owned(),
        _ => "String".to_owned(),
    }
}

/// Resolve built-in smithy.api types.
fn resolve_builtin_type(target: &str) -> Option<&'static str> {
    match target {
        "smithy.api#String" => Some("String"),
        "smithy.api#Boolean" | "smithy.api#PrimitiveBoolean" => Some("bool"),
        "smithy.api#Integer" | "smithy.api#PrimitiveInteger" => Some("i32"),
        "smithy.api#Long" | "smithy.api#PrimitiveLong" => Some("i64"),
        "smithy.api#Float" | "smithy.api#PrimitiveFloat" => Some("f32"),
        "smithy.api#Double" | "smithy.api#PrimitiveDouble" => Some("f64"),
        "smithy.api#Blob" => Some("bytes::Bytes"),
        "smithy.api#Timestamp" => Some("chrono::DateTime<chrono::Utc>"),
        "smithy.api#Unit" => Some("()"),
        "smithy.api#Document" => Some("serde_json::Value"),
        _ => None,
    }
}

/// Resolve simple type shapes (string aliases, boolean, integer, etc.) to Rust types.
fn resolve_simple_rust_type(shape: &Shape, target: &str) -> String {
    match shape {
        Shape::String(s) => {
            if s.traits.contains_key("smithy.api#streaming") {
                "bytes::Bytes".to_owned()
            } else {
                "String".to_owned()
            }
        }
        Shape::Boolean(_) => "bool".to_owned(),
        Shape::Integer(_) => "i32".to_owned(),
        Shape::Long(_) => "i64".to_owned(),
        Shape::Timestamp(_) => "chrono::DateTime<chrono::Utc>".to_owned(),
        Shape::Blob(b) => {
            if b.traits.contains_key("smithy.api#streaming") {
                "StreamingBlob".to_owned()
            } else {
                "bytes::Bytes".to_owned()
            }
        }
        Shape::Double(_) => "f64".to_owned(),
        Shape::Float(_) => "f32".to_owned(),
        _ => {
            let short = SmithyModel::short_name(target);
            short.to_owned()
        }
    }
}

/// Extract HTTP binding info from a member's traits.
fn extract_http_binding(member: &MemberShape) -> Option<HttpBinding> {
    if member.traits.contains_key("smithy.api#httpLabel") {
        return Some(HttpBinding::Label);
    }
    if let Some(val) = member.traits.get("smithy.api#httpQuery")
        && let Some(s) = val.as_str()
    {
        return Some(HttpBinding::Query(s.to_owned()));
    }
    if let Some(val) = member.traits.get("smithy.api#httpHeader")
        && let Some(s) = val.as_str()
    {
        return Some(HttpBinding::Header(s.to_owned()));
    }
    if member.traits.contains_key("smithy.api#httpPayload") {
        return Some(HttpBinding::Payload);
    }
    if let Some(val) = member.traits.get("smithy.api#httpPrefixHeaders")
        && let Some(s) = val.as_str()
    {
        return Some(HttpBinding::PrefixHeaders(s.to_owned()));
    }
    None
}

/// Convert a Smithy PascalCase member name to Rust snake_case.
fn to_snake_case(name: &str) -> String {
    use heck::ToSnakeCase;
    let snake = name.to_snake_case();
    // Handle Rust keywords
    match snake.as_str() {
        "type" => "r#type".to_owned(),
        "match" => "r#match".to_owned(),
        "return" => "r#return".to_owned(),
        "use" => "r#use".to_owned(),
        _ => snake,
    }
}

/// Convert a Smithy enum variant name to Rust PascalCase.
///
/// Handles SCREAMING_SNAKE_CASE, camelCase, and already-PascalCase names.
fn to_pascal_case(name: &str) -> String {
    use heck::ToPascalCase;
    name.to_pascal_case()
}

/// Collect all shape IDs transitively referenced from a given shape.
fn collect_referenced_shapes(
    model: &SmithyModel,
    shape_id: &str,
    visited: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth > MAX_RESOLVE_DEPTH
        || visited.contains(shape_id)
        || shape_id.starts_with("smithy.api#")
    {
        return;
    }
    visited.insert(shape_id.to_owned());

    let Some(shape) = model.shapes.get(shape_id) else {
        return;
    };

    match shape {
        Shape::Structure(s) => {
            for member in s.members.values() {
                collect_referenced_shapes(model, &member.target, visited, depth + 1);
            }
        }
        Shape::List(l) => {
            collect_referenced_shapes(model, &l.member.target, visited, depth + 1);
        }
        Shape::Map(m) => {
            collect_referenced_shapes(model, &m.key.target, visited, depth + 1);
            collect_referenced_shapes(model, &m.value.target, visited, depth + 1);
        }
        Shape::Union(u) => {
            for member in u.members.values() {
                collect_referenced_shapes(model, &member.target, visited, depth + 1);
            }
        }
        _ => {}
    }
}

/// Resolve fields for a structure shape.
fn resolve_struct_fields(
    model: &SmithyModel,
    structure: &StructureShape,
    needed_enums: &BTreeSet<String>,
) -> Vec<FieldInfo> {
    // Collect members into a BTreeMap for stable ordering
    let sorted_members: BTreeMap<&String, &MemberShape> = structure.members.iter().collect();

    sorted_members
        .into_iter()
        .map(|(name, member)| {
            let required = member.traits.contains_key("smithy.api#required");
            let base_type = resolve_rust_type(model, &member.target, needed_enums);
            let http_binding = extract_http_binding(member);

            // Determine if this should be wrapped in Option
            let rust_type = if required || is_collection_type(&base_type) {
                base_type
            } else {
                format!("Option<{base_type}>")
            };

            FieldInfo {
                smithy_name: name.clone(),
                rust_name: to_snake_case(name),
                rust_type,
                required,
                http_binding,
            }
        })
        .collect()
}

/// Check if a type is a collection type that should not be wrapped in Option.
fn is_collection_type(ty: &str) -> bool {
    ty.starts_with("Vec<") || ty.starts_with("HashMap<")
}

/// Resolve all needed shapes from the Smithy model.
pub fn resolve_model(model: &SmithyModel, config: &ServiceConfig) -> Result<ResolvedModel> {
    let (input_categories, output_categories) = build_categories(config);
    let namespace_prefix = config.namespace_prefix();

    // Step 1: Collect all operation info and referenced shapes.
    let mut operations = Vec::new();
    let mut all_referenced = BTreeSet::new();
    let mut input_shape_ids = BTreeSet::new();
    let mut output_shape_ids = BTreeSet::new();
    let mut error_shape_ids = BTreeSet::new();

    for op_name in &config.all_operations {
        let full_name = format!("{namespace_prefix}{op_name}");
        let shape = model
            .shapes
            .get(&full_name)
            .with_context(|| format!("Operation {op_name} not found in model"))?;

        let (input_target, output_target, op_errors) = match shape {
            Shape::Operation(op) => (
                op.input.as_ref().map(|r| r.target.clone()),
                op.output.as_ref().map(|r| r.target.clone()),
                &op.errors,
            ),
            _ => anyhow::bail!("{op_name} is not an operation shape"),
        };

        if let Some(ref inp) = input_target
            && inp != "smithy.api#Unit"
        {
            collect_referenced_shapes(model, inp, &mut all_referenced, 0);
            input_shape_ids.insert(inp.clone());
        }
        if let Some(ref out) = output_target
            && out != "smithy.api#Unit"
        {
            collect_referenced_shapes(model, out, &mut all_referenced, 0);
            output_shape_ids.insert(out.clone());
        }

        for err_ref in op_errors {
            error_shape_ids.insert(err_ref.target.clone());
        }

        operations.push(OperationInfo {
            name: op_name.clone(),
            input_shape: input_target.filter(|t| t != "smithy.api#Unit"),
            output_shape: output_target.filter(|t| t != "smithy.api#Unit"),
        });
    }

    // Step 2: Categorize referenced shapes into enums, shared structs, inputs, outputs.
    let mut enums = BTreeMap::new();
    let mut needed_enum_names = BTreeSet::new();

    // First pass: collect enum names.
    for shape_id in &all_referenced {
        if let Some(Shape::Enum(_)) = model.shapes.get(shape_id.as_str()) {
            let short = SmithyModel::short_name(shape_id);
            needed_enum_names.insert(short.to_owned());
        }
    }

    // Resolve enum variants.
    for shape_id in &all_referenced {
        if let Some(Shape::Enum(enum_shape)) = model.shapes.get(shape_id.as_str()) {
            let short = SmithyModel::short_name(shape_id).to_owned();
            let mut variants: Vec<EnumVariantInfo> = enum_shape
                .members
                .iter()
                .map(|(variant_name, member)| {
                    let string_value = member
                        .traits
                        .get("smithy.api#enumValue")
                        .and_then(|v| v.as_str())
                        .unwrap_or(variant_name.as_str())
                        .to_owned();
                    EnumVariantInfo {
                        rust_name: to_pascal_case(variant_name),
                        string_value,
                    }
                })
                .collect();
            // Sort variants for stable output
            variants.sort_by(|a, b| a.rust_name.cmp(&b.rust_name));
            enums.insert(short, variants);
        }
    }

    // Step 3: Collect all struct types referenced by input shapes (transitively).
    // This helps identify shapes that are used by both input and output and should be shared.
    let mut input_referenced = BTreeSet::new();
    for inp_id in &input_shape_ids {
        collect_referenced_shapes(model, inp_id, &mut input_referenced, 0);
    }
    let mut output_referenced = BTreeSet::new();
    for out_id in &output_shape_ids {
        collect_referenced_shapes(model, out_id, &mut output_referenced, 0);
    }

    // Step 4: Resolve structs - separate into input, output, and shared.
    // A shape is shared if it is referenced by both input and output shapes (excluding direct
    // input/output shapes themselves), or if it is an output shape but also referenced by inputs.
    let mut input_structs = BTreeMap::new();
    let mut output_structs = BTreeMap::new();
    let mut shared_structs = BTreeMap::new();

    for shape_id in &all_referenced {
        let Some(Shape::Structure(structure)) = model.shapes.get(shape_id.as_str()) else {
            continue;
        };

        // Skip error shapes
        if structure.traits.contains_key("smithy.api#error") {
            continue;
        }

        let short = SmithyModel::short_name(shape_id).to_owned();
        let fields = resolve_struct_fields(model, structure, &needed_enum_names);

        let is_input = input_shape_ids.contains(shape_id);
        let is_output = output_shape_ids.contains(shape_id);
        let in_input_tree = input_referenced.contains(shape_id);
        let in_output_tree = output_referenced.contains(shape_id);

        if is_input && is_output {
            // Both input and output - shared (unlikely but handle it)
            shared_structs.insert(short, fields);
        } else if is_input {
            let input_name = short
                .strip_suffix("Request")
                .map_or_else(|| short.clone(), |base| format!("{base}Input"));
            input_structs.insert(input_name, fields);
        } else if is_output {
            if in_input_tree {
                // Output shape also referenced by input shapes -> shared
                shared_structs.insert(short, fields);
            } else {
                output_structs.insert(short, fields);
            }
        } else if in_input_tree && in_output_tree {
            // Referenced by both input and output - shared
            shared_structs.insert(short, fields);
        } else {
            shared_structs.insert(short, fields);
        }
    }

    // Also resolve union types as shared structs (as enums with data).
    // For now, treat unions as structs with optional fields.
    for shape_id in &all_referenced {
        if let Some(Shape::Union(union_shape)) = model.shapes.get(shape_id.as_str()) {
            let short = SmithyModel::short_name(shape_id).to_owned();
            let fields: Vec<FieldInfo> = union_shape
                .members
                .iter()
                .map(|(name, member)| {
                    let base_type = resolve_rust_type(model, &member.target, &needed_enum_names);
                    FieldInfo {
                        smithy_name: name.clone(),
                        rust_name: to_snake_case(name),
                        rust_type: format!("Option<{base_type}>"),
                        required: false,
                        http_binding: None,
                    }
                })
                .collect();
            shared_structs.insert(short, fields);
        }
    }

    // Step 5: Collect error types from Smithy model.
    let mut errors = collect_errors(model, &error_shape_ids, config);

    // Sort errors by name for stable output
    errors.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(ResolvedModel {
        operations,
        input_categories,
        output_categories,
        enums,
        shared_structs,
        input_structs,
        output_structs,
        errors,
    })
}

/// Collect error types from Smithy model shapes and merge with custom errors from config.
fn collect_errors(
    model: &SmithyModel,
    error_shape_ids: &BTreeSet<String>,
    config: &ServiceConfig,
) -> Vec<ErrorInfo> {
    let mut seen_names = BTreeSet::new();
    let mut errors = Vec::new();

    // Collect errors from Smithy model
    for shape_id in error_shape_ids {
        let Some(Shape::Structure(structure)) = model.shapes.get(shape_id.as_str()) else {
            continue;
        };

        let Some(fault_value) = structure.traits.get("smithy.api#error") else {
            continue;
        };

        let wire_name = SmithyModel::short_name(shape_id).to_owned();
        let fault = fault_value.as_str().unwrap_or("client").to_owned();

        // Determine HTTP status code: from httpError trait, or default by fault
        let status_code = structure
            .traits
            .get("smithy.api#httpError")
            .and_then(|v| v.as_u64())
            .map_or_else(|| if fault == "server" { 500 } else { 400 }, |v| v as u16);

        // Use the wire name as the variant name (strip trailing "Exception" if present
        // for a cleaner Rust API, but keep the wire name for serialization)
        let name = wire_name.clone();

        let message = name.clone();

        seen_names.insert(name.clone());
        errors.push(ErrorInfo {
            name,
            wire_name,
            status_code,
            message,
            fault,
        });
    }

    // Merge custom errors from config
    for (name, custom) in &config.custom_errors {
        if !seen_names.contains(name) {
            seen_names.insert(name.clone());
            errors.push(ErrorInfo {
                name: name.clone(),
                wire_name: name.clone(),
                status_code: custom.status,
                message: custom.message.clone(),
                fault: if custom.status >= 500 {
                    "server".to_owned()
                } else {
                    "client".to_owned()
                },
            });
        }
    }

    errors
}
