//! #1184: the table commands through a mounted `EditorHandle` (the path a
//! toolbar button or context menu takes), on ragged tables with a cell
//! selection spanning holes; then undo and redo.

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::*;
use rinch_editor_view::EditorHandle;
use std::cell::RefCell;
use std::rc::Rc;

fn cell(s: &Schema, c: i64, r: i64, t: &str) -> Node {
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
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

fn doc(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    let mut n = 0;
    let rows = rows
        .iter()
        .map(|r| {
            let cells = r
                .iter()
                .map(|&(c, h)| {
                    n += 1;
                    cell(s, c, h, &format!("c{n}"))
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap();
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    s.branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap()
}

fn cell_pos(d: &Node, r: usize, i: usize) -> usize {
    let t = d.child(0);
    let mut pos = 1;
    for j in 0..r {
        pos += t.child(j).node_size();
    }
    pos += 1;
    for k in 0..i {
        pos += t.child(r).child(k).node_size();
    }
    pos
}

fn mount(d: Node) -> (Rc<RefCell<MockDomDocument>>, EditorHandle) {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let dd: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let id = dd.borrow_mut().create_element("div");
    let container = NodeHandle::new(id, Rc::downgrade(&dd));
    let h = EditorHandle::new(
        container,
        Rc::downgrade(&dd),
        Rc::new(Schema::starter_kit()),
        d,
        default_plugins(),
    );
    std::mem::forget(dd);
    (mock, h)
}

#[test]
fn every_table_command_over_a_hole_spanning_selection_through_the_handle() {
    let s = Schema::starter_kit();
    let shapes: Vec<Vec<Vec<(i64, i64)>>> = vec![
        vec![
            vec![(1, 1), (1, 1), (1, 1)],
            vec![(1, 1)],
            vec![(1, 1), (1, 1), (1, 1)],
        ],
        vec![vec![(1, 1), (1, 1), (1, 1)], vec![(1, 1)], vec![(1, 1)]],
        vec![
            vec![(1, 1), (1, 1), (1, 1), (1, 2)],
            vec![(1, 1)],
            vec![(1, 1), (1, 1), (1, 1), (1, 1)],
        ],
        vec![vec![(i64::MAX, 1)], vec![(1, 1), (1, 1)]],
    ];
    let cmds = [
        "addRowBefore",
        "addRowAfter",
        "addColumnBefore",
        "addColumnAfter",
        "deleteRow",
        "deleteColumn",
        "mergeCells",
        "splitCell",
        "deleteTable",
        "deleteCellSelection",
    ];
    let mut applied = 0;
    for rows in &shapes {
        let d = doc(&s, rows);
        let huge = rows[0][0].0 == i64::MAX;
        // anchor: last cell of row 0; head: first cell of the last non-empty row
        let last_row = (0..rows.len())
            .rev()
            .find(|&r| !rows[r].is_empty())
            .unwrap();
        let sel = Selection::cell(
            Pos(cell_pos(&d, 0, rows[0].len() - 1)),
            Pos(cell_pos(&d, last_row, 0)),
        );
        for c in cmds {
            if huge && (c.starts_with("addRow") || c == "splitCell") {
                continue;
            }
            let (_mock, h) = mount(d.clone());
            h.set_selection(sel.clone());
            let before = h.doc();
            if !h.command(c) {
                continue;
            }
            applied += 1;
            let after = h.doc();
            if after == before {
                continue;
            }
            assert!(h.command("undo"), "{c}: undo");
            assert_eq!(
                node_to_html(&h.doc()),
                node_to_html(&before),
                "{c}: undo on {rows:?}"
            );
            assert!(h.doc() == before);
            assert!(h.command("redo"), "{c}: redo");
            assert!(h.doc() == after, "{c}: redo");
        }
    }
    eprintln!("applied {applied}");
    assert!(applied >= 20, "applied {applied}");
}
