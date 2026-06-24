//! Document schema for validating structure.
//!
//! The schema defines which node types exist, which marks can be applied,
//! and how nodes can be nested. It is the source of truth for document
//! structure validation.

pub mod content_match;
pub mod mark;
pub mod node;
pub mod validation;

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

    /// Create a schema with the starter-kit defaults.
    ///
    /// Includes standard block nodes (paragraph, heading, lists, etc.),
    /// inline nodes (text, image, hard_break), and common marks
    /// (bold, italic, underline, strike, code, link, highlight, etc.).
    pub fn starter_kit() -> Self {
        let mut builder = SchemaBuilder::new();

        // === Nodes ===

        // doc - top-level container
        builder = builder.node("doc", NodeSpec::builder("doc").content("block+").build());

        // paragraph
        builder = builder.node(
            "paragraph",
            NodeSpec::builder("paragraph")
                .content("inline*")
                .group("block")
                .parse_html(vec!["p".into()])
                .build(),
        );

        // heading
        builder = builder.node(
            "heading",
            NodeSpec::builder("heading")
                .content("inline*")
                .group("block")
                .attr("level", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec![
                    "h1".into(),
                    "h2".into(),
                    "h3".into(),
                    "h4".into(),
                    "h5".into(),
                    "h6".into(),
                ])
                .build(),
        );

        // blockquote
        builder = builder.node(
            "blockquote",
            NodeSpec::builder("blockquote")
                .content("block+")
                .group("block")
                .parse_html(vec!["blockquote".into()])
                .build(),
        );

        // code_block
        builder = builder.node(
            "code_block",
            NodeSpec::builder("code_block")
                .content("text*")
                .group("block")
                .marks(MarkSet::None)
                .attr("language", AttrSpec::optional(""))
                .parse_html(vec!["pre".into()])
                .build(),
        );

        // bullet_list
        builder = builder.node(
            "bullet_list",
            NodeSpec::builder("bullet_list")
                .content("list_item+")
                .group("block")
                .parse_html(vec!["ul".into()])
                .build(),
        );

        // ordered_list
        builder = builder.node(
            "ordered_list",
            NodeSpec::builder("ordered_list")
                .content("list_item+")
                .group("block")
                .attr("start", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["ol".into()])
                .build(),
        );

        // list_item
        builder = builder.node(
            "list_item",
            NodeSpec::builder("list_item")
                .content("block+")
                .parse_html(vec!["li".into()])
                .build(),
        );

        // task_list
        builder = builder.node(
            "task_list",
            NodeSpec::builder("task_list")
                .content("task_item+")
                .group("block")
                .build(),
        );

        // task_item
        builder = builder.node(
            "task_item",
            NodeSpec::builder("task_item")
                .content("block+")
                .attr("checked", AttrSpec::optional(AttrValue::Bool(false)))
                .build(),
        );

        // horizontal_rule
        builder = builder.node("horizontal_rule", {
            let mut spec = NodeSpec::atom("horizontal_rule");
            spec.group = Some("block".into());
            spec.parse_html_tags = vec!["hr".into()];
            spec
        });

        // hard_break
        builder = builder.node("hard_break", {
            let mut spec = NodeSpec::atom("hard_break");
            spec.group = Some("inline".into());
            spec.inline = true;
            spec.parse_html_tags = vec!["br".into()];
            spec
        });

        // text
        builder = builder.node(
            "text",
            NodeSpec::builder("text").group("inline").inline().build(),
        );

        // image
        builder = builder.node("image", {
            let mut spec = NodeSpec::atom("image");
            spec.group = Some("inline".into());
            spec.inline = true;
            spec.attrs.insert("src".into(), AttrSpec::required());
            spec.attrs.insert("alt".into(), AttrSpec::optional(""));
            spec.attrs.insert("title".into(), AttrSpec::optional(""));
            spec.parse_html_tags = vec!["img".into()];
            spec
        });

        // === Tables ===
        // A table is a block whose children are rows; each row holds cells; each
        // cell holds block content. Cells (`table_cell`/`table_header_cell`) share
        // a `cell` group so a row's content is simply `cell+` (the content-match
        // grammar has no alternation, so a group stands in for `(td | th)+`).

        // table > table_row+
        builder = builder.node(
            "table",
            NodeSpec::builder("table")
                .content("table_row+")
                .group("block")
                .parse_html(vec!["table".into()])
                .build(),
        );

        // table_row > cell*
        // Zero-or-more (not `cell+`): a row can be **empty** when every one of its
        // columns is covered by a `rowspan` cell from a row above (e.g. after merging
        // a full-width rectangle of cells). This matches ProseMirror's row content.
        builder = builder.node(
            "table_row",
            NodeSpec::builder("table_row")
                .content("cell*")
                .parse_html(vec!["tr".into()])
                .build(),
        );

        // table_cell > block+
        // Cells are `isolating`: edits (backspace/delete, lift/wrap, find-wrapping)
        // never cross a cell boundary — a cell is its own editing context, just as
        // in ProseMirror. `find_cut_before`/`find_cut_after` already honor this flag.
        builder = builder.node(
            "table_cell",
            NodeSpec::builder("table_cell")
                .content("block+")
                .group("cell")
                .isolating(true)
                .attr("colspan", AttrSpec::optional(AttrValue::Int(1)))
                .attr("rowspan", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["td".into()])
                .build(),
        );

        // table_header_cell > block+
        builder = builder.node(
            "table_header_cell",
            NodeSpec::builder("table_header_cell")
                .content("block+")
                .group("cell")
                .isolating(true)
                .attr("colspan", AttrSpec::optional(AttrValue::Int(1)))
                .attr("rowspan", AttrSpec::optional(AttrValue::Int(1)))
                .parse_html(vec!["th".into()])
                .build(),
        );

        // === Marks ===

        // bold
        let mut bold = MarkSpec::simple("bold");
        bold.parse_html_tags = vec!["strong".into(), "b".into()];
        builder = builder.mark("bold", bold);

        // italic
        let mut italic = MarkSpec::simple("italic");
        italic.parse_html_tags = vec!["em".into(), "i".into()];
        builder = builder.mark("italic", italic);

        // underline
        let mut underline = MarkSpec::simple("underline");
        underline.parse_html_tags = vec!["u".into()];
        builder = builder.mark("underline", underline);

        // strike
        let mut strike = MarkSpec::simple("strike");
        strike.parse_html_tags = vec!["s".into(), "del".into(), "strike".into()];
        builder = builder.mark("strike", strike);

        // code
        let mut code = MarkSpec::simple("code");
        code.excludes = Some("bold italic underline strike link".to_string());
        code.parse_html_tags = vec!["code".into()];
        builder = builder.mark("code", code);

        // link
        let mut link_attrs = BTreeMap::new();
        link_attrs.insert("href".into(), AttrSpec::required());
        link_attrs.insert("title".into(), AttrSpec::optional(""));
        link_attrs.insert("target".into(), AttrSpec::optional(""));
        let mut link = MarkSpec::with_attrs("link", link_attrs);
        link.parse_html_tags = vec!["a".into()];
        builder = builder.mark("link", link);

        // highlight
        let mut hl_attrs = BTreeMap::new();
        hl_attrs.insert("color".into(), AttrSpec::optional(""));
        let mut highlight = MarkSpec::with_attrs("highlight", hl_attrs);
        highlight.parse_html_tags = vec!["mark".into()];
        builder = builder.mark("highlight", highlight);

        // text_color
        let mut tc_attrs = BTreeMap::new();
        tc_attrs.insert("color".into(), AttrSpec::required());
        let text_color = MarkSpec::with_attrs("text_color", tc_attrs);
        builder = builder.mark("text_color", text_color);

        // subscript
        let mut subscript = MarkSpec::simple("subscript");
        subscript.excludes = Some("superscript".to_string());
        subscript.parse_html_tags = vec!["sub".into()];
        builder = builder.mark("subscript", subscript);

        // superscript
        let mut superscript = MarkSpec::simple("superscript");
        superscript.excludes = Some("subscript".to_string());
        superscript.parse_html_tags = vec!["sup".into()];
        builder = builder.mark("superscript", superscript);

        builder.build()
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
    pub fn build(self) -> Schema {
        let groups = compute_groups(&self.nodes);
        // A node-type name must not collide with a group name, or content
        // expressions referencing it become ambiguous (PM rejects this).
        debug_assert!(
            self.nodes.keys().all(|n| !groups.contains_key(n.as_str())),
            "a node-type name collides with a group name"
        );
        let node_types = compile_node_types(&self.nodes, &groups);
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

/// Check if a list of children satisfies a content expression.
///
/// Compiles `expr` against the schema's groups and matches the child sequence
/// via [`ContentMatch`]. (Kept as a free function for the `validation` module and
/// the schema tests; node-type validation in the transform engine goes through
/// the precompiled `NodeType::content_match` directly.)
pub fn matches_content(schema: &Schema, expr: &str, children: &[&str]) -> bool {
    ContentMatch::compile(Some(expr), &schema.groups).matches(children)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_kit_has_all_nodes() {
        let schema = Schema::starter_kit();
        let expected_nodes = [
            "doc",
            "paragraph",
            "heading",
            "blockquote",
            "code_block",
            "bullet_list",
            "ordered_list",
            "list_item",
            "task_list",
            "task_item",
            "horizontal_rule",
            "hard_break",
            "text",
            "image",
        ];
        for name in &expected_nodes {
            assert!(schema.has_node(name), "missing node: {}", name);
        }
    }

    #[test]
    fn starter_kit_has_all_marks() {
        let schema = Schema::starter_kit();
        let expected_marks = [
            "bold",
            "italic",
            "underline",
            "strike",
            "code",
            "link",
            "highlight",
            "text_color",
            "subscript",
            "superscript",
        ];
        for name in &expected_marks {
            assert!(schema.has_mark(name), "missing mark: {}", name);
        }
    }

    #[test]
    fn starter_kit_top_node_is_doc() {
        let schema = Schema::starter_kit();
        assert_eq!(schema.top_node, "doc");
    }

    #[test]
    fn node_lookup() {
        let schema = Schema::starter_kit();
        let para = schema.node("paragraph").unwrap();
        assert_eq!(para.name, "paragraph");
        assert_eq!(para.content, Some("inline*".to_string()));
        assert!(schema.node("nonexistent").is_none());
    }

    #[test]
    fn mark_lookup() {
        let schema = Schema::starter_kit();
        let bold = schema.mark("bold").unwrap();
        assert_eq!(bold.name, "bold");
        assert!(schema.mark("nonexistent").is_none());
    }

    #[test]
    fn nodes_in_group_block() {
        let schema = Schema::starter_kit();
        let blocks = schema.nodes_in_group("block");
        assert!(blocks.contains(&"paragraph"));
        assert!(blocks.contains(&"heading"));
        assert!(blocks.contains(&"blockquote"));
        assert!(blocks.contains(&"code_block"));
        assert!(!blocks.contains(&"text"));
        assert!(!blocks.contains(&"doc"));
    }

    #[test]
    fn nodes_in_group_inline() {
        let schema = Schema::starter_kit();
        let inlines = schema.nodes_in_group("inline");
        assert!(inlines.contains(&"text"));
        assert!(inlines.contains(&"hard_break"));
        assert!(inlines.contains(&"image"));
        assert!(!inlines.contains(&"paragraph"));
    }

    #[test]
    fn allows_mark_on_paragraph() {
        let schema = Schema::starter_kit();
        assert!(schema.allows_mark("paragraph", "bold"));
        assert!(schema.allows_mark("paragraph", "italic"));
        assert!(schema.allows_mark("paragraph", "link"));
    }

    #[test]
    fn code_block_disallows_marks() {
        let schema = Schema::starter_kit();
        assert!(!schema.allows_mark("code_block", "bold"));
        assert!(!schema.allows_mark("code_block", "italic"));
        assert!(!schema.allows_mark("code_block", "link"));
    }

    #[test]
    fn allows_mark_nonexistent() {
        let schema = Schema::starter_kit();
        assert!(!schema.allows_mark("paragraph", "nonexistent"));
        assert!(!schema.allows_mark("nonexistent", "bold"));
    }

    #[test]
    fn allows_child_doc_paragraph() {
        let schema = Schema::starter_kit();
        assert!(schema.allows_child("doc", "paragraph"));
        assert!(schema.allows_child("doc", "heading"));
        assert!(schema.allows_child("doc", "blockquote"));
        assert!(!schema.allows_child("doc", "text"));
    }

    #[test]
    fn allows_child_paragraph_text() {
        let schema = Schema::starter_kit();
        assert!(schema.allows_child("paragraph", "text"));
        assert!(schema.allows_child("paragraph", "hard_break"));
        assert!(schema.allows_child("paragraph", "image"));
        assert!(!schema.allows_child("paragraph", "paragraph"));
    }

    #[test]
    fn allows_child_list() {
        let schema = Schema::starter_kit();
        assert!(schema.allows_child("bullet_list", "list_item"));
        assert!(!schema.allows_child("bullet_list", "paragraph"));
        assert!(schema.allows_child("list_item", "paragraph"));
    }

    #[test]
    fn allows_child_leaf_node() {
        let schema = Schema::starter_kit();
        assert!(!schema.allows_child("horizontal_rule", "text"));
        assert!(!schema.allows_child("hard_break", "text"));
    }

    #[test]
    fn builder_works() {
        let schema = Schema::builder()
            .node("doc", NodeSpec::builder("doc").content("block+").build())
            .node("paragraph", NodeSpec::block("paragraph"))
            .mark("bold", MarkSpec::simple("bold"))
            .top_node("doc")
            .build();

        assert!(schema.has_node("doc"));
        assert!(schema.has_node("paragraph"));
        assert!(schema.has_mark("bold"));
        assert_eq!(schema.top_node, "doc");
    }

    #[test]
    fn matches_content_star_empty() {
        let schema = Schema::starter_kit();
        assert!(matches_content(&schema, "inline*", &[]));
        assert!(matches_content(&schema, "block*", &[]));
    }

    #[test]
    fn matches_content_star_with_items() {
        let schema = Schema::starter_kit();
        assert!(matches_content(&schema, "inline*", &["text"]));
        assert!(matches_content(
            &schema,
            "inline*",
            &["text", "hard_break", "text"]
        ));
    }

    #[test]
    fn matches_content_plus_requires_one() {
        let schema = Schema::starter_kit();
        assert!(!matches_content(&schema, "block+", &[]));
        assert!(matches_content(&schema, "block+", &["paragraph"]));
        assert!(matches_content(
            &schema,
            "block+",
            &["paragraph", "heading"]
        ));
    }

    #[test]
    fn matches_content_specific_type() {
        let schema = Schema::starter_kit();
        assert!(matches_content(&schema, "list_item+", &["list_item"]));
        assert!(matches_content(
            &schema,
            "list_item+",
            &["list_item", "list_item"]
        ));
        assert!(!matches_content(&schema, "list_item+", &["paragraph"]));
    }

    #[test]
    fn matches_content_text_star() {
        let schema = Schema::starter_kit();
        assert!(matches_content(&schema, "text*", &[]));
        assert!(matches_content(&schema, "text*", &["text"]));
        assert!(!matches_content(&schema, "text*", &["paragraph"]));
    }

    #[test]
    fn matches_content_wrong_group() {
        let schema = Schema::starter_kit();
        assert!(!matches_content(&schema, "block+", &["text"]));
        assert!(!matches_content(&schema, "inline*", &["paragraph"]));
    }

    #[test]
    fn matches_content_compound() {
        let schema = Schema::starter_kit();
        assert!(matches_content(&schema, "paragraph block*", &["paragraph"]));
        assert!(matches_content(
            &schema,
            "paragraph block*",
            &["paragraph", "heading"]
        ));
        assert!(!matches_content(&schema, "paragraph block*", &["heading"]));
    }

    #[test]
    fn heading_has_level_attr() {
        let schema = Schema::starter_kit();
        let heading = schema.node("heading").unwrap();
        assert!(heading.attrs.contains_key("level"));
        assert_eq!(heading.attrs["level"].default, Some(AttrValue::Int(1)));
    }

    #[test]
    fn image_has_required_src() {
        let schema = Schema::starter_kit();
        let image = schema.node("image").unwrap();
        assert!(image.attrs["src"].required);
        assert!(!image.attrs["alt"].required);
    }

    #[test]
    fn link_has_required_href() {
        let schema = Schema::starter_kit();
        let link = schema.mark("link").unwrap();
        assert!(link.attrs["href"].required);
        assert!(!link.attrs["title"].required);
    }

    #[test]
    fn code_mark_excludes_formatting() {
        let schema = Schema::starter_kit();
        let code = schema.mark("code").unwrap();
        assert!(code.excludes_mark("bold"));
        assert!(code.excludes_mark("italic"));
        assert!(code.excludes_mark("underline"));
        assert!(code.excludes_mark("strike"));
        assert!(code.excludes_mark("link"));
        assert!(!code.excludes_mark("highlight"));
    }

    #[test]
    fn subscript_superscript_mutual_exclusion() {
        let schema = Schema::starter_kit();
        let sub = schema.mark("subscript").unwrap();
        let sup = schema.mark("superscript").unwrap();
        assert!(sub.excludes_mark("superscript"));
        assert!(sup.excludes_mark("subscript"));
        assert!(!sub.excludes_mark("bold"));
        assert!(!sup.excludes_mark("bold"));
    }
}
