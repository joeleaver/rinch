//! Toolbar DOM rendering with Tabler Icons.
//!
//! Renders ToolbarConfig groups and controls as interactive buttons
//! that dispatch editor commands when clicked.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};

use crate::{ControlButton, ToolbarConfig, ToolbarControl, ToolbarGroup};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use rinch_editor::Editor;
use rinch_editor::commands::{FormattingCommands, StructureCommands, TextCommands};
use rinch_editor::document::Position;
use rinch_editor::selection::Selection;

// Thread-local state for dropdown open/close persistence across re-renders.
thread_local! {
    static HEADING_DROPDOWN_OPEN: RefCell<bool> = const { RefCell::new(false) };
    static COLOR_PICKER_OPEN: RefCell<bool> = const { RefCell::new(false) };
}

/// Render the full toolbar from a ToolbarConfig.
pub fn render_toolbar(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    config: &ToolbarConfig,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let toolbar = scope.create_element("div");
    toolbar.set_attribute("class", "editor-toolbar");
    toolbar.set_attribute(
        "style",
        "display: flex; flex-wrap: wrap; gap: 8px; align-items: center; \
         padding: 8px 12px; border-bottom: 1px solid var(--rinch-color-gray-3); \
         background: var(--rinch-color-gray-0);",
    );

    for (i, group) in config.groups.iter().enumerate() {
        if i > 0 {
            let divider = scope.create_element("div");
            divider.set_attribute(
                "style",
                "width: 1px; height: 24px; background: var(--rinch-color-gray-3); margin: 0 4px;",
            );
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
    // Dispatch to specialized renderers for dropdown controls.
    match control {
        ToolbarControl::HeadingDropdown => {
            return render_heading_dropdown(scope, editor, on_change);
        }
        ToolbarControl::TextColorPicker => {
            return render_color_picker(scope, editor, on_change);
        }
        _ => {}
    }

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

    // Tooltip with shortcut hint
    let title = if let Some(shortcut) = meta.shortcut_hint() {
        format!("{} ({})", meta.tooltip(), shortcut)
    } else {
        meta.tooltip().to_string()
    };
    btn.set_attribute("title", &title);

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

/// Get the current block type label for the heading dropdown.
fn current_block_label(editor: &Editor) -> &'static str {
    let sel = editor.get_selection();
    if let Ok(rp) = editor.doc.resolve_position(sel.head)
        && let Some(bt) = editor.doc.block_type(rp.block_index) {
            match bt.as_str() {
                "heading" => {
                    if let Some(attrs) = editor.doc.block_attrs(rp.block_index) {
                        match attrs.get("level").map(|s| s.as_str()) {
                            Some("1") => return "Heading 1",
                            Some("2") => return "Heading 2",
                            Some("3") => return "Heading 3",
                            Some("4") => return "Heading 4",
                            Some("5") => return "Heading 5",
                            Some("6") => return "Heading 6",
                            _ => return "Heading",
                        }
                    }
                    return "Heading";
                }
                "paragraph" => return "Paragraph",
                "blockquote" => return "Blockquote",
                "code_block" => return "Code Block",
                "bullet_list" => return "Bullet List",
                "ordered_list" => return "Ordered List",
                _ => {}
            }
        }
    "Paragraph"
}

/// Render the heading dropdown (replaces separate H1-H6 + Paragraph buttons).
fn render_heading_dropdown(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let is_open = HEADING_DROPDOWN_OPEN.with(|o| *o.borrow());

    let container = scope.create_element("div");
    container.set_attribute(
        "style",
        "position: relative; display: inline-flex; align-items: center;",
    );

    // Button showing current block type + chevron
    let btn = scope.create_element("div");
    btn.set_attribute(
        "style",
        "display: inline-flex; align-items: center; gap: 4px; \
         padding: 4px 8px; border-radius: 4px; cursor: pointer; \
         border: 1px solid transparent; font-size: 13px; \
         transition: background 0.15s; min-width: 90px;",
    );
    btn.set_attribute("title", "Block type");

    let label = if let Ok(ed) = editor.try_borrow() {
        current_block_label(&ed)
    } else {
        "Paragraph"
    };
    let label_span = scope.create_element("span");
    label_span.set_text(label);
    btn.append_child(&label_span);

    let chevron = render_tabler_icon(scope, TablerIcon::ChevronDown, TablerIconStyle::Outline);
    btn.append_child(&chevron);

    // Toggle dropdown on click
    let on_change_toggle = on_change.clone();
    let toggle_handler = scope.register_handler(move || {
        HEADING_DROPDOWN_OPEN.with(|o| {
            let mut v = o.borrow_mut();
            *v = !*v;
        });
        // Close color picker if open
        COLOR_PICKER_OPEN.with(|o| *o.borrow_mut() = false);
        on_change_toggle();
    });
    btn.set_attribute("data-rid", &toggle_handler.to_string());
    container.append_child(&btn);

    // Dropdown menu
    if is_open {
        let dropdown = scope.create_element("div");
        dropdown.set_attribute(
            "style",
            "position: absolute; top: 100%; left: 0; z-index: 1000; \
             background: var(--rinch-color-body, #fff); \
             border: 1px solid var(--rinch-color-gray-3); \
             border-radius: 4px; box-shadow: 0 2px 8px rgba(0,0,0,0.12); \
             padding: 4px 0; min-width: 150px;",
        );

        let items: &[(&str, Option<u8>)] = &[
            ("Paragraph", None),
            ("Heading 1", Some(1)),
            ("Heading 2", Some(2)),
            ("Heading 3", Some(3)),
            ("Heading 4", Some(4)),
            ("Heading 5", Some(5)),
            ("Heading 6", Some(6)),
        ];

        for &(item_label, level) in items {
            let item = scope.create_element("div");

            let font_size = match level {
                Some(1) => "18px",
                Some(2) => "16px",
                Some(3) => "15px",
                _ => "13px",
            };
            let font_weight = if level.is_some() { "600" } else { "400" };

            let item_style = format!(
                "padding: 6px 12px; cursor: pointer; font-size: {}; font-weight: {}; \
                 transition: background 0.1s;",
                font_size, font_weight,
            );
            item.set_attribute("style", &item_style);
            item.set_text(item_label);

            let editor_clone = editor.clone();
            let on_change_clone = on_change.clone();
            let item_handler = scope.register_handler(move || {
                // Close dropdown
                HEADING_DROPDOWN_OPEN.with(|o| *o.borrow_mut() = false);

                if let Ok(mut ed) = editor_clone.try_borrow_mut() {
                    if let Some(lvl) = level {
                        let mut attrs = HashMap::new();
                        attrs.insert("level".to_string(), lvl.to_string());
                        let _ = StructureCommands::set_block_type_with_attrs(
                            &mut ed, "heading", attrs,
                        );
                    } else {
                        let _ = StructureCommands::set_block_type(&mut ed, "paragraph");
                    }
                }
                on_change_clone();
            });
            item.set_attribute("data-rid", &item_handler.to_string());

            dropdown.append_child(&item);
        }

        container.append_child(&dropdown);
    }

    container
}

/// Color palette for the color picker.
const COLOR_PALETTE: &[(&str, &str)] = &[
    ("Black", "#000000"),
    ("Gray", "#868e96"),
    ("Red", "#e03131"),
    ("Orange", "#e8590c"),
    ("Yellow", "#fcc419"),
    ("Green", "#2f9e44"),
    ("Cyan", "#1098ad"),
    ("Blue", "#1971c2"),
    ("Purple", "#7048e8"),
    ("Pink", "#c2255c"),
];

/// Render the color picker dropdown (replaces separate TextColor buttons).
fn render_color_picker(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let is_open = COLOR_PICKER_OPEN.with(|o| *o.borrow());

    let container = scope.create_element("div");
    container.set_attribute(
        "style",
        "position: relative; display: inline-flex; align-items: center;",
    );

    // Button with palette icon and colored underline
    let btn = scope.create_element("div");
    btn.set_attribute(
        "style",
        "display: inline-flex; align-items: center; justify-content: center; \
         flex-direction: column; width: 32px; height: 32px; \
         border-radius: 4px; cursor: pointer; border: 1px solid transparent; \
         transition: background 0.15s;",
    );
    btn.set_attribute("title", "Text color");

    let icon_node = render_tabler_icon(scope, TablerIcon::Palette, TablerIconStyle::Outline);
    btn.append_child(&icon_node);

    // Colored underline indicator
    let underline = scope.create_element("div");
    underline.set_attribute(
        "style",
        "width: 16px; height: 3px; border-radius: 1px; \
         background: var(--rinch-primary-color, #1971c2); margin-top: -2px;",
    );
    btn.append_child(&underline);

    // Toggle dropdown on click
    let on_change_toggle = on_change.clone();
    let toggle_handler = scope.register_handler(move || {
        COLOR_PICKER_OPEN.with(|o| {
            let mut v = o.borrow_mut();
            *v = !*v;
        });
        // Close heading dropdown if open
        HEADING_DROPDOWN_OPEN.with(|o| *o.borrow_mut() = false);
        on_change_toggle();
    });
    btn.set_attribute("data-rid", &toggle_handler.to_string());
    container.append_child(&btn);

    // Dropdown color grid
    if is_open {
        let dropdown = scope.create_element("div");
        dropdown.set_attribute(
            "style",
            "position: absolute; top: 100%; left: 0; z-index: 1000; \
             background: var(--rinch-color-body, #fff); \
             border: 1px solid var(--rinch-color-gray-3); \
             border-radius: 4px; box-shadow: 0 2px 8px rgba(0,0,0,0.12); \
             padding: 8px; display: flex; flex-wrap: wrap; gap: 4px; width: 140px;",
        );

        for &(color_name, color_hex) in COLOR_PALETTE {
            let swatch = scope.create_element("div");
            let swatch_style = format!(
                "width: 24px; height: 24px; border-radius: 4px; cursor: pointer; \
                 background: {}; border: 1px solid var(--rinch-color-gray-3); \
                 transition: transform 0.1s;",
                color_hex,
            );
            swatch.set_attribute("style", &swatch_style);
            swatch.set_attribute("title", color_name);

            let editor_clone = editor.clone();
            let on_change_clone = on_change.clone();
            let color_value = color_hex.to_string();
            let swatch_handler = scope.register_handler(move || {
                // Close dropdown
                COLOR_PICKER_OPEN.with(|o| *o.borrow_mut() = false);

                if let Ok(mut ed) = editor_clone.try_borrow_mut() {
                    let sel = ed.get_selection().clone();
                    if !sel.is_cursor() {
                        let range = sel.range();
                        let mut attrs = HashMap::new();
                        attrs.insert("color".to_string(), color_value.clone());
                        let _ = ed.doc.add_mark(
                            range,
                            rinch_editor::document::MarkData::with_attrs("textColor", attrs),
                        );
                    }
                }
                on_change_clone();
            });
            swatch.set_attribute("data-rid", &swatch_handler.to_string());

            dropdown.append_child(&swatch);
        }

        container.append_child(&dropdown);
    }

    container
}

/// Check if a toolbar control's format is currently active at the cursor.
fn is_control_active(editor: &Editor, control: &ToolbarControl) -> bool {
    let sel = editor.get_selection();
    let has_mark = |mark_type: &str| -> bool {
        editor.has_stored_mark(mark_type)
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
            if let Ok(rp) = editor.doc.resolve_position(sel.head)
                && let Some(bt) = editor.doc.block_type(rp.block_index)
                && bt == "heading"
            {
                // Check if level matches
                if let Some(attrs) = editor.doc.block_attrs(rp.block_index) {
                    return attrs
                        .get("level")
                        .map(|l| l == &level.to_string())
                        .unwrap_or(false);
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
                !attrs.contains_key("align")
                    || attrs.get("align").map(|a| a == "left").unwrap_or(false)
            } else {
                false
            }
        }
        ToolbarControl::AlignCenter => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor
                    .doc
                    .block_attrs(rp.block_index)
                    .unwrap_or_default()
                    .get("align")
                    == Some(&"center".to_string())
            } else {
                false
            }
        }
        ToolbarControl::AlignRight => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor
                    .doc
                    .block_attrs(rp.block_index)
                    .unwrap_or_default()
                    .get("align")
                    == Some(&"right".to_string())
            } else {
                false
            }
        }
        ToolbarControl::AlignJustify => {
            if let Ok(rp) = editor.doc.resolve_position(sel.head) {
                editor
                    .doc
                    .block_attrs(rp.block_index)
                    .unwrap_or_default()
                    .get("align")
                    == Some(&"justify".to_string())
            } else {
                false
            }
        }
        ToolbarControl::Link => has_mark("link"),
        // Dropdown and table controls do not have a simple active state
        ToolbarControl::HeadingDropdown
        | ToolbarControl::TextColorPicker
        | ToolbarControl::InsertRowBefore
        | ToolbarControl::InsertRowAfter
        | ToolbarControl::InsertColBefore
        | ToolbarControl::InsertColAfter
        | ToolbarControl::DeleteRow
        | ToolbarControl::DeleteCol
        | ToolbarControl::ToggleHeaderRow
        | ToolbarControl::MergeCells
        | ToolbarControl::SplitCell
        | ToolbarControl::DeleteTable => false,
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
        ToolbarControl::HeadingDropdown => Some(TablerIcon::Heading),
        ToolbarControl::TextColorPicker => Some(TablerIcon::Palette),
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
        ToolbarControl::InsertRowBefore => Some(TablerIcon::RowInsertTop),
        ToolbarControl::InsertRowAfter => Some(TablerIcon::RowInsertBottom),
        ToolbarControl::InsertColBefore => Some(TablerIcon::ColumnInsertLeft),
        ToolbarControl::InsertColAfter => Some(TablerIcon::ColumnInsertRight),
        ToolbarControl::DeleteRow => Some(TablerIcon::RowRemove),
        ToolbarControl::DeleteCol => Some(TablerIcon::ColumnRemove),
        ToolbarControl::ToggleHeaderRow => Some(TablerIcon::TableRow),
        ToolbarControl::MergeCells => Some(TablerIcon::TableShortcut),
        ToolbarControl::SplitCell => Some(TablerIcon::LayoutColumns),
        ToolbarControl::DeleteTable => Some(TablerIcon::TableOff),
        ToolbarControl::Custom { .. } => None,
    }
}

/// Helper: find the table at the cursor and return (table_id, row, col).
fn find_cursor_table(editor: &Editor) -> Option<(String, usize, usize)> {
    let sel = editor.get_selection();
    if let Ok(rp) = editor.doc.resolve_position(sel.head)
        && let Some(table_id) = editor.table_id_for_block(rp.block_index) {
            // For now, default to row 0, col 0 — a proper implementation would
            // track the cursor cell within the table.
            return Some((table_id, 0, 0));
        }
    None
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
        if !ranges.is_empty()
            && let Ok(mut ed) = editor.try_borrow_mut()
        {
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
                    let start_pos =
                        rinch_editor::document::Position::new(end_pos.0 - "link text".len());
                    ed.set_selection(rinch_editor::selection::Selection::new(start_pos, end_pos));
                    // Add link mark
                    let mut attrs = HashMap::new();
                    attrs.insert("href".to_string(), "https://example.com".to_string());
                    let range = ed.get_selection().range();
                    let _ = ed.doc.add_mark(
                        range,
                        rinch_editor::document::MarkData::with_attrs("link", attrs),
                    );
                } else {
                    // Has selection: add link mark to selected text
                    let range = sel.range();
                    let mut attrs = HashMap::new();
                    attrs.insert("href".to_string(), "https://example.com".to_string());
                    let _ = ed.doc.add_mark(
                        range,
                        rinch_editor::document::MarkData::with_attrs("link", attrs),
                    );
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
                    let _ = ed.doc.add_mark(
                        range,
                        rinch_editor::document::MarkData::with_attrs("textColor", attrs),
                    );
                }
            }
        }
        ToolbarControl::Heading(level) => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let mut attrs = HashMap::new();
                attrs.insert("level".to_string(), level.to_string());
                let _ = StructureCommands::set_block_type_with_attrs(&mut ed, "heading", attrs);
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
                // Insert a 3x3 table with header row
                let _ = ed.insert_table(3, 3);
            }
        }
        ToolbarControl::AlignLeft => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.remove("align");
                    let block_type = ed
                        .doc
                        .block_type(rp.block_index)
                        .unwrap_or_else(|| "paragraph".to_string());
                    let _ =
                        StructureCommands::set_block_type_with_attrs(&mut ed, &block_type, attrs);
                }
            }
        }
        ToolbarControl::AlignCenter => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "center".to_string());
                    let block_type = ed
                        .doc
                        .block_type(rp.block_index)
                        .unwrap_or_else(|| "paragraph".to_string());
                    let _ =
                        StructureCommands::set_block_type_with_attrs(&mut ed, &block_type, attrs);
                }
            }
        }
        ToolbarControl::AlignRight => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "right".to_string());
                    let block_type = ed
                        .doc
                        .block_type(rp.block_index)
                        .unwrap_or_else(|| "paragraph".to_string());
                    let _ =
                        StructureCommands::set_block_type_with_attrs(&mut ed, &block_type, attrs);
                }
            }
        }
        ToolbarControl::AlignJustify => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let mut attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                    attrs.insert("align".to_string(), "justify".to_string());
                    let block_type = ed
                        .doc
                        .block_type(rp.block_index)
                        .unwrap_or_else(|| "paragraph".to_string());
                    let _ =
                        StructureCommands::set_block_type_with_attrs(&mut ed, &block_type, attrs);
                }
            }
        }
        // HeadingDropdown and TextColorPicker are handled by their own renderers
        ToolbarControl::HeadingDropdown | ToolbarControl::TextColorPicker => {}
        // Table manipulation commands
        ToolbarControl::InsertRowBefore => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, row, _col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        table.insert_row(row);
                    }
        }
        ToolbarControl::InsertRowAfter => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, row, _col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        table.insert_row(row + 1);
                    }
        }
        ToolbarControl::InsertColBefore => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, _row, col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        table.insert_column(col);
                    }
        }
        ToolbarControl::InsertColAfter => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, _row, col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        table.insert_column(col + 1);
                    }
        }
        ToolbarControl::DeleteRow => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, row, _col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        let _ = table.delete_row(row);
                    }
        }
        ToolbarControl::DeleteCol => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, _row, col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        let _ = table.delete_column(col);
                    }
        }
        ToolbarControl::ToggleHeaderRow => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, _row, _col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        table.toggle_header_row();
                    }
        }
        ToolbarControl::MergeCells => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, row, col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        // Merge the current cell with the one to the right
                        let _ = table.merge_cells((row, col), (row, col.saturating_add(1)));
                    }
        }
        ToolbarControl::SplitCell => {
            if let Ok(mut ed) = editor.try_borrow_mut()
                && let Some((table_id, row, col)) = find_cursor_table(&ed)
                    && let Some(table) = ed.get_table_mut(&table_id) {
                        let _ = table.split_cell(row, col);
                    }
        }
        ToolbarControl::DeleteTable => {
            if let Ok(mut ed) = editor.try_borrow_mut() {
                let sel = ed.get_selection().clone();
                if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                    let _ = ed.delete_table(rp.block_index);
                }
            }
        }
        ToolbarControl::Custom { .. } => {}
    }

    on_change();
}
