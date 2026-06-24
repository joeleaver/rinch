//! Accessibility — derive an [`accesskit::TreeUpdate`] from an [`EditorState`].
//!
//! A **pure projection** of the document and selection into the platform-neutral
//! AccessKit tree, exactly like the HTML/markdown serializers: state in, tree out,
//! no host or platform deps. The desktop `rinch` crate owns the platform adapter
//! (`accesskit_unix`/`_windows`/`_macos`) and pushes this tree on every transaction;
//! the web view contributes nothing (the browser's contentEditable exposes the
//! native a11y tree). Gated by the `a11y` feature so `accesskit` (pure, `uuid`-only,
//! wasm-clean) is pulled only when wanted.
//!
//! ## Tree shape
//! - The document root → a [`Role::Document`] node.
//! - Each block → a node with a role from [`node_to_role`]; a **textblock**
//!   (paragraph/heading/code block) gets one [`Role::TextRun`] child carrying its
//!   text and per-character byte lengths; a **container** (list/blockquote/table…)
//!   recurses into its children; a **leaf** (image/hr) is a childless node.
//! - The [`EditorState::selection`], when a text selection, becomes an AccessKit
//!   [`TextSelection`] on the root whose endpoints reference the relevant TextRun
//!   nodes and character offsets.
//!
//! Node ids are derived from model document positions so they are stable while the
//! structure is: a block before position `p` → `NodeId(p + 1)` (the root is
//! `NodeId(0)`); a textblock's TextRun → `NodeId(TEXT_RUN_BIT | content_start)`
//! (the high bit keeps text runs disjoint from block ids).

use accesskit::{
    Action, Node as AkNode, NodeId, Role, TextPosition, TextSelection, Tree, TreeId, TreeUpdate,
};

use crate::model::Node;
use crate::pos::Pos;
use crate::selection::Selection;
use crate::state::EditorState;

/// The AccessKit id of the document root.
const ROOT_ID: NodeId = NodeId(0);
/// High bit marking the `NodeId` space of synthetic TextRun nodes, keeping them
/// disjoint from block ids (which are small document positions).
const TEXT_RUN_BIT: u64 = 1 << 62;

/// Map a model node type to its AccessKit [`Role`]. Mirrors the schema-driven tag
/// mapping in `serialize::html` (`node_dom_tag`), but to a11y roles.
pub fn node_to_role(node: &Node) -> Role {
    match node.type_name() {
        "doc" => Role::Document,
        "paragraph" => Role::Paragraph,
        "heading" => Role::Heading,
        "bullet_list" | "ordered_list" | "task_list" => Role::List,
        "list_item" | "task_item" => Role::ListItem,
        "blockquote" => Role::Blockquote,
        "code_block" => Role::Code,
        "table" => Role::Table,
        "table_row" => Role::Row,
        "table_cell" => Role::Cell,
        "table_header_cell" => Role::ColumnHeader,
        "image" => Role::Image,
        "horizontal_rule" => Role::Splitter,
        "hard_break" => Role::LineBreak,
        // An unknown block falls back to a transparent container.
        _ => Role::GenericContainer,
    }
}

/// Build the full [`TreeUpdate`] for `state`: the document tree plus the focused
/// selection. The whole tree is re-derived each call (a snapshot), which the
/// platform adapter pushes as a complete update.
pub fn build_tree_update(state: &EditorState) -> TreeUpdate {
    let doc = &state.doc;
    let mut nodes: Vec<(NodeId, AkNode)> = Vec::new();

    let mut root = AkNode::new(Role::Document);
    let mut pos = 0usize;
    for child in doc.content().children() {
        let cid = build_node(child, pos, &mut nodes);
        root.push_child(cid);
        pos += child.node_size();
    }

    // A text selection becomes an AccessKit TextSelection on the root (the single
    // editable region). A node or cell selection has no text caret to expose.
    if let Selection::Text(t) = &state.selection
        && let (Some(anchor), Some(focus)) = (
            pos_to_text_position(doc, t.anchor),
            pos_to_text_position(doc, t.head),
        )
    {
        root.set_text_selection(TextSelection { anchor, focus });
    }
    // The editor as a whole is the focused element; the AT reads the caret from the
    // text selection above.
    root.add_action(Action::Focus);
    nodes.push((ROOT_ID, root));

    TreeUpdate {
        nodes,
        tree: Some(Tree::new(ROOT_ID)),
        tree_id: TreeId::ROOT,
        focus: ROOT_ID,
    }
}

/// Build the AccessKit node for `node` (whose open token is at document position
/// `before_pos`), pushing it and any descendants into `nodes`, and return its id.
fn build_node(node: &Node, before_pos: usize, nodes: &mut Vec<(NodeId, AkNode)>) -> NodeId {
    let id = NodeId(before_pos as u64 + 1);
    let mut ak = AkNode::new(node_to_role(node));

    if node.is_textblock() {
        // One TextRun child carries the textblock's flattened inline text.
        let content_start = before_pos + 1;
        let (text, lengths) = textblock_text(node);
        let run_id = NodeId(TEXT_RUN_BIT | content_start as u64);
        let mut run = AkNode::new(Role::TextRun);
        run.set_value(text);
        run.set_character_lengths(lengths);
        nodes.push((run_id, run));
        ak.push_child(run_id);
    } else if node.is_leaf() {
        // Image alt text is the accessible label; the horizontal rule needs none.
        if node.type_name() == "image"
            && let Some(alt) = node.attrs().get_str("alt").filter(|s| !s.is_empty())
        {
            ak.set_label(alt.to_string());
        }
    } else {
        // Container: recurse, tracking each child's document position.
        let mut child_pos = before_pos + 1;
        for child in node.content().children() {
            let cid = build_node(child, child_pos, nodes);
            ak.push_child(cid);
            child_pos += child.node_size();
        }
    }

    nodes.push((id, ak));
    id
}

/// The flattened text of a textblock plus the byte length of each model character,
/// so `character_lengths.len()` equals the textblock's model character count (one
/// entry per text char and one per inline leaf) and `sum(lengths) == text.len()`.
/// Inline leaves render as a placeholder so caret offsets stay aligned with the
/// model position space (a hard break → `\n`, an image → a space).
fn textblock_text(block: &Node) -> (String, Vec<u8>) {
    let mut text = String::new();
    let mut lengths: Vec<u8> = Vec::new();
    for child in block.content().children() {
        if let Some(t) = child.text() {
            for c in t.chars() {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                text.push_str(s);
                lengths.push(s.len() as u8);
            }
        } else {
            let ch = if child.type_name() == "hard_break" {
                '\n'
            } else {
                ' '
            };
            text.push(ch);
            lengths.push(1);
        }
    }
    (text, lengths)
}

/// Map an AccessKit [`accesskit::TextSelection`] (e.g. from an AT
/// `Action::SetTextSelection`) back to a model [`Selection`] — the inverse of the
/// TextRun/character-index projection in [`build_tree_update`]. `None` if the
/// endpoints don't reference this document's text runs. Lets a screen reader move
/// the caret/selection.
pub fn accesskit_selection_to_model(
    doc: &Node,
    sel: &accesskit::TextSelection,
) -> Option<Selection> {
    let anchor = text_position_to_pos(doc, &sel.anchor)?;
    let head = text_position_to_pos(doc, &sel.focus)?;
    Some(Selection::text(anchor, head))
}

/// Map an AccessKit [`TextPosition`] back to a model [`Pos`]. The TextRun node id
/// encodes the enclosing textblock's content-start position; the character index is
/// the offset within it. `None` unless it resolves inside a textblock.
fn text_position_to_pos(doc: &Node, tp: &TextPosition) -> Option<Pos> {
    if tp.node.0 & TEXT_RUN_BIT == 0 {
        return None; // not one of our synthetic text runs
    }
    let content_start = (tp.node.0 & !TEXT_RUN_BIT) as usize;
    let pos = Pos(content_start + tp.character_index);
    let r = doc.resolve(pos).ok()?;
    r.parent().is_textblock().then_some(pos)
}

/// Map a model [`Pos`] to an AccessKit [`TextPosition`] (the TextRun node of the
/// enclosing textblock plus the character offset within it). `None` if `pos` is not
/// inside a textblock (e.g. between blocks).
fn pos_to_text_position(doc: &Node, pos: Pos) -> Option<TextPosition> {
    let r = doc.resolve(pos).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let content_start = pos.0 - r.parent_offset();
    Some(TextPosition {
        node: NodeId(TEXT_RUN_BIT | content_start as u64),
        character_index: r.parent_offset(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::model::{Attrs, Fragment};

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
    fn state(s: Schema, doc: Node) -> EditorState {
        EditorState::create(std::rc::Rc::new(s), doc, vec![])
    }
    /// Look up a node in the update by id.
    fn find(u: &TreeUpdate, id: NodeId) -> &AkNode {
        &u.nodes.iter().find(|(nid, _)| *nid == id).unwrap().1
    }

    #[test]
    fn paragraph_becomes_document_paragraph_textrun() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "hi")]);
        let st = state(s, d);
        let u = build_tree_update(&st);
        let root = find(&u, ROOT_ID);
        assert_eq!(root.role(), Role::Document);
        assert_eq!(root.children().len(), 1);
        let p = find(&u, root.children()[0]);
        assert_eq!(p.role(), Role::Paragraph);
        assert_eq!(p.children().len(), 1, "paragraph holds one text run");
        let run = find(&u, p.children()[0]);
        assert_eq!(run.role(), Role::TextRun);
        assert_eq!(run.value(), Some("hi"));
        assert_eq!(run.character_lengths(), &[1, 1]);
    }

    #[test]
    fn role_mapping_covers_block_types() {
        let s = sk();
        let heading = s
            .create_node(
                "heading",
                Attrs::from_iter([("level", crate::AttrValue::Int(2))]),
                Fragment::from_node(s.text("Title").unwrap()),
            )
            .unwrap();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let d = doc(&s, vec![heading, hr]);
        let st = state(s, d);
        let u = build_tree_update(&st);
        let root = find(&u, ROOT_ID);
        let roles: Vec<Role> = root
            .children()
            .iter()
            .map(|c| find(&u, *c).role())
            .collect();
        assert_eq!(roles, vec![Role::Heading, Role::Splitter]);
    }

    #[test]
    fn nested_list_nests_roles() {
        let s = sk();
        let li = s
            .branch("list_item", Fragment::from_node(para(&s, "item")))
            .unwrap();
        let ul = s.branch("bullet_list", Fragment::from_node(li)).unwrap();
        let d = doc(&s, vec![ul]);
        let st = state(s, d);
        let u = build_tree_update(&st);
        let root = find(&u, ROOT_ID);
        let list = find(&u, root.children()[0]);
        assert_eq!(list.role(), Role::List);
        let item = find(&u, list.children()[0]);
        assert_eq!(item.role(), Role::ListItem);
        let p = find(&u, item.children()[0]);
        assert_eq!(p.role(), Role::Paragraph);
    }

    #[test]
    fn table_roles() {
        let s = sk();
        let table = crate::commands::build_table(&s, 1, 2).unwrap();
        let d = doc(&s, vec![table]);
        let st = state(s, d);
        let u = build_tree_update(&st);
        let root = find(&u, ROOT_ID);
        let t = find(&u, root.children()[0]);
        assert_eq!(t.role(), Role::Table);
        let row = find(&u, t.children()[0]);
        assert_eq!(row.role(), Role::Row);
        assert_eq!(row.children().len(), 2, "two cells");
        assert_eq!(find(&u, row.children()[0]).role(), Role::Cell);
    }

    #[test]
    fn text_cursor_becomes_text_selection() {
        // doc(paragraph "ab"): positions 0[p 1 a 2 b 3]4. Cursor at Pos(2) (a|b).
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        let mut st = state(s, d);
        st.selection = Selection::cursor(Pos(2));
        let u = build_tree_update(&st);
        let root = find(&u, ROOT_ID);
        let sel = root.text_selection().expect("a text selection");
        // The paragraph's content starts at pos 1, so its TextRun id is TEXT_RUN_BIT|1.
        let run_id = NodeId(TEXT_RUN_BIT | 1);
        assert_eq!(sel.anchor.node, run_id);
        assert_eq!(sel.focus.node, run_id);
        // Cursor after the first char → character index 1.
        assert_eq!(sel.anchor.character_index, 1);
        assert_eq!(sel.focus.character_index, 1);
        // The referenced node really is a TextRun whose char count fits the index.
        let run = find(&u, run_id);
        assert_eq!(run.role(), Role::TextRun);
        assert!(sel.focus.character_index <= run.character_lengths().len());
    }

    #[test]
    fn range_selection_anchor_and_focus_differ() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "abcd")]);
        let mut st = state(s, d);
        st.selection = Selection::text(Pos(1), Pos(3)); // select "ab"
        let u = build_tree_update(&st);
        let sel = find(&u, ROOT_ID).text_selection().unwrap();
        assert_eq!(sel.anchor.character_index, 0);
        assert_eq!(sel.focus.character_index, 2);
    }

    #[test]
    fn selection_round_trips_through_accesskit() {
        // A range selection projected to an AccessKit TextSelection and mapped back
        // recovers the original model selection (the AT caret-move path).
        let s = sk();
        let d = doc(&s, vec![para(&s, "hello"), para(&s, "world")]);
        let mut st = state(s, d);
        // Select from inside the first paragraph to inside the second.
        let original = Selection::text(Pos(2), Pos(9));
        st.selection = original.clone();
        let u = build_tree_update(&st);
        let ak_sel = find(&u, ROOT_ID).text_selection().unwrap();
        let back = accesskit_selection_to_model(&st.doc, ak_sel).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn node_selection_has_no_text_selection() {
        // doc(paragraph "a", hr) with the hr node-selected → no caret to expose.
        let s = sk();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let d = doc(&s, vec![para(&s, "a"), hr]);
        let mut st = state(s, d);
        // hr starts at pos 3 (0[p 1 a 2]3 <hr 3..4> 4).
        st.selection = Selection::node_at(&st.doc, Pos(3)).expect("hr selectable");
        let u = build_tree_update(&st);
        assert!(find(&u, ROOT_ID).text_selection().is_none());
    }
}
