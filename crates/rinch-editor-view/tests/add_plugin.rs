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

#[test]
fn add_plugin_is_not_an_edit() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>helo</p>");
    let fired = Rc::new(std::cell::Cell::new(0));
    let f = fired.clone();
    h.on_change(move || f.set(f.get() + 1));
    let before = h.doc();
    assert!(h.add_plugin(Rc::new(Spell)));
    assert_eq!(fired.get(), 0, "on_change fired for a plugin registration");
    assert_eq!(h.doc(), before);
}

#[cfg(feature = "collaboration")]
#[test]
fn add_plugin_on_a_read_only_collaborating_editor_installs_and_broadcasts_nothing() {
    // It used to commit as a *load*, which a read-only collaborating editor
    // refuses — after the plugin had already been pushed, so it answered false
    // and every retry answered "already installed" (review of #836).
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>helo</p>");
    let sent = Rc::new(std::cell::Cell::new(0));
    let s2 = sent.clone();
    h.start_collaboration_host(move |_delta| s2.set(s2.get() + 1))
        .unwrap();
    let sent_before = sent.get();
    h.set_read_only(true);
    assert!(
        h.add_plugin(Rc::new(Spell)),
        "read-only collab refused add_plugin"
    );
    assert!(has_spell(&h));
    assert_eq!(
        sent.get(),
        sent_before,
        "a plugin registration was broadcast"
    );
    assert!(h.collab_take_error().is_none());
    // Positive control: the same session does broadcast a real edit.
    h.set_read_only(false);
    assert!(h.insert_text("x"));
    assert!(
        sent.get() > sent_before,
        "the outbound sink never fires at all"
    );
}

/// A stored mark (Ctrl+B on a collapsed caret) survives `add_plugin`: the next
/// character typed is bold. From the review of #847.
#[test]
fn add_plugin_keeps_stored_marks() {
    let h = rinch_editor_view::create_editor();
    h.load_html("<p>hello</p>");
    h.set_selection(rinch_editor_core::Selection::cursor(Pos(6)));
    assert!(h.command("toggleBold"));
    assert!(
        h.state().stored_marks.is_some(),
        "precondition: stored marks"
    );
    assert!(h.add_plugin(Rc::new(Spell)));
    assert!(h.insert_text("x"));
    let html = rinch_editor_core::serialize::html::node_to_html(&h.doc());
    assert!(html.contains("<strong>x</strong>"), "{html}");
}
