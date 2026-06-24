//! Rich Text Editor section — showcases the ProseMirror-style editor.
//!
//! The editor is a first-class **desktop** feature: a model-first document
//! (`rinch-editor-core`) rendered by a rinch-dom view. Mount it with the
//! [`Editor`] component and an [`EditorHandle`]; drive it from toolbar buttons
//! with `handle.command("toggleBold")`. The web view is a follow-up, so on
//! wasm this section shows a note instead.

use rinch::prelude::*;

#[component]
pub fn editor_section() -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Rich Text Editor" }
                Text { size: "lg", color: "dimmed",
                    "A ProseMirror-style rich-text editor: a model-first document with invertible steps, schema-enforced structure, marks, lists, tables, and a single command API. Mount it with the Editor {} component and drive it through an EditorHandle."
                }
            }
            Space { h: "xl" }

            {editor_body(__scope)}

            Space { h: "xl" }

            // ── Command API Reference ───────────────────────────────
            Title { order: 3, "Command API" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm",
                "Dispatch by name with handle.command(name). Queries (handle.is_mark_active / current_block_type / can_run) read editor state, never the DOM."
            }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "xs",
                    {render_api_row(__scope, "toggleBold / toggleItalic", "Toggle inline marks at the cursor or over a range")}
                    {render_api_row(__scope, "toggleUnderline / toggleStrike", "Underline / strikethrough marks")}
                    {render_api_row(__scope, "toggleCode / toggleHighlight", "Inline code and highlight marks")}
                    {render_api_row(__scope, "setParagraph / setHeading1..6", "Change the current block type")}
                    {render_api_row(__scope, "setCodeBlock", "Turn the block into a code block")}
                    {render_api_row(__scope, "toggleBulletList / toggleOrderedList", "Wrap into / lift out of a list")}
                    {render_api_row(__scope, "wrapInBlockquote", "Wrap the selection in a blockquote")}
                    {render_api_row(__scope, "sinkListItem / liftListItem", "Indent / outdent a list item")}
                    {render_api_row(__scope, "insertHorizontalRule", "Insert a horizontal rule")}
                    {render_api_row(__scope, "insertTable", "Insert a 3×3 table")}
                    {render_api_row(__scope, "addRowAfter / addColumnAfter", "Grow the table at the cursor")}
                    {render_api_row(__scope, "mergeCells / splitCell", "Merge or split a cell selection")}
                    {render_api_row(__scope, "undo / redo", "History (grouped, typing-merged)")}
                }
            }

            Space { h: "xl" }

            // ── Keyboard Shortcuts ──────────────────────────────────
            Title { order: 3, "Keyboard Shortcuts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Built-in shortcuts handled by the editor." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "xs",
                    {render_shortcut_row(__scope, "Ctrl+B", "Bold")}
                    {render_shortcut_row(__scope, "Ctrl+I", "Italic")}
                    {render_shortcut_row(__scope, "Ctrl+U", "Underline")}
                    {render_shortcut_row(__scope, "Ctrl+Z", "Undo")}
                    {render_shortcut_row(__scope, "Ctrl+Shift+Z / Ctrl+Y", "Redo")}
                    {render_shortcut_row(__scope, "Enter", "Split block / new list item")}
                    {render_shortcut_row(__scope, "Tab / Shift+Tab", "Move between table cells")}
                    {render_shortcut_row(__scope, "Backspace / Delete", "Delete backward / forward")}
                }
            }
        }
    }
}

/// Desktop: mount the real editor with a working command toolbar.
#[cfg(feature = "desktop")]
fn editor_body(__scope: &mut RenderScope) -> NodeHandle {
    let editor = create_editor();

    // One handle clone per toolbar action (EditorHandle is cheap to clone).
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
    let ed_table = editor.clone();
    let ed_undo = editor.clone();
    let ed_redo = editor.clone();

    rsx! {
        Fragment {
        Paper { p: "xs", radius: "md", with_border: true,
            style: "border-bottom: none; border-bottom-left-radius: 0; border-bottom-right-radius: 0;",
            Group { gap: "2",
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_bold.command("toggleBold"); },
                    span { style: "font-weight: 700; font-size: 14px;", "B" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_italic.command("toggleItalic"); },
                    span { style: "font-style: italic; font-size: 14px;", "I" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_under.command("toggleUnderline"); },
                    span { style: "text-decoration: underline; font-size: 14px;", "U" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_strike.command("toggleStrike"); },
                    span { style: "text-decoration: line-through; font-size: 14px;", "S" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_code.command("toggleCode"); },
                    span { style: "font-family: monospace; font-size: 13px;", "<>" }
                }

                div { style: "width: 1px; height: 20px; background: var(--rinch-color-border); margin: 0 4px;" }

                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_h1.command("setHeading1"); },
                    span { style: "font-weight: 700; font-size: 14px;", "H1" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_h2.command("setHeading2"); },
                    span { style: "font-weight: 700; font-size: 13px;", "H2" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_h3.command("setHeading3"); },
                    span { style: "font-weight: 600; font-size: 12px;", "H3" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_p.command("setParagraph"); },
                    span { style: "font-size: 13px;", "P" }
                }

                div { style: "width: 1px; height: 20px; background: var(--rinch-color-border); margin: 0 4px;" }

                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_quote.command("wrapInBlockquote"); },
                    span { style: "font-size: 16px;", "\u{201C}" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_ul.command("toggleBulletList"); },
                    span { style: "font-size: 13px;", "UL" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_ol.command("toggleOrderedList"); },
                    span { style: "font-size: 13px;", "OL" }
                }

                div { style: "width: 1px; height: 20px; background: var(--rinch-color-border); margin: 0 4px;" }

                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_indent.command("sinkListItem"); },
                    span { style: "font-size: 16px;", "\u{2192}" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_outdent.command("liftListItem"); },
                    span { style: "font-size: 16px;", "\u{2190}" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_table.command("insertTable"); },
                    span { style: "font-size: 13px;", "\u{229E}" }
                }

                div { style: "width: 1px; height: 20px; background: var(--rinch-color-border); margin: 0 4px;" }

                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_undo.command("undo"); },
                    span { style: "font-size: 16px;", "\u{21A9}" }
                }
                ActionIcon { variant: "subtle", size: "sm",
                    onclick: move || { ed_redo.command("redo"); },
                    span { style: "font-size: 16px;", "\u{21AA}" }
                }
            }
        }

        Paper { p: "0", radius: "md", with_border: true,
            style: "border-top-left-radius: 0; border-top-right-radius: 0;",
            Editor {
                editor: editor.clone(),
                content: "<h1>Rich Text</h1>\
                    <p>Select text and use the toolbar, or type with <strong>Ctrl+B</strong> / <em>Ctrl+I</em>. The editor ships its own default stylesheet.</p>\
                    <ul><li><p>Lists, headings, blockquotes</p></li>\
                    <li><p>Inline marks: <strong>bold</strong>, <em>italic</em>, <code>code</code></p></li></ul>\
                    <blockquote><p>Everything flows through invertible steps, so undo/redo is exact.</p></blockquote>",
            }
        }
        }
    }
}

/// Web (no desktop renderer yet): the editor view is desktop-only for now.
#[cfg(not(feature = "desktop"))]
fn editor_body(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Paper { p: "xl", radius: "md", with_border: true,
            Stack { gap: "sm",
                Text { style: "font-weight: 600;", "The rich-text editor view is a desktop feature." }
                Text { color: "dimmed", size: "sm",
                    "The model-first editor core is renderer-agnostic; the web view (over the browser's native contentEditable) is a planned follow-up. Run the desktop UI Zoo to try it."
                }
            }
        }
    }
}

/// Render a single API reference row.
fn render_api_row(__scope: &mut RenderScope, method: &str, desc: &str) -> NodeHandle {
    rsx! {
        div {
            style: "display: flex; justify-content: space-between; align-items: center; \
                    padding: 4px 8px; border-radius: 4px; background: var(--rinch-color-default); gap: 12px;",
            span {
                style: "font-family: monospace; font-size: 13px; font-weight: 600;",
                {method}
            }
            span {
                style: "font-size: 13px; color: var(--rinch-color-dimmed); text-align: right;",
                {desc}
            }
        }
    }
}

/// Render a single keyboard shortcut row.
fn render_shortcut_row(__scope: &mut RenderScope, shortcut: &str, desc: &str) -> NodeHandle {
    rsx! {
        div {
            style: "display: flex; justify-content: space-between; align-items: center; \
                    padding: 4px 8px; border-radius: 4px; background: var(--rinch-color-default);",
            span {
                style: "font-size: 13px;",
                {desc}
            }
            span {
                style: "font-size: 11px; font-family: monospace; padding: 2px 8px; border-radius: 4px; \
                        background: var(--rinch-color-default); color: var(--rinch-color-text);",
                {shortcut}
            }
        }
    }
}
