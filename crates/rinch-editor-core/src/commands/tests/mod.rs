//! Tests for the built-in commands, grouped by command family. The fixtures
//! every group shares live here.

mod enter;
mod indent_align;
mod insert;
mod joins_and_deletes;
mod list_nesting;
mod marks_and_blocks;
mod word_delete;

use super::*;
use crate::Fragment;
use crate::keymap::KeyBinding;
use crate::model::{Attrs, Node};
use crate::pos::Pos;
use crate::selection::Selection;
use crate::{Schema, default_plugins};
use std::rc::Rc;

fn editor(text: &str) -> EditorState {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
    EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)])
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

/// Whether any inline node in `doc` carries a mark named `name`.
fn doc_has_mark(doc: &Node, name: &str) -> bool {
    let mut found = false;
    doc.nodes_between(0, doc.content_size(), &mut |node, _, _| {
        if node.marks().iter().any(|m| m.type_name() == name) {
            found = true;
        }
        true
    });
    found
}

fn p_of(s: &Schema, t: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
        .unwrap()
}

fn li_of(s: &Schema, t: &str) -> Node {
    s.branch("list_item", Fragment::from_node(p_of(s, t)))
        .unwrap()
}

fn state_of(doc: Node, schema: Rc<Schema>, cursor: usize) -> EditorState {
    let mut state = EditorState::create(schema, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(Pos(cursor));
    state
}

/// Position just inside the first `paragraph` whose flattened text equals `text`.
fn cursor_in_para(doc: &Node, text: &str) -> usize {
    let mut found = None;
    doc.nodes_between(0, doc.content_size(), &mut |node, pos, _| {
        if found.is_none() && node.type_name() == "paragraph" && all_text(node) == text {
            found = Some(pos + 1);
        }
        true
    });
    found.expect("no paragraph with that text")
}
