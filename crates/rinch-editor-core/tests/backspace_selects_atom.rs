//! Backspace at the start of the block after a block atom (a horizontal rule, an
//! `embed`) **selects** the atom, and Delete at the end of the block before one
//! does too — ProseMirror's `selectNodeBackward` / `selectNodeForward`. A second
//! Backspace or Delete then removes it (the node selection's delete). When the
//! block the caret is in is empty, that block goes and the atom is selected
//! (ProseMirror's `joinBackward` / `joinForward` "delete the empty block, select
//! the atom" branch).
//!
//! Before node views (PR B) both keys did nothing there: an atom has no content
//! to join with, so the join failed and nothing else was tried.

use std::rc::Rc;

use rinch_editor_core::{
    BaseCommandsPlugin, EditorState, Fragment, Node, NodeSelection, Pos, Schema, Selection,
};

fn para(schema: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(schema.text(text).unwrap())
    };
    schema.branch("paragraph", content).unwrap()
}

fn atom(schema: &Schema, name: &str) -> Node {
    schema
        .create_node(name, Default::default(), Fragment::empty())
        .unwrap()
}

fn state(schema: &Rc<Schema>, blocks: Vec<Node>, selection: Selection) -> EditorState {
    let doc = schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap();
    let mut st = EditorState::create(schema.clone(), doc, vec![Rc::new(BaseCommandsPlugin)]);
    st.selection = selection;
    st
}

fn node_sel(anchor: usize) -> Selection {
    Selection::Node(NodeSelection {
        anchor: Pos(anchor),
        head: Pos(anchor + 1),
    })
}

fn types(st: &EditorState) -> Vec<String> {
    st.doc
        .content()
        .children()
        .iter()
        .map(|c| c.type_name().to_string())
        .collect()
}

const ATOMS: [&str; 2] = ["horizontal_rule", "embed"];

/// `p("ab") ATOM p("cd")`: positions 0..4, 4..5, 5..9; "cd" starts at 6.
#[test]
fn backspace_at_the_start_of_the_block_after_an_atom_selects_it() {
    let schema = Rc::new(Schema::starter_kit());
    for name in ATOMS {
        let blocks = vec![
            para(&schema, "ab"),
            atom(&schema, name),
            para(&schema, "cd"),
        ];
        let st = state(&schema, blocks, Selection::cursor(Pos(6)));
        let next = st
            .run("deleteCharBackward")
            .unwrap_or_else(|| panic!("{name}: Backspace acts"));
        assert!(
            next.doc.same_ref(&st.doc),
            "{name}: the document is unchanged"
        );
        assert_eq!(next.selection, node_sel(4), "{name}: the atom is selected");
        // A second Backspace removes it.
        let gone = next
            .run("deleteCharBackward")
            .unwrap_or_else(|| panic!("{name}: delete it"));
        assert_eq!(types(&gone), ["paragraph", "paragraph"], "{name}");
    }
}

#[test]
fn delete_at_the_end_of_the_block_before_an_atom_selects_it() {
    let schema = Rc::new(Schema::starter_kit());
    for name in ATOMS {
        let blocks = vec![
            para(&schema, "ab"),
            atom(&schema, name),
            para(&schema, "cd"),
        ];
        let st = state(&schema, blocks, Selection::cursor(Pos(3)));
        let next = st
            .run("deleteCharForward")
            .unwrap_or_else(|| panic!("{name}: Delete acts"));
        assert!(next.doc.same_ref(&st.doc), "{name}");
        assert_eq!(next.selection, node_sel(4), "{name}");
    }
}

/// An empty block after (before) the atom is removed and the atom selected.
#[test]
fn backspace_in_an_empty_block_after_an_atom_removes_the_block_and_selects_the_atom() {
    let schema = Rc::new(Schema::starter_kit());
    for name in ATOMS {
        let blocks = vec![para(&schema, "ab"), atom(&schema, name), para(&schema, "")];
        let st = state(&schema, blocks, Selection::cursor(Pos(6)));
        let next = st
            .run("deleteCharBackward")
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(
            types(&next),
            ["paragraph", name],
            "{name}: the empty block went"
        );
        assert_eq!(next.selection, node_sel(4), "{name}");

        let blocks = vec![para(&schema, ""), atom(&schema, name), para(&schema, "cd")];
        let st = state(&schema, blocks, Selection::cursor(Pos(1)));
        let next = st
            .run("deleteCharForward")
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(
            types(&next),
            [name, "paragraph"],
            "{name}: the empty block went"
        );
        assert_eq!(next.selection, node_sel(0), "{name}");
    }
}

/// The cut is found up the tree, as for a join: the first paragraph of a list
/// after an atom selects the atom.
#[test]
fn the_atom_is_found_across_a_wrapper_boundary() {
    let schema = Rc::new(Schema::starter_kit());
    let item = schema
        .branch("list_item", Fragment::from_node(para(&schema, "cd")))
        .unwrap();
    let list = schema
        .branch("bullet_list", Fragment::from_node(item))
        .unwrap();
    let blocks = vec![para(&schema, "ab"), atom(&schema, "embed"), list];
    // bullet_list opens at 5, list_item at 6, paragraph at 7, "cd" at 8.
    let st = state(&schema, blocks, Selection::cursor(Pos(8)));
    let next = st.run("deleteCharBackward").expect("Backspace acts");
    assert!(next.doc.same_ref(&st.doc));
    assert_eq!(next.selection, node_sel(4));
}

/// Mid-text and between two paragraphs nothing changed: Backspace deletes a
/// character, and at a paragraph after a paragraph it joins.
#[test]
fn backspace_elsewhere_is_unchanged() {
    let schema = Rc::new(Schema::starter_kit());
    let blocks = vec![para(&schema, "ab"), para(&schema, "cd")];
    let st = state(&schema, blocks, Selection::cursor(Pos(5)));
    let next = st.run("deleteCharBackward").unwrap();
    assert_eq!(types(&next), ["paragraph"], "joined");
}
