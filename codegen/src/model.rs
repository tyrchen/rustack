//! Smithy JSON AST model types for deserialization.
//!
//! This module provides the types needed to parse the Smithy 2.0 JSON AST format
//! used by AWS service models.

use std::collections::HashMap;

use serde::Deserialize;

/// Top-level Smithy model document.
#[derive(Debug, Deserialize)]
pub struct SmithyModel {
    /// Smithy version (e.g., "2.0").
    pub smithy: String,
    /// All shapes defined in the model, keyed by their full shape ID.
    pub shapes: HashMap<String, Shape>,
}

/// A single Smithy shape.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Shape {
    /// A structure shape (struct).
    #[serde(rename = "structure")]
    Structure(StructureShape),
    /// An operation shape.
    #[serde(rename = "operation")]
    Operation(OperationShape),
    /// A string shape (simple type or alias).
    #[serde(rename = "string")]
    String(SimpleShape),
    /// A boolean shape.
    #[serde(rename = "boolean")]
    Boolean(SimpleShape),
    /// A 32-bit integer shape.
    #[serde(rename = "integer")]
    Integer(SimpleShape),
    /// A 64-bit integer shape.
    #[serde(rename = "long")]
    Long(SimpleShape),
    /// A timestamp shape.
    #[serde(rename = "timestamp")]
    Timestamp(SimpleShape),
    /// A blob (binary data) shape.
    #[serde(rename = "blob")]
    Blob(SimpleShape),
    /// An enum shape.
    #[serde(rename = "enum")]
    Enum(EnumShape),
    /// A list shape.
    #[serde(rename = "list")]
    List(ListShape),
    /// A map shape.
    #[serde(rename = "map")]
    Map(MapShape),
    /// A union shape.
    #[serde(rename = "union")]
    Union(UnionShape),
    /// A service shape (ignored).
    #[serde(rename = "service")]
    Service(SimpleShape),
    /// A resource shape (ignored).
    #[serde(rename = "resource")]
    Resource(SimpleShape),
    /// An integer enum shape (ignored).
    #[serde(rename = "intEnum")]
    IntEnum(SimpleShape),
    /// A double shape.
    #[serde(rename = "double")]
    Double(SimpleShape),
    /// A float shape.
    #[serde(rename = "float")]
    Float(SimpleShape),
}

/// A simple shape with optional traits.
#[derive(Debug, Default, Deserialize)]
pub struct SimpleShape {
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A structure (struct) shape.
#[derive(Debug, Deserialize)]
pub struct StructureShape {
    /// Members of the structure.
    #[serde(default)]
    pub members: HashMap<String, MemberShape>,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A member within a structure or union.
#[derive(Debug, Deserialize)]
pub struct MemberShape {
    /// Target shape ID this member points to.
    pub target: String,
    /// Traits applied to this member.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// An operation shape.
#[derive(Debug, Deserialize)]
pub struct OperationShape {
    /// Input shape reference.
    pub input: Option<ShapeRef>,
    /// Output shape reference.
    pub output: Option<ShapeRef>,
    /// Error shape references.
    #[serde(default)]
    pub errors: Vec<ShapeRef>,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A reference to another shape.
#[derive(Debug, Deserialize)]
pub struct ShapeRef {
    /// The full shape ID being referenced.
    pub target: String,
}

/// An enum shape.
#[derive(Debug, Deserialize)]
pub struct EnumShape {
    /// Enum variants (member name -> member shape).
    #[serde(default)]
    pub members: HashMap<String, MemberShape>,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A list shape.
#[derive(Debug, Deserialize)]
pub struct ListShape {
    /// The shape of the list's elements.
    pub member: ShapeRef,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A map shape.
#[derive(Debug, Deserialize)]
pub struct MapShape {
    /// The shape of the map's keys.
    pub key: ShapeRef,
    /// The shape of the map's values.
    pub value: ShapeRef,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

/// A union shape.
#[derive(Debug, Deserialize)]
pub struct UnionShape {
    /// Union variants.
    #[serde(default)]
    pub members: HashMap<String, MemberShape>,
    /// Traits applied to this shape.
    #[serde(default)]
    pub traits: HashMap<String, serde_json::Value>,
}

impl SmithyModel {
    /// Get the short name from a fully qualified shape ID.
    ///
    /// For example, `com.amazonaws.s3#BucketName` returns `BucketName`.
    pub fn short_name(shape_id: &str) -> &str {
        shape_id.rsplit_once('#').map_or(shape_id, |(_, name)| name)
    }
}
