//! #1214: `Schema::node_from_doc` — the durable save/load path — caps a
//! `colspan` past 1000 as the HTML parser does (#1164) and as the editor's
//! own loads do (`rinch-editor-view/tests/colspan_cap_on_load.rs`).
#![cfg(feature = "serde")]

use rinch_editor_core::*;

fn cell(s: &Schema, c: i64) -> Node {
    let p = s.branch("paragraph", Fragment::empty()).unwrap();
    s.create_node(
        "table_header_cell",
        Attrs::from_iter([("colspan", AttrValue::Int(c))]),
        Fragment::from_node(p),
    )
    .unwrap()
}

#[test]
fn a_saved_colspan_past_1000_loads_capped() {
    let s = Schema::starter_kit();
    let row = s
        .create_node(
            "table_row",
            Attrs::new(),
            Fragment::from_children(vec![cell(&s, 3_000_000), cell(&s, 1000), cell(&s, 1001)]),
        )
        .unwrap();
    let t = s
        .create_node("table", Attrs::new(), Fragment::from_node(row))
        .unwrap();
    // A table nested in a list item: the cap reaches every table.
    let item = s
        .create_node(
            "list_item",
            Attrs::new(),
            Fragment::from_children(vec![s.branch("paragraph", Fragment::empty()).unwrap(), t]),
        )
        .unwrap();
    let blocks = vec![
        s.create_node("bullet_list", Attrs::new(), Fragment::from_node(item))
            .unwrap(),
    ];
    let doc = s.branch("doc", Fragment::from_children(blocks)).unwrap();
    let loaded = s.node_from_doc(&doc.to_doc().unwrap()).unwrap();
    let row = loaded.child(0).child(0).child(1).child(0);
    let got: Vec<_> = (0..3)
        .map(|i| row.child(i).attrs().get_int("colspan"))
        .collect();
    assert_eq!(got, [Some(1000), Some(1000), Some(1000)]);
}
