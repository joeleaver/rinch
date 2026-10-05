//! Review of PR #1394: failing fixtures for the defects found. Each is red at
//! the PR head (3aefd8c8).
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{PasteContent, Pos, Selection};
use rinch_editor_view::create_editor;

/// D1. A paste over a cell selection must not take cells out of the table.
/// Main e558e24b refuses the paste (nothing changes); the PR head deletes the
/// first selected cell (or row) and puts the content before the table.
#[test]
fn d1_a_paste_over_a_cell_selection_keeps_the_table_grid() {
    let table = "<table><tr><td><p>a</p></td><td><p>b</p></td><td><p>c</p></td></tr>\
                 <tr><td><p>d</p></td><td><p>e</p></td><td><p>f</p></td></tr></table><p>z</p>";
    for html in ["<p>P</p>", "X", "<ul><li><p>A</p></li></ul>", "<hr>"] {
        let ed = create_editor();
        ed.load_html(table);
        // Cells a and b: positions before each (table 0, row 1, cell a 2, cell b 7).
        ed.set_selection(Selection::cell(Pos(2), Pos(7)));
        ed.paste(&PasteContent {
            text: Some("T".into()),
            html: Some(html.into()),
        });
        let out = node_to_html(&ed.doc());
        let rows: Vec<usize> = out
            .split("<tr>")
            .skip(1)
            .map(|r| r.matches("<td>").count())
            .collect();
        assert_eq!(
            rows,
            [3, 3],
            "pasting {html}: the grid is intact; got {out}"
        );
        assert!(
            out.starts_with("<table>"),
            "pasting {html}: nothing lands outside the table; got {out}"
        );
    }
}

/// D2. A link (or any mark) around block elements keeps its mark. Main read
/// `<a href><h3>…</h3><p>…</p></a>` as linked text (blocks flattened); the PR
/// head keeps the blocks and drops the link, href and all. (Behind a
/// `<meta>`, as a browser copies it, main read nothing and pasted plain text,
/// so there the href was lost before too; `load_html` and meta-less clipboard
/// HTML are where this is new.)
#[test]
fn d2_a_link_around_blocks_keeps_its_href() {
    let ed = create_editor();
    ed.load_html("<p></p>");
    ed.set_selection(Selection::cursor(Pos(1)));
    assert!(ed.replace_selection_with_html(
        "<a href=\"https://e.x/post\"><h3>Card title</h3><p>summary</p></a>"
    ));
    let out = node_to_html(&ed.doc());
    assert!(out.contains("https://e.x/post"), "the href survives: {out}");
    // And a real <strong> around paragraphs stays bold.
    let ed = create_editor();
    ed.load_html("<strong><p>x</p><p>y</p></strong>");
    let out = node_to_html(&ed.doc());
    assert!(out.contains("<strong>x</strong>"), "bold survives: {out}");
}

/// D3. Google Docs puts a nested list beside the item it belongs to
/// (`<ul><li>one</li><ul><li>nested</li></ul><li>two</li></ul>`), behind a
/// `<meta charset>`. Main read the HTML as empty (the `<meta>` bug) and pasted
/// the plain text, complete. The PR head reads the HTML and the reader drops
/// every list child that is not an `<li>`: the nested items are gone.
#[test]
fn d3_a_google_docs_nested_list_keeps_its_nested_items() {
    let docs = "<meta charset='utf-8'><b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\">\
        <ul><li dir=\"ltr\"><p dir=\"ltr\"><span>one</span></p></li>\
        <ul><li dir=\"ltr\"><p dir=\"ltr\"><span>nested</span></p></li></ul>\
        <li dir=\"ltr\"><p dir=\"ltr\"><span>two</span></p></li></ul></b>";
    let ed = create_editor();
    ed.load_html("<p></p>");
    ed.set_selection(Selection::cursor(Pos(1)));
    assert!(ed.paste(&PasteContent {
        text: Some("one\nnested\ntwo".into()),
        html: Some(docs.into()),
    }));
    let out = node_to_html(&ed.doc());
    assert!(
        out.contains("nested"),
        "the nested item's text survives: {out}"
    );
}
