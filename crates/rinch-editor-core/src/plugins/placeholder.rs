//! The empty-editor placeholder as a [`Plugin`] (design A8).
//!
//! When the document is a single empty textblock, this plugin contributes one
//! [`Widget::Placeholder`] decoration carrying the configured prompt text. The
//! decoration is **not** part of the document — it flows through the same
//! decoration-diff path the view uses for the IME preedit (M6), so the
//! placeholder appears and disappears purely as a function of `state.doc`.
//!
//! This plugin is **not** in [`default_plugins`](crate::default_plugins): the
//! prompt text is per-editor, so the desktop view (M5.7) constructs it from the
//! `Editor { placeholder: … }` prop.

use crate::decoration::{Decoration, DecorationSet, Widget};
use crate::model::Node;
use crate::plugin::{Plugin, PluginKey};
use crate::pos::Pos;
use crate::state::EditorState;
use std::rc::Rc;

/// The stable key for the placeholder plugin's slot.
pub const PLACEHOLDER_KEY: PluginKey = PluginKey("placeholder");

/// Shows a dimmed prompt while the editor is empty (design A8).
#[derive(Debug, Clone)]
pub struct PlaceholderPlugin {
    text: Rc<str>,
}

impl PlaceholderPlugin {
    /// A placeholder showing `text` when the document is empty.
    pub fn new(text: impl Into<Rc<str>>) -> PlaceholderPlugin {
        PlaceholderPlugin { text: text.into() }
    }

    /// The configured prompt text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Plugin for PlaceholderPlugin {
    fn key(&self) -> PluginKey {
        PLACEHOLDER_KEY
    }

    fn decorations(&self, state: &EditorState) -> DecorationSet {
        if doc_is_empty(&state.doc) {
            // The single empty textblock is `doc.child(0)`; its inline content
            // begins at position 1 (just inside its opening boundary).
            DecorationSet::new(vec![Decoration::widget(
                Pos(1),
                0,
                Widget::Placeholder(self.text.clone()),
            )])
        } else {
            DecorationSet::empty()
        }
    }
}

/// True when `doc` is a single, empty textblock (e.g. one empty paragraph) — the
/// canonical "nothing typed yet" shape the placeholder targets.
fn doc_is_empty(doc: &Node) -> bool {
    doc.child_count() == 1 && {
        let only = doc.child(0);
        only.is_textblock() && only.child_count() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::model::Fragment;
    use std::rc::Rc;

    fn state_with_doc(doc: Node) -> EditorState {
        let schema = Rc::new(Schema::starter_kit());
        EditorState::create(
            schema,
            doc,
            vec![Rc::new(PlaceholderPlugin::new("Write something…"))],
        )
    }

    fn empty_doc(s: &Schema) -> Node {
        s.branch(
            "doc",
            Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
        )
        .unwrap()
    }

    fn nonempty_doc(s: &Schema) -> Node {
        s.branch(
            "doc",
            Fragment::from_node(
                s.branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
                    .unwrap(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn placeholder_shows_for_empty_doc() {
        let s = Schema::starter_kit();
        let state = state_with_doc(empty_doc(&s));
        let decos = state.decorations();
        assert_eq!(decos.len(), 1);
        let d = decos.iter().next().unwrap();
        assert_eq!(d.pos(), Pos(1));
        match d {
            Decoration::Widget { widget, .. } => {
                assert_eq!(*widget, Widget::Placeholder(Rc::from("Write something…")));
            }
        }
    }

    #[test]
    fn placeholder_hidden_once_content_exists() {
        let s = Schema::starter_kit();
        let state = state_with_doc(nonempty_doc(&s));
        assert!(state.decorations().is_empty());
    }

    #[test]
    fn placeholder_hidden_for_non_textblock_first_child() {
        // A doc whose only child is a horizontal rule is not "empty text".
        let s = Schema::starter_kit();
        let doc = s
            .branch(
                "doc",
                Fragment::from_node(s.branch("horizontal_rule", Fragment::empty()).unwrap()),
            )
            .unwrap();
        let state = state_with_doc(doc);
        assert!(state.decorations().is_empty());
    }

    #[test]
    fn doc_is_empty_rejects_multiple_blocks() {
        let s = Schema::starter_kit();
        let doc = s
            .branch(
                "doc",
                Fragment::from_children(vec![
                    s.branch("paragraph", Fragment::empty()).unwrap(),
                    s.branch("paragraph", Fragment::empty()).unwrap(),
                ]),
            )
            .unwrap();
        assert!(!doc_is_empty(&doc));
    }
}
