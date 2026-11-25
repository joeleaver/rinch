//! #3062: the app's key layer over the plugins' keymap —
//! `EditorHandle::bind_key` / `unbind_key` — and the editor keys a fresh
//! handle no longer has, checked through `dispatch_key`, the entry point both
//! platforms' key glue goes through.

use std::rc::Rc;

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{KeyBinding, MarkdownInputRulesPlugin, Pos, Selection};

use crate::{EditorHandle, TabBehavior, create_editor};

fn html(h: &EditorHandle) -> String {
    node_to_html(&h.doc())
}

/// A fresh editor holding `html`, with `from..to` selected.
fn editor(html: &str, from: usize, to: usize) -> EditorHandle {
    let h = create_editor();
    h.load_html(html);
    h.set_selection(Selection::text(Pos(from), Pos(to)));
    h
}

fn key(h: &EditorHandle, chord: &str) -> Option<bool> {
    h.dispatch_key(KeyBinding::parse(chord).expect("parses"))
}

/// The removed chords, through the handle: each key is unbound (the platform
/// leaves it to the page), and its command still runs by name.
#[test]
fn a_removed_chord_is_left_to_the_page_and_its_command_still_runs() {
    for (chord, command, want) in [
        ("Mod-Shift-s", "toggleStrike", "<p><s>abc</s></p>"),
        ("Mod-e", "toggleCode", "<p><code>abc</code></p>"),
        (
            "Mod-Shift-8",
            "toggleBulletList",
            "<ul><li><p>abc</p></li></ul>",
        ),
        (
            "Mod-Shift-b",
            "wrapInBlockquote",
            "<blockquote><p>abc</p></blockquote>",
        ),
        (
            "Mod-Shift-r",
            "setTextAlignRight",
            r#"<p style="text-align:right">abc</p>"#,
        ),
        ("Mod-Alt-1", "setHeading1", "<h1>abc</h1>"),
    ] {
        let h = editor("<p>abc</p>", 1, 4);
        assert_eq!(key(&h, chord), None, "{chord} is not the editor's");
        assert_eq!(html(&h), "<p>abc</p>", "{chord} changed nothing");
        assert!(h.command(command), "{command} still applies");
        assert_eq!(html(&h), want, "{command}");
    }
}

#[test]
fn bind_key_brings_a_chord_back_and_wins_over_the_plugins() {
    let h = editor("<p>abc</p>", 1, 4);
    assert!(h.bind_key("Mod-Shift-s", "toggleStrike"));
    assert_eq!(key(&h, "Mod-Shift-s"), Some(true));
    assert_eq!(html(&h), "<p><s>abc</s></p>");
    // Over a base binding: the app's command runs instead of bold.
    assert!(h.bind_key("Mod-b", "toggleCode"));
    assert_eq!(key(&h, "Mod-b"), Some(true));
    assert!(h.is_mark_active("code"), "the app's binding ran");
    assert!(!h.is_mark_active("bold"), "the plugin's did not");
    // Consumed where it does not apply, as a plugin's plain binding is.
    h.set_read_only(true);
    assert_eq!(key(&h, "Mod-Shift-s"), Some(false));
    assert!(
        !h.bind_key("Mod-Nope-", "toggleStrike"),
        "an unparsable chord"
    );
}

#[test]
fn unbind_key_masks_a_base_binding_until_bound_again() {
    let h = editor("<p>abc</p>", 1, 4);
    assert!(h.unbind_key("Mod-b"));
    assert_eq!(key(&h, "Mod-b"), None, "left to the page");
    assert!(!h.is_mark_active("bold"));
    assert!(h.command("toggleBold"), "the command stays");
    // Other base bindings are untouched.
    assert_eq!(key(&h, "Mod-i"), Some(true));
    // The fall-through Mod-Enter can go too, inside a code block where it
    // would otherwise apply.
    let code = editor("<pre>ab</pre>", 2, 2);
    assert!(code.unbind_key("Mod-Enter"));
    assert_eq!(key(&code, "Mod-Enter"), None);
    assert_eq!(html(&code), "<pre>ab</pre>", "no paragraph after it");
    // A later bind_key binds it again, to any command.
    assert!(h.bind_key("Mod-b", "toggleBold"));
    assert_eq!(key(&h, "Mod-b"), Some(true));
    // And unbind_key takes the app's own binding away too.
    assert!(h.bind_key("Mod-Shift-s", "toggleStrike"));
    assert!(h.unbind_key("Mod-Shift-s"));
    assert_eq!(key(&h, "Mod-Shift-s"), None);
    assert!(!h.unbind_key(""), "an unparsable chord");
}

/// The layer lives on the handle, so a plugin change, which rebuilds the
/// state's keymap from the plugins, keeps it.
#[test]
fn the_app_layer_survives_a_plugin_change() {
    let h = editor("<p>abc</p>", 1, 4);
    h.unbind_key("Mod-b");
    h.bind_key("Mod-Shift-s", "toggleStrike");
    h.add_plugin(Rc::new(MarkdownInputRulesPlugin));
    h.set_placeholder("Write");
    h.load_html("<p>abc</p>");
    h.set_selection(Selection::text(Pos(1), Pos(4)));
    assert_eq!(key(&h, "Mod-b"), None);
    assert_eq!(key(&h, "Mod-Shift-s"), Some(true));
}

/// Unbinding Tab stops it indenting a list under `IndentInListsAndTables`.
#[test]
fn unbinding_tab_stops_list_indenting() {
    let html = "<ul><li><p>one</p></li><li><p>two</p></li></ul>";
    let h = editor(html, 12, 12);
    h.set_tab_behavior(TabBehavior::IndentInListsAndTables);
    h.unbind_key("Tab");
    assert!(!h.tab_key(false), "the focus moves instead");
    h.bind_key("Tab", "sinkListItem");
    assert!(h.tab_key(false), "bound again: the second item sinks");
}
