//! The base-editing plugin: the built-in command catalogue and its key bindings.

use crate::command::Command;
use crate::keymap::KeyBinding;
use crate::model::Attrs;
use crate::plugin::{Plugin, PluginKey};

use super::*;

// ===================================================================
// The base-editing plugin (commands + keymap)
// ===================================================================

/// Bundles the built-in command catalogue and its default key bindings as a
/// [`Plugin`]. History (undo/redo) and markdown input rules are separate plugins.
#[derive(Debug, Default)]
pub struct BaseCommandsPlugin;

impl Plugin for BaseCommandsPlugin {
    fn key(&self) -> PluginKey {
        PluginKey("base-commands")
    }

    fn commands(&self) -> Vec<(&'static str, Command)> {
        let mut cmds: Vec<(&'static str, Command)> = vec![
            ("toggleBold", toggle_mark("bold")),
            ("toggleItalic", toggle_mark("italic")),
            ("toggleUnderline", toggle_mark("underline")),
            ("toggleStrike", toggle_mark("strike")),
            ("toggleCode", toggle_mark("code")),
            ("toggleSubscript", toggle_mark("subscript")),
            ("toggleSuperscript", toggle_mark("superscript")),
            ("toggleHighlight", toggle_mark("highlight")),
            ("removeLink", remove_link()),
            ("selectAll", select_all()),
            ("setParagraph", set_paragraph()),
            ("setTextAlignLeft", set_text_align("left")),
            ("setTextAlignCenter", set_text_align("center")),
            ("setTextAlignRight", set_text_align("right")),
            ("setTextAlignJustify", set_text_align("justify")),
            ("setCodeBlock", set_block_type("code_block", Attrs::new())),
            ("toggleBulletList", toggle_list("bullet_list")),
            ("toggleOrderedList", toggle_list("ordered_list")),
            ("toggleTaskList", toggle_list("task_list")),
            ("wrapInBlockquote", wrap_in("blockquote")),
            ("liftListItem", lift()),
            ("outdent", outdent()),
            ("sinkListItem", sink_list_item_any()),
            ("indent", indent()),
            ("insertHorizontalRule", insert_horizontal_rule()),
            ("insertHardBreak", insert_hard_break()),
            ("insertLineBreak", insert_line_break()),
            ("newlineInCode", newline_in_code()),
            ("exitCode", exit_code()),
            ("insertTable", insert_table(3, 3)),
            ("addRowBefore", table_ops::add_row_before()),
            ("addRowAfter", table_ops::add_row_after()),
            ("addColumnBefore", table_ops::add_column_before()),
            ("addColumnAfter", table_ops::add_column_after()),
            ("deleteRow", table_ops::delete_row()),
            ("deleteColumn", table_ops::delete_column()),
            ("deleteTable", table_ops::delete_table()),
            ("mergeCells", table_ops::merge_cells()),
            ("splitCell", table_ops::split_cell()),
            ("deleteCellSelection", table_ops::delete_cell_selection()),
            ("splitBlock", split_block()),
            ("splitListItem", split_list_item("list_item")),
            ("enter", split_block_or_list_item()),
            ("deleteSelection", delete_selection()),
            ("deleteCharBackward", delete_char_backward()),
            ("deleteCharForward", delete_char_forward()),
            ("deleteWordBackward", delete_word_backward()),
            ("deleteWordForward", delete_word_forward()),
        ];
        for level in 1..=6i64 {
            let name: &'static str = match level {
                1 => "setHeading1",
                2 => "setHeading2",
                3 => "setHeading3",
                4 => "setHeading4",
                5 => "setHeading5",
                _ => "setHeading6",
            };
            let attrs = Attrs::from_iter([("level", crate::AttrValue::Int(level))]);
            cmds.push((name, set_block_type("heading", attrs)));
        }
        cmds
    }

    fn selection_commands(&self) -> Vec<&'static str> {
        vec!["selectAll"]
    }

    /// The keys a Chrome `contenteditable` has, and nothing more (#3062): select
    /// all, bold / italic / underline, Enter and Shift+Enter, the deletes, and
    /// Tab / Shift+Tab for lists (offered through `EditorHandle::tab_key`, which
    /// by default moves the focus instead, as Chrome does). Mod-Enter
    /// (`exitCode`) is the one addition: nothing else leaves a code block that
    /// ends the document, and it falls through everywhere else.
    ///
    /// Every other command (strikethrough, inline code, the lists, blockquote,
    /// paragraph, alignment, headings) has no default key: Chrome has none, and
    /// each chord hid a browser or app shortcut. An app binds the ones it wants
    /// in its own keymap (`EditorHandle::bind_key`, or a plugin's `keymap`).
    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
        [
            // NOTE: never bind a clipboard key (Mod-c/x/v) here — the web keydown
            // handler must return `false` for those so the browser fires its native
            // ClipboardEvent; a binding would consume the key and break web copy/paste.
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
        ]
        .into_iter()
        .chain(word_delete_bindings())
        .filter_map(|(b, cmd)| KeyBinding::parse(b).map(|kb| (kb, cmd)))
        .collect()
    }

    /// The one binding of this keymap that leaves its key alone where its command
    /// does not apply: Mod-Enter (`exitCode`) outside a code block goes on to the
    /// app (its interceptor, the page's `keydown` listeners), so an app's
    /// Ctrl/Cmd+Enter "send" chord works in an editor (#2993). Every other
    /// binding consumes its key wherever the caret is: an editor that advertises a
    /// chord owns it, so Delete at the end of the document, which deletes
    /// nothing, is still the editor's.
    fn fall_through_keys(&self) -> Vec<KeyBinding> {
        KeyBinding::parse("Mod-Enter").into_iter().collect()
    }
}

/// The word-delete chords: Ctrl+Backspace / Ctrl+Delete on Windows and Linux,
/// Alt(Option)+Backspace / Alt+Delete on macOS, where Cmd+Backspace is the
/// line delete. `Mod` is the primary accelerator, which is Cmd on macOS — hence
/// the platform split, decided at compile time like the desktop shell's own
/// `cfg!(target_os = "macos")` checks.
///
/// **None on wasm.** A browser build cannot know the OS at compile time, and
/// the browser already resolves the platform's chord into a `beforeinput`
/// `deleteWordBackward` / `deleteWordForward`, which `rinch-web` maps onto these
/// commands by name. Binding `Mod-Backspace` there would take Cmd+Backspace on a
/// Mac (the web glue's `primary` is Cmd there) away from the browser's line
/// delete.
fn word_delete_bindings() -> Vec<(&'static str, &'static str)> {
    if cfg!(target_arch = "wasm32") {
        Vec::new()
    } else if cfg!(target_os = "macos") {
        vec![
            ("Alt-Backspace", "deleteWordBackward"),
            ("Alt-Delete", "deleteWordForward"),
        ]
    } else {
        vec![
            ("Mod-Backspace", "deleteWordBackward"),
            ("Mod-Delete", "deleteWordForward"),
        ]
    }
}
