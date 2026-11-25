//! #3062: the app's key layer over the plugins' keymap —
//! `EditorHandle::bind_key` / `unbind_key` — and the editor keys a fresh
//! handle no longer has, checked through `dispatch_key`, the entry point both
//! platforms' key glue goes through.

use std::rc::Rc;

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{KeyBinding, MarkdownInputRulesPlugin, Pos, Selection};

use crate::{EditorHandle, KeyBindError, TabBehavior, create_editor};

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
    assert_eq!(h.bind_key("Mod-Shift-s", "toggleStrike"), Ok(()));
    assert_eq!(key(&h, "Mod-Shift-s"), Some(true));
    assert_eq!(html(&h), "<p><s>abc</s></p>");
    // Over a base binding: the app's command runs instead of bold.
    assert_eq!(h.bind_key("Mod-b", "toggleCode"), Ok(()));
    assert_eq!(key(&h, "Mod-b"), Some(true));
    assert!(h.is_mark_active("code"), "the app's binding ran");
    assert!(!h.is_mark_active("bold"), "the plugin's did not");
    // Consumed where it does not apply, as a plugin's plain binding is.
    h.set_read_only(true);
    assert_eq!(key(&h, "Mod-Shift-s"), Some(false));
    assert_eq!(
        h.bind_key("Mod-Nope-", "toggleStrike"),
        Err(KeyBindError::Chord("Mod-Nope-".into())),
        "an unparsable chord"
    );
}

/// A command the editor does not have is refused, not bound to a key that
/// would then do nothing; the chord keeps whatever it had.
#[test]
fn bind_key_refuses_an_unknown_command() {
    let h = editor("<p>abc</p>", 1, 4);
    assert_eq!(
        h.bind_key("Mod-b", "toggleBolt"),
        Err(KeyBindError::UnknownCommand("toggleBolt".into()))
    );
    assert_eq!(
        h.bind_key("Mod-Shift-s", "toggleStrikethrough"),
        Err(KeyBindError::UnknownCommand("toggleStrikethrough".into()))
    );
    assert_eq!(key(&h, "Mod-Shift-s"), None, "nothing was bound");
    assert_eq!(key(&h, "Mod-b"), Some(true), "Mod-b still bolds");
    assert!(h.is_mark_active("bold"));
    // A plugin's command, once the plugin is there, binds.
    assert_eq!(h.bind_key("Mod-Shift-s", "toggleStrike"), Ok(()));
}

/// The app's binding of Tab and Shift+Tab is asked before the
/// `TabBehavior`: under the default `MoveFocus` the bound key runs its
/// command and stays the editor's. A read-only editor still lets Tab go
/// (#833).
#[test]
fn an_app_bound_tab_comes_before_the_tab_behavior() {
    let h = editor("<p>abc</p>", 1, 4);
    assert_eq!(h.tab_behavior(), TabBehavior::MoveFocus);
    assert!(!h.tab_key(false), "positive control: unbound, Tab leaves");
    assert_eq!(h.bind_key("Tab", "toggleBold"), Ok(()));
    assert_eq!(h.bind_key("Shift-Tab", "toggleItalic"), Ok(()));
    assert!(h.tab_key(false), "the app's Tab is the editor's");
    assert!(h.is_mark_active("bold"), "and ran its command");
    assert!(h.tab_key(true), "the app's Shift+Tab too");
    assert!(h.is_mark_active("italic"));
    h.set_read_only(true);
    assert!(!h.tab_key(false), "read-only: the focus moves");
}

#[test]
fn unbind_key_masks_a_base_binding_until_bound_again() {
    let h = editor("<p>abc</p>", 1, 4);
    assert_eq!(h.unbind_key("Mod-b"), Ok(()));
    assert_eq!(key(&h, "Mod-b"), None, "left to the page");
    assert!(!h.is_mark_active("bold"));
    assert!(h.command("toggleBold"), "the command stays");
    // Other base bindings are untouched.
    assert_eq!(key(&h, "Mod-i"), Some(true));
    // The fall-through Mod-Enter can go too, inside a code block where it
    // would otherwise apply.
    let code = editor("<pre>ab</pre>", 2, 2);
    assert_eq!(code.unbind_key("Mod-Enter"), Ok(()));
    assert_eq!(key(&code, "Mod-Enter"), None);
    assert_eq!(html(&code), "<pre>ab</pre>", "no paragraph after it");
    // A later bind_key binds it again, to any command.
    assert_eq!(h.bind_key("Mod-b", "toggleBold"), Ok(()));
    assert_eq!(key(&h, "Mod-b"), Some(true));
    // And unbind_key takes the app's own binding away too.
    assert_eq!(h.bind_key("Mod-Shift-s", "toggleStrike"), Ok(()));
    assert_eq!(h.unbind_key("Mod-Shift-s"), Ok(()));
    assert_eq!(key(&h, "Mod-Shift-s"), None);
    assert_eq!(
        h.unbind_key(""),
        Err(KeyBindError::Chord(String::new())),
        "an unparsable chord"
    );
}

/// The layer lives on the handle, so a plugin change, which rebuilds the
/// state's keymap from the plugins, keeps it.
#[test]
fn the_app_layer_survives_a_plugin_change() {
    let h = editor("<p>abc</p>", 1, 4);
    h.unbind_key("Mod-b").unwrap();
    h.bind_key("Mod-Shift-s", "toggleStrike").unwrap();
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
    h.unbind_key("Tab").unwrap();
    assert!(!h.tab_key(false), "the focus moves instead");
    h.bind_key("Tab", "sinkListItem").unwrap();
    assert!(h.tab_key(false), "bound again: the second item sinks");
}
