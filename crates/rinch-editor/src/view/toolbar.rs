//! Toolbar DOM rendering with Tabler Icons.
//!
//! Renders ToolbarConfig groups and controls as interactive buttons
//! that dispatch editor commands when clicked.

use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

use rinch_core::dom::{RenderScope, NodeHandle};

use rinch_editor_widgets::{ToolbarConfig, ToolbarGroup, ToolbarControl, ControlButton};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use crate::editor::Editor;
use crate::commands::{TextCommands, FormattingCommands, StructureCommands};

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
    btn.set_attribute("style",
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid transparent; \
         transition: background 0.15s;");
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
    let handler_id = scope.register_handler(Rc::new(move || {
        execute_toolbar_command(&editor_clone, &control_clone, &on_change_clone);
    }));
    btn.set_attribute("data-rid", &handler_id.to_string());

    btn
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
            // Link requires a dialog - placeholder
        }
        ToolbarControl::TextColor(_color) => {
            // Color picker - placeholder
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
                let _ = TextCommands::insert_text(&mut ed, "\n");
            }
        }
        ToolbarControl::Undo => {
            // Undo - placeholder (requires History integration)
        }
        ToolbarControl::Redo => {
            // Redo - placeholder
        }
        ToolbarControl::ClearFormatting => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let _ = FormattingCommands::clear_formatting(&mut ed);
            }
        }
        ToolbarControl::InsertTable => {
            // Table insertion - placeholder
        }
        ToolbarControl::AlignLeft | ToolbarControl::AlignCenter |
        ToolbarControl::AlignRight | ToolbarControl::AlignJustify => {
            // Alignment - placeholder
        }
        ToolbarControl::Custom { .. } => {}
    }

    on_change();
}
