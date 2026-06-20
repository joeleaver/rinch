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

use crate::schema::content_match::ContentMatch;
use crate::schema::{MarkSpec, NodeSpec};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

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
