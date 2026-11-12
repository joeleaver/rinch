//! #1930 / #1945: a range from text inside a table cell to text outside its
//! table could not be deleted, typed or pasted over: the plain replace refuses
//! it ("inconsistent open depths") and the fitted delete was not tried for a
//! range whose ends are in different isolating nodes. ProseMirror's
//! `Transform.delete` fits it (`replaceStep` → `Fitter`): what is left of the
//! node the range ends in joins the textblock it starts in, and
//! prosemirror-tables' `fixTables` pads the rows the delete left short
//! (rinch's `with_table_repair` for the selection gestures).

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html, slice_from_text};
use rinch_editor_core::*;

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn branch(s: &Schema, name: &str, children: Vec<Node>) -> Node {
    s.create_node(name, Attrs::new(), Fragment::from_children(children))
        .unwrap()
}

fn td(s: &Schema, text: &str) -> Node {
    branch(s, "table_cell", vec![para(s, text)])
}

/// `p(x), table(row(td ab, td cd), row(td ef, td gh)), p(zw)`: "a|b" is 8,
/// "e|f" is 22, "x|" is 2, "z|w" is 35.
fn grid_doc(s: &Schema) -> Node {
    let row = |a: &str, b: &str| branch(s, "table_row", vec![td(s, a), td(s, b)]);
    let table = branch(s, "table", vec![row("ab", "cd"), row("ef", "gh")]);
    branch(s, "doc", vec![para(s, "x"), table, para(s, "zw")])
}

/// `blockquote(p(x), table(row(td(p(ab)))))`, #1945's document.
fn quote_doc(s: &Schema) -> Node {
    let table = branch(s, "table", vec![branch(s, "table_row", vec![td(s, "ab")])]);
    branch(
        s,
        "doc",
        vec![branch(s, "blockquote", vec![para(s, "x"), table])],
    )
}

fn state(s: &Rc<Schema>, doc: Node, selection: Selection) -> EditorState {
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mut tr = st.tr();
    tr.set_selection(selection);
    st.apply(tr)
}

fn raw_delete(s: &Rc<Schema>, doc: &Node, from: usize, to: usize) -> String {
    let st = EditorState::create(s.clone(), doc.clone(), default_plugins());
    let mut tr = st.tr();
    tr.delete(from, to)
        .unwrap_or_else(|e| panic!("delete {from}..{to}: {e}"));
    let next = st.apply(tr);
    next.doc.rebind(next.schema()).unwrap();
    node_to_html(&next.doc)
}

const X_TO_E: &str = "<p>xf</p><table><tr><td><p>gh</p></td></tr></table><p>zw</p>";

#[test]
fn a_range_from_a_cell_to_text_after_the_table_deletes() {
    let s = Rc::new(Schema::starter_kit());
    let doc = grid_doc(&s);
    assert_eq!(
        doc.resolve(Pos(8)).unwrap().parent().child(0).text(),
        Some("ab")
    );
    assert_eq!(
        doc.resolve(Pos(35)).unwrap().parent().child(0).text(),
        Some("zw")
    );
    // ProseMirror's Fitter: the text after the range ("w") joins the
    // textblock the range starts in.
    assert_eq!(
        raw_delete(&s, &doc, 8, 35),
        "<p>x</p><table><tr><td><p>aw</p></td></tr></table>"
    );
}

#[test]
fn a_range_from_text_before_the_table_into_a_cell_deletes() {
    let s = Rc::new(Schema::starter_kit());
    let doc = grid_doc(&s);
    assert_eq!(raw_delete(&s, &doc, 2, 22), X_TO_E);
}

#[test]
fn the_selection_gestures_delete_it_and_pad_the_row() {
    let s = Rc::new(Schema::starter_kit());
    let doc = grid_doc(&s);
    // 8..35 and 2..22 leave a one-cell table, which tiles; 22..35 ("e|f" to
    // "z|w") leaves the second row one cell short, and the gesture pads it
    // (the raw delete does not).
    assert_eq!(
        raw_delete(&s, &doc, 22, 35),
        "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
         <tr><td><p>ew</p></td></tr></table>"
    );
    for (a, b, want) in [
        (8, 35, "<p>x</p><table><tr><td><p>aw</p></td></tr></table>"),
        (2, 22, X_TO_E),
        (
            22,
            35,
            "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
             <tr><td><p>ew</p></td><td><p></p></td></tr></table>",
        ),
    ] {
        for sel in [
            Selection::text(Pos(a), Pos(b)),
            Selection::text(Pos(b), Pos(a)),
        ] {
            let st = state(&s, doc.clone(), sel.clone());
            assert_eq!(st.selection, sel, "stays a text selection");
            for cmd in ["deleteSelection", "deleteCharBackward", "deleteCharForward"] {
                let next = st.run(cmd).unwrap_or_else(|| panic!("{cmd} {a}..{b}"));
                next.doc.rebind(next.schema()).unwrap();
                assert_eq!(node_to_html(&next.doc), want, "{cmd} {a}..{b}");
                assert_eq!(next.selection, Selection::cursor(Pos(a)), "{cmd} {a}..{b}");
                // One undo step puts the document back.
                let undone = next.run("undo").expect("undo");
                assert_eq!(undone.doc, doc, "{cmd} {a}..{b} undo");
            }
        }
    }
}

#[test]
fn typing_and_paste_replace_it() {
    let s = Rc::new(Schema::starter_kit());
    let doc = grid_doc(&s);
    let st = state(&s, doc.clone(), Selection::text(Pos(2), Pos(22)));

    // Typing: the handle's path when the plain replace refuses the range.
    let mut tr = st.tr();
    tr.delete_selection().unwrap();
    tr.insert_text("Q").unwrap();
    let typed = st.apply(tr);
    assert_eq!(
        node_to_html(&typed.doc),
        "<p>xQf</p><table><tr><td><p>gh</p></td></tr></table><p>zw</p>"
    );

    let mut tr = st.tr();
    tr.replace_selection(slice_from_text(&s, "pp").unwrap())
        .unwrap();
    let pasted = st.apply(tr);
    assert_eq!(
        node_to_html(&pasted.doc),
        "<p>xppf</p><table><tr><td><p>gh</p></td></tr></table><p>zw</p>"
    );

    let mut tr = st.tr();
    tr.replace_selection(slice_from_html(&s, "<p>p1</p><h2>p2</h2>").unwrap())
        .unwrap();
    let pasted = st.apply(tr);
    pasted.doc.rebind(pasted.schema()).unwrap();
    assert!(
        node_to_html(&pasted.doc).starts_with("<p>xp1</p><h2>p2"),
        "{}",
        node_to_html(&pasted.doc)
    );
}

#[test]
fn select_all_then_delete_in_a_quote_holding_a_paragraph_and_a_table() {
    // #1945.
    let s = Rc::new(Schema::starter_kit());
    let doc = quote_doc(&s);
    let st = EditorState::create(s.clone(), doc.clone(), default_plugins());
    let all = st.run("selectAll").expect("selectAll");
    assert_eq!(all.selection, Selection::text(Pos(2), Pos(10)));
    for cmd in ["deleteSelection", "deleteCharBackward", "deleteCharForward"] {
        let next = all.run(cmd).unwrap_or_else(|| panic!("{cmd}"));
        next.doc.rebind(next.schema()).unwrap();
        assert_eq!(
            node_to_html(&next.doc),
            "<blockquote><p></p></blockquote>",
            "{cmd}"
        );
        assert_eq!(next.selection, Selection::cursor(Pos(2)), "{cmd}");
        assert_eq!(next.run("undo").expect("undo").doc, doc, "{cmd} undo");
    }
}

#[test]
fn a_range_from_a_cell_into_a_table_nested_in_it_is_repaired() {
    // From "n|" in the outer cell to "a|b" in the nested table's first cell:
    // the fit joins "b" to "n" and takes the cell out of the nested row. Both
    // ends are inside one outer cell, but not in one innermost cell, so the
    // gesture checks the nested table and pads its row.
    let s = Rc::new(Schema::starter_kit());
    let row = |a: &str, b: &str| branch(&s, "table_row", vec![td(&s, a), td(&s, b)]);
    let nested = branch(&s, "table", vec![row("ab", "cd"), row("ef", "gh")]);
    let outer_cell = branch(&s, "table_cell", vec![para(&s, "n"), nested]);
    let outer = branch(
        &s,
        "table",
        vec![branch(&s, "table_row", vec![outer_cell, td(&s, "q")])],
    );
    let doc = branch(&s, "doc", vec![outer]);
    assert_eq!(
        doc.resolve(Pos(11)).unwrap().parent().child(0).text(),
        Some("ab")
    );
    let st = state(&s, doc.clone(), Selection::text(Pos(5), Pos(11)));
    let next = st.run("deleteSelection").expect("deletes");
    assert_eq!(
        node_to_html(&next.doc),
        "<table><tr><td><p>nb</p><table><tr><td><p>cd</p></td><td><p></p></td></tr>\
         <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table></td>\
         <td><p>q</p></td></tr></table>"
    );
    assert_eq!(next.run("undo").expect("undo").doc, doc);
}
