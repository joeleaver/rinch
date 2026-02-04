//! Rich Text Editor section - Working editor with toolbar, content, and status bar.

use std::rc::Rc;
use std::cell::RefCell;
use std::time::Instant;

use rinch::prelude::*;
use rinch_core::dom::RenderScope as CoreRenderScope;
use rinch_core::reactive::Effect;
use rinch_editor::editor::{Editor, EditorConfig};
use rinch_editor::schema::Schema;
use rinch_editor::view::{create_input_bridge, render_document_reactive, apply_changes, BlockSignals};
use rinch_editor_widgets::{ToolbarConfig, ControlButton, render_toolbar, render_status_bar};

// Thread-local state for multi-click detection
thread_local! {
    static LAST_CLICK: RefCell<Option<Instant>> = RefCell::new(None);
    static CLICK_COUNT: RefCell<u32> = RefCell::new(0);
}

/// State for the Editor section, stored in context.
#[derive(Clone)]
pub struct EditorSectionState {
    pub toolbar_preset: Signal<usize>, // 0=Full, 1=Minimal, 2=Markdown
    pub editor: Rc<RefCell<Editor>>,
    pub block_signals: Rc<RefCell<Option<Rc<BlockSignals>>>>,
}

/// Initialize the Editor section state. Call this from the main app function.
pub fn init_editor_state() {
    let editor = Editor::new(Schema::starter_kit(), EditorConfig::default())
        .expect("Failed to create editor");
    create_context(EditorSectionState {
        toolbar_preset: Signal::new(0),
        editor: Rc::new(RefCell::new(editor)),
        block_signals: Rc::new(RefCell::new(None)),
    });
}

fn get_toolbar_config(preset: usize) -> ToolbarConfig {
    match preset {
        1 => ToolbarConfig::default_minimal(),
        2 => ToolbarConfig::default_markdown(),
        _ => ToolbarConfig::default_full(),
    }
}

fn preset_name(preset: usize) -> &'static str {
    match preset {
        1 => "Minimal",
        2 => "Markdown",
        _ => "Full",
    }
}

pub fn editor_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<EditorSectionState>();

    let state = match state {
        Some(s) => s,
        None => {
            return rsx! {
                div { "Error: EditorSectionState not initialized" }
            };
        }
    };

    let toolbar_preset = state.toolbar_preset;
    let editor = state.editor.clone();

    // Create on_change callback that applies surgical updates
    let signals_for_change = state.block_signals.clone();
    let editor_for_change = editor.clone();
    let on_change: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(signals) = signals_for_change.borrow().as_ref() {
            apply_changes(&editor_for_change, signals);
        }
    });


    // Render the working toolbar
    let toolbar_config = get_toolbar_config(toolbar_preset.get());
    let toolbar_node = render_toolbar(__scope, editor.clone(), &toolbar_config, on_change.clone());

    // Create input bridge (hidden textarea + keyboard interceptor) BEFORE click handler
    // so we can use the focus callback in the click handler
    let (input_bridge_node, focus_textarea) = create_input_bridge(__scope, editor.clone(), on_change.clone());

    // Build reactive content area using Effect pattern (like show_dom)
    let content_div = __scope.create_element("div");
    content_div.set_attribute("class", "editor-content");
    content_div.set_attribute("style",
        "min-height: 300px; padding: 16px 24px; background: var(--rinch-color-body); \
         font-size: 16px; line-height: 1.6; color: var(--rinch-color-text); \
         outline: none; cursor: text;");

    // Register click handler on content area for cursor positioning and multi-click selection
    {
        let editor_for_click = editor.clone();
        let focus_textarea_for_click = focus_textarea.clone();
        let on_change_for_click = on_change.clone();
        let handler_id = __scope.register_handler(move || {
            // Focus the hidden textarea so keyboard input is captured
            focus_textarea_for_click();
            let ctx = rinch_core::events::get_click_context();

            // Multi-click detection
            let click_count = LAST_CLICK.with(|lc| {
                let now = Instant::now();
                let count = CLICK_COUNT.with(|cc| {
                    let mut c = cc.borrow_mut();
                    if let Some(last) = *lc.borrow() {
                        if now.duration_since(last).as_millis() < 500 {
                            *c = (*c % 3) + 1; // cycle 1 -> 2 -> 3 -> 1
                        } else {
                            *c = 1;
                        }
                    } else {
                        *c = 1;
                    }
                    *c
                });
                *lc.borrow_mut() = Some(now);
                count
            });

            match click_count {
                2 => {
                    // Double-click: select word at click position
                    if let Ok(mut ed) = editor_for_click.try_borrow_mut() {
                        let block_count = ed.doc.block_count();
                        if block_count > 0 && ctx.text_hit.valid {
                            let block_idx = ctx.text_hit.block_index.min(block_count.saturating_sub(1));
                            let block_text_len = ed.doc.block_text(block_idx).map(|t| t.len()).unwrap_or(0);
                            let char_offset = ctx.text_hit.byte_offset.min(block_text_len);

                            let mut abs_pos = 0usize;
                            for i in 0..block_idx {
                                abs_pos += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                            }
                            abs_pos += char_offset;
                            let doc_len = ed.doc.text_length();
                            abs_pos = abs_pos.min(doc_len);

                            // Find word boundaries
                            let text = ed.doc.to_text();
                            let chars: Vec<char> = text.chars().collect();
                            let p = abs_pos.min(chars.len());

                            let mut start = p;
                            while start > 0 && !chars[start - 1].is_whitespace() {
                                start -= 1;
                            }
                            let mut end = p;
                            while end < chars.len() && !chars[end].is_whitespace() {
                                end += 1;
                            }

                            use rinch_editor::document::Position;
                            use rinch_editor::selection::Selection;
                            if start != end {
                                ed.set_selection(Selection::new(Position::new(start), Position::new(end)));
                            }
                            ed.clear_stored_marks();

                            // Set blitz's native text selection for the word
                            let node_id = ctx.text_hit.inline_root_node_id;
                            let block_start_abs = abs_pos - char_offset;
                            let sel_start_in_block = start.saturating_sub(block_start_abs);
                            let sel_end_in_block = end.saturating_sub(block_start_abs);
                            rinch_core::events::dispatch_selection(
                                rinch_core::SelectionAction::Set {
                                    anchor_node: node_id,
                                    anchor_offset: sel_start_in_block,
                                    focus_node: node_id,
                                    focus_offset: sel_end_in_block,
                                }
                            );
                        }
                    }
                    // Don't call on_change here - it re-renders the DOM which
                    // destroys the nodes blitz is tracking for selection.
                    // Blitz handles visual word selection natively on double-click.
                }
                3 => {
                    // Triple-click: select entire block
                    if let Ok(mut ed) = editor_for_click.try_borrow_mut() {
                        let block_count = ed.doc.block_count();
                        if block_count > 0 && ctx.text_hit.valid {
                            let block_idx = ctx.text_hit.block_index.min(block_count.saturating_sub(1));
                            let block_len = ed.doc.block_text(block_idx).map(|t| t.len()).unwrap_or(0);

                            let mut abs_start = 0;
                            for i in 0..block_idx {
                                abs_start += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                            }
                            let abs_end = abs_start + block_len;

                            use rinch_editor::document::Position;
                            use rinch_editor::selection::Selection;
                            ed.set_selection(Selection::new(Position::new(abs_start), Position::new(abs_end)));
                            ed.clear_stored_marks();

                            // Set blitz's native text selection for the block
                            let node_id = ctx.text_hit.inline_root_node_id;
                            rinch_core::events::dispatch_selection(
                                rinch_core::SelectionAction::Set {
                                    anchor_node: node_id,
                                    anchor_offset: 0,
                                    focus_node: node_id,
                                    focus_offset: block_len,
                                }
                            );
                        }
                    }
                    // Don't call on_change - same reason as above.
                }
                _ => {
                    // Single click: position cursor or extend selection (with Shift)
                    let shift_held = rinch_core::events::get_modifier_state().shift;

                    if let Ok(mut ed) = editor_for_click.try_borrow_mut() {
                        let block_count = ed.doc.block_count();
                        if block_count == 0 {
                            return;
                        }

                        use rinch_editor::document::Position;
                        use rinch_editor::selection::Selection;

                        // Use blitz's text layout hit testing when available
                        let abs_pos = if ctx.text_hit.valid {
                            let bi = ctx.text_hit.block_index.min(block_count.saturating_sub(1));
                            let mut pos = 0usize;
                            for i in 0..bi {
                                pos += ed.doc.block_text(i).map(|t| t.len()).unwrap_or(0) + 1;
                            }
                            let block_len = ed.doc.block_text(bi).map(|t| t.len()).unwrap_or(0);
                            pos += ctx.text_hit.byte_offset.min(block_len);
                            pos.min(ed.doc.text_length())
                        } else {
                            // Fallback: place cursor at end of document
                            ed.doc.text_length()
                        };

                        if shift_held {
                            let anchor = ed.get_selection().anchor;
                            ed.set_selection(Selection::new(anchor, Position::new(abs_pos)));
                        } else {
                            ed.set_selection(Selection::cursor(Position(abs_pos)));
                        }
                        ed.clear_stored_marks();

                        // Blitz handles single-click text selection automatically.
                        // We only need to dispatch for shift-click (blitz doesn't handle it for non-inputs).
                        if shift_held && ctx.text_hit.valid {
                            rinch_core::events::dispatch_selection(
                                rinch_core::SelectionAction::ExtendToPoint {
                                    x: ctx.mouse_x,
                                    y: ctx.mouse_y,
                                }
                            );
                        }
                    }
                    // Update cursor visual after click-to-position
                    // (In rinch-dom we handle cursor rendering ourselves, not via blitz selection)
                    on_change_for_click();
                }
            }
        });
        content_div.set_attribute("data-rid", &handler_id.to_string());
    }

    // Register mouseup selection sync: after drag-to-select, sync blitz's visual
    // selection into the editor's internal selection model.
    {
        let editor_for_sync = editor.clone();
        let signals_for_sync = state.block_signals.clone();
        rinch_core::events::set_selection_sync_callback(move |ranges| {
            if !ranges.is_empty() {
            if let Ok(mut ed) = editor_for_sync.try_borrow_mut() {
                let block_count = ed.doc.block_count();
                let doc_len = ed.doc.text_length();
                if block_count == 0 {
                    return;
                }

                use rinch_editor::document::Position;
                use rinch_editor::selection::Selection;

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

                // Only update if we have a real range (not just a cursor)
                if abs_start != abs_end {
                    ed.set_selection(Selection::new(
                        Position::new(abs_start.min(doc_len)),
                        Position::new(abs_end.min(doc_len)),
                    ));
                    ed.clear_stored_marks();
                    // Don't call on_change here to avoid re-render that clears blitz selection
                }
            }
            } else {
                // Empty ranges = single click (no drag selection).
                // Bump to show the cursor caret that was set on mousedown.
                if let Some(signals) = signals_for_sync.borrow().as_ref() {
                    signals.bump();
                }
            }
            // When ranges are non-empty, DON'T bump — blitz is already showing
            // the native selection highlight and re-rendering would destroy it.
        });
    }

    // Initial content render - reactive with per-block Effects
    let (content_node, signals) = render_document_reactive(__scope, editor.clone());
    content_div.append_child(&content_node);
    *state.block_signals.borrow_mut() = Some(signals.clone());

    // Capture render_version signal for status bar
    let status_version = signals.render_version;

    // Build reactive status bar
    let status_div = __scope.create_element("div");
    {
        let status_handle = status_div.clone();
        let editor_for_status = editor.clone();
        let doc_weak = __scope.doc_weak();
        let container_id = status_div.node_id();
        let version = status_version;
        let current_content: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let current_scope: Rc<RefCell<Option<CoreRenderScope>>> = Rc::new(RefCell::new(None));

        // Initial render
        let initial_status = render_status_bar(__scope, &editor);
        status_div.append_child(&initial_status);

        let initial_version = version.get();
        let prev_version: Rc<RefCell<u64>> = Rc::new(RefCell::new(initial_version));

        let effect = Effect::new(move || {
            let v = version.get();
            let prev = *prev_version.borrow();
            if v == prev {
                return;
            }
            *prev_version.borrow_mut() = v;

            if let Some(old_scope) = current_scope.borrow_mut().take() {
                old_scope.dispose();
            }
            if let Some(old_content) = current_content.borrow_mut().take() {
                old_content.clear_animations();
                old_content.remove();
            }

            if let Some(doc) = doc_weak.upgrade() {
                let mut child_scope = CoreRenderScope::new(doc, container_id);
                let new_status = render_status_bar(&mut child_scope, &editor_for_status);
                status_handle.append_child(&new_status);
                *current_content.borrow_mut() = Some(new_status);
                *current_scope.borrow_mut() = Some(child_scope);
            }
        });
        __scope.create_effect_from(effect);
    }

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Rich Text Editor" }
                Text { size: "lg", color: "dimmed",
                    "Working rich-text editor with toolbar, keyboard shortcuts, and content rendering."
                }
            }
            Space { h: "xl" }

            // Preset selector
            Group { gap: "sm",
                Button {
                    variant: "filled",
                    onclick: move || toolbar_preset.set(0),
                    "Full"
                }
                Button {
                    variant: "light",
                    onclick: move || toolbar_preset.set(1),
                    "Minimal"
                }
                Button {
                    variant: "light",
                    onclick: move || toolbar_preset.set(2),
                    "Markdown"
                }
                Text { size: "sm", color: "dimmed",
                    "Active: "
                    {|| preset_name(toolbar_preset.get()).to_string()}
                }
            }
            Space { h: "md" }

            // Editor container
            Paper { p: "0", radius: "md", with_border: true,
                // Toolbar
                {toolbar_node}

                // Content area (reactive)
                {content_div}

                // Status bar (reactive)
                {status_div}
            }

            // Hidden input bridge (keyboard interceptor)
            {input_bridge_node}

            Space { h: "xl" }

            // Editor CSS
            div {
                style: "display: none;",
                {render_editor_styles(__scope)}
            }

            // Keyboard Shortcuts Reference
            Title { order: 3, "Keyboard Shortcuts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Controls with keyboard shortcuts from the current toolbar preset." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                div {
                    {render_shortcuts_list(__scope, toolbar_preset)}
                }
            }

            Space { h: "xl" }

            // Configuration info
            Title { order: 3, "Configuration" }
            Space { h: "sm" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { size: "sm",
                        "The toolbar is configured using the builder API. Choose from built-in presets or create a custom configuration with ToolbarConfig."
                    }
                    Space { h: "xs" }

                    SimpleGrid { cols: Some(3), spacing: Some("md".to_string()),
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Full" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_full().control_count(),
                                        ToolbarConfig::default_full().groups.len())}
                                }
                            }
                        }
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Minimal" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_minimal().control_count(),
                                        ToolbarConfig::default_minimal().groups.len())}
                                }
                            }
                        }
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Markdown" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_markdown().control_count(),
                                        ToolbarConfig::default_markdown().groups.len())}
                                }
                            }
                        }
                    }

                    Space { h: "xs" }
                    Text { size: "sm", color: "dimmed",
                        "Current preset: "
                        {|| {
                            let config = get_toolbar_config(toolbar_preset.get());
                            format!("{} controls across {} groups", config.control_count(), config.groups.len())
                        }}
                    }
                }
            }
        }
    }
}

/// Render editor-specific CSS styles.
fn render_editor_styles(__scope: &mut RenderScope) -> NodeHandle {
    use rinch_editor::view::cursor_blink_css;

    let style = __scope.create_element("style");
    let mut css = String::from(r#"
        .editor-content p { margin: 0 0 8px 0; }
        .editor-content h1 { font-size: 2em; font-weight: 700; margin: 16px 0 8px 0; }
        .editor-content h2 { font-size: 1.5em; font-weight: 700; margin: 14px 0 6px 0; }
        .editor-content h3 { font-size: 1.25em; font-weight: 600; margin: 12px 0 6px 0; }
        .editor-content h4 { font-size: 1.1em; font-weight: 600; margin: 10px 0 4px 0; }
        .editor-content h5 { font-size: 1em; font-weight: 600; margin: 8px 0 4px 0; }
        .editor-content h6 { font-size: 0.9em; font-weight: 600; margin: 8px 0 4px 0; }
        .editor-content blockquote {
            border-left: 3px solid var(--rinch-color-gray-4);
            padding-left: 16px; margin: 8px 0;
            color: var(--rinch-color-dimmed);
        }
        .editor-content pre {
            background: var(--rinch-color-gray-1);
            border-radius: var(--rinch-radius-sm);
            padding: 12px; margin: 8px 0;
            font-family: monospace; font-size: 14px;
            overflow-x: auto;
        }
        .editor-content code {
            background: var(--rinch-color-gray-1);
            padding: 2px 4px; border-radius: 3px;
            font-size: 0.9em;
        }
        .editor-content pre code {
            background: none; padding: 0; border-radius: 0;
        }
        .editor-content ul, .editor-content ol {
            margin: 8px 0; padding-left: 24px;
        }
        .editor-content li { margin: 2px 0; }
        .editor-content hr {
            border: none; border-top: 1px solid var(--rinch-color-gray-3);
            margin: 16px 0;
        }
        .editor-content mark {
            background: var(--rinch-color-yellow-2);
            padding: 1px 2px; border-radius: 2px;
        }
        .editor-content a {
            color: var(--rinch-primary-color);
            text-decoration: underline;
        }
        .editor-content strong { font-weight: 700; }
        .editor-content em { font-style: italic; }
        .editor-content u { text-decoration: underline; }
        .editor-content s { text-decoration: line-through; }
        .editor-content sub { vertical-align: sub; font-size: smaller; }
        .editor-content sup { vertical-align: super; font-size: smaller; }
        .editor-document { min-height: 100px; }
        @keyframes editor-caret-blink {
            0%, 100% { opacity: 1; }
            50% { opacity: 0; }
        }
        .editor-caret { pointer-events: none; }
        .editor-selection {
            background: rgba(34, 139, 230, 0.3);
            border-radius: 2px;
        }
        .editor-toolbar div:hover {
            background: var(--rinch-color-gray-1);
        }
    "#);

    // Add cursor blink animation CSS
    css.push_str(cursor_blink_css());

    style.set_text(&css);
    style
}

/// Renders a list of controls that have keyboard shortcuts.
fn render_shortcuts_list(__scope: &mut RenderScope, preset: Signal<usize>) -> NodeHandle {
    let config = get_toolbar_config(preset.get());

    let container = __scope.create_element("div");
    container.set_attribute("style", "display: flex; flex-direction: column; gap: 6px;");

    for group in &config.groups {
        for control in &group.controls {
            let btn_meta = ControlButton::from_control(control.clone());
            if let Some(shortcut) = btn_meta.shortcut_hint() {
                let row = __scope.create_element("div");
                row.set_attribute("style",
                    "display: flex; justify-content: space-between; align-items: center; \
                     padding: 4px 8px; border-radius: 4px; \
                     background: var(--rinch-color-gray-0);");

                let label = __scope.create_element("span");
                label.set_attribute("style", "font-size: 13px;");
                label.set_text(btn_meta.tooltip());
                row.append_child(&label);

                let badge = __scope.create_element("span");
                badge.set_attribute("style",
                    "font-size: 11px; font-family: monospace; \
                     padding: 2px 8px; border-radius: 4px; \
                     background: var(--rinch-color-gray-2); \
                     color: var(--rinch-color-text);");
                badge.set_text(shortcut);
                row.append_child(&badge);

                container.append_child(&row);
            }
        }
    }

    container
}
