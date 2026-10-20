//! Node views on `rinch-web`: the same views as the desktop example
//! (`examples/editor-node-views/src/views.rs`, included as is) in the browser.
//!
//! ```text
//! cd examples/editor-node-views-web && trunk serve --release
//! ```

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{Editor, create_editor};

/// The platform's editor API, for `views.rs`.
mod ed {
    pub use rinch_web::{EditorHandle, EditorKey, NodeViewCtx, NodeViewSpec};
}
#[path = "../../editor-node-views/src/views.rs"]
mod views;

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();
    views::register(&editor);
    let exported = Signal::new(String::new());

    const BTN: &str = "padding: 4px 10px; border: 1px solid #d0d7de; background: #fff; \
                       border-radius: 6px; font-size: 14px; cursor: pointer;";
    let (e_counter, e_table, e_badge, e_undo, e_redo, e_export) = (
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
    );

    rsx! {
        div { style: "font-family: system-ui, sans-serif; padding: 24px; max-width: 760px; margin: 0 auto;",
            div {
                data-nofocus: "",
                style: "display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px;",
                button { id: "nv-insert-counter", style: BTN, onclick: move || views::insert_counter(&e_counter), "Insert counter" }
                button { id: "nv-insert-table", style: BTN, onclick: move || views::insert_table(&e_table, 3, 3), "Insert table" }
                button { id: "nv-insert-badge", style: BTN, onclick: move || views::insert_badge(&e_badge), "Insert badge" }
                button { style: BTN, onclick: move || { e_undo.command("undo"); }, "Undo" }
                button { style: BTN, onclick: move || { e_redo.command("redo"); }, "Redo" }
                button {
                    id: "nv-export",
                    style: BTN,
                    onclick: move || {
                        let _fallback =
                            rinch_editor_core::serialize::set_html_fallback(views::html_fallback);
                        exported.set(rinch_editor_core::serialize::node_to_html(&e_export.doc()));
                    },
                    "Export HTML"
                }
            }
            div { style: "border: 1px solid #d0d7de; border-radius: 8px; padding: 4px 12px;",
                Editor { editor: editor.clone(), content: views::START }
            }
            pre { style: "white-space: pre-wrap; font-size: 12px; color: #57606a;", {|| exported.get()} }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    rinch_web::mount(ThemeProviderProps::default(), app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
