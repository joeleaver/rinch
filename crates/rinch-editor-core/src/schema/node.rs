//! Node types and specifications.

use crate::model::AttrValue;
use std::collections::BTreeMap;

// NOTE: the old dyn `Node` trait was dropped in the rearchitecture (M0). The
// concrete `Node` value type lives in `crate::model` (M1); the schema only deals
// in `NodeSpec` (the *specification* of a node type), never a node instance.

/// Specification for a node type in the schema.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    /// The node type name (e.g., "paragraph", "heading")
    pub name: String,

    /// Content expression defining allowed children.
    /// Examples: "inline*", "block+", "list_item+", "text*"
    /// None means no content (leaf node).
    pub content: Option<String>,

    /// Group this node belongs to (e.g., "block", "inline")
    pub group: Option<String>,

    /// Whether this is an inline node
    pub inline: bool,

    /// Whether this node is atomic (treated as a single unit, no editable content)
    pub atom: bool,

    /// Which marks are allowed on content in this node
    pub marks: MarkSet,

    /// Whether the node can be selected as a whole
    pub selectable: bool,

    /// Whether the node can be dragged
    pub draggable: bool,

    /// Whether the node isolates cursor (can't escape via arrow keys)
    pub isolating: bool,

    /// Node attributes with their defaults
    pub attrs: BTreeMap<String, AttrSpec>,

    /// HTML tags to parse as this node (e.g., ["p"], ["h1", "h2", "h3"...])
    pub parse_html_tags: Vec<String>,
}

impl NodeSpec {
    /// Create a new node spec builder.
    pub fn builder(name: impl Into<String>) -> NodeSpecBuilder {
        NodeSpecBuilder::new(name)
    }

    /// Create a basic block node spec with the given content expression.
    pub fn block(name: &str) -> Self {
        Self {
            name: name.to_string(),
            content: Some("inline*".to_string()),
            group: Some("block".to_string()),
            inline: false,
            atom: false,
            marks: MarkSet::All,
            selectable: true,
            draggable: false,
            isolating: false,
            attrs: BTreeMap::new(),
            parse_html_tags: Vec::new(),
        }
    }

    /// Create a basic inline node spec.
    pub fn inline(name: &str) -> Self {
        Self {
            name: name.to_string(),
            content: None,
            group: Some("inline".to_string()),
            inline: true,
            atom: false,
            marks: MarkSet::All,
            selectable: false,
            draggable: false,
            isolating: false,
            attrs: BTreeMap::new(),
            parse_html_tags: Vec::new(),
        }
    }

    /// Create a leaf/atom node spec.
    pub fn atom(name: &str) -> Self {
        Self {
            name: name.to_string(),
            content: None,
            group: None,
            inline: false,
            atom: true,
            marks: MarkSet::None,
            selectable: true,
            draggable: false,
            isolating: false,
            attrs: BTreeMap::new(),
            parse_html_tags: Vec::new(),
        }
    }

    /// Check if this is a block-level node.
    pub fn is_block(&self) -> bool {
        !self.inline
    }
}

/// Specifies which marks are allowed on content within a node.
#[derive(Debug, Clone)]
pub enum MarkSet {
    /// All marks allowed (default)
    All,
    /// No marks allowed
    None,
    /// Only specific marks allowed
    Only(Vec<String>),
    /// All marks except these
    Except(Vec<String>),
}

impl MarkSet {
    /// Check if a given mark type is allowed by this set.
    pub fn allows(&self, mark_type: &str) -> bool {
        match self {
            MarkSet::All => true,
            MarkSet::None => false,
            MarkSet::Only(allowed) => allowed.iter().any(|m| m == mark_type),
            MarkSet::Except(excluded) => !excluded.iter().any(|m| m == mark_type),
        }
    }
}

/// Attribute specification with an optional, **typed** default value.
///
/// The default is an [`AttrValue`] (not a string) so that `heading.level`
/// defaults to `Int(1)` and `task_item.checked` to `Bool(false)` — the typed
/// model the document tree carries (amendment A11). `default == None` marks a
/// required attribute (no default; absence is a schema-validation error at the
/// serialization / Step boundary via [`NodeType::compute_attrs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSpec {
    /// Default value (None = required attribute)
    pub default: Option<AttrValue>,
    /// Whether this attribute is required
    pub required: bool,
}

impl AttrSpec {
    /// Create an optional attribute with a typed default value.
    pub fn optional(default: impl Into<AttrValue>) -> Self {
        Self {
            default: Some(default.into()),
            required: false,
        }
    }

    /// Create a required attribute with no default.
    pub fn required() -> Self {
        Self {
            default: None,
            required: true,
        }
    }
}

/// Builder for node specifications.
#[derive(Debug)]
pub struct NodeSpecBuilder {
    spec: NodeSpec,
}

impl NodeSpecBuilder {
    /// Create a new builder with the given node name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            spec: NodeSpec {
                name: name.into(),
                content: None,
                group: None,
                inline: false,
                atom: false,
                marks: MarkSet::All,
                selectable: true,
                draggable: false,
                isolating: false,
                attrs: BTreeMap::new(),
                parse_html_tags: Vec::new(),
            },
        }
    }

    /// Set the content expression.
    pub fn content(mut self, expr: impl Into<String>) -> Self {
        self.spec.content = Some(expr.into());
        self
    }

    /// Set the group this node belongs to.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.spec.group = Some(group.into());
        self
    }

    /// Mark this as a block-level node (shorthand for group("block")).
    pub fn block(mut self) -> Self {
        self.spec.inline = false;
        self
    }

    /// Mark this as an inline node.
    pub fn inline(mut self) -> Self {
        self.spec.inline = true;
        self
    }

    /// Mark this as an atom node.
    pub fn atom(mut self) -> Self {
        self.spec.atom = true;
        self
    }

    /// Set the allowed marks.
    pub fn marks(mut self, marks: MarkSet) -> Self {
        self.spec.marks = marks;
        self
    }

    /// Set whether the node is selectable.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.spec.selectable = selectable;
        self
    }

    /// Set whether the node is draggable.
    pub fn draggable(mut self, draggable: bool) -> Self {
        self.spec.draggable = draggable;
        self
    }

    /// Set whether the node is isolating.
    pub fn isolating(mut self, isolating: bool) -> Self {
        self.spec.isolating = isolating;
        self
    }

    /// Add an attribute specification.
    pub fn attr(mut self, name: impl Into<String>, attr: AttrSpec) -> Self {
        self.spec.attrs.insert(name.into(), attr);
        self
    }

    /// Set HTML tags that parse to this node.
    pub fn parse_html(mut self, tags: Vec<String>) -> Self {
        self.spec.parse_html_tags = tags;
        self
    }

    /// Build the node spec.
    pub fn build(self) -> NodeSpec {
        self.spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_constructor_defaults() {
        let spec = NodeSpec::block("paragraph");
        assert_eq!(spec.name, "paragraph");
        assert_eq!(spec.content, Some("inline*".to_string()));
        assert_eq!(spec.group, Some("block".to_string()));
        assert!(!spec.inline);
        assert!(!spec.atom);
        assert!(spec.is_block());
    }

    #[test]
    fn inline_constructor_defaults() {
        let spec = NodeSpec::inline("text");
        assert_eq!(spec.name, "text");
        assert!(spec.content.is_none());
        assert_eq!(spec.group, Some("inline".to_string()));
        assert!(spec.inline);
        assert!(!spec.atom);
        assert!(!spec.is_block());
    }

    #[test]
    fn atom_constructor_defaults() {
        let spec = NodeSpec::atom("horizontal_rule");
        assert_eq!(spec.name, "horizontal_rule");
        assert!(spec.content.is_none());
        assert!(spec.atom);
        matches!(spec.marks, MarkSet::None);
    }

    #[test]
    fn builder_works() {
        let spec = NodeSpec::builder("heading")
            .content("inline*")
            .group("block")
            .attr("level", AttrSpec::optional(AttrValue::Int(1)))
            .parse_html(vec!["h1".into(), "h2".into()])
            .build();

        assert_eq!(spec.name, "heading");
        assert_eq!(spec.content, Some("inline*".to_string()));
        assert_eq!(spec.group, Some("block".to_string()));
        assert!(spec.attrs.contains_key("level"));
        assert_eq!(spec.attrs["level"].default, Some(AttrValue::Int(1)));
        assert_eq!(spec.parse_html_tags.len(), 2);
    }

    #[test]
    fn markset_all_allows_everything() {
        assert!(MarkSet::All.allows("bold"));
        assert!(MarkSet::All.allows("italic"));
    }

    #[test]
    fn markset_none_allows_nothing() {
        assert!(!MarkSet::None.allows("bold"));
        assert!(!MarkSet::None.allows("italic"));
    }

    #[test]
    fn markset_only_allows_specified() {
        let set = MarkSet::Only(vec!["bold".into(), "italic".into()]);
        assert!(set.allows("bold"));
        assert!(set.allows("italic"));
        assert!(!set.allows("underline"));
    }

    #[test]
    fn markset_except_excludes_specified() {
        let set = MarkSet::Except(vec!["code".into()]);
        assert!(set.allows("bold"));
        assert!(!set.allows("code"));
    }

    #[test]
    fn attrspec_optional_has_default() {
        let attr = AttrSpec::optional(AttrValue::Int(1));
        assert_eq!(attr.default, Some(AttrValue::Int(1)));
        assert!(!attr.required);
        // string defaults still work via the `Into<AttrValue>` bound
        let s = AttrSpec::optional("");
        assert_eq!(s.default, Some(AttrValue::Str("".into())));
    }

    #[test]
    fn attrspec_required_has_no_default() {
        let attr = AttrSpec::required();
        assert!(attr.default.is_none());
        assert!(attr.required);
    }
}
