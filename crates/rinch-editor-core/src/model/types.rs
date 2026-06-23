//! Interned node-type and mark-type handles.
//!
//! A [`NodeType`] / [`MarkType`] is a cheap, shared handle (an `Rc`) owned by the
//! [`Schema`](crate::Schema). Every `Node`/`Mark` of a given type holds a *clone*
//! of the same handle, so:
//!
//! - equality and hashing are pointer-identity (`Rc::ptr_eq`) — O(1), no string
//!   compare (amendment A12: the view diff leans on cheap type/`ptr_eq` checks);
//! - the type's spec and compiled [`ContentMatch`] are reachable from any node
//!   without a schema lookup.
//!
//! Handles from two *different* `Schema` instances are never equal, which is the
//! correct behavior — you cannot mix nodes across schemas.

use crate::EditorError;
use crate::model::{AttrValue, Attrs};
use crate::schema::content_match::ContentMatch;
use crate::schema::{AttrSpec, MarkSpec, NodeSpec};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Compute the full, typed attribute set for a node/mark of the given spec from a
/// (possibly partial) set of `provided` attributes — the port of ProseMirror's
/// `computeAttrs`. For each attribute the type *declares*:
///
/// - if `provided` carries it, keep that value;
/// - else if the spec has a default, fill the default;
/// - else (a required attribute with no default) return
///   [`EditorError::SchemaValidation`].
///
/// Attributes present in `provided` but **not declared** by the spec are dropped
/// (PM behavior) — a node never carries undeclared attrs, which keeps the model,
/// equality, and serialization clean. The result is total (every declared attr is
/// present) and deterministic (keys are ordered by [`Attrs`]'s `BTreeMap`), and
/// the function is idempotent: `compute(compute(x)) == compute(x)`.
fn compute_attrs(
    specs: &BTreeMap<String, AttrSpec>,
    provided: &Attrs,
    kind: &str,
    type_name: &str,
) -> Result<Attrs, EditorError> {
    if specs.is_empty() {
        return Ok(Attrs::new());
    }
    let mut pairs: Vec<(Box<str>, AttrValue)> = Vec::with_capacity(specs.len());
    for (name, spec) in specs {
        if let Some(v) = provided.get(name) {
            pairs.push((name.as_str().into(), v.clone()));
        } else if let Some(default) = &spec.default {
            pairs.push((name.as_str().into(), default.clone()));
        } else {
            return Err(EditorError::SchemaValidation(format!(
                "missing required attribute '{name}' on {kind} '{type_name}'"
            )));
        }
    }
    Ok(Attrs::from_iter(pairs))
}

struct NodeTypeInner {
    name: Box<str>,
    spec: NodeSpec,
    content_match: ContentMatch,
    is_text: bool,
}

/// A handle to a node type within a schema. Cheap to clone and compare.
#[derive(Clone)]
pub struct NodeType(Rc<NodeTypeInner>);

impl NodeType {
    pub(crate) fn new(name: Box<str>, spec: NodeSpec, content_match: ContentMatch) -> Self {
        let is_text = &*name == "text";
        NodeType(Rc::new(NodeTypeInner {
            name,
            spec,
            content_match,
            is_text,
        }))
    }

    /// The node-type name (e.g. `"paragraph"`).
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The node-type specification.
    pub fn spec(&self) -> &NodeSpec {
        &self.0.spec
    }

    /// The compiled content expression for this type.
    pub fn content_match(&self) -> &ContentMatch {
        &self.0.content_match
    }

    /// True for the special `text` node type.
    pub fn is_text(&self) -> bool {
        self.0.is_text
    }

    /// True for inline node types (`text`, `image`, `hard_break`).
    pub fn is_inline(&self) -> bool {
        self.0.spec.inline
    }

    /// True for block-level node types.
    pub fn is_block(&self) -> bool {
        !self.0.spec.inline
    }

    /// True for atom node types (rendered as a single opaque unit: `hr`, `image`).
    pub fn is_atom(&self) -> bool {
        self.0.spec.atom
    }

    /// A leaf type permits no children. Defined (PM-faithfully) as "the content
    /// match accepts nothing", which makes it the single source of truth shared
    /// with `Node::node_size`: a type with an absent or empty content expression
    /// is a leaf (`text`, `hard_break`, `horizontal_rule`, `image`), one with a
    /// real content expression is not. It governs the node's position-space size.
    pub fn is_leaf(&self) -> bool {
        self.0.content_match.is_leaf()
    }

    /// The group this type belongs to (`"block"` / `"inline"`), if any.
    pub fn group(&self) -> Option<&str> {
        self.0.spec.group.as_deref()
    }

    /// Whether content of this type may be joined with content of `other` — true
    /// for identical types, or when their content expressions share at least one
    /// acceptable child type. Used by the replace algorithm when merging the open
    /// ancestors of two positions. Port of `NodeType.compatibleContent`.
    pub fn compatible_content(&self, other: &NodeType) -> bool {
        self == other || self.0.content_match.compatible(&other.0.content_match)
    }

    /// A *textblock* is a block-level type whose content is inline (paragraph,
    /// heading, code_block) — the unit that `set_block_type`, `wrap`, and `lift`
    /// operate over. Approximates ProseMirror's `isTextblock`
    /// (`isBlock && inlineContent`) via "accepts a text child".
    pub fn is_textblock(&self) -> bool {
        self.is_block() && self.0.content_match.accepts("text")
    }

    /// Fill defaults and validate required attributes for this node type, given a
    /// (possibly partial) `provided` attribute set. See [`compute_attrs`]. Used by
    /// serialization (M3) and, in M4, by `Step::apply`.
    pub fn compute_attrs(&self, provided: &Attrs) -> Result<Attrs, EditorError> {
        compute_attrs(&self.0.spec.attrs, provided, "node", &self.0.name)
    }
}

impl PartialEq for NodeType {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NodeType {}
impl Hash for NodeType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as usize).hash(state);
    }
}
impl std::fmt::Debug for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeType({})", self.0.name)
    }
}

struct MarkTypeInner {
    name: Box<str>,
    spec: MarkSpec,
}

/// A handle to a mark type within a schema. Cheap to clone and compare.
#[derive(Clone)]
pub struct MarkType(Rc<MarkTypeInner>);

impl MarkType {
    pub(crate) fn new(name: Box<str>, spec: MarkSpec) -> Self {
        MarkType(Rc::new(MarkTypeInner { name, spec }))
    }

    /// The mark-type name (e.g. `"bold"`, `"link"`).
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The mark-type specification.
    pub fn spec(&self) -> &MarkSpec {
        &self.0.spec
    }

    /// Whether this mark excludes another mark type (cannot coexist with it).
    pub fn excludes(&self, other: &str) -> bool {
        self.0.spec.excludes_mark(other)
    }

    /// Fill defaults and validate required attributes for this mark type, given a
    /// (possibly partial) `provided` attribute set. See [`compute_attrs`].
    pub fn compute_attrs(&self, provided: &Attrs) -> Result<Attrs, EditorError> {
        compute_attrs(&self.0.spec.attrs, provided, "mark", &self.0.name)
    }
}

impl PartialEq for MarkType {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for MarkType {}
impl Hash for MarkType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as usize).hash(state);
    }
}
impl std::fmt::Debug for MarkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MarkType({})", self.0.name)
    }
}

#[cfg(test)]
mod tests {
    use crate::Schema;
    use crate::model::{AttrValue, Attrs};

    #[test]
    fn compute_attrs_fills_defaults() {
        let s = Schema::starter_kit();
        // heading.level defaults to Int(1)
        let heading = s.node_type("heading").unwrap();
        let computed = heading.compute_attrs(&Attrs::new()).unwrap();
        assert_eq!(computed.get_int("level"), Some(1));
        // task_item.checked defaults to Bool(false), ordered_list.start to Int(1)
        assert_eq!(
            s.node_type("task_item")
                .unwrap()
                .compute_attrs(&Attrs::new())
                .unwrap()
                .get_bool("checked"),
            Some(false)
        );
        assert_eq!(
            s.node_type("ordered_list")
                .unwrap()
                .compute_attrs(&Attrs::new())
                .unwrap()
                .get_int("start"),
            Some(1)
        );
    }

    #[test]
    fn compute_attrs_keeps_provided_over_default() {
        let s = Schema::starter_kit();
        let provided = Attrs::from_iter([("level", AttrValue::Int(3))]);
        let computed = s
            .node_type("heading")
            .unwrap()
            .compute_attrs(&provided)
            .unwrap();
        assert_eq!(computed.get_int("level"), Some(3));
    }

    #[test]
    fn compute_attrs_errors_on_missing_required() {
        let s = Schema::starter_kit();
        // image.src is required (no default)
        assert!(
            s.node_type("image")
                .unwrap()
                .compute_attrs(&Attrs::new())
                .is_err()
        );
        // link.href is required
        assert!(
            s.mark_type("link")
                .unwrap()
                .compute_attrs(&Attrs::new())
                .is_err()
        );
        // text_color.color is required
        assert!(
            s.mark_type("text_color")
                .unwrap()
                .compute_attrs(&Attrs::new())
                .is_err()
        );
    }

    #[test]
    fn compute_attrs_drops_undeclared_and_is_idempotent() {
        let s = Schema::starter_kit();
        let heading = s.node_type("heading").unwrap();
        // an undeclared attr is dropped; the declared one survives
        let provided = Attrs::from_iter([
            ("level", AttrValue::Int(2)),
            ("bogus", AttrValue::from("x")),
        ]);
        let once = heading.compute_attrs(&provided).unwrap();
        assert_eq!(once.get_int("level"), Some(2));
        assert_eq!(once.get("bogus"), None);
        // idempotent: computing again yields the same set
        let twice = heading.compute_attrs(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn compute_attrs_empty_for_attrless_type() {
        let s = Schema::starter_kit();
        // paragraph declares no attrs → empty result even if junk is provided
        let provided = Attrs::from_iter([("junk", AttrValue::Bool(true))]);
        let computed = s
            .node_type("paragraph")
            .unwrap()
            .compute_attrs(&provided)
            .unwrap();
        assert!(computed.is_empty());
    }
}
