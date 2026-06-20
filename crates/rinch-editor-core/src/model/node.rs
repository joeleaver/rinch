//! The [`Node`] value type: a single node in the persistent document tree.
//!
//! A node is an `Rc`-shared, immutable value. It is either a **text** node
//! (carrying a string + marks) or a **non-text** node (carrying typed attrs and a
//! [`Fragment`] of children). Every edit produces a *new* tree that structurally
//! shares unchanged subtrees with the old one, so cloning a `Node` is a refcount
//! bump and the view can diff two trees by `Rc::ptr_eq` (amendment A12).
//!
//! ## Position space
//!
//! There is exactly one integer position space over the tree (amendment A2 /
//! §3), measured in **Unicode scalar values (chars)** inside text — never bytes,
//! which designs out the audit's byte/char (S3) bug class. A node's
//! [`Node::node_size`] is:
//!
//! - a **text** node: its length in chars;
//! - a **leaf** non-text node (`hr`, `image` — no `content` expression): 1;
//! - any other node: `2 + content.size()` (an open token + content + a close
//!   token).

use crate::EditorError;
use crate::model::{Attrs, Fragment, Mark, NodeType};
use crate::pos::{Pos, ResolvedPos};
use crate::schema::Schema;
use std::fmt;
use std::rc::Rc;

pub(crate) struct NodeInner {
    typ: NodeType,
    attrs: Attrs,
    content: Fragment,
    marks: Vec<Mark>,
    text: Option<Box<str>>,
    /// Length of `text` in chars (0 for non-text). Cached so `node_size` is O(1).
    text_len: usize,
}

/// A node in the document tree.
#[derive(Clone)]
pub struct Node(Rc<NodeInner>);

impl Node {
    /// Construct a non-text (branch or atom) node.
    pub(crate) fn new_branch(typ: NodeType, attrs: Attrs, content: Fragment) -> Node {
        debug_assert!(!typ.is_text(), "use new_text for text nodes");
        Node(Rc::new(NodeInner {
            typ,
            attrs,
            content,
            marks: Vec::new(),
            text: None,
            text_len: 0,
        }))
    }

    /// Construct a text node with the given marks.
    pub(crate) fn new_text(typ: NodeType, text: Box<str>, marks: Vec<Mark>) -> Node {
        debug_assert!(typ.is_text(), "new_text requires the `text` node type");
        let text_len = text.chars().count();
        Node(Rc::new(NodeInner {
            typ,
            attrs: Attrs::new(),
            content: Fragment::empty(),
            marks,
            text: Some(text),
            text_len,
        }))
    }

    /// The node's type handle.
    pub fn node_type(&self) -> &NodeType {
        &self.0.typ
    }

    /// The node's type name (e.g. `"paragraph"`, `"text"`).
    pub fn type_name(&self) -> &str {
        self.0.typ.name()
    }

    /// The node's typed attributes.
    pub fn attrs(&self) -> &Attrs {
        &self.0.attrs
    }

    /// The node's content (children).
    pub fn content(&self) -> &Fragment {
        &self.0.content
    }

    /// The marks applied to this (inline/text) node.
    pub fn marks(&self) -> &[Mark] {
        &self.0.marks
    }

    /// The text of a text node, or `None`.
    pub fn text(&self) -> Option<&str> {
        self.0.text.as_deref()
    }

    /// True if this is a text node.
    pub fn is_text(&self) -> bool {
        self.0.text.is_some()
    }

    /// True for inline node types.
    pub fn is_inline(&self) -> bool {
        self.0.typ.is_inline()
    }

    /// True for block node types.
    pub fn is_block(&self) -> bool {
        self.0.typ.is_block()
    }

    /// True if this type permits no children (text, hr, image, hard_break).
    pub fn is_leaf(&self) -> bool {
        self.0.typ.is_leaf()
    }

    /// True for atom node types (opaque single unit: hr, image).
    pub fn is_atom(&self) -> bool {
        self.0.typ.is_atom()
    }

    /// Number of children.
    pub fn child_count(&self) -> usize {
        self.0.content.child_count()
    }

    /// Child at index `i` (panics if out of range).
    pub fn child(&self, i: usize) -> &Node {
        self.0.content.child(i)
    }

    /// Length of a text node in chars (0 for non-text).
    pub fn text_len(&self) -> usize {
        self.0.text_len
    }

    /// This node's size in the position space (see the module docs).
    pub fn node_size(&self) -> usize {
        if self.is_text() {
            self.0.text_len
        } else if self.0.typ.is_leaf() {
            1
        } else {
            2 + self.0.content.size()
        }
    }

    /// The size of this node's content (0 for leaves).
    pub fn content_size(&self) -> usize {
        self.0.content.size()
    }

    /// Resolve a position (interpreting `self` as the document root) into a
    /// [`ResolvedPos`] giving the path/depth/parent-offset at that position.
    pub fn resolve(&self, pos: Pos) -> Result<ResolvedPos, EditorError> {
        ResolvedPos::resolve(self, pos)
    }

    /// Pointer-identity check — the view-diff fast path.
    pub fn same_ref(&self, other: &Node) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.0.typ == other.0.typ
                && self.0.attrs == other.0.attrs
                && self.0.text == other.0.text
                && self.0.marks == other.0.marks
                && self.0.content == other.0.content)
    }
}
impl Eq for Node {}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(t) = self.text() {
            if self.0.marks.is_empty() {
                write!(f, "text({t:?})")
            } else {
                let names: Vec<&str> = self.0.marks.iter().map(Mark::type_name).collect();
                write!(f, "text({t:?}, marks={names:?})")
            }
        } else if self.0.content.is_empty() {
            write!(f, "{}", self.type_name())
        } else {
            write!(f, "{}({:?})", self.type_name(), self.0.content)
        }
    }
}

/// Document-building factory methods on the schema. These are the only way to
/// mint a [`Node`]: the schema owns the type handles every node references.
///
/// M1 note: `branch`/`node` do **not** validate content against the schema —
/// validation happens at the `Step` boundary in M2. They are for constructing
/// initial/test documents.
impl Schema {
    /// Create a text node (no marks).
    ///
    /// Returns `Err(UnknownNodeType("text"))` if the schema has no `text` node
    /// type — fallible (not a panic) because a custom schema may legitimately omit
    /// it (e.g. a structural-only schema).
    pub fn text(&self, text: &str) -> Result<Node, EditorError> {
        self.text_with_marks(text, Vec::new())
    }

    /// Create a text node carrying `marks`.
    pub fn text_with_marks(&self, text: &str, marks: Vec<Mark>) -> Result<Node, EditorError> {
        let typ = self
            .node_type("text")
            .ok_or_else(|| EditorError::UnknownNodeType("text".to_string()))?
            .clone();
        Ok(Node::new_text(typ, text.into(), marks))
    }

    /// Create a non-text node of `type_name` with `attrs` and `content`.
    ///
    /// Named `create_node` (not `node`) to avoid colliding with
    /// [`Schema::node`], which looks up a [`NodeSpec`].
    pub fn create_node(
        &self,
        type_name: &str,
        attrs: Attrs,
        content: Fragment,
    ) -> Result<Node, EditorError> {
        let typ = self
            .node_type(type_name)
            .ok_or_else(|| EditorError::UnknownNodeType(type_name.to_string()))?
            .clone();
        Ok(Node::new_branch(typ, attrs, content))
    }

    /// Convenience: a non-text node with default (empty) attrs.
    pub fn branch(&self, type_name: &str, content: Fragment) -> Result<Node, EditorError> {
        self.create_node(type_name, Attrs::new(), content)
    }
}

#[cfg(test)]
mod tests {
    use crate::Schema;
    use crate::model::Fragment;
    use crate::pos::Pos;

    /// doc(paragraph("hi"))
    fn simple_doc() -> (Schema, crate::model::Node) {
        let s = Schema::starter_kit();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        (s, doc)
    }

    #[test]
    fn node_sizes() {
        let s = Schema::starter_kit();
        // text "hi" -> 2 chars
        assert_eq!(s.text("hi").unwrap().node_size(), 2);
        // empty paragraph -> open + close = 2
        let empty_p = s.branch("paragraph", Fragment::empty()).unwrap();
        assert_eq!(empty_p.node_size(), 2);
        assert_eq!(empty_p.content_size(), 0);
        // paragraph("hi") -> 2 + 2 = 4
        let p = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        assert_eq!(p.node_size(), 4);
        // hr is a leaf atom -> 1
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        assert_eq!(hr.node_size(), 1);
    }

    #[test]
    fn text_size_counts_chars_not_bytes() {
        let s = Schema::starter_kit();
        // "café" is 5 bytes but 4 chars; "🎉" is 4 bytes, 1 char.
        assert_eq!(s.text("café").unwrap().node_size(), 4);
        assert_eq!(s.text("🎉").unwrap().node_size(), 1);
        assert_eq!(s.text("a🎉b").unwrap().node_size(), 3);
    }

    #[test]
    fn doc_content_size() {
        let (_s, doc) = simple_doc();
        // paragraph occupies 4, doc content size = 4, valid positions 0..=4
        assert_eq!(doc.content_size(), 4);
    }

    #[test]
    fn resolve_top_level_boundaries() {
        let (_s, doc) = simple_doc();
        // pos 0: before the paragraph, at doc depth 0
        let r0 = doc.resolve(Pos(0)).unwrap();
        assert_eq!(r0.depth(), 0);
        assert_eq!(r0.parent().type_name(), "doc");
        assert_eq!(r0.index(0), 0);

        // pos 4: after the paragraph, at doc depth 0
        let r4 = doc.resolve(Pos(4)).unwrap();
        assert_eq!(r4.depth(), 0);
        assert_eq!(r4.parent().type_name(), "doc");
        assert_eq!(r4.index(0), 1); // after the only child
    }

    #[test]
    fn resolve_inside_text() {
        let (_s, doc) = simple_doc();
        // pos 1: inside paragraph, before "h"
        let r1 = doc.resolve(Pos(1)).unwrap();
        assert_eq!(r1.depth(), 1);
        assert_eq!(r1.parent().type_name(), "paragraph");
        assert_eq!(r1.parent_offset(), 0);

        // pos 2: between 'h' and 'i'
        let r2 = doc.resolve(Pos(2)).unwrap();
        assert_eq!(r2.depth(), 1);
        assert_eq!(r2.parent().type_name(), "paragraph");
        assert_eq!(r2.parent_offset(), 1);

        // pos 3: after "hi", still inside paragraph
        let r3 = doc.resolve(Pos(3)).unwrap();
        assert_eq!(r3.depth(), 1);
        assert_eq!(r3.parent().type_name(), "paragraph");
        assert_eq!(r3.parent_offset(), 2);
    }

    #[test]
    fn resolve_positions_and_geometry() {
        let (_s, doc) = simple_doc();
        let r2 = doc.resolve(Pos(2)).unwrap();
        // node at depth 1 is the paragraph; its content starts at doc pos 1
        assert_eq!(r2.start(1), 1);
        assert_eq!(r2.end(1), 3); // content "hi" ends at 3 (before the close token)
        assert_eq!(r2.before(1), Some(0)); // paragraph open token at 0
        assert_eq!(r2.after(1), Some(4)); // paragraph occupies 0..4
        assert_eq!(r2.start(0), 0); // doc content starts at 0
    }

    #[test]
    fn resolve_nested_blockquote() {
        // doc(blockquote(paragraph("hi")))
        let s = Schema::starter_kit();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let bq = s.branch("blockquote", Fragment::from_node(para)).unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
        // blockquote nodeSize = 2 + paragraph(4) = 6; doc content size = 6
        assert_eq!(doc.content_size(), 6);
        // pos 2: inside paragraph before "h" -> depth 2 (doc > blockquote > paragraph)
        let r = doc.resolve(Pos(2)).unwrap();
        assert_eq!(r.depth(), 2);
        assert_eq!(r.node(0).type_name(), "doc");
        assert_eq!(r.node(1).type_name(), "blockquote");
        assert_eq!(r.node(2).type_name(), "paragraph");
        assert_eq!(r.parent_offset(), 0);
        // paragraph content starts at doc pos 2
        assert_eq!(r.start(2), 2);
    }

    #[test]
    fn resolve_two_paragraphs() {
        // doc(paragraph("ab"), paragraph("c"))
        let s = Schema::starter_kit();
        let p1 = s
            .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
            .unwrap();
        let p2 = s
            .branch("paragraph", Fragment::from_node(s.text("c").unwrap()))
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![p1, p2]))
            .unwrap();
        // p1 = 2+2 = 4 (0..4), p2 = 2+1 = 3 (4..7); content size 7
        assert_eq!(doc.content_size(), 7);
        // pos 4 is the boundary between the two paragraphs at doc depth 0
        let r4 = doc.resolve(Pos(4)).unwrap();
        assert_eq!(r4.depth(), 0);
        assert_eq!(r4.index(0), 1); // gap after first paragraph
        // pos 5 is inside the second paragraph, before "c"
        let r5 = doc.resolve(Pos(5)).unwrap();
        assert_eq!(r5.depth(), 1);
        assert_eq!(r5.parent().type_name(), "paragraph");
        assert_eq!(r5.parent_offset(), 0);
    }

    #[test]
    fn resolve_out_of_range_errs() {
        let (_s, doc) = simple_doc();
        assert!(doc.resolve(Pos(5)).is_err()); // content size is 4
    }

    #[test]
    fn ptr_eq_fast_path_and_structural_eq() {
        let s = Schema::starter_kit();
        let a = s.text("hi").unwrap();
        let b = a.clone();
        assert!(a.same_ref(&b)); // clone shares the Rc
        let c = s.text("hi").unwrap();
        assert!(!a.same_ref(&c)); // distinct allocation
        assert_eq!(a, c); // but structurally equal
        assert_ne!(a, s.text("ho").unwrap());
    }

    #[test]
    fn text_node_inside_runs_and_at_seam() {
        // doc(paragraph(text "ab", text "cd")) — two adjacent runs
        let s = Schema::starter_kit();
        let para = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("ab").unwrap(), s.text("cd").unwrap()]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        assert_eq!(doc.content_size(), 6); // paragraph nodeSize 2 + 4

        let tn = |p: usize| {
            doc.resolve(Pos(p))
                .unwrap()
                .text_node()
                .map(|(n, off)| (n.text().unwrap().to_string(), off))
        };
        assert_eq!(tn(1), Some(("ab".into(), 0))); // left edge of first run
        assert_eq!(tn(2), Some(("ab".into(), 1))); // between a and b
        assert_eq!(tn(3), Some(("cd".into(), 0))); // seam -> left edge of next run
        assert_eq!(tn(4), Some(("cd".into(), 1))); // between c and d
        assert_eq!(tn(5), None); // end of last run / end of paragraph
    }

    #[test]
    fn text_node_empty_paragraph_is_none() {
        let s = Schema::starter_kit();
        let para = s.branch("paragraph", Fragment::empty()).unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        // pos 1 is the only interior position of the empty paragraph
        assert!(doc.resolve(Pos(1)).unwrap().text_node().is_none());
    }

    #[test]
    fn resolve_multibyte_nested_boundary() {
        // doc(blockquote(paragraph("héllo"))) — é is multibyte but ONE char
        let s = Schema::starter_kit();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("héllo").unwrap()))
            .unwrap();
        let bq = s.branch("blockquote", Fragment::from_node(para)).unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
        assert_eq!(doc.content_size(), 9); // paragraph 2+5, blockquote 2+7

        // pos 5: inside the text after "hél" (3 chars) -> depth 2, parent_offset 3
        let r = doc.resolve(Pos(5)).unwrap();
        assert_eq!(r.depth(), 2);
        assert_eq!(r.parent_offset(), 3);
        let (n, off) = r.text_node().unwrap();
        assert_eq!((n.text(), off), (Some("héllo"), 3));

        // pos 8: gap after the paragraph, still inside the blockquote
        let g = doc.resolve(Pos(8)).unwrap();
        assert_eq!(g.depth(), 1);
        assert_eq!(g.parent().type_name(), "blockquote");
        assert_eq!(g.index(1), 1);
    }

    #[test]
    fn empty_content_expression_is_a_leaf() {
        // A node declared with content "" accepts no children and is sized as a
        // leaf (size 1). is_leaf is the single source of truth (the content match),
        // so sizing and content-validation agree.
        use crate::NodeSpec;
        let s = Schema::builder()
            .node("doc", NodeSpec::builder("doc").content("block+").build())
            .node(
                "spacer",
                NodeSpec::builder("spacer")
                    .content("")
                    .group("block")
                    .build(),
            )
            .node(
                "text",
                NodeSpec::builder("text").group("inline").inline().build(),
            )
            .build();
        assert!(s.node_type("spacer").unwrap().is_leaf());
        assert_eq!(
            s.branch("spacer", Fragment::empty()).unwrap().node_size(),
            1
        );
    }
}
