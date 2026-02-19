//! Rich Text Editor section — showcases contenteditable + CE API.
//!
//! Users don't interact with CeOps directly. They set `contenteditable="true"`
//! on a div and the framework handles everything. For programmatic access
//! (toolbar buttons), `with_active_ce_api()` provides the thread-local CE API.

use rinch::prelude::*;
use rinch_core::with_active_ce_api;

/// Helper: call a CE API method on the active contenteditable element.
fn ce_do(f: impl FnOnce(&mut dyn rinch_core::ce::ContentEditableApi) + 'static) {
    with_active_ce_api(|api| f(&mut *api.borrow_mut()));
}

#[component]
pub fn editor_section() -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Rich Text Editor" }
                Text { size: "lg", color: "dimmed",
                    "ContentEditable powered by the CE API. Set contenteditable=\"true\" on any div — the framework handles input, cursor, and block structure. Use with_active_ce_api() for programmatic formatting."
                }
            }
            Space { h: "xl" }

            // ── Toolbar ─────────────────────────────────────────────
            Paper { p: "xs", radius: "md", with_border: true,
                style: "border-bottom: none; border-bottom-left-radius: 0; border-bottom-right-radius: 0;",
                Group { gap: "2",
                    // Inline formatting
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.toggle_wrap("strong")),
                        span { style: "font-weight: 700; font-size: 14px;", "B" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.toggle_wrap("em")),
                        span { style: "font-style: italic; font-size: 14px;", "I" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.toggle_wrap("u")),
                        span { style: "text-decoration: underline; font-size: 14px;", "U" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.toggle_wrap("s")),
                        span { style: "text-decoration: line-through; font-size: 14px;", "S" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.toggle_wrap("code")),
                        span { style: "font-family: monospace; font-size: 13px;", "<>" }
                    }

                    // Separator
                    div { style: "width: 1px; height: 20px; background: var(--rinch-color-gray-3); margin: 0 4px;" }

                    // Block types
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("h1")),
                        span { style: "font-weight: 700; font-size: 14px;", "H1" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("h2")),
                        span { style: "font-weight: 700; font-size: 13px;", "H2" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("h3")),
                        span { style: "font-weight: 600; font-size: 12px;", "H3" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("p")),
                        span { style: "font-size: 13px;", "P" }
                    }

                    // Separator
                    div { style: "width: 1px; height: 20px; background: var(--rinch-color-gray-3); margin: 0 4px;" }

                    // Block quote
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("blockquote")),
                        span { style: "font-size: 16px;", "\u{201C}" }
                    }

                    // Lists
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("ul")),
                        span { style: "font-size: 13px;", "UL" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.set_block_type("ol")),
                        span { style: "font-size: 13px;", "OL" }
                    }

                    // Separator
                    div { style: "width: 1px; height: 20px; background: var(--rinch-color-gray-3); margin: 0 4px;" }

                    // Indent / Outdent
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.indent()),
                        span { style: "font-size: 16px;", "\u{2192}" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.outdent()),
                        span { style: "font-size: 16px;", "\u{2190}" }
                    }

                    // Separator
                    div { style: "width: 1px; height: 20px; background: var(--rinch-color-gray-3); margin: 0 4px;" }

                    // Undo / Redo
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.undo()),
                        span { style: "font-size: 16px;", "\u{21A9}" }
                    }
                    ActionIcon { variant: "subtle", size: "sm",
                        onclick: move || ce_do(|api| api.redo()),
                        span { style: "font-size: 16px;", "\u{21AA}" }
                    }
                }
            }

            // ── Editor Surface ──────────────────────────────────────
            Paper { p: "0", radius: "md", with_border: true,
                style: "border-top-left-radius: 0; border-top-right-radius: 0;",
                div {
                    contenteditable: "true",
                    class: "editor-content",
                    style: "min-height: 300px; padding: 16px 24px; background: var(--rinch-color-body); \
                            font-size: 16px; line-height: 1.6; color: var(--rinch-color-text); \
                            outline: none; cursor: text;",
                    p { "Start typing here. Select text and use the toolbar buttons or keyboard shortcuts to format." }
                    p { "The editor uses " strong { "contenteditable" } " with the " em { "CE API" } " for all mutations." }
                }
            }

            Space { h: "xl" }

            // ── CE API Reference ────────────────────────────────────
            Title { order: 3, "CE API Reference" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm",
                "Available methods via with_active_ce_api(). The framework calls these automatically for keyboard input. Toolbar buttons call them programmatically."
            }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "xs",
                    {render_api_row(__scope, "insert_text(text)", "Insert text at cursor")}
                    {render_api_row(__scope, "delete_backward()", "Backspace")}
                    {render_api_row(__scope, "delete_forward()", "Delete key")}
                    {render_api_row(__scope, "delete_selection()", "Delete selected text")}
                    {render_api_row(__scope, "split_block()", "Enter — split block at cursor")}
                    {render_api_row(__scope, "set_block_type(tag)", "Change block to h1, h2, p, blockquote, etc.")}
                    {render_api_row(__scope, "toggle_wrap(tag)", "Toggle bold/italic/underline/code")}
                    {render_api_row(__scope, "wrap_selection(tag)", "Wrap selection in formatting element")}
                    {render_api_row(__scope, "unwrap_selection(tag)", "Remove formatting from selection")}
                    {render_api_row(__scope, "indent()", "Indent block / increase list nesting")}
                    {render_api_row(__scope, "outdent()", "Outdent block / decrease list nesting")}
                    {render_api_row(__scope, "undo() / redo()", "Undo and redo operations")}
                    {render_api_row(__scope, "get_selection()", "Read current cursor / selection state")}
                    {render_api_row(__scope, "set_selection(sel)", "Set cursor / selection position")}
                }
            }

            Space { h: "xl" }

            // ── Keyboard Shortcuts ──────────────────────────────────
            Title { order: 3, "Keyboard Shortcuts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Built-in shortcuts handled by the framework." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "xs",
                    {render_shortcut_row(__scope, "Ctrl+B", "Bold")}
                    {render_shortcut_row(__scope, "Ctrl+I", "Italic")}
                    {render_shortcut_row(__scope, "Ctrl+U", "Underline")}
                    {render_shortcut_row(__scope, "Ctrl+Shift+S", "Strikethrough")}
                    {render_shortcut_row(__scope, "Ctrl+E", "Inline code")}
                    {render_shortcut_row(__scope, "Ctrl+Z", "Undo")}
                    {render_shortcut_row(__scope, "Ctrl+Shift+Z", "Redo")}
                    {render_shortcut_row(__scope, "Enter", "Split block")}
                    {render_shortcut_row(__scope, "Tab", "Indent")}
                    {render_shortcut_row(__scope, "Shift+Tab", "Outdent")}
                    {render_shortcut_row(__scope, "Backspace", "Delete backward")}
                    {render_shortcut_row(__scope, "Delete", "Delete forward")}
                }
            }

            // Editor CSS
            div {
                style: "display: none;",
                {render_editor_styles(__scope)}
            }
        }
    }
}

/// Render a single API reference row.
fn render_api_row(__scope: &mut RenderScope, method: &str, desc: &str) -> NodeHandle {
    let row = __scope.create_element("div");
    row.set_attribute(
        "style",
        "display: flex; justify-content: space-between; align-items: center; \
         padding: 4px 8px; border-radius: 4px; background: var(--rinch-color-gray-0);",
    );

    let code = __scope.create_element("span");
    code.set_attribute("style", "font-family: monospace; font-size: 13px; font-weight: 600;");
    code.set_text(method);
    row.append_child(&code);

    let label = __scope.create_element("span");
    label.set_attribute("style", "font-size: 13px; color: var(--rinch-color-dimmed);");
    label.set_text(desc);
    row.append_child(&label);

    row
}

/// Render a single keyboard shortcut row.
fn render_shortcut_row(__scope: &mut RenderScope, shortcut: &str, desc: &str) -> NodeHandle {
    let row = __scope.create_element("div");
    row.set_attribute(
        "style",
        "display: flex; justify-content: space-between; align-items: center; \
         padding: 4px 8px; border-radius: 4px; background: var(--rinch-color-gray-0);",
    );

    let label = __scope.create_element("span");
    label.set_attribute("style", "font-size: 13px;");
    label.set_text(desc);
    row.append_child(&label);

    let badge = __scope.create_element("span");
    badge.set_attribute(
        "style",
        "font-size: 11px; font-family: monospace; padding: 2px 8px; border-radius: 4px; \
         background: var(--rinch-color-gray-2); color: var(--rinch-color-text);",
    );
    badge.set_text(shortcut);
    row.append_child(&badge);

    row
}

/// Editor-specific CSS styles for content elements.
#[component]
fn render_editor_styles() -> NodeHandle {
    let style = __scope.create_element("style");
    let css = r#"
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
    "#;

    style.set_text(css);
    style
}
