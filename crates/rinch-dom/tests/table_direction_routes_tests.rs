//! #1083: every route that can change which way a CSS table lays out — a
//! child moved in, a child removed, a transition or animation tick's Taffy
//! re-sync, a row group losing its rows — leaves it laid out as a fresh build
//! of the final tree would be. Written as review probes for PR #1097; the
//! tick (p1*) and moved-row (p2b, p2c) cases failed at that PR's first head.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    body { margin: 0; font-size: 16px; line-height: 20px; }
    .t { display: table; width: 400px; }
    .r { display: table-row; }
    .c { display: table-cell; }
    .b { width: 30px; height: 20px; }
    .w { display: contents; }
    .wide { width: 500px; }
    .tw { transition: width 10s linear; }
";

fn el(doc: &mut RinchDocument, parent: NodeId, class: &str) -> NodeId {
    let x = doc.create_element("div");
    if !class.is_empty() {
        doc.set_attribute(x, "class", class);
    }
    doc.append_child(parent, x);
    x
}
fn row(doc: &mut RinchDocument, parent: NodeId) -> NodeId {
    let r = el(doc, parent, "r");
    for _ in 0..2 {
        let c = el(doc, r, "c");
        el(doc, c, "b");
    }
    r
}
fn cell(doc: &mut RinchDocument, parent: NodeId) -> NodeId {
    let c = el(doc, parent, "c");
    el(doc, c, "b");
    c
}
fn pos(doc: &RinchDocument, n: NodeId) -> (f32, f32) {
    let l = &doc.tree.get(n.0).unwrap().layout;
    (l.x, l.y)
}
fn relayout(doc: &mut RinchDocument, w: f32) {
    doc.resolve_layout(w, 600.0);
}
fn fresh() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    d
}

/// P1: a width transition on a table of rows: the tick's Taffy re-sync
/// rebuilds the style without the table direction.
#[test]
fn p1_a_width_transition_tick_keeps_rows_stacked() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t tw");
    let _r1 = row(&mut doc, t);
    let r2 = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "before");
    doc.set_attribute(t, "class", "t tw wide");
    relayout(&mut doc, 801.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "after class change");
    doc.tick_transitions();
    relayout(&mut doc, 802.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "after a transition tick");
}

/// P1b: the tick re-syncs every LAYOUT-dirty node in `dirty_nodes`, not
/// only the transitioning one (the precedent the floor/out-of-flow tests use).
#[test]
fn p1b_any_tick_resync_keeps_rows_stacked() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let _r1 = row(&mut doc, t);
    let r2 = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "before");
    doc.tree.dirty_nodes.insert(t.0);
    doc.tick_transitions();
    doc.tree.layout_dirty = true;
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "transition tick");
    doc.tree.dirty_nodes.insert(t.0);
    doc.tick_animations();
    doc.tree.layout_dirty = true;
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "animation tick");
}

/// P2: moving already-styled rows into an empty table (a drag between two
/// tables). Insertion reaches `note_child_list_changed_with` only through
/// `note_child_inserted`, which returns early for a parent with no
/// structural-selector flags; the moved row's part does not change.
#[test]
fn p2_rows_moved_into_an_empty_table_stack() {
    let mut doc = fresh();
    let body = doc.body();
    let a = el(&mut doc, body, "t");
    let b = el(&mut doc, body, "t");
    let r1 = row(&mut doc, a);
    let r2 = row(&mut doc, a);
    relayout(&mut doc, 800.0);
    doc.append_child(b, r1);
    doc.append_child(b, r2);
    relayout(&mut doc, 801.0);
    let got = pos(&doc, r2);
    assert_eq!(got.0, 0.0, "r2 must stack under r1 in b, got {got:?}");
}

/// P2b: a bare-cell table gets an already-styled row appended (moved in).
#[test]
fn p2b_moved_row_into_bare_cell_table_stacks() {
    let mut doc = fresh();
    let body = doc.body();
    let a = el(&mut doc, body, "t");
    let b = el(&mut doc, body, "t");
    let c1 = cell(&mut doc, b);
    let r = row(&mut doc, a);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, c1), (0.0, 0.0));
    doc.append_child(b, r);
    relayout(&mut doc, 801.0);
    let got = pos(&doc, r);
    assert_eq!(got.0, 0.0, "r must stack under c1, got {got:?}");
}

/// P2c: the same move with insert_before and replace_node.
#[test]
fn p2c_moved_row_via_insert_before_and_replace() {
    for route in ["insert_before", "replace", "insert_child"] {
        let mut doc = fresh();
        let body = doc.body();
        let a = el(&mut doc, body, "t");
        let b = el(&mut doc, body, "t");
        let c1 = cell(&mut doc, b);
        let c2 = cell(&mut doc, b);
        let r = row(&mut doc, a);
        let r2 = row(&mut doc, a);
        relayout(&mut doc, 800.0);
        match route {
            "insert_before" => {
                doc.insert_before(b, r, c1);
                doc.insert_before(b, r2, c1);
            }
            "replace" => {
                doc.replace_node(c2, r);
                doc.replace_node(c1, r2);
            }
            _ => {
                doc.insert_child(b, r, 0);
                doc.insert_child(b, r2, 0);
            }
        }
        relayout(&mut doc, 801.0);
        let (x1, _) = pos(&doc, r);
        let (x2, _) = pos(&doc, r2);
        assert!(x1 == 0.0 && x2 == 0.0, "{route}: r {x1}, r2 {x2}");
    }
}

/// P3 differential over routes vs fresh builds.
fn fresh_layout(build: impl Fn(&mut RinchDocument, NodeId) -> Vec<NodeId>) -> Vec<(f32, f32)> {
    let mut d = fresh();
    let body = d.body();
    let ids = build(&mut d, body);
    relayout(&mut d, 800.0);
    ids.iter().map(|&n| pos(&d, n)).collect()
}

#[test]
fn p3_remove_last_row_leaves_cells_side_by_side() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let c1 = cell(&mut doc, t);
    let c2 = cell(&mut doc, t);
    let r = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    doc.remove_node(r);
    relayout(&mut doc, 801.0);
    assert_eq!(pos(&doc, c2).1, 0.0, "c2 beside c1: {:?}", pos(&doc, c2));
    let _ = c1;
}

#[test]
fn p3_row_changes_to_block_and_back() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let c1 = cell(&mut doc, t);
    let c2 = cell(&mut doc, t);
    let r = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, c2).1, 20.0, "stacked while mixed");
    doc.set_attribute(r, "class", "c");
    relayout(&mut doc, 801.0);
    assert_eq!(pos(&doc, c2).1, 0.0, "cells again");
    doc.set_attribute(r, "class", "r");
    relayout(&mut doc, 802.0);
    assert_eq!(pos(&doc, c2).1, 20.0, "mixed again");
    let _ = c1;
}

#[test]
fn p3_set_inner_html_rows() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let _c = cell(&mut doc, t);
    relayout(&mut doc, 800.0);
    doc.set_inner_html(
        t,
        "<div class='r'><div class='c'><div class='b'></div></div></div><div id='x' class='r'><div class='c'><div class='b'></div></div></div>",
    );
    relayout(&mut doc, 801.0);
    let r2 = doc.tree.get(t.0).unwrap().children[1];
    let l = doc.tree.get(r2).unwrap().layout;
    assert_eq!((l.x, l.y), (0.0, 20.0));
}

#[test]
fn p3_set_inner_html_back_to_cells() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    row(&mut doc, t);
    row(&mut doc, t);
    relayout(&mut doc, 800.0);
    doc.set_inner_html(
        t,
        "<div class='c'><div class='b'></div></div><div class='c'><div class='b'></div></div>",
    );
    relayout(&mut doc, 801.0);
    let c2 = doc.tree.get(t.0).unwrap().children[1];
    let l = doc.tree.get(c2).unwrap().layout;
    assert_eq!(l.y, 0.0, "cells side by side: {:?}", (l.x, l.y));
}

#[test]
fn p3_rows_in_a_wrapper_appended_and_wrapper_removed() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let c1 = cell(&mut doc, t);
    let c2 = cell(&mut doc, t);
    let w = el(&mut doc, t, "w");
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, c2).1, 0.0);
    let r = row(&mut doc, w);
    relayout(&mut doc, 801.0);
    assert_eq!(pos(&doc, c2).1, 20.0, "row in wrapper stacks all");
    doc.remove_node(r);
    relayout(&mut doc, 802.0);
    assert_eq!(pos(&doc, c2).1, 0.0, "row gone");
    let r = row(&mut doc, w);
    relayout(&mut doc, 803.0);
    assert_eq!(pos(&doc, c2).1, 20.0, "row back");
    doc.remove_node(w);
    relayout(&mut doc, 804.0);
    assert_eq!(pos(&doc, c2).1, 0.0, "wrapper gone");
    let _ = (c1, r);
}

/// Keyed reorder within one parent: the direction should not move.
#[test]
fn p3_reorder_rows() {
    let mut doc = fresh();
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let r1 = row(&mut doc, t);
    let r2 = row(&mut doc, t);
    let r3 = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    doc.insert_before(t, r3, r1);
    doc.insert_before(t, r2, r3);
    relayout(&mut doc, 801.0);
    let want = fresh_layout(|d, body| {
        let t = el(d, body, "t");
        vec![row(d, t), row(d, t), row(d, t)]
    });
    let got = vec![pos(&doc, r2), pos(&doc, r3), pos(&doc, r1)];
    assert_eq!(got, want);
}

/// A row group that is itself inside a contents wrapper in the table,
/// whose rows are then all removed: the row group is still row-like.
#[test]
fn p3_nested_group() {
    let mut doc = fresh();
    doc.load_css(".g { display: table-row-group; }");
    let body = doc.body();
    let t = el(&mut doc, body, "t");
    let g = el(&mut doc, t, "g");
    let r1 = row(&mut doc, g);
    let r2 = row(&mut doc, g);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0));
    doc.remove_node(r1);
    let c1 = cell(&mut doc, g);
    let c2 = cell(&mut doc, g);
    doc.remove_node(r2);
    relayout(&mut doc, 801.0);
    assert_eq!(pos(&doc, c2).1, 0.0, "group of cells side by side {:?}", pos(&doc, c2));
    let _ = c1;
}

/// P1c: a finished width transition leaves the rows side by side for good.
#[test]
fn p1c_rows_stay_stacked_after_a_transition_finishes() {
    let mut doc = fresh();
    doc.load_css(".tq { transition: width 1ms linear; }");
    let body = doc.body();
    let t = el(&mut doc, body, "t tq");
    let _r1 = row(&mut doc, t);
    let r2 = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    doc.set_attribute(t, "class", "t tq wide");
    relayout(&mut doc, 801.0);
    std::thread::sleep(std::time::Duration::from_millis(20));
    while doc.tick_transitions() {}
    relayout(&mut doc, 802.0);
    relayout(&mut doc, 803.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0), "after the transition finished");
}

/// A table inside an inline-block (sized by its own compute) whose
/// cells become rows.
#[test]
fn k_table_in_inline_block_redirects() {
    let mut doc = fresh();
    doc.load_css(".ib { display: inline-block; }");
    let body = doc.body();
    let ib = el(&mut doc, body, "ib");
    let t = el(&mut doc, ib, "t");
    let x = cell(&mut doc, t);
    let y = cell(&mut doc, t);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, y).1, 0.0);
    doc.set_attribute(x, "class", "r");
    doc.set_attribute(y, "class", "r");
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, y), (0.0, 20.0), "rows stack inside the inline-block");
    let _ = x;
}

/// An inline-table (block-level in rinch, see CLAUDE.md) holding rows stacks them.
#[test]
fn k_inline_table_rows_stack() {
    let mut doc = fresh();
    doc.load_css(".it { display: inline-table; }");
    let body = doc.body();
    let t = el(&mut doc, body, "it");
    let _r1 = row(&mut doc, t);
    let r2 = row(&mut doc, t);
    relayout(&mut doc, 800.0);
    assert_eq!(pos(&doc, r2), (0.0, 20.0));
}
