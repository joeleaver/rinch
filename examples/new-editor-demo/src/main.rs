//! Harness for the new (M5+) ProseMirror-style editor. It mounts an `EditorHandle`
//! with the `Editor {}` component, loads some rich content, and exercises the
//! **built-in default stylesheet** (no hand-rolled editor CSS here), the light/dark
//! toggle, and the **table** commands (add/remove row & column, merge, split). The
//! runtime renders the caret — and a cell-selection wash — from the editor's
//! selection after layout.

use rinch::prelude::*;
use rinch_editor_core::tables::{self, TableMap};
use rinch_editor_core::{Pos, Selection};

/// Select every cell of the first table in the document (so `mergeCells` has a
/// rectangle to act on). A demo convenience — interactively you shift-drag or
/// shift-arrow across cells.
fn select_whole_table(ed: &EditorHandle) {
    let doc = ed.doc();
    // Find the first table and the document position just inside it.
    let mut table_start = None;
    let mut pos = 0usize;
    for child in doc.content().children() {
        if tables::is_table(child) {
            table_start = Some(pos + 1);
            break;
        }
        pos += child.node_size();
    }
    let Some(start) = table_start else { return };
    let Some(table) = doc.node_at(start - 1) else {
        return;
    };
    let map = TableMap::compute(&table, start);
    if let (Some(a), Some(b)) = (
        map.cell_at(0, 0),
        map.cell_at(map.height() - 1, map.width() - 1),
    ) {
        ed.set_selection(Selection::cell(Pos(a), Pos(b)));
    }
}

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();
    let dark = Signal::new(false);

    const TBTN: &str = "padding: 6px 12px; border: 1px solid #d0d7de; border-radius: 6px; background: #ffffff; color: #1f2328; cursor: pointer; font-family: sans-serif; font-size: 14px;";

    // One clone per closure that needs the handle.
    let ed_dark = editor.clone();
    let ed_sel = editor.clone();
    let ed_rowp = editor.clone();
    let ed_rowm = editor.clone();
    let ed_colp = editor.clone();
    let ed_colm = editor.clone();
    let ed_merge = editor.clone();
    let ed_split = editor.clone();

    let tree = rsx! {
        div {
            style: {move || format!(
                "min-height: 100vh; padding: 32px; background: {};",
                if dark.get() { "#010409" } else { "#f6f8fa" }
            )},
            div {
                style: "display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;",
                h2 {
                    style: {move || format!(
                        "font-family: sans-serif; margin: 0; color: {};",
                        if dark.get() { "#e6edf3" } else { "#1f2328" }
                    )},
                    "New Editor — tables"
                }
                button {
                    onclick: move || {
                        dark.update(|d| *d = !*d);
                        ed_dark.set_dark_mode(dark.get());
                    },
                    {move || if dark.get() { "Light mode" } else { "Dark mode" }}
                }
            }
            div {
                style: "display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 14px; font-family: sans-serif;",
                button { style: TBTN, onclick: move || { ed_rowp.command("addRowAfter"); }, "Row +" }
                button { style: TBTN, onclick: move || { ed_rowm.command("deleteRow"); }, "Row −" }
                button { style: TBTN, onclick: move || { ed_colp.command("addColumnAfter"); }, "Col +" }
                button { style: TBTN, onclick: move || { ed_colm.command("deleteColumn"); }, "Col −" }
                button { style: TBTN, onclick: move || { select_whole_table(&ed_sel); }, "Select table" }
                button { style: TBTN, onclick: move || { ed_merge.command("mergeCells"); }, "Merge cells" }
                button { style: TBTN, onclick: move || { ed_split.command("splitCell"); }, "Split cell" }
            }
            Editor {
                editor: editor.clone(),
                content: "<h1>Tables</h1>\
                    <p>A 3×3 grid. Click into a cell, then use the toolbar.</p>\
                    <table>\
                    <tr><th><p>Name</p></th><th><p>Role</p></th><th><p>Since</p></th></tr>\
                    <tr><td><p>Ada</p></td><td><p>Engineer</p></td><td><p>1843</p></td></tr>\
                    <tr><td><p>Grace</p></td><td><p>Admiral</p></td><td><p>1944</p></td></tr></table>\
                    <p>Below: a header cell spanning two columns (colspan=2).</p>\
                    <table>\
                    <tr><th colspan=\"2\"><p>Merged header</p></th></tr>\
                    <tr><td><p>left</p></td><td><p>right</p></td></tr></table>",
            }
        }
    };
    editor.set_selection(Selection::cursor(Pos(5)));
    tree
}

fn main() {
    run("New Editor Demo", 960, 800, app);
}
