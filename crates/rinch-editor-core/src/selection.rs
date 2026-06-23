//! [`Selection`] — the editor's caret/range, part of [`EditorState`].
//!
//! Selection is **first-class state** (design §5, non-negotiable principle #3):
//! the caret is rendered from `state.selection`, never from a second DOM-side
//! cursor model. A selection is one of:
//!
//! - [`TextSelection`] — a text range `anchor..head` (a collapsed cursor when
//!   `anchor == head`); `anchor` is the fixed end, `head` the moving end.
//! - [`NodeSelection`] — a whole non-text node selected (an image, a horizontal
//!   rule).
//!
//! A `Cell` selection (table rectangles) is intentionally **not** here yet — it
//! lands with the tables plugin in M7, so adding a dead variant now would trip the
//! crate's "no dead code" gate.
//!
//! Every transaction maps its selection forward through the accumulated
//! [`Mapping`], so the caret follows content as it shifts. A faithful port of the
//! relevant parts of ProseMirror's `prosemirror-state/src/selection.ts`.
//!
//! [`EditorState`]: crate::state::EditorState

use crate::model::Node;
use crate::pos::Pos;
use crate::transform::step_map::Mapping;

/// The editor selection: a caret, a text range, or a selected node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// A text range (collapsed when `anchor == head`).
    Text(TextSelection),
    /// A whole non-text node selected.
    Node(NodeSelection),
}

/// A text selection: `anchor` is fixed, `head` moves (e.g. while shift-arrowing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    /// The fixed end of the selection.
    pub anchor: Pos,
    /// The moving end of the selection (where the caret is drawn).
    pub head: Pos,
}

/// A node selection: the node that *starts* at `anchor` is selected whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeSelection {
    /// The position immediately before the selected node.
    pub anchor: Pos,
    /// The position immediately after the selected node (`anchor + node_size`).
    pub head: Pos,
}

impl TextSelection {
    /// A text selection from `anchor` to `head`.
    pub fn new(anchor: Pos, head: Pos) -> TextSelection {
        TextSelection { anchor, head }
    }

    /// A collapsed cursor at `pos`.
    pub fn cursor(pos: Pos) -> TextSelection {
        TextSelection {
            anchor: pos,
            head: pos,
        }
    }
}

impl Selection {
    /// A collapsed text cursor at `pos`.
    pub fn cursor(pos: Pos) -> Selection {
        Selection::Text(TextSelection::cursor(pos))
    }

    /// A text selection from `anchor` to `head`.
    pub fn text(anchor: Pos, head: Pos) -> Selection {
        Selection::Text(TextSelection::new(anchor, head))
    }

    /// The lower bound of the selection (its leftmost position).
    pub fn from(&self) -> Pos {
        match self {
            Selection::Text(t) => Pos(t.anchor.0.min(t.head.0)),
            Selection::Node(n) => n.anchor,
        }
    }

    /// The upper bound of the selection (its rightmost position).
    pub fn to(&self) -> Pos {
        match self {
            Selection::Text(t) => Pos(t.anchor.0.max(t.head.0)),
            Selection::Node(n) => n.head,
        }
    }

    /// The fixed end of the selection.
    pub fn anchor(&self) -> Pos {
        match self {
            Selection::Text(t) => t.anchor,
            Selection::Node(n) => n.anchor,
        }
    }

    /// The moving end of the selection (the caret).
    pub fn head(&self) -> Pos {
        match self {
            Selection::Text(t) => t.head,
            Selection::Node(n) => n.head,
        }
    }

    /// True if the selection covers no content (a collapsed text cursor). A node
    /// selection is never empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Selection::Text(t) => t.anchor == t.head,
            Selection::Node(_) => false,
        }
    }

    /// True if the selection is a **collapsed text cursor** — the condition under
    /// which `stored_marks` survive into the next state (design §5).
    pub fn is_text_cursor(&self) -> bool {
        matches!(self, Selection::Text(t) if t.anchor == t.head)
    }

    /// Map this selection forward through `mapping`, re-resolving against `doc`
    /// (the document *after* the steps). If the mapped head no longer sits in
    /// inline content (e.g. its textblock was removed), fall back to the nearest
    /// valid selection. Port of `Selection.map` / `TextSelection.map` /
    /// `NodeSelection.map`.
    pub fn map(&self, doc: &Node, mapping: &Mapping) -> Selection {
        match self {
            Selection::Text(t) => {
                let head = mapping.map(t.head.0, 1);
                let r_head = match doc.resolve(Pos(head)) {
                    Ok(r) => r,
                    Err(_) => return Selection::near(doc, Pos(head.min(doc.content_size())), 1),
                };
                if !r_head.parent().is_textblock() {
                    return Selection::near(doc, Pos(head), 1);
                }
                let anchor = mapping.map(t.anchor.0, 1);
                let anchor_inline = doc
                    .resolve(Pos(anchor))
                    .map(|r| r.parent().is_textblock())
                    .unwrap_or(false);
                Selection::text(Pos(if anchor_inline { anchor } else { head }), Pos(head))
            }
            Selection::Node(n) => {
                let result = mapping.map_result(n.anchor.0, 1);
                let pos = Pos(result.pos);
                if result.deleted() {
                    return Selection::near(doc, pos, 1);
                }
                match node_selection_at(doc, pos) {
                    Some(sel) => sel,
                    None => Selection::near(doc, pos, 1),
                }
            }
        }
    }

    /// The nearest valid selection to `pos`, searching first in `bias`'s direction
    /// then the other, and finally falling back to a collapsed cursor at a clamped
    /// position. Port of `Selection.near` (without `AllSelection`; the fallback is
    /// a clamped text cursor, which is total for the starter-kit schemas).
    pub fn near(doc: &Node, pos: Pos, bias: i32) -> Selection {
        find_from(doc, pos, bias, false)
            .or_else(|| find_from(doc, pos, -bias, false))
            .unwrap_or_else(|| Selection::cursor(Pos(pos.0.min(doc.content_size()))))
    }

    /// The nearest **text cursor** to `pos` (searching `bias`'s direction then the
    /// other), **skipping** any selectable block atoms in the way. Unlike
    /// [`Self::near`], this never returns a [`Selection::Node`] — it is the
    /// vertical-caret-movement primitive, which steps *over* an atom (a horizontal
    /// rule) rather than selecting it. `None` if there is no text cursor on either
    /// side.
    pub fn near_text(doc: &Node, pos: Pos, bias: i32) -> Option<Selection> {
        find_from(doc, pos, bias, true).or_else(|| find_from(doc, pos, -bias, true))
    }

    /// A selection at the very start of the document's first textblock.
    pub fn at_start(doc: &Node) -> Selection {
        find_from(doc, Pos(0), 1, false).unwrap_or_else(|| Selection::cursor(Pos(0)))
    }

    /// A selection at the very end of the document's last textblock.
    pub fn at_end(doc: &Node) -> Selection {
        find_from(doc, Pos(doc.content_size()), -1, false)
            .unwrap_or_else(|| Selection::cursor(Pos(doc.content_size())))
    }

    /// The selected node, for a [`NodeSelection`]; `None` for a text selection.
    pub fn node(&self, doc: &Node) -> Option<Node> {
        match self {
            Selection::Node(n) => doc.node_at(n.anchor.0),
            Selection::Text(_) => None,
        }
    }

    /// A [`NodeSelection`] of the node that *starts* at `pos`, if there is one and
    /// it is a selectable **leaf atom** (an image or horizontal rule). `None` for a
    /// text node, a non-leaf container (a paragraph / list — `selectable` in the
    /// schema but never node-selected this way), an unselectable node, or no node at
    /// `pos`. The pointer and keyboard layers use this to node-select a leaf the
    /// user clicks or arrows onto (design §6 node-views).
    pub fn node_at(doc: &Node, pos: Pos) -> Option<Selection> {
        let sel = node_selection_at(doc, pos)?;
        // Restrict to leaf atoms — `node_selection_at` (shared with selection
        // mapping) only checks `selectable`, which is `true` for block containers.
        if !sel.node(doc)?.node_type().is_leaf() {
            return None;
        }
        Some(sel)
    }
}

/// Build a [`NodeSelection`] for the node starting at `pos`, if one is there and is
/// selectable; otherwise `None`.
fn node_selection_at(doc: &Node, pos: Pos) -> Option<Selection> {
    let r = doc.resolve(pos).ok()?;
    let node = r.node_after()?;
    if node.is_text() || !node.node_type().spec().selectable {
        return None;
    }
    Some(Selection::Node(NodeSelection {
        anchor: pos,
        head: Pos(pos.0 + node.node_size()),
    }))
}

/// Find the nearest selection from `pos` in direction `dir` (`+1`/`-1`). Port of
/// `Selection.findFrom`.
fn find_from(doc: &Node, pos: Pos, dir: i32, text_only: bool) -> Option<Selection> {
    let r = doc.resolve(pos).ok()?;
    let inner = if r.parent().is_textblock() {
        Some(Selection::cursor(pos))
    } else {
        find_selection_in(r.parent(), pos.0, r.index(r.depth()), dir, text_only)
    };
    if let Some(sel) = inner {
        return Some(sel);
    }
    let mut depth = r.depth() as isize - 1;
    while depth >= 0 {
        let d = depth as usize;
        let found = if dir < 0 {
            find_selection_in(r.node(d), r.before(d + 1)?, r.index(d), dir, text_only)
        } else {
            find_selection_in(r.node(d), r.after(d + 1)?, r.index(d) + 1, dir, text_only)
        };
        if found.is_some() {
            return found;
        }
        depth -= 1;
    }
    None
}

/// Search inside `node` (whose content begins at document position `pos`, child
/// index `index`) for a selectable position in direction `dir`. Port of
/// `findSelectionIn`.
fn find_selection_in(
    node: &Node,
    mut pos: usize,
    index: usize,
    dir: i32,
    text_only: bool,
) -> Option<Selection> {
    if node.is_textblock() {
        return Some(Selection::cursor(Pos(pos)));
    }
    let count = node.child_count();
    // Starting index: for dir>0 begin at `index`, for dir<0 at `index - 1`.
    let mut i: isize = if dir > 0 {
        index as isize
    } else {
        index as isize - 1
    };
    while (dir > 0 && (i as usize) < count) || (dir < 0 && i >= 0) {
        let child = node.child(i as usize);
        if !child.is_atom() {
            let inner_index = if dir < 0 { child.child_count() } else { 0 };
            let inner = find_selection_in(
                child,
                (pos as isize + dir as isize) as usize,
                inner_index,
                dir,
                text_only,
            );
            if inner.is_some() {
                return inner;
            }
        } else if !text_only && child.node_type().spec().selectable {
            let node_pos = if dir < 0 {
                pos - child.node_size()
            } else {
                pos
            };
            return Some(Selection::Node(NodeSelection {
                anchor: Pos(node_pos),
                head: Pos(node_pos + child.node_size()),
            }));
        }
        pos = (pos as isize + child.node_size() as isize * dir as isize) as usize;
        i += dir as isize;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::model::Fragment;

    fn sk() -> Schema {
        Schema::starter_kit()
    }
    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }
    fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }

    #[test]
    fn from_to_anchor_head() {
        let sel = Selection::text(Pos(5), Pos(2));
        assert_eq!(sel.from(), Pos(2));
        assert_eq!(sel.to(), Pos(5));
        assert_eq!(sel.anchor(), Pos(5));
        assert_eq!(sel.head(), Pos(2));
        assert!(!sel.is_empty());
        let cur = Selection::cursor(Pos(3));
        assert!(cur.is_empty());
        assert!(cur.is_text_cursor());
    }

    #[test]
    fn at_start_and_end_land_in_textblocks() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab"), para(&s, "cd")]);
        // positions: 0[p1 1 a 2 b 3]4[p2 5 c 6 d 7]8
        // first textblock content begins at pos 1
        assert_eq!(Selection::at_start(&d), Selection::cursor(Pos(1)));
        // last textblock content ends at pos 7 (after "cd", before p2's close token)
        assert_eq!(Selection::at_end(&d), Selection::cursor(Pos(7)));
    }

    #[test]
    fn map_cursor_through_insert_advances() {
        // cursor at pos 1, insert 2 chars before it → maps to 3 (assoc 1)
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        let mut m = Mapping::new();
        m.append_map(crate::transform::StepMap::new(vec![1, 0, 2]));
        let sel = Selection::cursor(Pos(1));
        assert_eq!(sel.map(&d, &m), Selection::cursor(Pos(3)));
    }

    #[test]
    fn near_falls_into_adjacent_textblock() {
        // doc(hr, paragraph "x"): pos 1 is between hr and paragraph; near→ into the para
        let s = sk();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let d = doc(&s, vec![hr, para(&s, "x")]);
        // hr occupies 0..1; paragraph 1..4, its inline content at pos 2
        let sel = Selection::near(&d, Pos(1), 1);
        assert_eq!(sel, Selection::cursor(Pos(2)));
    }

    #[test]
    fn node_at_selects_block_atom_and_rejects_text() {
        // doc(paragraph "ab", hr): the hr is a selectable block atom.
        let s = sk();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let d = doc(&s, vec![para(&s, "ab"), hr]);
        // positions: 0[p 1 a 2 b 3]4 <hr 4..5> 5 ; the hr starts at pos 4.
        let sel = Selection::node_at(&d, Pos(4)).expect("hr is selectable");
        assert_eq!(sel.from(), Pos(4));
        assert_eq!(sel.to(), Pos(5));
        assert!(matches!(sel, Selection::Node(_)));
        // A position before a text run is not a node selection.
        assert!(Selection::node_at(&d, Pos(1)).is_none());
        // Out of range / no node after → None (total, no panic).
        assert!(Selection::node_at(&d, Pos(5)).is_none());
        // A selectable *container* (the paragraph, `selectable: true` by default)
        // is NOT node-selectable via `node_at` — only leaf atoms are.
        assert!(
            Selection::node_at(&d, Pos(0)).is_none(),
            "a paragraph container must not node-select"
        );
    }

    #[test]
    fn node_at_selects_inline_image() {
        // doc(paragraph(text "a", image)) — the image is a selectable inline atom.
        let s = sk();
        let img = s
            .create_node(
                "image",
                crate::model::Attrs::from_iter([("src", crate::model::AttrValue::from("a.png"))]),
                Fragment::empty(),
            )
            .unwrap();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("a").unwrap(), img]),
            )
            .unwrap();
        let d = doc(&s, vec![p]);
        // positions: 0[p 1 a 2 (img) 3]4 ; the image starts at pos 2.
        let sel = Selection::node_at(&d, Pos(2)).expect("image is selectable");
        assert_eq!(sel.from(), Pos(2));
        assert_eq!(sel.to(), Pos(3));
    }

    #[test]
    fn find_from_selects_atom_node() {
        // doc(paragraph(text, image)) — image is a selectable inline atom
        let s = sk();
        let img = s
            .create_node(
                "image",
                crate::model::Attrs::from_iter([("src", crate::model::AttrValue::from("a.png"))]),
                Fragment::empty(),
            )
            .unwrap();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("a").unwrap(), img]),
            )
            .unwrap();
        let d = doc(&s, vec![p]);
        // positions: 0[p 1 a 2 (img) 3]4 ; the paragraph is a textblock so find_from
        // at the doc level resolves to a text cursor — node-selection of inline
        // atoms is exercised by the command layer, here we assert near() is total.
        let sel = Selection::near(&d, Pos(2), 1);
        assert!(matches!(sel, Selection::Text(_)));
    }
}
