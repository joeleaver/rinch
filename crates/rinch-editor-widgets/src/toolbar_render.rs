//! Toolbar DOM rendering with Tabler Icons.
//!
//! Renders ToolbarConfig groups and controls as interactive buttons
//! that dispatch editor commands when clicked.

use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

use rinch_core::dom::{RenderScope, NodeHandle};

use crate::{ToolbarConfig, ToolbarGroup, ToolbarControl, ControlButton};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use rinch_editor::Editor;
use rinch_editor::commands::{TextCommands, FormattingCommands, StructureCommands};
use rinch_editor::document::Position;
use rinch_editor::selection::Selection;

/// Render the full toolbar from a ToolbarConfig.
pub fn render_toolbar(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    config: &ToolbarConfig,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let toolbar = scope.create_element("div");
    toolbar.set_attribute("class", "editor-toolbar");
    toolbar.set_attribute("style",
        "display: flex; flex-wrap: wrap; gap: 8px; align-items: center; \
         padding: 8px 12px; border-bottom: 1px solid var(--rinch-color-gray-3); \
         background: var(--rinch-color-gray-0);");

    for (i, group) in config.groups.iter().enumerate() {
        if i > 0 {
            let divider = scope.create_element("div");
            divider.set_attribute("style",
                "width: 1px; height: 24px; background: var(--rinch-color-gray-3); margin: 0 4px;");
            toolbar.append_child(&divider);
        }

        let group_node = render_toolbar_group(scope, editor.clone(), group, on_change.clone());
        toolbar.append_child(&group_node);
    }

    toolbar
}

/// Render a single toolbar group.
fn render_toolbar_group(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    group: &ToolbarGroup,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let group_div = scope.create_element("div");
    group_div.set_attribute("style", "display: flex; gap: 2px; align-items: center;");

    for control in &group.controls {
        let btn_node = render_toolbar_button(scope, editor.clone(), control, on_change.clone());
        group_div.append_child(&btn_node);
    }

    group_div
}

/// Render a single toolbar button with icon and click handler.
fn render_toolbar_button(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    control: &ToolbarControl,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let meta = ControlButton::from_control(control.clone());

    let btn = scope.create_element("div");

    // Check active state
    let is_active = if let Ok(ed) = editor.try_borrow() {
        is_control_active(&ed, control)
    } else {
        false
    };

    let style = if is_active {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid var(--rinch-primary-color); \
         background: var(--rinch-color-blue-1); \
         color: var(--rinch-primary-color); \
         transition: background 0.15s;"
    } else {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid transparent; \
         transition: background 0.15s;"
    };
    btn.set_attribute("style", style);
    btn.set_attribute("title", meta.tooltip());

    // Try to render a Tabler icon; fall back to text label
    if let Some(icon) = control_to_tabler_icon(control) {
        let icon_node = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
        btn.append_child(&icon_node);
    } else {
        let label_node = scope.create_element("span");
        label_node.set_attribute("style", "font-size: 12px; font-weight: 600;");
        label_node.set_text(meta.label());
        btn.append_child(&label_node);
    }

    // Register click handler
    let control_clone = control.clone();
    let editor_clone = editor.clone();
    let on_change_clone = on_change.clone();
    let handler_id = scope.register_handler(move || {
        execute_toolbar_command(&editor_clone, &control_clone, &*on_change_clone);
    });
    btn.set_attribute("data-rid", &handler_id.to_string());

    btn
}

/// Check if a toolbar control's format is currently active at the cursor.
fn is_control_active(editor: &Editor, control: &ToolbarControl) -> bool {
    let sel = editor.get_selection();
    let marks = editor.doc.marks_at(sel.head);
    let stored = &editor.stored_marks;

    let has_mark = |mark_type: &str| -> bool {
        marks.iter().any(|m| m.mark_type == mark_type) || stored.iter().any(|m| m == mark_type)
    };

    match control {
        ToolbarControl::Bold => has_mark("bold"),
        ToolbarControl::Italic => has_mark("italic"),
        ToolbarControl::Underline => has_mark("underline"),
        ToolbarControl::Strike => has_mark("strike"),
        ToolbarControl::Code => has_mark("code"),
        ToolbarControl::Highlight => has_mark("highlight"),
        ToolbarControl::Subscript => has_mark("subscript"),
        ToolbarControl::Superscript => has_mark("superscript"),
        ToolbarControl::Heading(level) => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                if let Some(bt) = editor.doc.block_type(rp.block_index) {
                    if bt == "heading" {
                        // Check if level matches
                        if let Some(attrs) = editor.doc.block_attrs(rp.block_index) {
                            return attrs.get("level").map(|l| l == &level.to_string()).unwrap_or(false);
                        }
                    }
                }
            }
            false
        }
        ToolbarControl::Paragraph => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_type(rp.block_index) == Some("paragraph".to_string())
            } else {
                false
            }
        }
        ToolbarControl::BulletList => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_type(rp.block_index) == Some("bullet_list".to_string())
            } else {
                false
            }
        }
        ToolbarControl::OrderedList => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_type(rp.block_index) == Some("ordered_list".to_string())
            } else {
                false
            }
        }
        ToolbarControl::Blockquote => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_type(rp.block_index) == Some("blockquote".to_string())
            } else {
                false
            }
        }
        ToolbarControl::CodeBlock => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_type(rp.block_index) == Some("code_block".to_string())
            } else {
                false
            }
        }
        ToolbarControl::AlignLeft => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                let attrs = editor.doc.block_attrs(rp.block_index).unwrap_or_default();
                !attrs.contains_key("align") || attrs.get("align").map(|a| a == "left").unwrap_or(false)
            } else {
                false
            }
        }
        ToolbarControl::AlignCenter => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_attrs(rp.block_index).unwrap_or_default().get("align") == Some(&"center".to_string())
            } else {
                false
            }
        }
        ToolbarControl::AlignRight => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_attrs(rp.block_index).unwrap_or_default().get("align") == Some(&"right".to_string())
            } else {
                false
            }
        }
        ToolbarControl::AlignJustify => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor.doc.block_attrs(rp.block_index).unwrap_or_default().get("align") == Some(&"justify".to_string())
            } else {
                false
            }
        }
        ToolbarControl::Link => {
            marks.iter().any(|m| m.mark_type == "link") || stored.iter().any(|m| m == "link")
        }
        _ => false,
    }
}

/// Map a ToolbarControl to the corresponding TablerIcon variant.
fn control_to_tabler_icon(control: &ToolbarControl) -> Option<TablerIcon> {
    match control {
        ToolbarControl::Bold => Some(TablerIcon::Bold),
        ToolbarControl::Italic => Some(TablerIcon::Italic),
        ToolbarControl::Underline => Some(TablerIcon::Underline),
        ToolbarControl::Strike => Some(TablerIcon::Strikethrough),
        ToolbarControl::Code => Some(TablerIcon::Code),
        ToolbarControl::Highlight => Some(TablerIcon::Highlight),
        ToolbarControl::Subscript => Some(TablerIcon::Subscript),
        ToolbarControl::Superscript => Some(TablerIcon::Superscript),
        ToolbarControl::Link => Some(TablerIcon::Link),
        ToolbarControl::TextColor(_) => Some(TablerIcon::Palette),
        ToolbarControl::Heading(1) => Some(TablerIcon::H1),
        ToolbarControl::Heading(2) => Some(TablerIcon::H2),
        ToolbarControl::Heading(3) => Some(TablerIcon::H3),
        ToolbarControl::Heading(4) => Some(TablerIcon::H4),
        ToolbarControl::Heading(5) => Some(TablerIcon::H5),
        ToolbarControl::Heading(6) => Some(TablerIcon::H6),
        ToolbarControl::Heading(_) => Some(TablerIcon::Heading),
        ToolbarControl::Paragraph => Some(TablerIcon::Pilcrow),
        ToolbarControl::BulletList => Some(TablerIcon::List),
        ToolbarControl::OrderedList => Some(TablerIcon::ListNumbers),
        ToolbarControl::Blockquote => Some(TablerIcon::Blockquote),
        ToolbarControl::CodeBlock => Some(TablerIcon::SourceCode),
        ToolbarControl::HorizontalRule => Some(TablerIcon::SeparatorHorizontal),
        ToolbarControl::HardBreak => Some(TablerIcon::TextWrap),
        ToolbarControl::Undo => Some(TablerIcon::ArrowBackUp),
        ToolbarControl::Redo => Some(TablerIcon::ArrowForwardUp),
        ToolbarControl::ClearFormatting => Some(TablerIcon::ClearFormatting),
        ToolbarControl::InsertTable => Some(TablerIcon::Table),
        ToolbarControl::AlignLeft => Some(TablerIcon::AlignLeft),
        ToolbarControl::AlignCenter => Some(TablerIcon::AlignCenter),
        ToolbarControl::AlignRight => Some(TablerIcon::AlignRight),
        ToolbarControl::AlignJustify => Some(TablerIcon::AlignJustified),
        ToolbarControl::Custom { .. } => None,
    }
}

/// Execute the editor command associated with a toolbar control.
fn execute_toolbar_command(
    editor: &Rc<RefCell<Editor>>,
    control: &ToolbarControl,
    on_change: &dyn Fn(),
) {
    // Sync editor's internal selection from blitz's visual selection.
    // query_selection_ranges returns (block_index, start_byte, end_byte) tuples.
    {
        let ranges = rinch_core::events::query_selection_ranges();
        if !ranges.is_empty() {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let block_count = ed.doc.block_count();
                let doc_len = ed.doc.text_length();

                // Compute absolute position for the first range's start
                let (first_bi, first_start, _) = ranges[0];
                let first_bi = first_bi.min(block_count.saturating_sub(1));
                let mut abs_start = 0usize;
                for i in 0..first_bi {
                    abs_start += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                }
                let first_block_len = ed.doc.block_text(first_bi).map(|t| t.len()).unwrap_or(0);
                abs_start += first_start.min(first_block_len);

                // Compute absolute position for the last range's end
                let (last_bi, _, last_end) = *ranges.last().unwrap();
                let last_bi = last_bi.min(block_count.saturating_sub(1));
                let mut abs_end = 0usize;
                for i in 0..last_bi {
                    abs_end += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                }
                let last_block_len = ed.doc.block_text(last_bi).map(|t| t.len()).unwrap_or(0);
                abs_end += last_end.min(last_block_len);

                ed.set_selection(Selection::new(
                    Position::new(abs_start.min(doc_len)),
                    Position::new(abs_end.min(doc_len)),
                ));
            }
        }
    }

    match control {
        ToolbarControl::Bold => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "bold");
            }
        }
        ToolbarControl::Italic => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "italic");
            }
        }
        ToolbarControl::Underline => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "underline");
            }
        }
        ToolbarControl::Strike => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "strike");
            }
        }
        ToolbarControl::Code => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "code");
            }
        }
        ToolbarControl::Highlight => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "highlight");
            }
        }
        ToolbarControl::Subscript => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "subscript");
            }
        }
        ToolbarControl::Superscript => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::toggle_mark(&mut ed, "superscript");
            }
        }
        ToolbarControl::Link => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if sel.is_cursor() {
                    // No selection: insert placeholder link text
                    let _ = TextCommands::insert_text(&mut ed, "link text");
                    // Select the inserted text
                    let end_pos = ed.get_selection().head;
                    let start_pos = rinch_editor::document::Position::new(end_pos.0 - "link text".len());
                    ed.set_selection(rinch_editor::selection::Selection::new(start_pos, end_pos));
                    // Add link mark
                    let mut attrs = HashMap::new();
                    attrs.insert("href".to_string(), "https://example.com".to_string());
                    let range = ed.get_selection().range();
                    let _ = ed.doc.add_mark(range, rinch_editor::document::MarkData::with_attrs("link", attrs));
                } else {
                    // Has selection: add link mark to selected text
                    let range = sel.range();
                    let mut attrs = HashMap::new();
                    attrs.insert("href".to_string(), "https://example.com".to_string());
                    let _ = ed.doc.add_mark(range, rinch_editor::document::MarkData::with_attrs("link", attrs));
                }
            }
        }
        ToolbarControl::TextColor(color) => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if !sel.is_cursor() {
                    let range = sel.range();
                    let mut attrs = HashMap::new();
                    attrs.insert("color".to_string(), color.clone());
                    let _ = ed.doc.add_mark(range, rinch_editor::document::MarkData::with_attrs("textColor", attrs));
                }
            }
        }
        ToolbarControl::Heading(level) => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let mut attrs = HashMap::new();
                attrs.insert("level".to_string(), level.to_string());
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let _ = ed.doc.set_block_type(rp.block_index, "heading", Some(attrs));
                }
            }
        }
        ToolbarControl::Paragraph => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::set_block_type(&mut ed, "paragraph");
            }
        }
        ToolbarControl::BulletList => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::set_block_type(&mut ed, "bullet_list");
            }
        }
        ToolbarControl::OrderedList => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::set_block_type(&mut ed, "ordered_list");
            }
        }
        ToolbarControl::Blockquote => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::set_block_type(&mut ed, "blockquote");
            }
        }
        ToolbarControl::CodeBlock => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::set_block_type(&mut ed, "code_block");
            }
        }
        ToolbarControl::HorizontalRule => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::split_block(&mut ed);
                let _ = StructureCommands::set_block_type(&mut ed, "horizontal_rule");
            }
        }
        ToolbarControl::HardBreak => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = StructureCommands::split_block(&mut ed);
            }
        }
        ToolbarControl::Undo => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = ed.undo();
            }
        }
        ToolbarControl::Redo => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = ed.redo();
            }
        }
        ToolbarControl::ClearFormatting => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::clear_formatting(&mut ed);
            }
        }
        ToolbarControl::InsertTable => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                // Insert a placeholder table using split blocks
                let _ = StructureCommands::split_block(&mut ed);
                let _ = TextCommands::insert_text(&mut ed, "| Header 1 | Header 2 | Header 3 |");
                let _ = StructureCommands::split_block(&mut ed);
                let _ = TextCommands::insert_text(&mut ed, "|----------|----------|----------|");
                let _ = StructureCommands::split_block(&mut ed);
                let _ = TextCommands::insert_text(&mut ed, "| Cell 1   | Cell 2   | Cell 3   |");
                let _ = StructureCommands::split_block(&mut ed);
                let _ = TextCommands::insert_text(&mut ed, "| Cell 4   | Cell 5   | Cell 6   |");
            }
        }
        ToolbarControl::AlignLeft => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.remove("align");
                    let block_type = ed.doc.block_type(rp.block_index).unwrap_or_else(|| "paragraph".to_string());
                    let _ = ed.doc.set_block_type(rp.block_index, &block_type, Some(attrs));
                }
            }
        }
        ToolbarControl::AlignCenter => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "center".to_string());
                    let block_type = ed.doc.block_type(rp.block_index).unwrap_or_else(|| "paragraph".to_string());
                    let _ = ed.doc.set_block_type(rp.block_index, &block_type, Some(attrs));
                }
            }
        }
        ToolbarControl::AlignRight => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "right".to_string());
                    let block_type = ed.doc.block_type(rp.block_index).unwrap_or_else(|| "paragraph".to_string());
                    let _ = ed.doc.set_block_type(rp.block_index, &block_type, Some(attrs));
                }
            }
        }
        ToolbarControl::AlignJustify => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "justify".to_string());
                    let block_type = ed.doc.block_type(rp.block_index).unwrap_or_else(|| "paragraph".to_string());
                    let _ = ed.doc.set_block_type(rp.block_index, &block_type, Some(attrs));
                }
            }
        }
        ToolbarControl::Custom { .. } => {}
    }

    on_change();
}
