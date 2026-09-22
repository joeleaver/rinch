//! Fixtures from the review of #836: `EditorHandle::add_plugin`.
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{Attrs, EditorState, Plugin, PluginKey, Pos};
use std::rc::Rc;

struct Spell;
impl Plugin for Spell {
    fn key(&self) -> PluginKey {
        PluginKey("review836.spell")
    }
    fn decorations(&self, _state: &EditorState) -> DecorationSet {
        DecorationSet::new(vec![Decoration::inline(
            Pos(1),
            Pos(3),
            Attrs::new().with("class", "pm-spell-error"),
        )])
    }
}

fn has_spell(h: &rinch_editor_view::EditorHandle) -> bool {
    h.state()
        .decorations()
        .iter()
        .any(|d| d.inline_class() == Some("pm-spell-error"))
}

#[test]
fn add_plugin_installs_decorations() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>helo</p>");
    assert!(!has_spell(&h));
    assert!(h.add_plugin(Rc::new(Spell)));
    assert!(has_spell(&h), "add_plugin did not reach the state");
    assert!(!h.add_plugin(Rc::new(Spell)), "second add is a no-op");
}

#[test]
fn add_plugin_keeps_selection() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>hello</p>");
    h.set_selection(rinch_editor_core::Selection::cursor(Pos(3)));
    assert!(h.add_plugin(Rc::new(Spell)));
    assert_eq!(h.selection(), rinch_editor_core::Selection::cursor(Pos(3)));
}

#[test]
fn add_plugin_on_a_read_only_editor() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>helo</p>");
    h.set_read_only(true);
    assert!(h.add_plugin(Rc::new(Spell)), "read-only refused add_plugin");
    assert!(has_spell(&h));
}

#[cfg(feature = "collaboration")]
#[test]
fn add_plugin_on_a_read_only_collaborating_editor_is_not_half_applied() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>helo</p>");
    h.start_collaboration_host(|_delta| {}).unwrap();
    h.set_read_only(true);
    let added = h.add_plugin(Rc::new(Spell));
    let installed = has_spell(&h);
    // Either it is installed (preferred: adding a plugin is not an edit) or it is
    // refused *and* a retry after leaving read-only works. What must not happen is
    // "refused, but the key is now taken so every retry is refused too".
    h.set_read_only(false);
    let retry = h.add_plugin(Rc::new(Spell));
    assert!(
        (added && installed) || (!added && retry && has_spell(&h)),
        "added={added} installed={installed} retry={retry} now={}",
        has_spell(&h)
    );
}
