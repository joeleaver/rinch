//! `setCodeBlock` (any `set_block_type`) clears what the new type cannot hold, as
//! ProseMirror's `setBlockType` does through `clearIncompatible` (#1227): a mark the
//! type does not allow is removed, a `hard_break` going into a code block becomes the
//! line break `"\n"`, and any other inline node the type does not take is deleted. It
//! used to retype the block and keep its content, so a bold word and a hard break made
//! `<pre>a<strong>b</strong>c<br></pre>`, which the starter kit's `code_block`
//! (`text*`, no marks) forbids.

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{BaseCommandsPlugin, EditorState, Pos, Schema, Selection};

fn state(html: &str, selection: Selection) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(schema.clone(), doc, vec![Rc::new(BaseCommandsPlugin)]);
    st.selection = selection;
    st
}

fn run(html: &str, selection: Selection, cmd: &str) -> String {
    let next = state(html, selection)
        .run(cmd)
        .unwrap_or_else(|| panic!("`{cmd}` applies"));
    node_to_html(&next.doc)
}

#[test]
fn a_code_block_takes_no_marks_and_a_hard_break_is_a_line_break() {
    assert_eq!(
        run(
            "<p>a<strong>b</strong>c<br>d</p>",
            Selection::cursor(Pos(1)),
            "setCodeBlock"
        ),
        "<pre>abc\nd</pre>"
    );
}

#[test]
fn an_image_going_into_a_code_block_is_deleted() {
    assert_eq!(
        run(
            r#"<p>ab<img src="a.png" alt="x">c</p>"#,
            Selection::cursor(Pos(1)),
            "setCodeBlock"
        ),
        "<pre>abc</pre>"
    );
}

#[test]
fn every_block_in_the_range_is_cleared_and_the_later_ones_are_found_after_the_first_shrinks() {
    // The first block loses its image (one position), so the second block's position
    // must be mapped through that deletion before it is cleared and retyped.
    assert_eq!(
        run(
            r#"<p>a<img src="a.png" alt="x"><em>b</em></p><p>c<br><em>d</em></p>"#,
            Selection::text(Pos(1), Pos(9)),
            "setCodeBlock"
        ),
        "<pre>ab</pre><pre>c\nd</pre>"
    );
}

#[test]
fn a_retype_that_allows_everything_keeps_everything() {
    assert_eq!(
        run(
            r#"<p>a<strong>b</strong>c<br><img src="a.png" alt="x"></p>"#,
            Selection::cursor(Pos(1)),
            "setHeading2"
        ),
        r#"<h2>a<strong>b</strong>c<br><img alt="x" src="a.png"></h2>"#
    );
}
