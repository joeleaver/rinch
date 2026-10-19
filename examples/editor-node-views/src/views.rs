//! The node views of the `editor-node-views` example, shared by the desktop binary
//! and the web one (`examples/editor-node-views-web` includes this file).
//!
//! Three views, each drawn by an app's own component inside the document:
//!
//! - a **counter** `embed` (`data-kind="counter"`): its count lives in the node's
//!   `data-count`, so a click is an edit — undoable, saved with the document, and
//!   sent to collaborators;
//! - an editable **table** `embed` (`data-kind="table"`): a grid of `<input>`s,
//!   each cell's text in a `data-c-<row>-<col>` attribute written with
//!   [`NodeViewCtx::set_attrs`] when the cell is committed (Enter, or leaving it);
//! - a **badge** on an inline `image` carrying `data-badge`: the image is drawn as
//!   a small live chip on the line, clicked to cycle its state.
//!
//! `ed` is the editor API of the platform (`rinch::editor` on the desktop,
//! `rinch_web` in the browser): the views are the same code on both.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Selection};

use crate::ed::{EditorHandle, EditorKey, NodeViewCtx, NodeViewSpec};

/// The badge states a chip cycles through.
const BADGES: [&str; 3] = ["new", "hot", "done"];

/// A table cell's attribute name.
fn cell_key(row: usize, col: usize) -> String {
    format!("data-c-{row}-{col}")
}

/// The attribute `key` of a view's node, as a string ("" when absent). Read in
/// a reactive closure, it follows the node (`ctx.attrs` is a memo, and `Copy`).
fn attr_of(attrs: Memo<Attrs>, key: &str) -> String {
    attrs.get().get_str(key).unwrap_or("").to_string()
}

/// Register the example's views on `editor` for as long as the editor lives (or,
/// registered while a component renders, until that component unmounts).
pub fn register(editor: &EditorHandle) {
    editor.register_node_view(counter()).keep();
    editor.register_node_view(table()).keep();
    editor.register_node_view(badge()).keep();
}

// ── Counter ──────────────────────────────────────────────────────────────────

fn counter() -> NodeViewSpec {
    NodeViewSpec::new("embed", |__scope: &mut RenderScope, ctx: NodeViewCtx| {
        let attrs = ctx.attrs;
        let count = move || attr_of(attrs, "data-count").parse::<i64>().unwrap_or(0);
        let (add, sub) = (ctx.clone(), ctx.clone());
        let read_only = ctx.read_only;
        rsx! {
            div {
                class: "nv-counter",
                style: "display: flex; align-items: center; gap: 12px; padding: 8px 12px; \
                        border: 1px solid #d0d7de; border-radius: 8px; background: #f6f8fa;",
                span { style: "font-weight: 600;", "Counter" }
                button {
                    class: "nv-counter-sub",
                    disabled: {move || read_only.get()},
                    // The count is the node's attribute: one undoable edit per click.
                    onclick: move || {
                        let n = count() - 1;
                        sub.set_attrs(|a| *a = a.with("data-count", AttrValue::from(n.to_string())));
                    },
                    "−"
                }
                span { class: "nv-counter-value", style: "min-width: 2em; text-align: center;",
                    {move || count().to_string()}
                }
                button {
                    class: "nv-counter-add",
                    disabled: {move || read_only.get()},
                    onclick: move || {
                        let n = count() + 1;
                        add.set_attrs(|a| *a = a.with("data-count", AttrValue::from(n.to_string())));
                    },
                    "+"
                }
            }
        }
    })
    .matches(|a| a.get_str("data-kind") == Some("counter"))
}

// ── Table ────────────────────────────────────────────────────────────────────

thread_local! {
    /// Every mounted table's inputs, by cell: what the table's `on_key` moves the
    /// focus between (an arrow key in a cell goes to the cell beside it). Each
    /// mount takes its own entry out when it unmounts.
    static TABLES: RefCell<Vec<(usize, Vec<Vec<NodeHandle>>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_TABLE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The `(table, row, col)` of the cell input holding the keyboard, if any.
fn focused_cell() -> Option<(usize, usize, usize)> {
    TABLES.with(|t| {
        let t = t.borrow();
        let any = t.iter().flat_map(|(_, g)| g.iter().flatten()).next()?;
        let active = any.active_element()?;
        t.iter().find_map(|(id, grid)| {
            grid.iter().enumerate().find_map(|(r, row)| {
                row.iter()
                    .position(|c| c.node_id() == active.node_id())
                    .map(|c| (*id, r, c))
            })
        })
    })
}

fn table() -> NodeViewSpec {
    NodeViewSpec::new("embed", |__scope: &mut RenderScope, ctx: NodeViewCtx| {
        let size = |key: &str| -> usize {
            ctx.attrs
                .get()
                .get_str(key)
                .and_then(|v| v.parse().ok())
                .unwrap_or(2)
                .clamp(1, 8)
        };
        let (rows, cols) = (size("data-rows"), size("data-cols"));
        let id = NEXT_TABLE.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let grid: Rc<RefCell<Vec<Vec<NodeHandle>>>> = Rc::default();

        // rinch has no HTML table layout (`<table>`/`<tr>`/`<td>` lay out as plain
        // boxes), so the grid is a CSS grid whose rows are `display: contents`:
        // each row's cells become the grid's own items.
        let root = rsx! {
            div {
                class: "nv-table",
                style: {format!(
                    "display: grid; grid-template-columns: repeat({cols}, minmax(0, 1fr)); \
                     gap: 2px; padding: 6px; border: 1px solid #d0d7de; border-radius: 8px; \
                     background: #d0d7de;"
                )},
            }
        };
        for r in 0..rows {
            let row = rsx! { div { class: "nv-table-row", style: "display: contents;" } };
            let mut inputs = Vec::new();
            for c in 0..cols {
                let key = cell_key(r, c);
                let (attrs, edit) = (ctx.attrs, ctx.clone());
                let shown_key = key.clone();
                let read_only = ctx.read_only;
                // #1591: `value_fn:` on a raw `<input>` is not the component prop it
                // is on `TextInput` (it becomes a stray attribute); a raw input
                // takes a reactive `value: {|| …}`.
                let input = rsx! {
                    input {
                        class: "nv-table-cell",
                        style: "min-width: 0; padding: 4px 6px; border: 0; background: #fff; \
                                font-size: 14px;",
                        readonly: {move || read_only.get()},
                        value: {move || attr_of(attrs, &shown_key)},
                        // Committed on Enter or when the cell loses the keyboard:
                        // one undoable edit, sent to collaborators as this key alone.
                        // (#1590: `onsubmit:` on a raw `<input>` is wired as a click;
                        // `onchange` is the commit a raw input has.)
                        onchange: move |v: String| {
                            let key = key.clone();
                            edit.set_attrs(move |a| *a = a.with(key.as_str(), AttrValue::from(v)));
                        },
                    }
                };
                row.append_child(&input);
                inputs.push(input);
            }
            root.append_child(&row);
            grid.borrow_mut().push(inputs);
        }
        TABLES.with(|t| t.borrow_mut().push((id, grid.borrow().clone())));
        __scope.on_cleanup(move || {
            TABLES.with(|t| t.borrow_mut().retain(|(i, _)| *i != id));
        });
        root
    })
    .matches(|a| a.get_str("data-kind") == Some("table"))
    // The keys of a cell, before the cell sees them. `on_key` is offered key
    // **presses** only, so this moves once per press. (A document-level
    // `set_keyboard_interceptor`, and a registered node's `on_key`, see releases
    // too: there, return early on `k.is_up()`, or every arrow moves twice.)
    //
    // Up and Down move between rows; Left and Right stay the input's own (its
    // caret). Tab and Shift+Tab are not claimed: the runtime (or the browser)
    // moves the keyboard to the next input in the document — the next cell, and
    // from the last cell out of the view. Enter commits the cell (the input's
    // `onchange`). An Escape the view does not claim gives the keyboard back to
    // the editor, its caret right after the table.
    .on_key(|key: &EditorKey<'_>| {
        let down = match key.key {
            "ArrowDown" => true,
            "ArrowUp" => false,
            _ => return false,
        };
        let Some((id, r, c)) = focused_cell() else {
            return false;
        };
        let target = TABLES.with(|t| {
            let t = t.borrow();
            let grid = &t.iter().find(|(i, _)| *i == id)?.1;
            let r = if down { r + 1 } else { r.checked_sub(1)? };
            grid.get(r)?.get(c).cloned()
        });
        match target {
            Some(input) => {
                input.focus();
                true
            }
            // Past the first or last row: leave it to the input.
            None => false,
        }
    })
}

// ── Badge (a claimed inline image) ───────────────────────────────────────────

fn badge() -> NodeViewSpec {
    NodeViewSpec::new("image", |__scope: &mut RenderScope, ctx: NodeViewCtx| {
        let attrs = ctx.attrs;
        let next = ctx.clone();
        rsx! {
            span {
                class: "nv-badge",
                style: {move || format!(
                    "display: inline-block; padding: 0 8px; border-radius: 9px; \
                     font-size: 12px; line-height: 18px; color: #fff; cursor: pointer; \
                     background: {};",
                    match attr_of(attrs, "data-badge").as_str() {
                        "hot" => "#cf222e",
                        "done" => "#1a7f37",
                        _ => "#0969da",
                    }
                )},
                onclick: move || {
                    let now = attr_of(attrs, "data-badge");
                    let i = BADGES.iter().position(|b| *b == now).map_or(0, |i| (i + 1) % BADGES.len());
                    next.set_attrs(|a| *a = a.with("data-badge", AttrValue::from(BADGES[i])));
                },
                {move || attr_of(attrs, "data-badge")}
            }
        }
    })
    .matches(|a| a.get_str("data-badge").is_some())
}

// ── Inserting and exporting ──────────────────────────────────────────────────

/// Insert a counter embed at the selection.
pub fn insert_counter(editor: &EditorHandle) {
    editor.insert_embed(Attrs::from_iter([
        ("data-kind", AttrValue::from("counter")),
        ("data-count", AttrValue::from("0")),
    ]));
}

/// Insert a `rows` × `cols` table embed at the selection.
pub fn insert_table(editor: &EditorHandle, rows: usize, cols: usize) {
    editor.insert_embed(Attrs::from_iter([
        ("data-kind", AttrValue::from("table")),
        ("data-rows", AttrValue::from(rows.to_string())),
        ("data-cols", AttrValue::from(cols.to_string())),
    ]));
}

/// Insert a badged image at the caret.
pub fn insert_badge(editor: &EditorHandle) {
    editor.update(|state| {
        let image = state
            .schema()
            .create_node(
                "image",
                Attrs::from_iter([
                    ("src", AttrValue::from("badge.png")),
                    ("alt", AttrValue::from("badge")),
                    ("data-badge", AttrValue::from("new")),
                ]),
                Fragment::empty(),
            )
            .ok()?;
        let at = state.selection.from().0;
        let mut tr = state.tr();
        tr.replace_with(at, state.selection.to().0, Fragment::from_node(image))
            .ok()?;
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(at + 1)));
        Some(tr)
    });
}

/// What another application sees of the views in exported HTML
/// (`set_html_fallback`): a static table for a table, the count for a counter,
/// the badge's word for a badge — never the live component.
pub fn html_fallback(node: &Node) -> Option<String> {
    let attrs = node.attrs();
    let text = |k: &str| {
        attrs
            .get_str(k)
            .unwrap_or("")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
    };
    match (node.type_name(), attrs.get_str("data-kind")) {
        ("embed", Some("counter")) => Some(format!("<p>Count: {}</p>", text("data-count"))),
        ("embed", Some("table")) => {
            let n = |k: &str| attrs.get_str(k).and_then(|v| v.parse().ok()).unwrap_or(2usize);
            let mut s = String::from("<table>");
            for r in 0..n("data-rows") {
                s.push_str("<tr>");
                for c in 0..n("data-cols") {
                    s.push_str(&format!("<td>{}</td>", text(&cell_key(r, c))));
                }
                s.push_str("</tr>");
            }
            s.push_str("</table>");
            Some(s)
        }
        // An image's markup must be phrasing content (it is written in a span).
        ("image", _) if attrs.get_str("data-badge").is_some() => {
            Some(format!("<b>[{}]</b>", text("data-badge")))
        }
        _ => None,
    }
}

/// The document the example starts with.
pub const START: &str = r#"<h1>Node views</h1><p>An app's own components, drawn inside the document. The badge <img src="badge.png" alt="badge" data-badge="new"> is an image the app draws as a chip: click it.</p><div data-pm-type="embed" data-kind="counter" data-count="3"></div><p>A table whose cells are inputs (Up/Down move between rows, Tab to the next cell, Escape back to the text):</p><div data-pm-type="embed" data-kind="table" data-rows="3" data-cols="3" data-c-0-0="Fruit" data-c-0-1="Count" data-c-0-2="Note" data-c-1-0="Apple" data-c-1-1="3"></div><p>Every change a view makes is an edit: undo it with Ctrl+Z.</p>"#;
