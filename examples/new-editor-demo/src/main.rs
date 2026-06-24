//! Harness for the new (M5+) ProseMirror-style editor. It mounts an `EditorHandle`
//! with the `Editor {}` component, loads some rich content, and exercises the
//! **built-in default stylesheet** (no hand-rolled editor CSS here) plus the
//! light/dark toggle. The runtime renders the caret from the editor's selection
//! after layout.

use rinch::prelude::*;
use rinch_editor_core::{Pos, Selection};

#[component]
fn app() -> NodeHandle {
    // The declarative API: create a handle for programmatic control, then place it
    // with the `Editor {}` component. The handle works before and after mount.
    let editor = create_editor();
    let dark = Signal::new(false);
    let ed = editor.clone();

    let tree = rsx! {
        div {
            style: {move || format!(
                "min-height: 100vh; padding: 32px; background: {};",
                if dark.get() { "#010409" } else { "#f6f8fa" }
            )},
            div {
                style: "display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;",
                h2 {
                    style: {move || format!(
                        "font-family: sans-serif; margin: 0; color: {};",
                        if dark.get() { "#e6edf3" } else { "#1f2328" }
                    )},
                    "New Editor — default styles"
                }
                button {
                    onclick: move || {
                        dark.update(|d| *d = !*d);
                        ed.set_dark_mode(dark.get());
                    },
                    {move || if dark.get() { "Light mode" } else { "Dark mode" }}
                }
            }
            Editor {
                editor: editor.clone(),
                content: "<h1>The default look</h1>\
                    <p>Hello <strong>bold</strong>, <em>italic</em>, <u>underline</u>, \
                    <code>inline code</code>, and a <a href=\"https://example.com\">link</a>.</p>\
                    <blockquote><p>A quote block, styled by the editor's own defaults.</p></blockquote>\
                    <ul><li><p>first item</p></li><li><p>second item</p></li></ul>\
                    <pre>fn main() {\n    println!(\"hello\");\n}</pre>\
                    <hr>\
                    <table><tr><th><p>Name</p></th><th><p>Role</p></th></tr>\
                    <tr><td><p>Ada</p></td><td><p>Engineer</p></td></tr>\
                    <tr><td><p>Grace</p></td><td><p>Admiral</p></td></tr></table>",
            }
        }
    };
    // The `content` prop loaded the document when the component mounted above;
    // now place a cursor inside the first heading so the caret shows.
    editor.set_selection(Selection::cursor(Pos(5)));
    tree
}

fn main() {
    run("New Editor Demo", 920, 760, app);
}
