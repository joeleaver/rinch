//! DOM rendering using Rinch's NodeHandle API.
//!
//! HTML serialization approach: the entire editor document is serialized to an
//! HTML string, and `set_inner_html` atomically replaces the container's content.
//! This avoids fine-grained DOM mutations that can cause blitz to panic when its
//! internal state references modified nodes.

use std::rc::Rc;
use std::cell::RefCell;

use rinch_core::dom::{RenderScope, NodeHandle};
use rinch_core::reactive::{Signal, Effect};
use crate::document::{EditorDocument, InlineRun, MarkData, ResolvedPosition};
use crate::editor::Editor;
use crate::selection::Selection;

/// Reactive state for the editor view.
///
/// A single `render_version` signal is bumped on any change. A single Effect
/// re-serializes the entire document to HTML and calls `set_inner_html`.
pub struct BlockSignals {
    /// Global version signal, bumped on any change.
    pub render_version: Signal<u64>,
}

impl BlockSignals {
    pub fn new() -> Self {
        Self {
            render_version: Signal::new(0),
        }
    }

    /// Bump the render version to trigger a re-render.
    pub fn bump(&self) {
        self.render_version.update(|v| *v += 1);
    }
}

/// Apply editor changes by bumping the single render_version signal.
///
/// Call this after any editor command.
pub fn apply_changes(editor: &Rc<RefCell<Editor>>, signals: &Rc<BlockSignals>) {
    let _changes = if let Ok(mut ed) = editor.try_borrow_mut() {
        ed.take_changes()
    } else {
        tracing::warn!("apply_changes: editor borrow_mut FAILED");
        return;
    };
    tracing::info!("apply_changes: bumping render_version");
    signals.bump();
}

/// Render the document reactively with a single Effect.
///
/// Returns `(container, block_signals)`. The container is the DOM node.
/// After each editor command, call `apply_changes(editor, &signals)` to
/// trigger a re-render via HTML serialization.
pub fn render_document_reactive(
    scope: &mut RenderScope,
    editor: Rc<RefCell<Editor>>,
) -> (NodeHandle, Rc<BlockSignals>) {
    let signals = Rc::new(BlockSignals::new());

    let container = scope.create_element("div");
    container.set_attribute("class", "editor-document");

    // Initial render
    if let Ok(ed) = editor.try_borrow() {
        let html = editor_to_html(&ed);
        container.set_inner_html(&html);
    }

    // Single reactive Effect
    let version = signals.render_version;
    let editor_clone = editor.clone();
    let container_clone = container.clone();
    let initial_v = version.get();
    let prev_v: Rc<RefCell<u64>> = Rc::new(RefCell::new(initial_v));

    let effect = Effect::new(move || {
        let v = version.get();
        let prev = *prev_v.borrow();
        if v == prev {
            return;
        }
        *prev_v.borrow_mut() = v;
        tracing::info!("EDITOR RENDER EFFECT: v={}, serializing HTML", v);

        rinch_core::events::request_selection_clear();

        match editor_clone.try_borrow() {
            Ok(ed) => {
                let html = editor_to_html(&ed);
                tracing::info!("EDITOR RENDER EFFECT: html len={}, first 200 chars: {}", html.len(), &html[..html.len().min(200)]);
                container_clone.set_inner_html(&html);
            }
            Err(e) => {
                tracing::warn!("EDITOR RENDER EFFECT: editor borrow FAILED: {}", e);
            }
        }
    });
    scope.create_effect_from(effect);

    (container, signals)
}

// ============================================================================
// HTML Serialization
// ============================================================================

/// Convert the editor document to an HTML string, including the caret.
fn editor_to_html(editor: &Editor) -> String {
    let mut html = String::new();
    let block_count = editor.doc.block_count();
    let sel = editor.get_selection();
    let cursor_pos = if sel.is_cursor() {
        editor.doc.resolve_position(sel.head).ok()
    } else {
        None
    };

    for i in 0..block_count {
        let block_type = editor.doc.block_type(i).unwrap_or_else(|| "paragraph".into());
        let attrs = editor.doc.block_attrs(i).unwrap_or_default();

        let (open_tag, close_tag) = block_tags(&block_type, &attrs);
        html.push_str(&open_tag);

        if block_type == "horizontal_rule" {
            html.push_str(&close_tag);
            continue;
        }

        let runs = editor.doc.block_inline_runs(i);
        let cursor_in_block = cursor_pos.as_ref().filter(|rp| rp.block_index == i);

        if runs.is_empty() {
            if cursor_in_block.is_some() {
                html.push_str(&caret_html(&editor.stored_marks));
            }
        } else if let Some(rp) = cursor_in_block {
            render_runs_with_caret(&mut html, &runs, rp.text_offset, &editor.stored_marks);
        } else {
            render_runs(&mut html, &runs);
        }

        html.push_str(&close_tag);
    }

    html
}

fn block_tags(block_type: &str, attrs: &std::collections::HashMap<String, String>) -> (String, String) {
    match block_type {
        "paragraph" => {
            let style = attrs
                .get("align")
                .map(|a| format!(" style=\"text-align: {}\"", html_escape(a)))
                .unwrap_or_default();
            (
                format!("<p data-block-type=\"paragraph\"{}>", style),
                "</p>".into(),
            )
        }
        "heading" => {
            let level = attrs.get("level").map(|s| s.as_str()).unwrap_or("1");
            let tag = format!("h{}", level);
            (format!("<{}>", tag), format!("</{}>", tag))
        }
        "blockquote" => ("<blockquote>".into(), "</blockquote>".into()),
        "code_block" => ("<pre><code>".into(), "</code></pre>".into()),
        "bullet_list" | "ordered_list" | "list_item" => ("<li>".into(), "</li>".into()),
        "horizontal_rule" => ("<hr>".into(), "".into()),
        _ => ("<p>".into(), "</p>".into()),
    }
}

fn render_runs(html: &mut String, runs: &[InlineRun]) {
    for run in runs {
        render_single_run(html, run);
    }
}

fn render_runs_with_caret(
    html: &mut String,
    runs: &[InlineRun],
    cursor_offset: usize,
    stored_marks: &[String],
) {
    let mut chars_seen = 0;
    let mut caret_rendered = false;

    for run in runs {
        if run.inline_type == "hard_break" {
            if chars_seen == cursor_offset && !caret_rendered {
                html.push_str(&caret_html(stored_marks));
                caret_rendered = true;
            }
            html.push_str("<br>");
            continue;
        }

        let run_len = run.text.len();

        if !caret_rendered && cursor_offset >= chars_seen && cursor_offset <= chars_seen + run_len {
            let offset_in_run = cursor_offset - chars_seen;
            let before = &run.text[..offset_in_run];
            let after = &run.text[offset_in_run..];

            if !before.is_empty() {
                let before_run = InlineRun {
                    text: before.to_string(),
                    inline_type: "text".into(),
                    marks: run.marks.clone(),
                };
                render_single_run(html, &before_run);
            }

            html.push_str(&caret_html(stored_marks));
            caret_rendered = true;

            if !after.is_empty() {
                let after_run = InlineRun {
                    text: after.to_string(),
                    inline_type: "text".into(),
                    marks: run.marks.clone(),
                };
                render_single_run(html, &after_run);
            }
        } else {
            render_single_run(html, run);
        }

        chars_seen += run_len;
    }

    if !caret_rendered {
        html.push_str(&caret_html(stored_marks));
    }
}

fn render_single_run(html: &mut String, run: &InlineRun) {
    if run.inline_type == "hard_break" {
        html.push_str("<br>");
        return;
    }

    // Open mark tags
    for mark in &run.marks {
        match mark.mark_type.as_str() {
            "bold" => html.push_str("<strong>"),
            "italic" => html.push_str("<em>"),
            "underline" => html.push_str("<u>"),
            "strike" | "strikethrough" => html.push_str("<s>"),
            "code" => html.push_str("<code>"),
            "link" => {
                let href = mark.attrs.get("href").map(|h| h.as_str()).unwrap_or("#");
                html.push_str(&format!("<a href=\"{}\">", html_escape(href)));
            }
            "superscript" => html.push_str("<sup>"),
            "subscript" => html.push_str("<sub>"),
            "highlight" => {
                let color = mark
                    .attrs
                    .get("color")
                    .map(|c| c.as_str())
                    .unwrap_or("yellow");
                html.push_str(&format!(
                    "<mark style=\"background-color: {}\">",
                    html_escape(color)
                ));
            }
            "textColor" | "textStyle" => {
                let color = mark.attrs.get("color").map(|c| c.as_str()).unwrap_or("");
                if !color.is_empty() {
                    html.push_str(&format!("<span style=\"color: {}\">", html_escape(color)));
                }
            }
            _ => {}
        }
    }

    // Escape and write text
    html.push_str(&html_escape(&run.text));

    // Close mark tags (reverse order)
    for mark in run.marks.iter().rev() {
        match mark.mark_type.as_str() {
            "bold" => html.push_str("</strong>"),
            "italic" => html.push_str("</em>"),
            "underline" => html.push_str("</u>"),
            "strike" | "strikethrough" => html.push_str("</s>"),
            "code" => html.push_str("</code>"),
            "link" => html.push_str("</a>"),
            "superscript" => html.push_str("</sup>"),
            "subscript" => html.push_str("</sub>"),
            "highlight" => html.push_str("</mark>"),
            "textColor" | "textStyle" => {
                let color = mark.attrs.get("color").map(|c| c.as_str()).unwrap_or("");
                if !color.is_empty() {
                    html.push_str("</span>");
                }
            }
            _ => {}
        }
    }
}

fn caret_html(stored_marks: &[String]) -> String {
    let extra_style = if stored_marks.is_empty() {
        ""
    } else {
        " box-shadow: 0 0 0 1px var(--rinch-primary-color, #228be6);"
    };
    format!(
        "<span class=\"editor-caret\" style=\"display: inline-block; width: 0; height: 1.2em; \
         border-left: 2px solid var(--rinch-primary-color, #228be6); \
         vertical-align: text-bottom; \
         animation: editor-caret-blink 1s step-end infinite;{}\"></span>",
        extra_style
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ============================================================================
// Static Renderer (non-reactive, used by other subsystems)
// ============================================================================

/// Renderer for converting document model to DOM.
#[derive(Debug)]
pub struct Renderer;

impl Renderer {
    /// Render the entire document to a container div.
    pub fn render_document(scope: &mut RenderScope, doc: &EditorDocument) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("class", "editor-document");

        let block_count = doc.block_count();
        let mut i = 0;
        while i < block_count {
            let block_type = doc.block_type(i).unwrap_or_else(|| "paragraph".into());

            if Self::is_list_block(&block_type) {
                let list_container = scope.create_element(Self::list_container_tag(&block_type));

                while i < block_count {
                    let bt = doc.block_type(i).unwrap_or_else(|| "paragraph".into());
                    if bt != block_type {
                        break;
                    }
                    let li_node = Self::render_block(scope, doc, i);
                    list_container.append_child(&li_node);
                    i += 1;
                }

                container.append_child(&list_container);
            } else {
                let block_node = Self::render_block(scope, doc, i);
                container.append_child(&block_node);
                i += 1;
            }
        }

        container
    }

    /// Render a single block to the appropriate HTML element.
    pub fn render_block(scope: &mut RenderScope, doc: &EditorDocument, block_index: usize) -> NodeHandle {
        let block_type = doc.block_type(block_index).unwrap_or_else(|| "paragraph".into());
        let attrs = doc.block_attrs(block_index).unwrap_or_default();

        let tag = match block_type.as_str() {
            "paragraph" => "p",
            "heading" => {
                match attrs.get("level").map(|s| s.as_str()) {
                    Some("1") => "h1",
                    Some("2") => "h2",
                    Some("3") => "h3",
                    Some("4") => "h4",
                    Some("5") => "h5",
                    Some("6") => "h6",
                    _ => "h1",
                }
            }
            "blockquote" => "blockquote",
            "code_block" => "pre",
            "bullet_list" => "li",
            "ordered_list" => "li",
            "list_item" => "li",
            "horizontal_rule" => "hr",
            _ => "p",
        };

        let element = scope.create_element(tag);
        element.set_attribute("data-block-index", &block_index.to_string());

        // Apply alignment if present
        if let Some(align) = attrs.get("align") {
            element.set_attribute("style", &format!("text-align: {};", align));
        }

        // For hr, no inline content needed
        if block_type == "horizontal_rule" {
            return element;
        }

        // For code_block, wrap content in <code>
        if block_type == "code_block" {
            let code = scope.create_element("code");
            let runs = doc.block_inline_runs(block_index);
            Self::render_inline_runs(scope, &code, &runs);
            element.append_child(&code);
            return element;
        }

        // Render inline content
        let runs = doc.block_inline_runs(block_index);
        Self::render_inline_runs(scope, &element, &runs);

        element
    }

    /// Check if a block type is a list type that should be grouped.
    fn is_list_block(block_type: &str) -> bool {
        block_type == "bullet_list" || block_type == "ordered_list"
    }

    /// Get the list container tag for a list block type.
    fn list_container_tag(block_type: &str) -> &'static str {
        if block_type == "bullet_list" { "ul" } else { "ol" }
    }

    /// Render inline runs into a parent element.
    fn render_inline_runs(scope: &mut RenderScope, parent: &NodeHandle, runs: &[InlineRun]) {
        for run in runs {
            let node = Self::render_inline_run(scope, run);
            parent.append_child(&node);
        }
    }

    /// Render a single inline run (text with marks or hard break).
    pub fn render_inline_run(scope: &mut RenderScope, run: &InlineRun) -> NodeHandle {
        match run.inline_type.as_str() {
            "hard_break" => scope.create_element("br"),
            "text" => {
                if run.marks.is_empty() {
                    // Plain text node
                    scope.create_text(&run.text)
                } else {
                    // Wrap text in nested mark elements (outermost first)
                    Self::wrap_in_marks(scope, &run.text, &run.marks)
                }
            }
            _ => scope.create_text(""),
        }
    }

    /// Wrap text content in nested mark elements.
    fn wrap_in_marks(scope: &mut RenderScope, text: &str, marks: &[MarkData]) -> NodeHandle {
        if marks.is_empty() {
            return scope.create_text(text);
        }

        // Build from innermost to outermost
        let text_node = scope.create_text(text);
        let mut current = text_node;

        for mark in marks.iter().rev() {
            let wrapper = Self::mark_element(scope, mark);
            wrapper.append_child(&current);
            current = wrapper;
        }

        current
    }

    /// Render the document with a visible caret or selection highlighting.
    pub fn render_document_with_cursor(
        scope: &mut RenderScope,
        doc: &EditorDocument,
        selection: &Selection,
        stored_marks: &[String],
    ) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("class", "editor-document");

        let block_count = doc.block_count();

        if selection.is_cursor() {
            // Cursor mode: render with caret
            let resolved = doc.resolve_position(selection.head);

            let mut i = 0;
            while i < block_count {
                let block_type = doc.block_type(i).unwrap_or_else(|| "paragraph".into());

                if Self::is_list_block(&block_type) {
                    let list_container = scope.create_element(Self::list_container_tag(&block_type));

                    while i < block_count {
                        let bt = doc.block_type(i).unwrap_or_else(|| "paragraph".into());
                        if bt != block_type {
                            break;
                        }

                        let cursor_in_block = resolved.as_ref().ok().filter(|rp| rp.block_index == i);
                        let block_node = if let Some(rp) = cursor_in_block {
                            Self::render_block_with_cursor(scope, doc, i, rp.text_offset, stored_marks)
                        } else {
                            Self::render_block(scope, doc, i)
                        };
                        list_container.append_child(&block_node);
                        i += 1;
                    }

                    container.append_child(&list_container);
                } else {
                    let cursor_in_block = resolved.as_ref().ok().filter(|rp| rp.block_index == i);
                    let block_node = if let Some(rp) = cursor_in_block {
                        Self::render_block_with_cursor(scope, doc, i, rp.text_offset, stored_marks)
                    } else {
                        Self::render_block(scope, doc, i)
                    };
                    container.append_child(&block_node);
                    i += 1;
                }
            }
        } else {
            // Range selection: render with highlighting
            let sel_start = selection.start();
            let sel_end = selection.end();
            let start_resolved = doc.resolve_position(sel_start);
            let end_resolved = doc.resolve_position(sel_end);

            if let (Ok(start_rp), Ok(end_rp)) = (start_resolved, end_resolved) {
                let mut i = 0;
                while i < block_count {
                    let block_type = doc.block_type(i).unwrap_or_else(|| "paragraph".into());

                    if Self::is_list_block(&block_type) {
                        let list_container = scope.create_element(Self::list_container_tag(&block_type));

                        while i < block_count {
                            let bt = doc.block_type(i).unwrap_or_else(|| "paragraph".into());
                            if bt != block_type {
                                break;
                            }

                            let block_node = Self::render_block_for_selection(
                                scope, doc, i, &start_rp, &end_rp);
                            list_container.append_child(&block_node);
                            i += 1;
                        }

                        container.append_child(&list_container);
                    } else {
                        let block_node = Self::render_block_for_selection(
                            scope, doc, i, &start_rp, &end_rp);
                        container.append_child(&block_node);
                        i += 1;
                    }
                }
            } else {
                // Fallback: render without selection
                let mut i = 0;
                while i < block_count {
                    let block_type = doc.block_type(i).unwrap_or_else(|| "paragraph".into());

                    if Self::is_list_block(&block_type) {
                        let list_container = scope.create_element(Self::list_container_tag(&block_type));

                        while i < block_count {
                            let bt = doc.block_type(i).unwrap_or_else(|| "paragraph".into());
                            if bt != block_type {
                                break;
                            }
                            let block_node = Self::render_block(scope, doc, i);
                            list_container.append_child(&block_node);
                            i += 1;
                        }

                        container.append_child(&list_container);
                    } else {
                        let block_node = Self::render_block(scope, doc, i);
                        container.append_child(&block_node);
                        i += 1;
                    }
                }
            }
        }

        container
    }

    /// Render a block with a caret at the given text offset.
    pub fn render_block_with_cursor(
        scope: &mut RenderScope,
        doc: &EditorDocument,
        block_index: usize,
        cursor_offset: usize,
        stored_marks: &[String],
    ) -> NodeHandle {
        let block_type = doc.block_type(block_index).unwrap_or_else(|| "paragraph".into());
        let attrs = doc.block_attrs(block_index).unwrap_or_default();

        let tag = match block_type.as_str() {
            "paragraph" => "p",
            "heading" => {
                match attrs.get("level").map(|s| s.as_str()) {
                    Some("1") => "h1",
                    Some("2") => "h2",
                    Some("3") => "h3",
                    Some("4") => "h4",
                    Some("5") => "h5",
                    Some("6") => "h6",
                    _ => "h1",
                }
            }
            "blockquote" => "blockquote",
            "code_block" => "pre",
            "bullet_list" => "li",
            "ordered_list" => "li",
            "list_item" => "li",
            "horizontal_rule" => "hr",
            _ => "p",
        };

        let element = scope.create_element(tag);
        element.set_attribute("data-block-index", &block_index.to_string());

        // Apply alignment if present
        if let Some(align) = attrs.get("align") {
            element.set_attribute("style", &format!("text-align: {};", align));
        }

        if block_type == "horizontal_rule" {
            return element;
        }

        let runs = doc.block_inline_runs(block_index);

        if runs.is_empty() {
            // Empty block - show just the caret
            let caret = Self::create_caret(scope, stored_marks);
            element.append_child(&caret);
            return element;
        }

        // Render runs with caret inserted at cursor_offset
        let mut chars_seen = 0;
        let mut caret_rendered = false;

        for run in &runs {
            if run.inline_type == "hard_break" {
                if chars_seen == cursor_offset && !caret_rendered {
                    let caret = Self::create_caret(scope, stored_marks);
                    element.append_child(&caret);
                    caret_rendered = true;
                }
                let br = scope.create_element("br");
                element.append_child(&br);
                continue;
            }

            let run_len = run.text.len();

            if !caret_rendered && cursor_offset >= chars_seen && cursor_offset <= chars_seen + run_len {
                // Caret is within this run - split the text around the caret
                let offset_in_run = cursor_offset - chars_seen;
                let before = &run.text[..offset_in_run];
                let after = &run.text[offset_in_run..];

                if !before.is_empty() {
                    let before_run = InlineRun {
                        text: before.to_string(),
                        inline_type: "text".into(),
                        marks: run.marks.clone(),
                    };
                    let node = Self::render_inline_run(scope, &before_run);
                    element.append_child(&node);
                }

                let caret = Self::create_caret(scope, stored_marks);
                element.append_child(&caret);
                caret_rendered = true;

                if !after.is_empty() {
                    let after_run = InlineRun {
                        text: after.to_string(),
                        inline_type: "text".into(),
                        marks: run.marks.clone(),
                    };
                    let node = Self::render_inline_run(scope, &after_run);
                    element.append_child(&node);
                }
            } else {
                let node = Self::render_inline_run(scope, run);
                element.append_child(&node);
            }

            chars_seen += run_len;
        }

        // If caret hasn't been rendered yet (cursor at end of block)
        if !caret_rendered {
            let caret = Self::create_caret(scope, stored_marks);
            element.append_child(&caret);
        }

        element
    }

    /// Create the caret element (blinking cursor).
    pub(crate) fn create_caret(scope: &mut RenderScope, stored_marks: &[String]) -> NodeHandle {
        let caret = scope.create_element("span");
        caret.set_attribute("class", "editor-caret");

        // Use width:0 with a left border so the caret doesn't affect the text layout.
        let mut style = "display: inline-block; width: 0; height: 1.2em; \
            border-left: 2px solid var(--rinch-primary-color, #228be6); \
            vertical-align: text-bottom; \
            animation: editor-caret-blink 1s step-end infinite;".to_string();

        if !stored_marks.is_empty() {
            // Visual indicator of active stored marks
            style.push_str(" box-shadow: 0 0 0 1px var(--rinch-primary-color, #228be6);");
        }

        caret.set_attribute("style", &style);
        caret
    }

    /// Helper to decide how to render a block in a range selection context.
    fn render_block_for_selection(
        scope: &mut RenderScope,
        doc: &EditorDocument,
        block_index: usize,
        start_rp: &ResolvedPosition,
        end_rp: &ResolvedPosition,
    ) -> NodeHandle {
        if block_index < start_rp.block_index || block_index > end_rp.block_index {
            Self::render_block(scope, doc, block_index)
        } else if block_index == start_rp.block_index && block_index == end_rp.block_index {
            Self::render_block_with_selection(scope, doc, block_index,
                Some(start_rp.text_offset), Some(end_rp.text_offset))
        } else if block_index == start_rp.block_index {
            Self::render_block_with_selection(scope, doc, block_index,
                Some(start_rp.text_offset), None)
        } else if block_index == end_rp.block_index {
            Self::render_block_with_selection(scope, doc, block_index,
                None, Some(end_rp.text_offset))
        } else {
            Self::render_block_with_selection(scope, doc, block_index, None, None)
        }
    }

    /// Render a block with selection highlighting.
    fn render_block_with_selection(
        scope: &mut RenderScope,
        doc: &EditorDocument,
        block_index: usize,
        sel_start_offset: Option<usize>,
        sel_end_offset: Option<usize>,
    ) -> NodeHandle {
        let block_type = doc.block_type(block_index).unwrap_or_else(|| "paragraph".into());
        let attrs = doc.block_attrs(block_index).unwrap_or_default();

        let tag = match block_type.as_str() {
            "paragraph" => "p",
            "heading" => match attrs.get("level").map(|s| s.as_str()) {
                Some("1") => "h1", Some("2") => "h2", Some("3") => "h3",
                Some("4") => "h4", Some("5") => "h5", Some("6") => "h6",
                _ => "h1",
            },
            "blockquote" => "blockquote",
            "code_block" => "pre",
            "bullet_list" => "li",
            "ordered_list" => "li",
            "list_item" => "li",
            "horizontal_rule" => "hr",
            _ => "p",
        };

        let element = scope.create_element(tag);
        element.set_attribute("data-block-index", &block_index.to_string());

        // Apply alignment if present
        if let Some(align) = attrs.get("align") {
            element.set_attribute("style", &format!("text-align: {};", align));
        }

        if block_type == "horizontal_rule" {
            return element;
        }

        let runs = doc.block_inline_runs(block_index);
        let block_text = doc.block_text(block_index).unwrap_or_default();
        let block_len = block_text.len();

        let sel_start = sel_start_offset.unwrap_or(0);
        let sel_end = sel_end_offset.unwrap_or(block_len);

        if runs.is_empty() {
            // Empty block within selection - show highlight with non-breaking space
            if sel_start == 0 {
                let sel_span = scope.create_element("span");
                sel_span.set_attribute("class", "editor-selection");
                let space = scope.create_text("\u{00A0}");
                sel_span.append_child(&space);
                element.append_child(&sel_span);
            }
            return element;
        }

        // Walk through runs and split at selection boundaries
        let mut chars_seen = 0;

        for run in &runs {
            if run.inline_type == "hard_break" {
                let br = scope.create_element("br");
                element.append_child(&br);
                continue;
            }

            let run_len = run.text.len();
            let run_start = chars_seen;
            let run_end = chars_seen + run_len;

            let overlap_start = sel_start.max(run_start);
            let overlap_end = sel_end.min(run_end);

            if overlap_start >= overlap_end {
                // No overlap - render normally
                let node = Self::render_inline_run(scope, run);
                element.append_child(&node);
            } else {
                let before_len = overlap_start - run_start;
                let after_start = overlap_end - run_start;

                // Before selection
                if before_len > 0 {
                    let before_run = InlineRun {
                        text: run.text[..before_len].to_string(),
                        inline_type: "text".into(),
                        marks: run.marks.clone(),
                    };
                    let node = Self::render_inline_run(scope, &before_run);
                    element.append_child(&node);
                }

                // Selected portion
                let sel_span = scope.create_element("span");
                sel_span.set_attribute("class", "editor-selection");
                let sel_inner = InlineRun {
                    text: run.text[before_len..after_start].to_string(),
                    inline_type: "text".into(),
                    marks: run.marks.clone(),
                };
                let inner_node = Self::render_inline_run(scope, &sel_inner);
                sel_span.append_child(&inner_node);
                element.append_child(&sel_span);

                // After selection
                if after_start < run_len {
                    let after_run = InlineRun {
                        text: run.text[after_start..].to_string(),
                        inline_type: "text".into(),
                        marks: run.marks.clone(),
                    };
                    let node = Self::render_inline_run(scope, &after_run);
                    element.append_child(&node);
                }
            }

            chars_seen += run_len;
        }

        element
    }

    /// Create the HTML element for a mark.
    fn mark_element(scope: &mut RenderScope, mark: &MarkData) -> NodeHandle {
        match mark.mark_type.as_str() {
            "bold" => scope.create_element("strong"),
            "italic" => scope.create_element("em"),
            "underline" => scope.create_element("u"),
            "strike" | "strikethrough" => scope.create_element("s"),
            "code" => scope.create_element("code"),
            "highlight" => scope.create_element("mark"),
            "subscript" => scope.create_element("sub"),
            "superscript" => scope.create_element("sup"),
            "link" => {
                let a = scope.create_element("a");
                if let Some(href) = mark.attrs.get("href") {
                    a.set_attribute("href", href);
                }
                a
            }
            "textColor" => {
                let span = scope.create_element("span");
                if let Some(color) = mark.attrs.get("color") {
                    span.set_attribute("style", &format!("color: {};", color));
                }
                span
            }
            _ => {
                // Unknown mark: wrap in span
                let span = scope.create_element("span");
                span.set_attribute("data-mark", &mark.mark_type);
                span
            }
        }
    }
}
