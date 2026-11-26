//! The default paste of a Google Docs copy (#3096): one empty line per
//! empty Docs paragraph, not two, and no black colour on every word.
//!
//! Docs writes an empty paragraph as a bare `<br />` between its `<p>`s and
//! `color:#000000` on every run. The clipboard here is synthetic, built after
//! that shape; `text/plain` carries the same lines, as Docs puts it there.
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Node, PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};

const RUN: &str = "font-size:11pt;font-family:Arial,sans-serif;color:#000000;\
                   background-color:transparent;font-weight:400;font-style:normal;\
                   font-variant:normal;text-decoration:none;vertical-align:baseline;\
                   white-space:pre;white-space:pre-wrap;";

fn p(text: &str) -> String {
    format!(
        "<p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\">\
         <span style=\"{RUN}\">{text}</span></p>"
    )
}

/// Three paragraphs, one empty paragraph after the first and two after the
/// second, as Docs copies them.
fn clipboard() -> PasteContent {
    let html = format!(
        "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" \
         id=\"docs-internal-guid-5e6f7a8b-7fff-1c2d-3e4f-5a6b7c8d9e0f\">{}<br />{}<br /><br />{}</b>",
        p("first"),
        p("second"),
        p("third"),
    );
    PasteContent {
        text: Some("first\n\nsecond\n\n\nthird".into()),
        html: Some(html),
    }
}

fn paste_into(target: &str, at: usize) -> EditorHandle {
    let editor = create_editor();
    editor.load_html(target);
    editor.set_selection(Selection::text(Pos(at), Pos(at)));
    assert!(editor.paste(&clipboard()), "the paste was refused");
    editor
}

/// The lines the editor draws: one per textblock and one per hard break (a
/// break at the end of a block too: the view's trailing-break placeholder).
fn lines(node: &Node) -> usize {
    let own = usize::from(node.is_textblock() || node.type_name() == "hard_break");
    own + node.content().children().iter().map(lines).sum::<usize>()
}

#[test]
fn a_docs_paste_on_an_empty_line_has_the_lines_docs_shows() {
    let editor = paste_into("<p></p>", 1);
    let doc = editor.doc();
    assert_eq!(
        node_to_html(&doc),
        "<p>first</p><p></p><p>second</p><p><br></p><p>third</p>"
    );
    // first, 1 empty, second, 2 empty, third: what Chrome shows for the
    // markup, and what the plain text says.
    assert_eq!(lines(&doc), 6);
}

#[test]
fn a_docs_paste_inside_a_line_keeps_the_text_around_it() {
    let editor = paste_into("<p>ab</p>", 2);
    let doc = editor.doc();
    assert_eq!(
        node_to_html(&doc),
        "<p>afirst</p><p></p><p>second</p><p><br></p><p>thirdb</p>"
    );
    assert_eq!(lines(&doc), 6);
}

/// The paste of the same lines as plain text agrees with the HTML one.
#[test]
fn the_plain_text_of_a_docs_copy_has_the_same_lines() {
    let editor = create_editor();
    editor.load_html("<p></p>");
    editor.set_selection(Selection::text(Pos(1), Pos(1)));
    assert!(editor.paste(&PasteContent::text(clipboard().text.unwrap())));
    assert_eq!(lines(&editor.doc()), 6);
}
