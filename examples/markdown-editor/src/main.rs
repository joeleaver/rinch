//! Markdown Editor — standalone rich-text editor example.
//!
//! A focused editor app that exercises the contenteditable + CE API.
//! Built with the `debug` feature so MCP tools can inspect and interact with it.

use rinch::prelude::*;
use rinch_core::with_active_ce_api;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Helper: call a CE API method on the active contenteditable element.
fn ce_do(f: impl FnOnce(&mut dyn rinch_core::ce::ContentEditableApi) + 'static) {
    with_active_ce_api(|api| f(&mut *api.borrow_mut()));
}

/// Global CSS for the editor app.
const CSS: &str = r#"
* {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

html, body {
    height: 100vh;
}

body {
    font-family: var(--rinch-font-family);
    background: var(--rinch-color-body);
    color: var(--rinch-color-text);
    overflow: hidden;
}

.editor-root {
    display: flex;
    flex-direction: column;
    height: 100vh;
}

.toolbar {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 2px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--rinch-color-gray-3);
    background: var(--rinch-color-body);
    flex-shrink: 0;
}

.toolbar-separator {
    width: 1px;
    height: 20px;
    background: var(--rinch-color-gray-3);
    margin: 0 6px;
}

.editor-scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
}

.editor-content {
    min-height: 100%;
    max-width: 720px;
    margin: 0 auto;
    padding: 32px 48px;
    font-size: 16px;
    line-height: 1.7;
    color: var(--rinch-color-text);
    outline: none;
    cursor: text;
}

/* Block element styles */
.editor-content p { margin: 0 0 8px 0; }
.editor-content h1 { font-size: 2em; font-weight: 700; margin: 24px 0 12px 0; }
.editor-content h2 { font-size: 1.5em; font-weight: 700; margin: 20px 0 10px 0; }
.editor-content h3 { font-size: 1.25em; font-weight: 600; margin: 16px 0 8px 0; }
.editor-content h4 { font-size: 1.1em; font-weight: 600; margin: 12px 0 6px 0; }
.editor-content h5 { font-size: 1em; font-weight: 600; margin: 10px 0 4px 0; }
.editor-content h6 { font-size: 0.9em; font-weight: 600; margin: 10px 0 4px 0; color: var(--rinch-color-dimmed); }

.editor-content blockquote {
    border-left: 3px solid var(--rinch-color-gray-4);
    padding-left: 16px;
    margin: 12px 0;
    color: var(--rinch-color-dimmed);
}

.editor-content pre {
    background: var(--rinch-color-gray-1);
    border-radius: var(--rinch-radius-sm);
    padding: 12px 16px;
    margin: 12px 0;
    font-family: monospace;
    font-size: 14px;
    overflow-x: auto;
}

.editor-content code {
    background: var(--rinch-color-gray-1);
    padding: 2px 5px;
    border-radius: 3px;
    font-size: 0.9em;
}

.editor-content pre code {
    background: none;
    padding: 0;
    border-radius: 0;
}

.editor-content ul, .editor-content ol {
    margin: 8px 0;
    padding-left: 24px;
}

.editor-content li { margin: 4px 0; }

.editor-content hr {
    border: none;
    border-top: 1px solid var(--rinch-color-gray-3);
    margin: 20px 0;
}

.editor-content strong { font-weight: 700; }
.editor-content em { font-style: italic; }
.editor-content u { text-decoration: underline; }
.editor-content s { text-decoration: line-through; }

.editor-content a {
    color: var(--rinch-primary-color);
    text-decoration: underline;
}

.editor-content mark {
    background: var(--rinch-color-yellow-2);
    padding: 1px 2px;
    border-radius: 2px;
}
"#;

/// Toolbar separator element.
#[component]
fn separator() -> NodeHandle {
    rsx! { div { class: "toolbar-separator" } }
}

/// A toolbar button wrapping an ActionIcon.
#[component]
fn toolbar_button(icon: TablerIcon, tooltip: &str, on_click: impl Fn() + 'static) -> NodeHandle {
    let _ = tooltip; // TODO: tooltip support
    rsx! {
        ActionIcon {
            variant: "subtle",
            size: "sm",
            onclick: on_click,
            {render_tabler_icon(__scope, icon, TablerIconStyle::Outline)}
        }
    }
}

/// The formatting toolbar.
#[component]
fn editor_toolbar() -> NodeHandle {
    rsx! {
        div { class: "toolbar",
            // Inline formatting
            {toolbar_button(__scope, TablerIcon::Bold, "Bold (Ctrl+B)", move || ce_do(|api| api.toggle_wrap("strong")))}
            {toolbar_button(__scope, TablerIcon::Italic, "Italic (Ctrl+I)", move || ce_do(|api| api.toggle_wrap("em")))}
            {toolbar_button(__scope, TablerIcon::Underline, "Underline (Ctrl+U)", move || ce_do(|api| api.toggle_wrap("u")))}
            {toolbar_button(__scope, TablerIcon::Strikethrough, "Strikethrough", move || ce_do(|api| api.toggle_wrap("s")))}
            {toolbar_button(__scope, TablerIcon::Code, "Inline Code (Ctrl+E)", move || ce_do(|api| api.toggle_wrap("code")))}

            {separator(__scope)}

            // Block types
            {toolbar_button(__scope, TablerIcon::H1, "Heading 1", move || ce_do(|api| api.set_block_type("h1")))}
            {toolbar_button(__scope, TablerIcon::H2, "Heading 2", move || ce_do(|api| api.set_block_type("h2")))}
            {toolbar_button(__scope, TablerIcon::H3, "Heading 3", move || ce_do(|api| api.set_block_type("h3")))}

            {separator(__scope)}

            // Block elements
            {toolbar_button(__scope, TablerIcon::Blockquote, "Blockquote", move || ce_do(|api| api.set_block_type("blockquote")))}
            {toolbar_button(__scope, TablerIcon::List, "Bullet List", move || ce_do(|api| api.set_block_type("ul")))}
            {toolbar_button(__scope, TablerIcon::ListNumbers, "Numbered List", move || ce_do(|api| api.set_block_type("ol")))}

            {separator(__scope)}

            // Indent / Outdent
            {toolbar_button(__scope, TablerIcon::IndentIncrease, "Indent (Tab)", move || ce_do(|api| api.indent()))}
            {toolbar_button(__scope, TablerIcon::IndentDecrease, "Outdent (Shift+Tab)", move || ce_do(|api| api.outdent()))}

            {separator(__scope)}

            // Undo / Redo
            {toolbar_button(__scope, TablerIcon::ArrowBackUp, "Undo (Ctrl+Z)", move || ce_do(|api| api.undo()))}
            {toolbar_button(__scope, TablerIcon::ArrowForwardUp, "Redo (Ctrl+Shift+Z)", move || ce_do(|api| api.redo()))}
        }
    }
}

/// The main editor surface.
#[component]
fn editor_surface() -> NodeHandle {
    rsx! {
        div { class: "editor-scroll",
            div {
                contenteditable: "true",
                class: "editor-content",
                h1 { "Welcome to the Markdown Editor" }
                p {
                    "This is a "
                    strong { "rich-text editor" }
                    " built with rinch's "
                    em { "ContentEditable API" }
                    ". Select text and use the toolbar or keyboard shortcuts to format."
                }
                h2 { "Features" }
                ul {
                    li { "Inline formatting: " strong { "bold" } ", " em { "italic" } ", " u { "underline" } ", " s { "strikethrough" } ", " code { "code" } }
                    li { "Block types: headings, paragraphs, blockquotes" }
                    li { "Lists: bullet and numbered, with indent/outdent" }
                    li { "Undo and redo" }
                }
                h2 { "Keyboard Shortcuts" }
                p { strong { "Ctrl+B" } " Bold  |  " strong { "Ctrl+I" } " Italic  |  " strong { "Ctrl+U" } " Underline  |  " strong { "Ctrl+E" } " Code" }
                p { strong { "Tab" } " Indent  |  " strong { "Shift+Tab" } " Outdent  |  " strong { "Ctrl+Z" } " Undo  |  " strong { "Ctrl+Shift+Z" } " Redo" }
                h2 { "Try It Out" }
                p { "Click anywhere and start typing. The editor handles cursor movement, text selection, and block splitting automatically." }
                blockquote { p { "This is a blockquote. You can create one by clicking the blockquote button in the toolbar." } }
                p { "Happy editing!" }
            }
        }
    }
}

/// Main app component.
#[component]
fn app() -> NodeHandle {
    rsx! {
        Fragment {
            style { {CSS} }
            div { class: "editor-root",
                {editor_toolbar(__scope)}
                {editor_surface(__scope)}
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    run_with_theme("Markdown Editor", 900, 700, app, theme);
}
