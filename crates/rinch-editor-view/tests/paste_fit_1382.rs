//! A pasted slice is fitted to where it lands (#1382, #1389): block content
//! pasted at a caret no longer falls back to `text/plain`.
//!
//! The expected documents are ProseMirror's (`replaceRange` + the `Fitter`):
//! content that is open at the start continues the line it is pasted on, the
//! rest keeps its structure, and the text after the caret joins the pasted
//! content's last textblock; a closed block splits the textblock.
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;

const LIST: &str = "<ul><li><p><strong>A</strong></p></li><li><p>B</p></li></ul>";

fn editor(target: &str, from: usize, to: usize) -> EditorHandle {
    let dst = create_editor();
    dst.load_html(target);
    dst.set_selection(Selection::text(Pos(from), Pos(to)));
    dst
}

/// Paste `html` over `from..to` of `target`; the HTML flavour must be what
/// lands (no `text/plain` fallback).
fn paste(target: &str, from: usize, to: usize, html: &str) -> String {
    let dst = editor(target, from, to);
    assert!(
        dst.replace_selection_with_html(html),
        "the HTML paste of {html:?} into {target:?} at {from}..{to} was refused"
    );
    node_to_html(&dst.doc())
}

/// The same, with the caret's position after the paste.
fn paste_caret(target: &str, at: usize, html: &str) -> (String, usize) {
    let dst = editor(target, at, at);
    assert!(dst.replace_selection_with_html(html));
    let sel = dst.selection();
    assert!(sel.is_empty(), "the paste leaves a caret");
    (node_to_html(&dst.doc()), sel.from().0)
}

/// The issue's reproduction: a selection inside a bullet list, copied and
/// pasted on an empty line, is that list with its marks.
#[test]
fn copied_list_items_paste_as_a_list_on_an_empty_line() {
    let src = create_editor();
    src.load_html(&format!("<p>before</p>{LIST}<p>after</p>"));
    src.set_selection(Selection::text(Pos(11), Pos(19)));
    let (html, text) = src.selection_clipboard().unwrap();
    assert_eq!(html, LIST, "the copy carries the list around its items");
    let dst = editor("<p></p>", 1, 1);
    assert!(dst.paste(&PasteContent {
        text: Some(text),
        html: Some(html),
    }));
    assert_eq!(node_to_html(&dst.doc()), LIST);
    assert_eq!(dst.selection().from().0, 9, "the caret ends after B");
}

/// A copy from inside an ordered or a task list says which list it was.
#[test]
fn a_copy_from_inside_a_list_keeps_the_kind_of_list() {
    let copy = |doc: &str, from: usize, to: usize| {
        let src = editor(doc, from, to);
        src.selection_clipboard().unwrap().0
    };
    assert_eq!(
        copy(
            "<ol start=\"3\"><li><p>ab</p></li><li><p>cd</p></li></ol>",
            4,
            10
        ),
        "<ol start=\"3\"><li><p>b</p></li><li><p>c</p></li></ol>"
    );
    let tasks = "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>ab</p></li>\
                 <li data-type=\"taskItem\" data-checked=\"false\"><p>cd</p></li></ul>";
    assert_eq!(
        copy(tasks, 4, 10),
        "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>b</p></li>\
         <li data-type=\"taskItem\" data-checked=\"false\"><p>c</p></li></ul>"
    );
    // A selection inside one textblock is its inline content, as before.
    assert_eq!(copy("<ul><li><p>abcd</p></li></ul>", 4, 6), "bc");
    // Blocks the document takes as they are need nothing around them.
    assert_eq!(
        copy("<blockquote><p>ab</p><p>cd</p></blockquote>", 3, 7),
        "<p>b</p><p>c</p>"
    );
}

/// Lists as a browser, GitHub or Google Docs put them on the clipboard.
#[test]
fn browser_lists_paste_as_lists() {
    let want = "<ul><li><p>A</p></li><li><p>B</p></li></ul>";
    for html in [
        "<ul><li>A</li><li>B</li></ul>",
        "<meta charset='utf-8'><ul><li>A</li><li>B</li></ul>",
        "<html><body><!--StartFragment--><ul>\n<li>A</li>\n<li>B</li>\n</ul><!--EndFragment--></body></html>",
        "<meta charset='utf-8'><b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\"><ul style=\"margin:0\"><li dir=\"ltr\"><p dir=\"ltr\"><span>A</span></p></li><li dir=\"ltr\"><p dir=\"ltr\"><span>B</span></p></li></ul></b>",
        "<ul dir=\"auto\">\n<li>A</li>\n<li>B</li>\n</ul>",
        // Items alone, as a selection inside one list was copied before.
        "<li>A</li><li>B</li>",
    ] {
        assert_eq!(paste("<p></p>", 1, 1, html), want, "{html}");
    }
    assert_eq!(
        paste("<p></p>", 1, 1, "<ol start=\"4\"><li>A</li></ol>"),
        "<ol start=\"4\"><li><p>A</p></li></ol>"
    );
    assert_eq!(
        paste("<p>x</p><p></p>", 4, 4, "<ul><li>A</li></ul>"),
        "<p>x</p><ul><li><p>A</p></li></ul>"
    );
}

/// Inside text, the first item continues the line, the rest stays a list and
/// the text after the caret joins the last item.
#[test]
fn a_list_pasted_inside_text() {
    assert_eq!(
        paste_caret("<p>abc</p>", 2, LIST),
        (
            "<p>a<strong>A</strong></p><ul><li><p>Bbc</p></li></ul>".to_string(),
            8
        )
    );
    // At the end of a line there is no text to carry.
    assert_eq!(
        paste("<p>abc</p>", 4, 4, LIST),
        "<p>abc<strong>A</strong></p><ul><li><p>B</p></li></ul>"
    );
    // One item is just its text.
    assert_eq!(
        paste("<p>abc</p>", 2, 2, "<ul><li>X</li></ul>"),
        "<p>aXbc</p>"
    );
}

/// A block that is closed at both ends splits the textblock it lands in, and
/// leaves no empty block at a textblock's edge.
#[test]
fn a_closed_block_splits_the_textblock() {
    assert_eq!(paste("<p>abc</p>", 2, 2, "<hr>"), "<p>a</p><hr><p>bc</p>");
    assert_eq!(paste("<p>abc</p>", 1, 1, "<hr>"), "<hr><p>abc</p>");
    assert_eq!(paste("<p>abc</p>", 4, 4, "<hr>"), "<p>abc</p><hr>");
    assert_eq!(paste("<p>x</p><p></p>", 4, 4, "<hr>"), "<p>x</p><hr>");
    // Over a selection: the selected text goes, the rest is split.
    assert_eq!(paste("<p>abc</p>", 2, 3, "<hr>"), "<p>a</p><hr><p>c</p>");
    let table = "<table><tbody><tr><td><p>1</p></td><td><p>2</p></td></tr></tbody></table>";
    let out = paste("<h2>abc</h2>", 2, 2, table);
    assert!(
        out.starts_with("<h2>a</h2><table>") && out.ends_with("</table><h2>bc</h2>"),
        "{out}"
    );
    assert!(
        out.contains("<p>1</p>") && out.contains("<p>2</p>"),
        "{out}"
    );
}

/// In a list item, pasted items are sibling items, never a list in a
/// paragraph; the target list keeps its kind.
#[test]
fn list_items_pasted_in_a_list_are_sibling_items() {
    assert_eq!(
        paste("<ul><li><p>xy</p></li></ul>", 4, 4, LIST),
        "<ul><li><p>x<strong>A</strong></p></li><li><p>By</p></li></ul>"
    );
    assert_eq!(
        paste("<ol><li><p>xy</p></li><li><p>z</p></li></ol>", 4, 4, LIST),
        "<ol><li><p>x<strong>A</strong></p></li><li><p>By</p></li><li><p>z</p></li></ol>"
    );
    // On an empty item, the pasted items take its place.
    assert_eq!(
        paste("<ul><li><p>x</p></li><li><p></p></li></ul>", 8, 8, LIST),
        "<ul><li><p>x</p></li><li><p><strong>A</strong></p></li><li><p>B</p></li></ul>"
    );
}

const TASKS: &str = "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>A</p></li>\
     <li data-type=\"taskItem\" data-checked=\"false\"><p>B</p></li></ul>";

/// Task items keep their checkboxes: as a list on an empty line, as items on
/// an empty item, and for every item but the one that continues a line.
#[test]
fn task_items_keep_their_checkboxes() {
    let want = TASKS.replace(">     <", "><");
    assert_eq!(paste("<p></p>", 1, 1, TASKS), want);
    let one = "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"false\"><p></p></li></ul>";
    assert_eq!(paste(one, 3, 3, TASKS), want);
    assert_eq!(
        paste("<p>ab</p>", 2, 2, TASKS),
        "<p>aA</p><ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"false\"><p>Bb</p></li></ul>"
    );
}

/// #1389: a paragraph and then a task list, on an empty line.
#[test]
fn a_paragraph_then_a_task_list_on_an_empty_line() {
    let item = "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>a</p></li></ul>";
    assert_eq!(
        paste("<p></p>", 1, 1, &format!("<p>intro</p>{item}")),
        format!("<p>intro</p>{item}")
    );
    assert_eq!(
        paste("<p></p>", 1, 1, &format!("{item}<p>outro</p>")),
        format!("{item}<p>outro</p>")
    );
    assert_eq!(
        paste("<p>xy</p>", 2, 2, &format!("<p>intro</p>{item}")),
        "<p>xintro</p><ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>ay</p></li></ul>"
    );
}

/// A heading, quote or code block pasted on an empty line is that block; in
/// text it is its text.
#[test]
fn defining_blocks_replace_an_empty_line() {
    assert_eq!(paste("<p></p>", 1, 1, "<h2>Title</h2>"), "<h2>Title</h2>");
    assert_eq!(paste("<p>ab</p>", 2, 2, "<h2>Title</h2>"), "<p>aTitleb</p>");
    assert_eq!(
        paste("<p></p>", 1, 1, "<blockquote><p>q</p><p>r</p></blockquote>"),
        "<blockquote><p>q</p><p>r</p></blockquote>"
    );
    assert_eq!(
        paste("<p></p>", 1, 1, "<pre>let x;</pre>"),
        "<pre>let x;</pre>"
    );
    // Two paragraphs are not defining: they merge as before.
    assert_eq!(
        paste("<h2>ab</h2>", 2, 2, "<p>1</p><p>2</p>"),
        "<h2>a1</h2><p>2b</p>"
    );
}

/// A table cell keeps what is pasted in it.
#[test]
fn a_paste_in_a_table_cell_stays_in_the_cell() {
    let table = |cell: &str| format!("<table><tr><td>{cell}</td><td><p>z</p></td></tr></table>");
    // table(0) row(1) cell(2) paragraph(3): the caret is at 4.
    assert_eq!(paste(&table("<p></p>"), 4, 4, LIST), table(LIST));
    assert_eq!(
        paste(&table("<p>ab</p>"), 5, 5, "<hr>"),
        table("<p>a</p><hr><p>b</p>")
    );
}

/// In a code block a paste is its plain text, line breaks kept.
#[test]
fn a_paste_in_a_code_block_is_plain_text() {
    let dst = editor("<pre>ab</pre>", 2, 2);
    assert!(dst.paste(&PasteContent {
        text: Some("A\nB".into()),
        html: Some(LIST.into()),
    }));
    assert_eq!(node_to_html(&dst.doc()), "<pre>aA\nBb</pre>");
    assert_eq!(dst.selection().from().0, 5);
    // The plain half alone does the same.
    let dst = editor("<pre>ab</pre>", 2, 2);
    assert!(dst.replace_selection_with_text("1\r\n2"));
    assert_eq!(node_to_html(&dst.doc()), "<pre>a1\n2b</pre>");
}

/// Over a selection that crosses blocks.
#[test]
fn a_list_pasted_over_a_selection_across_blocks() {
    assert_eq!(
        paste("<p>abc</p><p>def</p>", 2, 8, LIST),
        "<p>a<strong>A</strong></p><ul><li><p>Bf</p></li></ul>"
    );
    // The whole of a paragraph's text selected: the list takes its place.
    assert_eq!(
        paste("<p>x</p><p>abc</p>", 4, 7, LIST),
        format!("<p>x</p>{LIST}")
    );
}

/// One paste is one undo step and one `on_change`; a read-only editor
/// refuses it.
#[test]
fn a_fitted_paste_is_one_edit() {
    let dst = editor("<p>abc</p>", 2, 2);
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    dst.on_change(move || seen.set(seen.get() + 1));
    assert!(dst.paste(&PasteContent {
        text: Some("A\nB".into()),
        html: Some(LIST.into()),
    }));
    assert_eq!(changes.get(), 1);
    assert_eq!(
        node_to_html(&dst.doc()),
        "<p>a<strong>A</strong></p><ul><li><p>Bbc</p></li></ul>"
    );
    assert!(dst.command("undo"));
    assert_eq!(node_to_html(&dst.doc()), "<p>abc</p>");
    assert!(!dst.command("undo"), "the paste was one step");

    let locked = editor("<p>abc</p>", 2, 2);
    locked.set_read_only(true);
    assert!(!locked.paste(&PasteContent {
        text: Some("A\nB".into()),
        html: Some(LIST.into()),
    }));
    assert_eq!(node_to_html(&locked.doc()), "<p>abc</p>");
}

/// Markup around the content a browser copied changes nothing: the `<meta>`
/// a browser puts first has no end tag (looking for one dropped the whole
/// paste), and Google Docs' `<b>` wrapper is not bold.
#[test]
fn a_browsers_wrappers_do_not_eat_the_paste() {
    assert_eq!(
        paste("<p>ab</p>", 2, 2, "<meta charset='utf-8'><span>X</span>"),
        "<p>aXb</p>"
    );
    assert_eq!(
        paste(
            "<p></p>",
            1,
            1,
            "<meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\"><link rel=\"x\" href=\"y>z\"><p>one</p><p>two</p>"
        ),
        "<p>one</p><p>two</p>"
    );
    // A real `<b>` around inline content is still bold.
    assert_eq!(
        paste("<p></p>", 1, 1, "<b>bold</b>"),
        "<p><strong>bold</strong></p>"
    );
}

/// A selection across table cells is copied as a table.
#[test]
fn a_selection_across_cells_is_copied_as_a_table() {
    let src = editor(
        "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr></table>",
        5,
        11,
    );
    let (html, _) = src.selection_clipboard().unwrap();
    assert_eq!(
        html,
        "<table><tr><td><p>b</p></td><td><p>c</p></td></tr></table>"
    );
    assert_eq!(
        paste("<p>xy</p>", 2, 2, &html),
        format!("<p>x</p>{html}<p>y</p>")
    );
}

/// A collaborating editor records a fitted paste like any edit: the peer
/// gets the list. A task list is outside what collaboration carries, and
/// stalls outbound as it did.
#[cfg(feature = "collaboration")]
#[test]
fn a_fitted_paste_reaches_the_peer() {
    use std::cell::RefCell;
    type Q = Rc<RefCell<Vec<Vec<u8>>>>;
    let host = editor("<p>abc</p>", 2, 2);
    let to_guest: Q = Rc::default();
    let sink = to_guest.clone();
    let snap = host
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    let guest = create_editor();
    guest.start_collaboration_guest(&snap, |_| {}).unwrap();
    assert!(host.replace_selection_with_html(LIST));
    assert!(host.collab_outbound_stall().is_none());
    for delta in to_guest.borrow_mut().drain(..) {
        assert!(guest.collab_receive(&delta));
    }
    let want = "<p>a<strong>A</strong></p><ul><li><p>Bbc</p></li></ul>";
    assert_eq!(node_to_html(&host.doc()), want);
    assert_eq!(node_to_html(&guest.doc()), want);

    assert!(host.replace_selection_with_html(TASKS));
    assert!(
        host.collab_outbound_stall().is_some(),
        "a task list is outside the collaboration scope"
    );
}

/// Over a cell selection the selected cells are cleared and the content goes
/// into the top-left one: the grid is never touched, and it is one undo step.
#[test]
fn a_paste_over_a_cell_selection_goes_into_its_top_left_cell() {
    let table = |a: &str, b: &str, d: &str, e: &str| {
        format!(
            "<table><tr><td>{a}</td><td>{b}</td><td><p>c</p></td></tr>\
             <tr><td>{d}</td><td>{e}</td><td><p>f</p></td></tr></table><p>z</p>"
        )
    };
    let start = table("<p>a</p>", "<p>b</p>", "<p>d</p>", "<p>e</p>");
    let over = |anchor: usize, head: usize, html: &str| {
        let dst = create_editor();
        dst.load_html(&start);
        dst.set_selection(Selection::cell(Pos(anchor), Pos(head)));
        assert!(dst.replace_selection_with_html(html));
        let out = node_to_html(&dst.doc());
        assert!(dst.command("undo"));
        assert_eq!(node_to_html(&dst.doc()), start, "one undo");
        assert!(!dst.command("undo"));
        out
    };
    // Cells a and b (the positions before each).
    assert_eq!(
        over(2, 7, "<p>P</p>"),
        table("<p>P</p>", "<p></p>", "<p>d</p>", "<p>e</p>")
    );
    assert_eq!(
        over(7, 2, LIST),
        table(LIST, "<p></p>", "<p>d</p>", "<p>e</p>")
    );
    assert_eq!(
        over(2, 7, "<hr>"),
        table("<hr>", "<p></p>", "<p>d</p>", "<p>e</p>")
    );
    // Corners a and e: all four cells.
    assert_eq!(
        over(2, 24, "<p>P</p>"),
        table("<p>P</p>", "<p></p>", "<p></p>", "<p></p>")
    );
    // Cells that are already empty: nothing to clear, the content still lands.
    let dst = create_editor();
    dst.load_html("<table><tr><td><p></p></td><td><p></p></td></tr></table>");
    dst.set_selection(Selection::cell(Pos(2), Pos(6)));
    assert!(dst.replace_selection_with_text("T"));
    assert_eq!(
        node_to_html(&dst.doc()),
        "<table><tr><td><p>T</p></td><td><p></p></td></tr></table>"
    );
}

/// Lists of the same family join: a bullet or ordered list pasted in either
/// becomes sibling items. A task list and a plain list do not (their items
/// are different nodes), so one pasted in the other nests, as it does in
/// ProseMirror.
#[test]
fn a_list_of_another_family_nests() {
    assert_eq!(
        paste(
            "<ul><li><p></p></li></ul>",
            3,
            3,
            "<ol><li>A</li><li>B</li></ol>"
        ),
        "<ul><li><p>A</p></li><li><p>B</p></li></ul>"
    );
    assert_eq!(
        paste("<ul><li><p>xy</p></li></ul>", 4, 4, TASKS),
        "<ul><li><p>xA</p><ul data-type=\"taskList\">\
         <li data-type=\"taskItem\" data-checked=\"false\"><p>By</p></li></ul></li></ul>"
    );
    let want = TASKS.replace(">     <", "><");
    assert_eq!(
        paste("<ul><li><p></p></li></ul>", 3, 3, TASKS),
        format!("<ul><li>{want}</li></ul>")
    );
}

/// A Google Docs nested list, as Docs writes it, keeps its nested items.
#[test]
fn a_google_docs_nested_list_pastes_nested() {
    let docs = "<meta charset='utf-8'><b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\">\
        <ul><li dir=\"ltr\"><p dir=\"ltr\"><span>one</span></p></li>\
        <ul><li dir=\"ltr\"><p dir=\"ltr\"><span>nested</span></p></li></ul>\
        <li dir=\"ltr\"><p dir=\"ltr\"><span>two</span></p></li></ul></b>";
    assert_eq!(
        paste("<p></p>", 1, 1, docs),
        "<ul><li><p>one</p><ul><li><p>nested</p></li></ul></li><li><p>two</p></li></ul>"
    );
}
