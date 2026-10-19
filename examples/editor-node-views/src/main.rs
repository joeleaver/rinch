//! Node views on the desktop: an app's own components drawn inside the rich-text
//! editor's document — a counter and an editable table (both `embed` nodes) and a
//! badge drawn for an inline image. The views themselves are in `views.rs`, which
//! the web example (`examples/editor-node-views-web`) shares as is.
//!
//! ```text
//! cargo run -p editor-node-views --release
//! ```

use rinch::prelude::*;

/// The platform's editor API, for `views.rs`.
mod ed {
    pub use rinch::editor::{EditorHandle, EditorKey, NodeViewCtx, NodeViewSpec};
}
mod views;

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();
    views::register(&editor);
    let exported = Signal::new(String::new());

    const BTN: &str = "padding: 4px 10px; border: 1px solid #d0d7de; background: #fff; \
                       border-radius: 6px; font-size: 14px;";
    let (e_counter, e_table, e_badge, e_undo, e_redo, e_export, e_ro) = (
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
        editor.clone(),
    );
    let read_only = Signal::new(false);

    rsx! {
        div { style: "font-family: sans-serif; padding: 24px; max-width: 760px; margin: 0 auto;",
            // `data-nofocus`: a press on the toolbar keeps the keyboard in the editor.
            div {
                data-nofocus: "",
                style: "display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px;",
                button { style: BTN, onclick: move || views::insert_counter(&e_counter), "Insert counter" }
                button { style: BTN, onclick: move || views::insert_table(&e_table, 3, 3), "Insert table" }
                button { style: BTN, onclick: move || views::insert_badge(&e_badge), "Insert badge" }
                button { style: BTN, onclick: move || { e_undo.command("undo"); }, "Undo" }
                button { style: BTN, onclick: move || { e_redo.command("redo"); }, "Redo" }
                button {
                    style: BTN,
                    onclick: move || {
                        read_only.update(|r| *r = !*r);
                        e_ro.set_read_only(read_only.get());
                    },
                    {|| if read_only.get() { "Editable" } else { "Read-only" }}
                }
                button {
                    style: BTN,
                    onclick: move || {
                        // What another application gets: each view's static
                        // fallback inside the atom's element.
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
            pre {
                style: "white-space: pre-wrap; font-size: 12px; color: #57606a;",
                {|| exported.get()}
            }
        }
    }
}

fn main() {
    App::new(app).title("Node views").size(860, 760).run();
}
