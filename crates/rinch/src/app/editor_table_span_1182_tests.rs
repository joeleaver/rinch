//! #1182 on the desktop: an editor table whose cells' spans reach the model
//! unbounded — by `load_doc`, or an app's own transaction — is laid out as the
//! grid the model's `TableMap` sees, not as the grid its raw `colspan` /
//! `rowspan` attributes ask for.
//!
//! The shape is four rows, each one cell of `colspan = 1_000_000, rowspan =
//! i64::MAX`. Written raw, Stylo clamps each span to 10000 (its
//! `MAX_GRID_LINE`), so each cell fills the whole clamped template and the
//! next is placed below it: four cells of 10000 rows are 40000 grid lines,
//! past the `i16` Taffy numbers its lines in, and the layout **panicked**
//! (`OriginZero grid line cannot be more than the number of positive grid
//! lines`). The map's grid is `2^22 / 4 = 1_048_576` columns wide: the first
//! cell spans 1_000_000 of them and all four rows, the second the 48_576 left
//! beside it and the three rows below its own, and the last two find their
//! rows full, are in no slot, and are laid out as bands across the grid.
//!
//! What the desktop draws of that is still clamped by Stylo: the template and
//! every span stop at 10000 tracks, so the second cell does not fit beside the
//! first and is placed below it. That is Stylo's limit, not the view's; the
//! browser twin (`rinch-web`'s `editor_table_spans_1182`) sees the map's grid
//! as it is.
#![cfg(feature = "desktop")]
use super::*;
use rinch_core::dom::NodeId;
use rinch_editor_core::AttrValue;
use rinch_editor_core::model::{Attrs, Fragment, Node};
use std::cell::Cell;

fn table_doc(handle: &crate::editor::EditorHandle, rows: &[Vec<(i64, i64)>]) -> Node {
    let state = handle.state();
    let s = state.schema();
    let rows: Vec<Node> = rows
        .iter()
        .enumerate()
        .map(|(ri, r)| {
            let cells: Vec<Node> = r
                .iter()
                .enumerate()
                .map(|(ci, &(c, h))| {
                    let p = s
                        .create_node(
                            "paragraph",
                            Attrs::new(),
                            Fragment::from_node(s.text(&format!("r{ri}c{ci}")).unwrap()),
                        )
                        .unwrap();
                    s.create_node(
                        "table_cell",
                        Attrs::from_iter([
                            ("colspan", AttrValue::Int(c)),
                            ("rowspan", AttrValue::Int(h)),
                        ]),
                        Fragment::from_node(p),
                    )
                    .unwrap()
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap();
    s.branch("doc", Fragment::from_node(t)).unwrap()
}

/// Mount an editor holding `rows` in a 400px column and lay it out.
fn mounted(rows: &[Vec<(i64, i64)>]) -> (RinchApp, Vec<usize>, usize) {
    let editor_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let editor_in = editor_id.clone();
    let handle = crate::editor::create_editor();
    handle.load_doc(table_doc(&handle, rows));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 400px; line-height: 20px; font-size: 16px");
        let editor = handle_in.mount(scope);
        editor_in.set(Some(editor.node_id().0));
        root.append_child(&editor);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let _ = editor_id.get().unwrap();
    let doc = app.doc.as_ref().unwrap().clone();
    let d = doc.borrow();
    let cells: Vec<usize> = (0..d.tree.nodes.len())
        .filter(|&id| {
            d.tree.nodes.get(id).is_some()
                && d.get_attribute(NodeId(id), "data-pm-type").as_deref() == Some("table_cell")
        })
        .collect();
    let table = (0..d.tree.nodes.len())
        .find(|&id| {
            d.tree.nodes.get(id).is_some()
                && d.get_attribute(NodeId(id), "data-pm-type").as_deref() == Some("table")
        })
        .unwrap();
    drop(d);
    (app, cells, table)
}

fn rect(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let l = app.doc.as_ref().unwrap().borrow().tree.nodes[id].layout;
    (l.x, l.y, l.width, l.height)
}

#[test]
fn a_table_of_unbounded_spans_lays_out_as_its_table_map() {
    let big = (1_000_000, i64::MAX);
    let (app, cells, table) = mounted(&[vec![big], vec![big], vec![big], vec![big]]);
    assert_eq!(cells.len(), 4, "control: four cells mounted");
    let (_, _, tw, th) = rect(&app, table);
    let rects: Vec<_> = cells.iter().map(|&c| rect(&app, c)).collect();
    let (x0, y0, w0, h0) = rects[0];
    // Every cell is as wide as the grid: the first two by their spans (which
    // Stylo clamps to the clamped template), the others as bands across it.
    // (The table's own 1px border is inside its box.)
    for (i, &(x, _, w, _)) in rects.iter().enumerate() {
        assert_eq!(
            (x, w),
            (x0, w0),
            "cell {i} is not the grid's width: {rects:?}"
        );
    }
    assert!(w0 > 0.0 && w0 <= tw, "{rects:?} in a {tw}-wide table");
    // The cells stack one after another, and the table is as tall as them:
    // rows 1-4 hold the first, 5-7 the second, 8 and 9 the bands. (Each box is
    // rounded to whole pixels on its own, so neighbours may be a pixel apart.)
    let mut y = y0 + h0;
    for (i, &(_, by, _, bh)) in rects.iter().enumerate().skip(1) {
        assert!(
            (by - y).abs() <= 1.0,
            "cell {i} at {by}, expected {y}: {rects:?}"
        );
        y = by + bh;
    }
    assert!(
        (th - (y - y0)).abs() <= 2.5,
        "the table is {th} tall, its cells {}: {rects:?}",
        y - y0
    );
}

/// Control: an ordinary merged table lays out as before — a `colspan = 2`
/// header over two cells, which sit side by side under it.
#[test]
fn an_ordinary_merged_table_lays_out_as_before() {
    let (app, cells, _) = mounted(&[vec![(2, 1)], vec![(1, 1), (1, 1)]]);
    let r: Vec<_> = cells.iter().map(|&c| rect(&app, c)).collect();
    assert_eq!(r.len(), 3);
    assert!((r[0].2 - (r[1].2 + r[2].2)).abs() < 0.5, "{r:?}");
    assert_eq!(r[1].1, r[2].1, "{r:?}");
    assert!(r[2].0 > r[1].0 && r[1].1 > r[0].1, "{r:?}");
}

/// #1209: a row a rowspan leaves short keeps its cells. In `[[A rowspan=2],
/// [B]]` the map puts B in row 1, column 1 — beside A's lower half — where CSS
/// auto-placement, which knows no rows, packed it into row 0 beside A's top.
#[test]
fn a_cell_in_a_row_a_rowspan_leaves_short_stays_in_its_row() {
    let (app, cells, _) = mounted(&[vec![(1, 2)], vec![(1, 1)]]);
    let r: Vec<_> = cells.iter().map(|&c| rect(&app, c)).collect();
    assert_eq!(r.len(), 2, "control: two cells mounted");
    let (ax, ay, aw, ah) = r[0];
    let (bx, by, _, bh) = r[1];
    assert!(bx >= ax + aw - 1.0, "B is in column 1: {r:?}");
    assert!(by > ay + 1.0, "B is in row 1, not beside A's top: {r:?}");
    assert!(
        (by + bh - (ay + ah)).abs() <= 1.0,
        "A spans both rows, ending with B: {r:?}"
    );
}

/// Off the one-short-row shape: a three-row rowspan on the left, a colspan 2
/// in the middle row and two cells in the last. The map is `A B B / A C D`
/// under an `A . .` first row (left to the right of A empty), so B is in row
/// 1 and C, D in row 2 under its two columns. Auto-placement lifted the whole
/// right side a row: B beside A's top, C and D in row 1.
#[test]
fn every_row_under_a_short_one_keeps_its_cells() {
    let (app, cells, _) = mounted(&[vec![(1, 3)], vec![(2, 1)], vec![(1, 1), (1, 1)]]);
    let r: Vec<_> = cells.iter().map(|&c| rect(&app, c)).collect();
    assert_eq!(r.len(), 4, "control: four cells mounted");
    let [a, b, c, d] = [r[0], r[1], r[2], r[3]];
    assert!(b.1 > a.1 + 1.0, "B is below row 0: {r:?}");
    assert!(c.1 > b.1 + 1.0 && (c.1 - d.1).abs() < 0.5, "C, D in row 2: {r:?}");
    assert!((c.0 - b.0).abs() < 0.5 && d.0 > c.0, "C under B's left: {r:?}");
    assert!(
        (d.1 + d.3 - (a.1 + a.3)).abs() <= 1.0,
        "A ends with the last row: {r:?}"
    );
}
