//! Document schema for validating structure.
//!
//! The schema defines which node types exist, which marks can be applied,
//! and how nodes can be nested. It is the source of truth for document
//! structure validation.

pub mod content_match;
pub mod data_attrs;
pub mod mark;
pub mod node;
pub mod validation;

pub use data_attrs::{
    DataAttrSplit, MAX_DATA_ATTR_NAME, MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS,
    RESERVED_DATA_ATTR_PREFIXES, RESERVED_DATA_ATTRS, app_data_attrs, is_app_data_attr,
    is_data_attr_name, is_reserved_data_attr, kept_data_attrs,
};
pub use mark::{MarkSpec, MarkSpecBuilder};
pub use node::{AttrSpec, MarkSet, NodeSpec, NodeSpecBuilder};

use crate::model::AttrValue;
use crate::model::types::{MarkType, NodeType};
use content_match::ContentMatch;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Schema defines the valid structure of a document.
///
/// This includes:
/// - Available node types (paragraph, heading, list, etc.)
/// - Available mark types (bold, italic, link, etc.)
/// - Validation rules for document structure
#[derive(Debug, Clone)]
pub struct Schema {
    /// Node type specifications
    pub nodes: HashMap<String, NodeSpec>,
    /// Mark type specifications
    pub marks: HashMap<String, MarkSpec>,
    /// The top-level node type (usually "doc")
    pub top_node: String,
    /// Interned node-type handles, compiled from `nodes` (name + spec + content match).
    node_types: HashMap<Box<str>, NodeType>,
    /// Interned mark-type handles, compiled from `marks`.
    mark_types: HashMap<Box<str>, MarkType>,
    /// `group name -> set of node-type names` in that group (for content matching).
    groups: HashMap<String, HashSet<Box<str>>>,
}

/// The widest an image's `width` attr is written or shown, in CSS pixels:
/// the HTML import clamps an `<img width>` to it, and the HTML writer and the
/// view clamp a model value above it. A bound of our own, as the import's
/// `colspan` cap of 1000 is Chrome's; Chrome has none for `width`.
pub const IMAGE_MAX_WIDTH: i64 = 65535;

impl Schema {
    /// Create a new empty schema with "doc" as the top node.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            marks: HashMap::new(),
            top_node: "doc".to_string(),
            node_types: HashMap::new(),
            mark_types: HashMap::new(),
            groups: HashMap::new(),
        }
    }

    /// Create a new schema builder.
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    /// Get a node spec by name.
    pub fn node(&self, name: &str) -> Option<&NodeSpec> {
        self.nodes.get(name)
    }

    /// Get a mark spec by name.
    pub fn mark(&self, name: &str) -> Option<&MarkSpec> {
        self.marks.get(name)
    }

    /// Check if a node type exists in this schema.
    pub fn has_node(&self, name: &str) -> bool {
        self.nodes.contains_key(name)
    }

    /// Check if a mark type exists in this schema.
    pub fn has_mark(&self, name: &str) -> bool {
        self.marks.contains_key(name)
    }

    /// Get all node names that belong to a given group.
    pub fn nodes_in_group(&self, group: &str) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|(_, spec)| spec.group.as_deref() == Some(group))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Check if a mark can be applied to content within a node type.
    ///
    /// Returns false if the node's `marks` field disallows the mark,
    /// or if the mark type doesn't exist in the schema.
    pub fn allows_mark(&self, node_type: &str, mark_type: &str) -> bool {
        if !self.has_mark(mark_type) {
            return false;
        }
        match self.node(node_type) {
            Some(spec) => spec.marks.allows(mark_type),
            None => false,
        }
    }

    /// Check if a node type can contain a child of the given type.
    ///
    /// Uses the parent's content expression to determine if the child
    /// type (or its group) is allowed.
    pub fn allows_child(&self, parent_type: &str, child_type: &str) -> bool {
        match self.node_type(parent_type) {
            Some(nt) => nt.content_match().accepts(child_type),
            None => false,
        }
    }

    /// Get the interned node-type handle for `name`.
    ///
    /// Unlike [`Schema::node`] (which returns the raw `NodeSpec`), this returns a
    /// cheap, clonable [`NodeType`] handle carrying the compiled content match —
    /// the form the document model holds in every node.
    pub fn node_type(&self, name: &str) -> Option<&NodeType> {
        self.node_types.get(name)
    }

    /// Get the interned mark-type handle for `name`.
    pub fn mark_type(&self, name: &str) -> Option<&MarkType> {
        self.mark_types.get(name)
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a list of children satisfies a content expression.
///
/// Compiles `expr` against the schema's groups and matches the child sequence
/// via [`ContentMatch`]. (Kept as a free function for the `validation` module and
/// the schema tests; node-type validation in the transform engine goes through
/// the precompiled `NodeType::content_match` directly.)
pub fn matches_content(schema: &Schema, expr: &str, children: &[&str]) -> bool {
    ContentMatch::compile(Some(expr), &schema.groups).matches(children)
}

mod builder;
mod starter_kit;
#[cfg(test)]
mod tests;

pub use builder::SchemaBuilder;
