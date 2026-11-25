//! #3062: the default keymap keeps only what a Chrome `contenteditable` does.
//!
//! The 17 formatting chords (strikethrough, inline code, the three lists,
//! blockquote, paragraph, the four alignments, the six headings) are no longer
//! bound, and their commands still run when called by name. Mod-B/I/U and
//! Mod-Enter stay. Mod-y (redo) is not a macOS chord: Cmd+Y opens History in
//! Chrome there.

use std::rc::Rc;

use rinch_editor_core::keymap::{host_is_macos, set_host_is_macos};
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::*;

fn state(html: &str, from: usize, to: usize) -> EditorState {
    let s = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&s, html).expect("parses");
    let doc = s.branch("doc", slice.content).expect("a document");
    let st = EditorState::create(s, doc, default_plugins());
    let mut tr = st.tr();
    tr.set_selection(Selection::text(Pos(from), Pos(to)));
    st.apply(tr)
}

fn bound(st: &EditorState, key: &str) -> Option<String> {
    st.keymap()
        .command_for(&KeyBinding::parse(key).expect("parses"))
        .map(str::to_string)
}

/// Each removed chord, the command it ran, and the HTML that command makes of
/// `<p>abc</p>` with "abc" selected — the proof the command itself is intact.
const REMOVED: [(&str, &str, &str); 17] = [
    ("Mod-Shift-s", "toggleStrike", "<p><s>abc</s></p>"),
    ("Mod-e", "toggleCode", "<p><code>abc</code></p>"),
    (
        "Mod-Shift-7",
        "toggleTaskList",
        r#"<ul data-type="taskList"><li data-type="taskItem" data-checked="false"><p>abc</p></li></ul>"#,
    ),
    (
        "Mod-Shift-8",
        "toggleBulletList",
        "<ul><li><p>abc</p></li></ul>",
    ),
    (
        "Mod-Shift-9",
        "toggleOrderedList",
        "<ol><li><p>abc</p></li></ol>",
    ),
    (
        "Mod-Shift-b",
        "wrapInBlockquote",
        "<blockquote><p>abc</p></blockquote>",
    ),
    ("Mod-Shift-0", "setParagraph", "<p>abc</p>"),
    (
        "Mod-Shift-l",
        "setTextAlignLeft",
        r#"<p style="text-align:left">abc</p>"#,
    ),
    (
        "Mod-Shift-e",
        "setTextAlignCenter",
        r#"<p style="text-align:center">abc</p>"#,
    ),
    (
        "Mod-Shift-r",
        "setTextAlignRight",
        r#"<p style="text-align:right">abc</p>"#,
    ),
    (
        "Mod-Shift-j",
        "setTextAlignJustify",
        r#"<p style="text-align:justify">abc</p>"#,
    ),
    ("Mod-Alt-1", "setHeading1", "<h1>abc</h1>"),
    ("Mod-Alt-2", "setHeading2", "<h2>abc</h2>"),
    ("Mod-Alt-3", "setHeading3", "<h3>abc</h3>"),
    ("Mod-Alt-4", "setHeading4", "<h4>abc</h4>"),
    ("Mod-Alt-5", "setHeading5", "<h5>abc</h5>"),
    ("Mod-Alt-6", "setHeading6", "<h6>abc</h6>"),
];

#[test]
fn the_formatting_chords_are_unbound() {
    let st = state("<p>abc</p>", 1, 4);
    for (key, command, _) in REMOVED {
        assert_eq!(bound(&st, key), None, "{key} no longer runs {command}");
    }
}

#[test]
fn their_commands_still_run_by_name() {
    for (key, command, html) in REMOVED {
        // `setParagraph` needs a block that is not a paragraph to change.
        let start = if command == "setParagraph" {
            "<h2>abc</h2>"
        } else {
            "<p>abc</p>"
        };
        let st = state(start, 1, 4);
        let after = st
            .run(command)
            .unwrap_or_else(|| panic!("{command} (was {key}) applies"));
        assert_eq!(node_to_html(&after.doc), html, "{command}");
    }
}

#[test]
fn the_chrome_chords_and_mod_enter_stay() {
    let st = state("<p>abc</p>", 1, 4);
    for (key, command) in [
        ("Mod-a", "selectAll"),
        ("Mod-b", "toggleBold"),
        ("Mod-i", "toggleItalic"),
        ("Mod-u", "toggleUnderline"),
        ("Enter", "enter"),
        ("Shift-Enter", "insertLineBreak"),
        ("Mod-Enter", "exitCode"),
        ("Backspace", "deleteCharBackward"),
        ("Delete", "deleteCharForward"),
        ("Tab", "sinkListItem"),
        ("Shift-Tab", "liftListItem"),
        ("Mod-z", "undo"),
        ("Mod-Shift-z", "redo"),
    ] {
        assert_eq!(bound(&st, key).as_deref(), Some(command), "{key}");
    }
}

#[test]
fn mod_y_redoes_off_macos_only() {
    let st = state("<p>abc</p>", 1, 4);
    let was = host_is_macos();
    set_host_is_macos(false);
    assert_eq!(bound(&st, "Mod-y").as_deref(), Some("redo"), "Ctrl+Y");
    set_host_is_macos(true);
    assert_eq!(bound(&st, "Mod-y"), None, "Cmd+Y is Chrome's History");
    assert_eq!(
        bound(&st, "Mod-Shift-z").as_deref(),
        Some("redo"),
        "Cmd+Shift+Z stays"
    );
    set_host_is_macos(was);
}

/// A plugin after the history that binds Mod-y binds it everywhere: the
/// exclusion is the history's declaration, not the key's.
#[test]
fn a_later_binding_of_mod_y_is_not_excluded() {
    struct AppKeys;
    impl Plugin for AppKeys {
        fn key(&self) -> PluginKey {
            PluginKey("app-keys-3062")
        }
        fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
            KeyBinding::parse("Mod-y")
                .map(|b| (b, "toggleStrike"))
                .into_iter()
                .collect()
        }
    }
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", slice_from_html(&s, "<p>a</p>").unwrap().content)
        .unwrap();
    let mut plugins = default_plugins();
    plugins.push(Rc::new(AppKeys));
    let st = EditorState::create(s, doc, plugins);
    let was = host_is_macos();
    set_host_is_macos(true);
    assert_eq!(bound(&st, "Mod-y").as_deref(), Some("toggleStrike"));
    set_host_is_macos(was);
}
