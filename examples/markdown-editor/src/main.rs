//! Markdown Editor — standalone rich-text editor example.
//!
//! A focused editor app built on the ProseMirror-style rinch editor. It mounts an
//! [`EditorHandle`] with the [`Editor`] component, loads starter content as HTML,
//! and drives formatting from a toolbar of `handle.command("...")` buttons — there
//! is one mutation path (`EditorState::apply`) and the host is always a pure
//! projection of the model. Built with the `debug` feature so MCP tools can inspect
//! and interact with it.

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// App chrome CSS. The editor *content* is styled by the editor's own built-in
/// light/dark stylesheet — we only style the surrounding shell (toolbar, scroll
/// area) here, never the editor internals.
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
    max-width: 760px;
    margin: 0 auto;
    padding: 32px 48px;
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

/// The formatting toolbar. Every button dispatches a named command through its own
/// cheap [`EditorHandle`] clone — the canonical way to drive the editor.
#[component]
fn editor_toolbar(editor: EditorHandle) -> NodeHandle {
    // One handle clone per toolbar action (EditorHandle is an Rc — cheap to clone).
    let ed_bold = editor.clone();
    let ed_italic = editor.clone();
    let ed_under = editor.clone();
    let ed_strike = editor.clone();
    let ed_code = editor.clone();
    let ed_h1 = editor.clone();
    let ed_h2 = editor.clone();
    let ed_h3 = editor.clone();
    let ed_p = editor.clone();
    let ed_quote = editor.clone();
    let ed_ul = editor.clone();
    let ed_ol = editor.clone();
    let ed_indent = editor.clone();
    let ed_outdent = editor.clone();
    let ed_hr = editor.clone();
    let ed_table = editor.clone();
    let ed_undo = editor.clone();
    let ed_redo = editor.clone();

    rsx! {
        div { class: "toolbar",
            // Inline marks
            {toolbar_button(__scope, TablerIcon::Bold, "Bold (Ctrl+B)", move || { ed_bold.command("toggleBold"); })}
            {toolbar_button(__scope, TablerIcon::Italic, "Italic (Ctrl+I)", move || { ed_italic.command("toggleItalic"); })}
            {toolbar_button(__scope, TablerIcon::Underline, "Underline (Ctrl+U)", move || { ed_under.command("toggleUnderline"); })}
            {toolbar_button(__scope, TablerIcon::Strikethrough, "Strikethrough", move || { ed_strike.command("toggleStrike"); })}
            {toolbar_button(__scope, TablerIcon::Code, "Inline code", move || { ed_code.command("toggleCode"); })}

            {separator(__scope)}

            // Block types
            {toolbar_button(__scope, TablerIcon::H1, "Heading 1", move || { ed_h1.command("setHeading1"); })}
            {toolbar_button(__scope, TablerIcon::H2, "Heading 2", move || { ed_h2.command("setHeading2"); })}
            {toolbar_button(__scope, TablerIcon::H3, "Heading 3", move || { ed_h3.command("setHeading3"); })}
            {toolbar_button(__scope, TablerIcon::Pilcrow, "Paragraph", move || { ed_p.command("setParagraph"); })}

            {separator(__scope)}

            // Block containers
            {toolbar_button(__scope, TablerIcon::Blockquote, "Blockquote", move || { ed_quote.command("wrapInBlockquote"); })}
            {toolbar_button(__scope, TablerIcon::List, "Bullet list", move || { ed_ul.command("toggleBulletList"); })}
            {toolbar_button(__scope, TablerIcon::ListNumbers, "Ordered list", move || { ed_ol.command("toggleOrderedList"); })}

            {separator(__scope)}

            // Indent / outdent (list items)
            {toolbar_button(__scope, TablerIcon::IndentIncrease, "Indent (sinkListItem)", move || { ed_indent.command("sinkListItem"); })}
            {toolbar_button(__scope, TablerIcon::IndentDecrease, "Outdent (liftListItem)", move || { ed_outdent.command("liftListItem"); })}

            {separator(__scope)}

            // Inserts
            {toolbar_button(__scope, TablerIcon::Separator, "Horizontal rule", move || { ed_hr.command("insertHorizontalRule"); })}
            {toolbar_button(__scope, TablerIcon::Table, "Insert table", move || { ed_table.command("insertTable"); })}

            {separator(__scope)}

            // History
            {toolbar_button(__scope, TablerIcon::ArrowBackUp, "Undo (Ctrl+Z)", move || { ed_undo.command("undo"); })}
            {toolbar_button(__scope, TablerIcon::ArrowForwardUp, "Redo (Ctrl+Shift+Z)", move || { ed_redo.command("redo"); })}
        }
    }
}

/// Starter content, parsed into the document on mount via the `content:` prop.
/// Schema-whitelisted HTML — block tags become nodes, inline tags become marks.
const STARTER: &str = "<h1>Welcome to the Markdown Editor</h1>\
    <p>This is a <strong>rich-text editor</strong> built on rinch's \
    <em>ProseMirror-style</em> editor. Select text and use the toolbar, or type \
    with <strong>Ctrl+B</strong> / <em>Ctrl+I</em>.</p>\
    <h2>Features</h2>\
    <ul>\
    <li><p>Inline marks: <strong>bold</strong>, <em>italic</em>, <u>underline</u>, \
    <s>strikethrough</s>, <code>code</code></p></li>\
    <li><p>Block types: headings, paragraphs, blockquotes, code blocks</p></li>\
    <li><p>Lists (bullet and ordered) with indent / outdent</p></li>\
    <li><p>Tables, horizontal rules, and exact undo / redo</p></li>\
    </ul>\
    <h2>Try it out</h2>\
    <p>Click anywhere and start typing. The editor renders the caret and selection \
    straight from the model — every edit is an invertible step, so undo / redo is \
    exact.</p>\
    <blockquote><p>Everything flows through one mutation path; the document on screen \
    is always a pure projection of the editor state.</p></blockquote>\
    <p>Happy editing!</p>";

/// Main app component.
#[component]
fn app() -> NodeHandle {
    let editor = create_editor();

    rsx! {
        Fragment {
            style { {CSS} }
            div { class: "editor-root",
                {editor_toolbar(__scope, editor.clone())}
                div { class: "editor-scroll",
                    Editor {
                        editor: editor.clone(),
                        content: STARTER,
                        class: "editor-content",
                    }
                }
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
