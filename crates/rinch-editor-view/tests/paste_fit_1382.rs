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
        copy("<ol start=\"3\"><li><p>ab</p></li><li><p>cd</p></li></ol>", 4, 9),
        "<ol start=\"3\"><li><p>b</p></li><li><p>c</p></li></ol>"
    );
    let tasks = "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>ab</p></li>\
                 <li data-type=\"taskItem\" data-checked=\"false\"><p>cd</p></li></ul>";
    assert_eq!(
        copy(tasks, 4, 9),
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
    assert_eq!(
        paste("<p>abc</p>", 2, 2, "<hr>"),
        "<p>a</p><hr><p>bc</p>"
    );
    assert_eq!(paste("<p>abc</p>", 1, 1, "<hr>"), "<hr><p>abc</p>");
    assert_eq!(paste("<p>abc</p>", 4, 4, "<hr>"), "<p>abc</p><hr>");
    assert_eq!(paste("<p>x</p><p></p>", 4, 4, "<hr>"), "<p>x</p><hr>");
    // Over a selection: the selected text goes, the rest is split.
    assert_eq!(
        paste("<p>abc</p>", 2, 3, "<hr>"),
        "<p>a</p><hr><p>c</p>"
    );
    let table = "<table><tbody><tr><td><p>1</p></td><td><p>2</p></td></tr></tbody></table>";
    let out = paste("<h2>abc</h2>", 2, 2, table);
    assert!(
        out.starts_with("<h2>a</h2><table>") && out.ends_with("</table><h2>bc</h2>"),
        "{out}"
    );
    assert!(out.contains("<p>1</p>") && out.contains("<p>2</p>"), "{out}");
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
    assert_eq!(
        paste("<p>ab</p>", 2, 2, "<h2>Title</h2>"),
        "<p>aTitleb</p>"
    );
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
    let table = |cell: &str| {
        format!("<table><tbody><tr><td>{cell}</td><td><p>z</p></td></tr></tbody></table>")
    };
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
    assert_eq!(paste("<p>x</p><p>abc</p>", 4, 7, LIST), format!("<p>x</p>{LIST}"));
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
