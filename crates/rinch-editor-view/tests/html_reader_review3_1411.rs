//! Round-3 review of PR #1411 (head 26dc14e1): the code paste through an
//! editor. `defect_*` fail at the head; main gives what they assert.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};

const ONE: &str = "<meta charset='utf-8'><div style=\"font-family: Consolas, monospace;white-space: pre;\">\
                   <div><span>count</span><span> </span><span>+=</span><span> </span><span>1</span></div></div>";
const LINES: &str = "<meta charset='utf-8'><div style=\"font-family: monospace;white-space: pre;\"><div>\
                     <span>fn</span><span> main() {</span></div><div><span>    body();</span></div><div>\
                     <span>}</span></div></div>";

fn editor(target: &str, from: usize, to: usize) -> EditorHandle {
    let e = create_editor();
    e.load_html(target);
    e.set_selection(Selection::text(Pos(from), Pos(to)));
    e
}

fn paste(e: &EditorHandle, html: &str, text: &str) -> bool {
    e.paste(&PasteContent {
        html: Some(html.to_string()),
        text: Some(text.to_string()),
    })
}

/// One line copied from VS Code, pasted with the caret at the start of a
/// line: the line stays a paragraph.
#[test]
fn defect_one_vs_code_line_pasted_at_the_start_of_a_line_leaves_it_a_paragraph() {
    let e = editor("<p>abcd</p>", 1, 1);
    assert!(paste(&e, ONE, "count += 1"));
    assert_eq!(node_to_html(&e.doc()), "<p>count += 1abcd</p>");
}

/// A heading selected with Shift+Down (from its start to the start of the
/// next line) and replaced by three lines of code: no heading holding line
/// ends, and the next line's text is not in it.
#[test]
fn defect_code_pasted_over_a_heading_selected_to_the_next_line_is_a_line_each() {
    let e = editor("<h2>ab</h2><p>cd</p>", 1, 5);
    assert!(paste(&e, LINES, "fn main() {\n    body();\n}"));
    assert_eq!(
        node_to_html(&e.doc()),
        "<h2>fn main() {</h2><p>    body();</p><p>}cd</p>"
    );
}
