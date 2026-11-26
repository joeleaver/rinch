//! A `<br>` that is the last thing in a block is the browser's placeholder
//! and makes no line (#3096). The writer therefore writes a textblock that
//! ends in a hard break with one more `<br>`, as a browser's editor does.
//!
//! Chrome's line counts, measured by rendering each shape with no margins:
//! `<p><br></p>`, `<div><br></div>`, `<ul><li><br></li></ul>`,
//! `<h2><br></h2>`, `<table><tr><td><br></td></tr></table>`, `<p>a<br></p>`
//! and `<p><span>a<br></span></p>` are each 1 line; `<p>a<br><br></p>` and
//! `<p><br><br></p>` are 2; `<p>a</p><p><br></p><p>b</p>` is 3.
use rinch_editor_core::model::Fragment;
use rinch_editor_core::serialize::{
    clipboard_slice, node_to_html, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{Node, Schema};

fn read(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_pasted_html(&schema, html).unwrap();
    schema.branch("doc", slice.content.clone()).unwrap()
}

/// The lines the editor draws: one per textblock and one per hard break (a
/// break at the end of a block too: the view's trailing-break placeholder).
fn lines(node: &Node) -> usize {
    let own = usize::from(node.is_textblock() || node.type_name() == "hard_break");
    own + node.content().children().iter().map(lines).sum::<usize>()
}

#[test]
fn a_blocks_last_br_is_a_placeholder() {
    for (html, expected) in [
        ("<p><br></p>", "<p></p>"),
        ("<div><br></div>", "<p></p>"),
        ("<ul><li><br></li></ul>", "<ul><li><p></p></li></ul>"),
        ("<h2><br></h2>", "<h2></h2>"),
        (
            "<table><tr><td><br></td></tr></table>",
            "<table><tr><td><p></p></td></tr></table>",
        ),
        ("<p>a<br></p>", "<p>a</p>"),
        ("<p><span>a<br></span></p>", "<p>a</p>"),
        ("<p><strong>a<br></strong></p>", "<p><strong>a</strong></p>"),
    ] {
        let doc = read(html);
        assert_eq!(node_to_html(&doc), expected, "{html}");
        assert_eq!(lines(&doc), 1, "{html}: Chrome draws one line");
    }
}

#[test]
fn a_br_before_the_placeholder_is_a_line() {
    for (html, expected, n) in [
        ("<p>a<br><br></p>", "<p>a<br><br></p>", 2),
        ("<p><br><br></p>", "<p><br><br></p>", 2),
        ("<p>a</p><p><br></p><p>b</p>", "<p>a</p><p></p><p>b</p>", 3),
    ] {
        let doc = read(html);
        assert_eq!(node_to_html(&doc), expected, "{html}");
        assert_eq!(lines(&doc), n, "{html}");
    }
}

/// A paragraph of `content`: text for a `&str`, a hard break for `None`.
fn para(schema: &Schema, content: &[Option<&str>]) -> Node {
    let nodes = content
        .iter()
        .map(|c| match c {
            Some(t) => schema.text(t).unwrap(),
            None => schema.branch("hard_break", Fragment::empty()).unwrap(),
        })
        .collect();
    schema
        .branch("paragraph", Fragment::from_children(nodes))
        .unwrap()
}

fn doc(schema: &Schema, paras: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(paras))
        .unwrap()
}

/// The writer adds the placeholder after a trailing hard break, so the
/// markup shows the lines the editor shows, anywhere it is pasted.
#[test]
fn the_writer_writes_a_trailing_hard_break_with_its_placeholder() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![
            para(&schema, &[Some("a"), None]),
            para(&schema, &[None]),
            para(&schema, &[Some("a"), None, Some("b")]),
        ],
    );
    assert_eq!(
        node_to_html(&d),
        "<p>a<br><br></p><p><br><br></p><p>a<br>b</p>"
    );
    // load_html's reader reads it back the same.
    let again = slice_from_html(&schema, &node_to_html(&d)).unwrap();
    assert_eq!(schema.branch("doc", again.content).unwrap(), d);
}

/// rinch's own copy of a paragraph ending in a hard break pastes back as
/// that paragraph: the clipboard HTML round-trips.
#[test]
fn a_copied_trailing_hard_break_pastes_back() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![para(&schema, &[Some("a"), None]), para(&schema, &[None])],
    );
    let copied = clipboard_slice(&d, 0, d.content_size()).unwrap();
    let html = slice_to_html(&copied);
    let pasted = slice_from_pasted_html(&schema, &html).unwrap();
    assert_eq!(schema.branch("doc", pasted.content).unwrap(), d, "{html}");
}
