//! Web rich-text editor demo for `rinch-web`.
//!
//! Mounts the *same* model-first editor the desktop uses ([`Editor`]/[`EditorHandle`]
//! from `rinch-editor-view`, re-exported by `rinch-web`), projected onto the browser
//! DOM via `WebDocument` — **no** `contenteditable`. The toolbar drives it through
//! `handle.command(...)`, exactly as on desktop. Stable element `id`s make it
//! Playwright-assertable.

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{Editor, create_editor};

#[component]
fn app() -> NodeHandle {
    // One handle drives both the toolbar and the mounted editor. EditorHandle is cheap
    // to clone (an Rc); one clone per closure.
    let editor = create_editor();
    let ed_bold = editor.clone();
    let ed_italic = editor.clone();
    let ed_underline = editor.clone();
    let ed_h1 = editor.clone();
    let ed_h2 = editor.clone();
    let ed_p = editor.clone();
    let ed_ul = editor.clone();
    let ed_ol = editor.clone();
    let ed_task = editor.clone();
    let ed_quote = editor.clone();
    let ed_indent = editor.clone();
    let ed_outdent = editor.clone();
    let ed_undo = editor.clone();
    let ed_redo = editor.clone();

    let btn = "padding: 4px 10px; border: 1px solid #d0d7de; background: #fff; border-radius: 6px; cursor: pointer; font-size: 14px;";

    rsx! {
        div { style: "font-family: -apple-system, system-ui, sans-serif; padding: 24px; max-width: 760px; margin: 0 auto;",
            h2 { "rinch-web rich-text editor" }
            p { id: "subtitle", style: "color: #57606a;",
                "The model-first editor on the browser DOM — no contenteditable."
            }

            // ── Toolbar (drives the handle, like desktop) ──────────────────────
            div {
                id: "toolbar",
                style: "display: flex; flex-wrap: wrap; gap: 6px; margin: 12px 0; padding: 8px; border: 1px solid #d0d7de; border-radius: 8px;",
                button { id: "tb-bold",      style: btn, onclick: move || { ed_bold.command("toggleBold"); },        "B" }
                button { id: "tb-italic",    style: btn, onclick: move || { ed_italic.command("toggleItalic"); },    "I" }
                button { id: "tb-underline", style: btn, onclick: move || { ed_underline.command("toggleUnderline"); }, "U" }
                button { id: "tb-h1",        style: btn, onclick: move || { ed_h1.command("setHeading1"); },         "H1" }
                button { id: "tb-h2",        style: btn, onclick: move || { ed_h2.command("setHeading2"); },         "H2" }
                button { id: "tb-p",         style: btn, onclick: move || { ed_p.command("setParagraph"); },         "P" }
                button { id: "tb-ul",        style: btn, onclick: move || { ed_ul.command("toggleBulletList"); },    "• List" }
                button { id: "tb-ol",        style: btn, onclick: move || { ed_ol.command("toggleOrderedList"); },   "1. List" }
                button { id: "tb-task",      style: btn, onclick: move || { ed_task.command("toggleTaskList"); },    "☑ Tasks" }
                button { id: "tb-quote",     style: btn, onclick: move || { ed_quote.command("wrapInBlockquote"); }, "Quote" }
                button { id: "tb-indent",    style: btn, onclick: move || { ed_indent.command("indent"); },          "Indent" }
                button { id: "tb-outdent",   style: btn, onclick: move || { ed_outdent.command("outdent"); },        "Outdent" }
                button { id: "tb-undo",      style: btn, onclick: move || { ed_undo.command("undo"); },              "Undo" }
                button { id: "tb-redo",      style: btn, onclick: move || { ed_redo.command("redo"); },              "Redo" }
            }

            // ── The editor ─────────────────────────────────────────────────────
            div {
                id: "editor-host",
                style: "border: 1px solid #d0d7de; border-radius: 8px; overflow: hidden;",
                Editor {
                    editor: editor.clone(),
                    content: "<h1>Rich Text on the Web</h1><p>Click to place the caret, then type. Select text and use the toolbar, or <strong>Ctrl+B</strong> / <em>Ctrl+I</em>.</p><ul><li>Lists, headings, blockquotes</li><li>Bold, italic, underline marks</li></ul><p>Everything flows through the same invertible steps as desktop.</p>",
                }
            }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    rinch_web::mount(ThemeProviderProps::default(), app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
