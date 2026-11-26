//! Building a [`Schema`]: collecting node and mark specs and compiling their
//! interned type handles.

use super::*;

/// Builder for constructing schemas.
#[derive(Debug)]
pub struct SchemaBuilder {
    nodes: HashMap<String, NodeSpec>,
    marks: HashMap<String, MarkSpec>,
    top_node: String,
}

impl SchemaBuilder {
    /// Create a new schema builder.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            marks: HashMap::new(),
            top_node: "doc".to_string(),
        }
    }

    /// Add a node specification.
    pub fn node(mut self, name: impl Into<String>, spec: NodeSpec) -> Self {
        self.nodes.insert(name.into(), spec);
        self
    }

    /// Add a mark specification.
    pub fn mark(mut self, name: impl Into<String>, spec: MarkSpec) -> Self {
        self.marks.insert(name.into(), spec);
        self
    }

    /// Set the top-level node type.
    pub fn top_node(mut self, name: impl Into<String>) -> Self {
        self.top_node = name.into();
        self
    }

    /// Build the schema, compiling interned node/mark-type handles and the
    /// per-type content matches.
    ///
    /// # Panics
    ///
    /// When a node type that is neither a leaf nor a textblock (a list, a
    /// quote, a table) sets [`NodeSpec::data_attrs`]: HTML carries app data
    /// attributes on a leaf's element (a void element, or a block atom's
    /// `<div>`) and on a textblock's own element (`<p>`, `<h1>`), and a
    /// container would lose them on a copy that cuts through it.
    pub fn build(self) -> Schema {
        let groups = compute_groups(&self.nodes);
        // A node-type name must not collide with a group name, or content
        // expressions referencing it become ambiguous (PM rejects this).
        debug_assert!(
            self.nodes.keys().all(|n| !groups.contains_key(n.as_str())),
            "a node-type name collides with a group name"
        );
        let node_types = compile_node_types(&self.nodes, &groups);
        // App data attributes go through HTML on a leaf's or a textblock's own
        // element only, so a container that kept them would lose them on copy
        // and paste.
        for (name, spec) in &self.nodes {
            let typ = &node_types[name.as_str()];
            assert!(
                !spec.data_attrs || typ.is_leaf() || typ.is_textblock(),
                "node type `{name}` sets `data_attrs`, which only a leaf node type \
                 (an atom such as `image`) or a textblock (such as `paragraph`) may"
            );
        }
        let mark_types = compile_mark_types(&self.marks);
        Schema {
            nodes: self.nodes,
            marks: self.marks,
            top_node: self.top_node,
            node_types,
            mark_types,
            groups,
        }
    }
}

impl Default for SchemaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `group name -> set of node-type names` map from node specs.
fn compute_groups(nodes: &HashMap<String, NodeSpec>) -> HashMap<String, HashSet<Box<str>>> {
    let mut groups: HashMap<String, HashSet<Box<str>>> = HashMap::new();
    for (name, spec) in nodes {
        if let Some(g) = &spec.group {
            groups
                .entry(g.clone())
                .or_default()
                .insert(name.as_str().into());
        }
    }
    groups
}

/// Compile interned node-type handles (name + spec + content match) from specs.
fn compile_node_types(
    nodes: &HashMap<String, NodeSpec>,
    groups: &HashMap<String, HashSet<Box<str>>>,
) -> HashMap<Box<str>, NodeType> {
    nodes
        .iter()
        .map(|(name, spec)| {
            let cm = ContentMatch::compile(spec.content.as_deref(), groups);
            (
                name.as_str().into(),
                NodeType::new(name.as_str().into(), spec.clone(), cm),
            )
        })
        .collect()
}

/// Compile interned mark-type handles from specs.
fn compile_mark_types(marks: &HashMap<String, MarkSpec>) -> HashMap<Box<str>, MarkType> {
    marks
        .iter()
        .map(|(name, spec)| {
            (
                name.as_str().into(),
                MarkType::new(name.as_str().into(), spec.clone()),
            )
        })
        .collect()
}
