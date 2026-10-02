//! #1214: a `colspan` past Chrome's 1000 is capped where a document enters an
//! editor (`EditorHandle::new`, `load_doc`), as the HTML parser has capped it
//! since #1164. A `TableMap` is as wide as its widest row; a 2-row table with
//! one `colspan = 3,000,000` cell was 2^21 columns wide (#1176's slot budget),
//! so `addRowAfter` built 2,097,152 cells: 15.6 s and 5 GB through a mounted
//! view. A `rowspan` needs no cap: a map is never taller than its rows.

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_editor_core::*;
use rinch_editor_view::EditorHandle;
use std::cell::RefCell;
use std::rc::Rc;

fn cell(s: &Schema, c: i64, r: i64) -> Node {
    let p = s
        .branch("paragraph", Fragment::from_node(s.text("c").unwrap()))
        .unwrap();
    s.create_node(
        "table_cell",
        Attrs::from_iter([
            ("colspan", AttrValue::Int(c)),
            ("rowspan", AttrValue::Int(r)),
        ]),
        Fragment::from_node(p),
    )
    .unwrap()
}

/// A table of `rows`, then a paragraph.
fn doc(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    let rows = rows
        .iter()
        .map(|r| {
            let cells = r.iter().map(|&(c, h)| cell(s, c, h)).collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap();
    let tail = s.branch("paragraph", Fragment::empty()).unwrap();
    s.branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap()
}

/// Mounts `d` on `schema` — the SAME `Rc<Schema>` the caller built `d` with,
/// so this test file's fixtures are about the colspan cap, not about
/// cross-schema adoption (#440's `rebind` on `EditorHandle::new` otherwise
/// rebuilds every doc here, since a fresh `Schema::starter_kit()` per `mount`
/// call is never the same `Rc` as the one `doc()`/`cell()` built with).
fn mount(schema: &Rc<Schema>, d: Node) -> EditorHandle {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let dd: Rc<RefCell<dyn DomDocument>> = mock;
    let id = dd.borrow_mut().create_element("div");
    let container = NodeHandle::new(id, Rc::downgrade(&dd));
    let h = EditorHandle::new(
        container,
        Rc::downgrade(&dd),
        schema.clone(),
        d,
        default_plugins(),
    );
    std::mem::forget(dd);
    h
}

fn spans(doc: &Node) -> Vec<Vec<(Option<i64>, Option<i64>)>> {
    let t = doc.child(0);
    (0..t.child_count())
        .map(|r| {
            let row = t.child(r);
            (0..row.child_count())
                .map(|i| {
                    let a = row.child(i).attrs();
                    (a.get_int("colspan"), a.get_int("rowspan"))
                })
                .collect()
        })
        .collect()
}

fn cells(doc: &Node) -> usize {
    let t = doc.child(0);
    (0..t.child_count()).map(|r| t.child(r).child_count()).sum()
}

/// The issue's pin: the wide cell arrives through `new` and through
/// `load_doc` capped at 1000, so `addRowAfter` adds 1000 cells and
/// `splitCell` makes 999 more (not ~2^21). Spans at or under the cap, and every
/// `rowspan`, are left alone.
#[test]
fn a_colspan_past_1000_is_capped_where_a_document_is_loaded() {
    let s = Rc::new(Schema::starter_kit());
    let wide = doc(&s, &[vec![(3_000_000, 1), (7, 70_000)], vec![(1000, 2)]]);
    let want = vec![
        vec![(Some(1000), Some(1)), (Some(7), Some(70_000))],
        vec![(Some(1000), Some(2))],
    ];
    let by_new = mount(&s, wide.clone());
    assert_eq!(spans(&by_new.doc()), want, "EditorHandle::new");
    let by_load = mount(&s, doc(&s, &[vec![(1, 1)]]));
    by_load.load_doc(wide.clone());
    assert_eq!(spans(&by_load.doc()), want, "load_doc");
    // A table the cap does not touch is the very node it was.
    let fine = doc(&s, &[vec![(1000, 1), (2, 3)], vec![(1, 1)]]);
    let h = mount(&s, fine.clone());
    assert!(h.doc().same_ref(&fine), "nothing to cap, nothing rebuilt");

    for (cmd, added) in [("addRowAfter", 1000), ("splitCell", 999)] {
        let h = mount(&s, wide.clone());
        h.set_selection(Selection::cursor(Pos(4)));
        let before = cells(&h.doc());
        assert!(h.command(cmd), "{cmd}");
        assert_eq!(cells(&h.doc()) - before, added, "{cmd}");
    }
}

/// The cap is a load's, not a command's: a command may still widen a cell
/// past 1000 (`addColumnAfter` beside a 1000-wide cell makes it 1001), and
/// the next command leaves it there.
#[test]
fn a_command_is_not_capped() {
    let s = Rc::new(Schema::starter_kit());
    let h = mount(&s, doc(&s, &[vec![(1000, 1)], vec![(1, 1)]]));
    // In the second row's only cell: the column after it runs through the
    // first row's wide cell, which widens.
    let row0 = h.doc().child(0).child(0).node_size();
    h.set_selection(Selection::cursor(Pos(1 + row0 + 3)));
    assert!(h.command("addColumnAfter"));
    assert_eq!(spans(&h.doc())[0][0].0, Some(1001));
    assert!(h.command("addRowAfter"));
    assert_eq!(spans(&h.doc())[0][0].0, Some(1001));
}

/// Untouched siblings of a capped table keep their identity: the cap rebuilds
/// only the path to a capped cell (from the review of #1245).
#[test]
fn untouched_siblings_are_shared() {
    let s = Schema::starter_kit();
    let p1 = s
        .branch("paragraph", Fragment::from_node(s.text("a").unwrap()))
        .unwrap();
    let table = |c| {
        let row = s
            .create_node(
                "table_row",
                Attrs::new(),
                Fragment::from_node(cell(&s, c, 1)),
            )
            .unwrap();
        s.create_node("table", Attrs::new(), Fragment::from_node(row))
            .unwrap()
    };
    let (fine, wide) = (table(2), table(5000));
    let d = s
        .branch(
            "doc",
            Fragment::from_children(vec![p1.clone(), fine.clone(), wide]),
        )
        .unwrap();
    let out = tables::cap_colspans(&d);
    assert!(!out.same_ref(&d));
    assert!(out.child(0).same_ref(&p1));
    assert!(out.child(1).same_ref(&fine));
    assert_eq!(
        out.child(2).child(0).child(0).attrs().get_int("colspan"),
        Some(1000)
    );
}

/// An app's own transaction is not capped (the #1182 layout fixtures depend
/// on it).
#[test]
fn an_update_keeps_an_unbounded_colspan() {
    let s = Rc::new(Schema::starter_kit());
    let h = mount(&s, doc(&s, &[vec![(1, 1)]]));
    let wide = doc(&h.state().schema().clone(), &[vec![(1_000_000, 1)]]);
    assert!(h.update(|st| {
        let mut tr = st.tr();
        tr.replace_with(0, st.doc.content_size(), wide.content().clone())
            .ok()?;
        Some(tr)
    }));
    assert_eq!(spans(&h.doc())[0][0].0, Some(1_000_000));
}
