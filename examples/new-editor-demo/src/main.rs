//! Minimal harness for the new (M5) ProseMirror-style editor, used to drive the
//! desktop view under MCP: it mounts an `EditorHandle`, loads some rich content,
//! and places the editor container in the layout. The runtime renders the caret
//! from the editor's selection after layout.

use rinch::prelude::*;
use rinch_editor_core::{Pos, Selection};

#[component]
fn app() -> NodeHandle {
    // The declarative API: create a handle for programmatic control, then place it
    // with the `Editor {}` component. The handle works before and after mount.
    let editor = create_editor();

    // Style the editor's blocks a little so structure is visible in screenshots.
    let css = "\
        [data-pm-editor] { border: 1px solid #ccc; padding: 16px; min-height: 200px; \
            font-family: sans-serif; font-size: 16px; line-height: 1.5; } \
        [data-pm-editor] h1 { font-size: 28px; margin: 0 0 12px; } \
        [data-pm-editor] p { margin: 0 0 8px; } \
        [data-pm-editor] blockquote { border-left: 3px solid #ccc; padding-left: 12px; \
            color: #555; margin: 0 0 8px; } \
        [data-pm-editor] ul { padding-left: 24px; margin: 0; } \
        [data-pm-editor] li { display: flex; align-items: baseline; gap: 6px; } \
        [data-pm-editor] hr { border: none; border-top: 2px solid #ccc; margin: 14px 0; \
            height: 0; } \
        [data-pm-placeholder] { color: #999; position: absolute; pointer-events: none; }";

    let tree = rsx! {
        div { style: "padding: 24px; background: #fff;",
            style { {css} }
            h2 { style: "font-family: sans-serif;", "New Editor (M5) — caret demo" }
            Editor {
                editor: editor.clone(),
                content: "<h1>New editor</h1>\
                    <p>Hello <strong>bold</strong> and <em>italic</em> world.</p>\
                    <hr>\
                    <blockquote><p>A quote block.</p></blockquote>\
                    <ul><li><p>first item</p></li><li><p>second item</p></li></ul>",
            }
        }
    };
    // The `content` prop loaded the document when the component mounted above;
    // now place a cursor inside the first paragraph (mid-"Hello") so the caret shows.
    editor.set_selection(Selection::cursor(Pos(16)));
    tree
}

fn main() {
    run("New Editor Demo", 900, 700, app);
}
