//! Input bridge: connects keyboard events to editor commands.
//!
//! Registers a global keyboard interceptor that routes key events to
//! the appropriate editor commands. Also provides a hidden textarea
//! element for capturing text input and IME.

use std::rc::Rc;
use std::cell::RefCell;
use std::cell::RefCell as StdRefCell;

use rinch_core::dom::{RenderScope, NodeHandle};
use rinch_core::events::{KeyEventData, set_keyboard_interceptor, clear_keyboard_interceptor};

use crate::editor::Editor;
use crate::commands::{TextCommands, FormattingCommands, StructureCommands};
use crate::document::{Position, Range};
use crate::selection::Selection;

thread_local! {
    static EDITOR_CLIPBOARD: StdRefCell<String> = StdRefCell::new(String::new());
    static DESIRED_COLUMN: StdRefCell<Option<usize>> = StdRefCell::new(None);
}

/// Create the input bridge: a hidden textarea and keyboard interceptor.
///
/// The hidden textarea captures text input (characters, IME, clipboard paste).
/// The keyboard interceptor captures special keys (Enter, Backspace, arrows, shortcuts).
///
/// Returns (textarea_node, focus_callback). Call focus_callback when the editor content
/// is clicked to direct keyboard input to the hidden textarea.
///
/// Call `on_change` after every command to trigger re-rendering.
pub fn create_input_bridge(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    on_change: Rc<dyn Fn()>,
) -> (NodeHandle, Rc<dyn Fn()>) {
    // Create hidden textarea for capturing text input
    let textarea = scope.create_element("textarea");
    textarea.set_attribute("style", "\
        position: absolute; \
        left: -9999px; \
        top: -9999px; \
        width: 1px; \
        height: 1px; \
        opacity: 0; \
        pointer-events: none; \
    ");
    textarea.set_attribute("autocomplete", "off");
    textarea.set_attribute("autocorrect", "off");
    textarea.set_attribute("spellcheck", "false");

    // Register textarea input handler
    let editor_for_input = editor.clone();
    let on_change_for_input = on_change.clone();
    let handler_id = scope.register_input_handler({
        move |value: String| {
            handle_textarea_input(&editor_for_input, &value, &*on_change_for_input);
        }
    });
    textarea.set_attribute("data-oninput", &handler_id.to_string());

    // Keep keyboard interceptor for navigation and shortcuts ONLY
    let editor_for_keys = editor.clone();
    let on_change_for_keys = on_change.clone();
    let textarea_for_keys = textarea.clone();
    set_keyboard_interceptor(move |data: &KeyEventData| {
        let handled = handle_key_event(&editor_for_keys, data, &*on_change_for_keys);
        // Sync textarea after cursor movement keys
        if handled && is_cursor_movement(&data.key) {
            if let Ok(ed) = editor_for_keys.try_borrow() {
                sync_textarea_with_editor(&ed, &textarea_for_keys);
            }
        }
        handled
    });

    // Create focus callback that can be called from editor click handler
    let textarea_for_focus = textarea.clone();
    let focus_callback: Rc<dyn Fn()> = Rc::new(move || {
        textarea_for_focus.focus();
    });

    (textarea, focus_callback)
}

/// Cleanup the input bridge (call when editor is unmounted).
pub fn destroy_input_bridge() {
    clear_keyboard_interceptor();
}

/// Handle input from the hidden textarea.
/// The textarea value is diffed against the current block text to find the delta.
fn handle_textarea_input(
    editor: &Rc<RefCell<Editor>>,
    new_value: &str,
    on_change: &dyn Fn(),
) {
    tracing::info!("handle_textarea_input: new_value={:?}", new_value);
    if let Ok(mut ed) = editor.try_borrow_mut() {
        tracing::info!("handle_textarea_input: block_count={}", ed.doc.block_count());
        // Get current selection position to know which block we're in
        let sel = ed.get_selection().clone();
        tracing::info!("handle_textarea_input: sel.head={:?}", sel.head);
        if let Ok(rp) = ed.doc.resolve_position(sel.head) {
            tracing::info!("handle_textarea_input: resolved block_index={}", rp.block_index);
            let current_text = ed.doc.block_text(rp.block_index).unwrap_or_default();

            // Diff: find what changed
            let (prefix_match, suffix_match) = diff_strings(&current_text, new_value);
            let old_mid = &current_text[prefix_match..current_text.len() - suffix_match];
            let new_mid = &new_value[prefix_match..new_value.len() - suffix_match];

            if old_mid.is_empty() && new_mid.is_empty() {
                return; // No change
            }

            // Calculate absolute position of the change
            let mut abs_start = 0;
            for i in 0..rp.block_index {
                abs_start += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
            }
            abs_start += prefix_match;

            // Delete old text if any
            if !old_mid.is_empty() {
                let range = Range::new(abs_start, abs_start + old_mid.len());
                let _ = TextCommands::delete_range(&mut ed, range);
            }

            // Insert new text if any
            if !new_mid.is_empty() {
                ed.set_selection(Selection::cursor(Position::new(abs_start)));
                let _ = TextCommands::insert_text(&mut ed, new_mid);
            }

            // Check input rules after space
            if new_mid.ends_with(' ') {
                check_input_rules(&mut ed);
            }

            // Check for ``` code block trigger
            if new_mid.contains('`') {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let block_text = ed.doc.block_text(rp.block_index).unwrap_or_default();
                    if block_text == "```" {
                        let mut abs = 0;
                        for i in 0..rp.block_index {
                            abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                        }
                        let _ = ed.doc.delete_range(Range::new(abs, abs + 3));
                        let _ = ed.doc.set_block_type(rp.block_index, "code_block", None);
                        ed.set_selection(Selection::cursor(Position::new(abs)));
                    }
                }
            }
        }
    }
    on_change();
}

/// Find common prefix and suffix lengths between two strings.
fn diff_strings(old: &str, new: &str) -> (usize, usize) {
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    // Common prefix
    let prefix = old_bytes.iter().zip(new_bytes.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Common suffix (don't overlap with prefix)
    let old_remaining = old_bytes.len() - prefix;
    let new_remaining = new_bytes.len() - prefix;
    let suffix = old_bytes.iter().rev().zip(new_bytes.iter().rev())
        .take(old_remaining.min(new_remaining))
        .take_while(|(a, b)| a == b)
        .count();

    (prefix, suffix)
}

/// Get or set desired column for vertical navigation.
fn get_or_set_desired_column(editor: &Editor) -> usize {
    DESIRED_COLUMN.with(|dc| {
        if let Some(col) = *dc.borrow() {
            col
        } else {
            let sel = editor.get_selection();
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                let col = rp.text_offset;
                *dc.borrow_mut() = Some(col);
                col
            } else {
                0
            }
        }
    })
}

/// Clear desired column (called on horizontal movement).
fn clear_desired_column() {
    DESIRED_COLUMN.with(|dc| {
        *dc.borrow_mut() = None;
    });
}

/// Check if a key is a cursor movement key.
fn is_cursor_movement(key: &str) -> bool {
    matches!(key, "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" |
                  "Home" | "End" | "PageUp" | "PageDown")
}

/// Sync the hidden textarea value and cursor with editor state.
/// Call this after cursor navigation to ensure text input goes to the right place.
fn sync_textarea_with_editor(editor: &Editor, textarea: &NodeHandle) {
    let sel = editor.get_selection();
    if let Ok(rp) = editor.doc.resolve_position(sel.head) {
        let block_text = editor.doc.block_text(rp.block_index).unwrap_or_default();
        textarea.set_attribute("value", &block_text);
        textarea.set_attribute("data-cursor-pos", &rp.text_offset.to_string());
        textarea.set_attribute("data-selection-start", &rp.text_offset.to_string());
    }
}

/// Find the previous word boundary from a position.
fn prev_word_boundary(editor: &Editor, pos: Position) -> Position {
    if pos.0 == 0 { return pos; }
    let text = editor.doc.to_text();
    let chars: Vec<char> = text.chars().collect();
    let mut i = pos.0;
    // Skip trailing whitespace
    while i > 0 && chars.get(i - 1).map_or(false, |c| c.is_whitespace()) {
        i -= 1;
    }
    // Skip word characters
    while i > 0 && chars.get(i - 1).map_or(false, |c| !c.is_whitespace()) {
        i -= 1;
    }
    Position::new(i)
}

/// Find the next word boundary from a position.
fn next_word_boundary(editor: &Editor, pos: Position) -> Position {
    let text = editor.doc.to_text();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = pos.0;
    // Skip current word characters
    while i < len && !chars[i].is_whitespace() {
        i += 1;
    }
    // Skip whitespace
    while i < len && chars[i].is_whitespace() {
        i += 1;
    }
    Position::new(i)
}

/// Handle a keyboard event, dispatching to the appropriate editor command.
/// Returns true if the event was handled (should not propagate).
fn handle_key_event(
    editor: &Rc<RefCell<Editor>>,
    data: &KeyEventData,
    on_change: &dyn Fn(),
) -> bool {
    // Clear desired column on any key except ArrowUp/Down
    if data.key != "ArrowUp" && data.key != "ArrowDown" {
        clear_desired_column();
    }

    // Modifier shortcuts (Ctrl/Cmd)
    if data.ctrl || data.meta {
        return handle_ctrl_shortcut(editor, data, on_change);
    }

    // Non-modifier special keys
    match data.key.as_str() {
        "Enter" => {
            if data.shift {
                // Shift+Enter: just split block without type conversion
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::split_block(&mut ed);
                }
                on_change();
                return true;
            }
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let block_type = ed.doc.block_type(rp.block_index);
                    let block_text = ed.doc.block_text(rp.block_index).unwrap_or_default();

                    // Check for --- horizontal rule
                    if block_text == "---" {
                        // Delete the text
                        let mut abs_start = 0;
                        for i in 0..rp.block_index {
                            abs_start += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                        }
                        let _ = ed.doc.delete_range(Range::new(abs_start, abs_start + 3));
                        let _ = ed.doc.set_block_type(rp.block_index, "horizontal_rule", None);
                        // Split to create new paragraph after
                        let _ = StructureCommands::split_block(&mut ed);
                    } else if matches!(block_type.as_deref(), Some("bullet_list") | Some("ordered_list") | Some("list_item"))
                        && block_text.is_empty()
                    {
                        // If in an empty list item, exit the list (convert to paragraph)
                        let _ = ed.doc.set_block_type(rp.block_index, "paragraph", None);
                    } else {
                        // Normal split behavior
                        let block_type_before = block_type;
                        let _ = StructureCommands::split_block(&mut ed);

                        // After split, reset heading/blockquote/code_block to paragraph
                        if let Some(bt) = block_type_before {
                            match bt.as_str() {
                                "heading" | "blockquote" | "code_block" => {
                                    let new_sel = ed.get_selection().clone();
                                    if let Ok(rp) = ed.doc.resolve_position(new_sel.head) {
                                        let _ = ed.doc.set_block_type(rp.block_index, "paragraph", None);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                } else {
                    let _ = StructureCommands::split_block(&mut ed);
                }
            }
            on_change();
            true
        }
        "Backspace" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    if sel.is_cursor() && rp.text_offset == 0 {
                        let block_type = ed.doc.block_type(rp.block_index);
                        match block_type.as_deref() {
                            Some("heading") | Some("blockquote") | Some("code_block") |
                            Some("bullet_list") | Some("ordered_list") | Some("list_item") => {
                                // Convert non-paragraph block to paragraph first
                                let _ = ed.doc.set_block_type(rp.block_index, "paragraph", None);
                            }
                            _ => {
                                // At start of paragraph: join with previous block
                                if rp.block_index > 0 {
                                    let _ = StructureCommands::join_backward(&mut ed);
                                }
                            }
                        }
                    } else if !sel.is_cursor() {
                        // Range selection: delete the selection
                        let _ = TextCommands::delete_backward(&mut ed);
                    } else {
                        // Not at start of block: normal delete backward
                        let _ = TextCommands::delete_backward(&mut ed);
                    }
                } else {
                    let _ = TextCommands::delete_backward(&mut ed);
                }
            }
            on_change();
            true
        }
        "Delete" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = TextCommands::delete_forward(&mut ed);
            }
            on_change();
            true
        }
        "ArrowLeft" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let pos = ed.get_selection().head;
                if pos.0 > 0 {
                    move_or_extend(&mut ed, pos - 1, data.shift);
                } else if !data.shift {
                    ed.set_selection(Selection::cursor(pos));
                    ed.clear_stored_marks();
                }
            }
            on_change();
            true
        }
        "ArrowRight" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let pos = ed.get_selection().head;
                let len = ed.doc.text_length();
                if pos.0 < len {
                    move_or_extend(&mut ed, pos + 1, data.shift);
                } else if !data.shift {
                    ed.set_selection(Selection::cursor(pos));
                    ed.clear_stored_marks();
                }
            }
            on_change();
            true
        }
        "ArrowUp" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    if rp.block_index > 0 {
                        let desired_col = get_or_set_desired_column(&ed);
                        let prev_block = rp.block_index - 1;
                        let prev_len = ed.doc.block_text(prev_block).map(|t| t.len()).unwrap_or(0);
                        let target_offset = desired_col.min(prev_len);

                        // Calculate absolute position
                        let mut abs = 0;
                        for i in 0..prev_block {
                            abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                        }
                        abs += target_offset;

                        move_or_extend(&mut ed, Position::new(abs), data.shift);
                    }
                }
            }
            on_change();
            true
        }
        "ArrowDown" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    if rp.block_index + 1 < ed.doc.block_count() {
                        let desired_col = get_or_set_desired_column(&ed);
                        let next_block = rp.block_index + 1;
                        let next_len = ed.doc.block_text(next_block).map(|t| t.len()).unwrap_or(0);
                        let target_offset = desired_col.min(next_len);

                        // Calculate absolute position
                        let mut abs = 0;
                        for i in 0..next_block {
                            abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                        }
                        abs += target_offset;

                        move_or_extend(&mut ed, Position::new(abs), data.shift);
                    }
                }
            }
            on_change();
            true
        }
        "Tab" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let block_type = ed.doc.block_type(rp.block_index);
                    if data.shift {
                        // Shift+Tab: outdent - if in a list, convert to paragraph
                        match block_type.as_deref() {
                            Some("bullet_list") | Some("ordered_list") | Some("list_item") => {
                                let _ = ed.doc.set_block_type(rp.block_index, "paragraph", None);
                            }
                            Some("blockquote") => {
                                let _ = ed.doc.set_block_type(rp.block_index, "paragraph", None);
                            }
                            _ => {} // No-op for other types
                        }
                    } else {
                        // Tab: indent - if in paragraph, could indent to list
                        // For now just insert tab for non-list blocks
                        match block_type.as_deref() {
                            Some("code_block") => {
                                let _ = TextCommands::insert_text(&mut ed, "\t");
                            }
                            _ => {
                                let _ = TextCommands::insert_text(&mut ed, "\t");
                            }
                        }
                    }
                } else {
                    let _ = TextCommands::insert_text(&mut ed, "\t");
                }
            }
            on_change();
            true
        }
        "Home" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut abs = 0;
                    for i in 0..rp.block_index {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        "End" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut abs = 0;
                    for i in 0..rp.block_index {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    abs += ed.doc.block_text(rp.block_index).map(|t| t.len()).unwrap_or(0);
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        "Escape" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    ed.set_selection(Selection::cursor(sel.head));
                    ed.clear_stored_marks();
                }
            }
            on_change();
            true
        }
        "PageUp" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let target_block = rp.block_index.saturating_sub(20);
                    let target_len = ed.doc.block_text(target_block).map(|t| t.len()).unwrap_or(0);
                    let target_offset = rp.text_offset.min(target_len);
                    let mut abs = 0;
                    for i in 0..target_block {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    abs += target_offset;
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        "PageDown" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let block_count = ed.doc.block_count();
                    let target_block = (rp.block_index + 20).min(block_count.saturating_sub(1));
                    let target_len = ed.doc.block_text(target_block).map(|t| t.len()).unwrap_or(0);
                    let target_offset = rp.text_offset.min(target_len);
                    let mut abs = 0;
                    for i in 0..target_block {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    abs += target_offset;
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        _ => {
            // Insert printable characters
            if !data.ctrl && !data.meta && !data.alt && data.key.chars().count() == 1 {
                let ch = data.key.chars().next().unwrap();
                if !ch.is_control() {
                    if let Ok(mut ed) = editor.try_borrow_mut() {
                        let _ = TextCommands::insert_text(&mut ed, &data.key);
                        // Check for ``` code block trigger
                        if data.key == "`" {
                            let sel = ed.get_selection().clone();
                            if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                                let block_text = ed.doc.block_text(rp.block_index).unwrap_or_default();
                                if block_text == "```" {
                                    let mut abs_start = 0;
                                    for i in 0..rp.block_index {
                                        abs_start += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                                    }
                                    let _ = ed.doc.delete_range(Range::new(abs_start, abs_start + 3));
                                    let _ = ed.doc.set_block_type(rp.block_index, "code_block", None);
                                    ed.set_selection(Selection::cursor(Position::new(abs_start)));
                                }
                            }
                        }
                    }
                    on_change();
                    return true;
                }
            }
            // Handle Space (reported as named key)
            if !data.ctrl && !data.meta && !data.alt && data.key == "Space" {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = TextCommands::insert_text(&mut ed, " ");
                    check_input_rules(&mut ed);
                }
                on_change();
                return true;
            }
            false
        }
    }
}

/// Handle Ctrl/Cmd keyboard shortcuts.
fn handle_ctrl_shortcut(
    editor: &Rc<RefCell<Editor>>,
    data: &KeyEventData,
    on_change: &dyn Fn(),
) -> bool {
    let key_lower = data.key.to_lowercase();

    // Ctrl+Shift combinations
    if data.shift {
        match key_lower.as_str() {
            "x" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = FormattingCommands::toggle_mark(&mut ed, "strike");
                }
                on_change();
                return true;
            }
            "h" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = FormattingCommands::toggle_mark(&mut ed, "highlight");
                }
                on_change();
                return true;
            }
            "z" => {
                // Ctrl+Shift+Z = redo
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = ed.redo();
                }
                on_change();
                return true;
            }
            "8" | "*" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::set_block_type(&mut ed, "bullet_list");
                }
                on_change();
                return true;
            }
            "7" | "&" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::set_block_type(&mut ed, "ordered_list");
                }
                on_change();
                return true;
            }
            "b" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::set_block_type(&mut ed, "blockquote");
                }
                on_change();
                return true;
            }
            _ => {}
        }
    }

    // Ctrl+Alt combinations (headings)
    if data.alt {
        match key_lower.as_str() {
            "1" => { set_heading(editor, 1, on_change); return true; }
            "2" => { set_heading(editor, 2, on_change); return true; }
            "3" => { set_heading(editor, 3, on_change); return true; }
            "4" => { set_heading(editor, 4, on_change); return true; }
            "5" => { set_heading(editor, 5, on_change); return true; }
            "6" => { set_heading(editor, 6, on_change); return true; }
            "0" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::set_block_type(&mut ed, "paragraph");
                }
                on_change();
                return true;
            }
            "c" => {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = StructureCommands::set_block_type(&mut ed, "code_block");
                }
                on_change();
                return true;
            }
            _ => {}
        }
    }

    // Plain Ctrl shortcuts
    match key_lower.as_str() {
        "home" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                move_or_extend(&mut ed, Position::new(0), data.shift);
            }
            on_change();
            true
        }
        "end" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let len = ed.doc.text_length();
                move_or_extend(&mut ed, Position::new(len), data.shift);
            }
            on_change();
            true
        }
        "b" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "bold");
            }
            on_change();
            true
        }
        "i" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "italic");
            }
            on_change();
            true
        }
        "u" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "underline");
            }
            on_change();
            true
        }
        "e" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "code");
            }
            on_change();
            true
        }
        "z" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = ed.undo();
            }
            on_change();
            true
        }
        "y" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = ed.redo();
            }
            on_change();
            true
        }
        "a" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let len = ed.doc.text_length();
                ed.set_selection(Selection::new(Position::new(0), Position::new(len)));
            }
            on_change();
            true
        }
        "c" => {
            // Copy
            if let Ok(ed) = editor.try_borrow() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    let text = ed.text_in_range(sel.range());
                    EDITOR_CLIPBOARD.with(|cb| {
                        *cb.borrow_mut() = text;
                    });
                }
            }
            true
        }
        "x" => {
            // Cut
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    let text = ed.text_in_range(sel.range());
                    EDITOR_CLIPBOARD.with(|cb| {
                        *cb.borrow_mut() = text;
                    });
                    let _ = TextCommands::delete_range(&mut ed, sel.range());
                }
            }
            on_change();
            true
        }
        "v" => {
            // Paste
            let text = EDITOR_CLIPBOARD.with(|cb| cb.borrow().clone());
            if !text.is_empty() {
                if let Ok(mut ed) = editor.try_borrow_mut() {
                    let _ = TextCommands::insert_text(&mut ed, &text);
                }
                on_change();
            }
            true
        }
        "arrowleft" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let pos = ed.get_selection().head;
                let new_pos = prev_word_boundary(&ed, pos);
                move_or_extend(&mut ed, new_pos, data.shift);
            }
            on_change();
            true
        }
        "arrowright" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let pos = ed.get_selection().head;
                let new_pos = next_word_boundary(&ed, pos);
                move_or_extend(&mut ed, new_pos, data.shift);
            }
            on_change();
            true
        }
        "arrowup" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut abs = 0;
                    for i in 0..rp.block_index {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        "arrowdown" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut abs = 0;
                    for i in 0..rp.block_index {
                        abs += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                    }
                    abs += ed.doc.block_text(rp.block_index).map(|t| t.len()).unwrap_or(0);
                    move_or_extend(&mut ed, Position::new(abs), data.shift);
                }
            }
            on_change();
            true
        }
        "backspace" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    let _ = TextCommands::delete_range(&mut ed, sel.range());
                } else {
                    let word_start = prev_word_boundary(&ed, sel.head);
                    if word_start != sel.head {
                        let range = Range::new(word_start, sel.head);
                        let _ = TextCommands::delete_range(&mut ed, range);
                    }
                }
            }
            on_change();
            true
        }
        "delete" => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    let _ = TextCommands::delete_range(&mut ed, sel.range());
                } else {
                    let word_end = next_word_boundary(&ed, sel.head);
                    if word_end != sel.head {
                        let range = Range::new(sel.head, word_end);
                        let _ = TextCommands::delete_range(&mut ed, range);
                    }
                }
            }
            on_change();
            true
        }
        _ => false,
    }
}

/// Move cursor or extend selection (when shift is held).
fn move_or_extend(editor: &mut Editor, new_head: Position, shift: bool) {
    if shift {
        let anchor = editor.get_selection().anchor;
        editor.set_selection(Selection::new(anchor, new_head));
    } else {
        editor.set_selection(Selection::cursor(new_head));
    }
    editor.clear_stored_marks();
}

/// Helper: set heading level on current block.
fn set_heading(editor: &Rc<RefCell<Editor>>, level: u8, on_change: &dyn Fn()) {
    if let Ok(mut ed) = editor.try_borrow_mut() {
        let sel = ed.get_selection().clone();
        if let Ok(rp) = ed.doc.resolve_position(sel.head) {
            let mut attrs = std::collections::HashMap::new();
            attrs.insert("level".to_string(), level.to_string());
            let _ = ed.doc.set_block_type(rp.block_index, "heading", Some(attrs));
        }
    }
    on_change();
}

/// Check and apply markdown-style input rules.
/// Call this AFTER inserting a space character.
/// Returns true if a rule was applied.
fn check_input_rules(editor: &mut Editor) -> bool {
    let sel = editor.get_selection().clone();
    if let Ok(rp) = editor.doc.resolve_position(sel.head) {
        let block_text = editor.doc.block_text(rp.block_index).unwrap_or_default();

        // Get just the beginning of the block text up to cursor
        let prefix = &block_text[..rp.text_offset];

        match prefix {
            "###### " => {
                apply_heading_rule(editor, rp.block_index, 6, 7);
                return true;
            }
            "##### " => {
                apply_heading_rule(editor, rp.block_index, 5, 6);
                return true;
            }
            "#### " => {
                apply_heading_rule(editor, rp.block_index, 4, 5);
                return true;
            }
            "### " => {
                apply_heading_rule(editor, rp.block_index, 3, 4);
                return true;
            }
            "## " => {
                apply_heading_rule(editor, rp.block_index, 2, 3);
                return true;
            }
            "# " => {
                apply_heading_rule(editor, rp.block_index, 1, 2);
                return true;
            }
            "- " | "* " => {
                apply_block_rule(editor, rp.block_index, "bullet_list", 2);
                return true;
            }
            "> " => {
                apply_block_rule(editor, rp.block_index, "blockquote", 2);
                return true;
            }
            _ => {
                // Check for "1. " pattern (ordered list)
                if prefix.len() >= 3 {
                    let first_char = prefix.chars().next().unwrap_or(' ');
                    if first_char.is_ascii_digit() && prefix.ends_with(". ") {
                        // Check it's like "1. " or "2. " etc.
                        let num_part = &prefix[..prefix.len() - 2];
                        if num_part.chars().all(|c| c.is_ascii_digit()) {
                            apply_block_rule(editor, rp.block_index, "ordered_list", prefix.len());
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Apply heading input rule: delete markdown prefix and set block type.
fn apply_heading_rule(editor: &mut Editor, block_index: usize, level: u8, prefix_len: usize) {
    // Delete the markdown prefix
    let mut abs_start = 0;
    for i in 0..block_index {
        abs_start += editor.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
    }
    let _ = editor.doc.delete_range(Range::new(abs_start, abs_start + prefix_len));

    // Set block type to heading with level
    let mut attrs = std::collections::HashMap::new();
    attrs.insert("level".to_string(), level.to_string());
    let _ = editor.doc.set_block_type(block_index, "heading", Some(attrs));

    // Update cursor position (moved back by prefix_len)
    let new_pos = abs_start;
    editor.set_selection(Selection::cursor(Position::new(new_pos)));
}

/// Apply block type input rule: delete markdown prefix and set block type.
fn apply_block_rule(editor: &mut Editor, block_index: usize, block_type: &str, prefix_len: usize) {
    // Delete the markdown prefix
    let mut abs_start = 0;
    for i in 0..block_index {
        abs_start += editor.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
    }
    let _ = editor.doc.delete_range(Range::new(abs_start, abs_start + prefix_len));

    // Set block type
    let _ = editor.doc.set_block_type(block_index, block_type, None);

    // Update cursor position
    let new_pos = abs_start;
    editor.set_selection(Selection::cursor(Position::new(new_pos)));
}
