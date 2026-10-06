//! Fixtures from the round-2 review of PR #1411 (head b5ef069b): the code
//! paste through an editor — undo, collaboration, and where it lands.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};

/// What VS Code copies for three lines.
const LINES: &str = "<meta charset='utf-8'><div style=\"font-family: monospace;white-space: pre;\"><div>\
                     <span>fn</span><span> main() {</span></div><div><span>    body();</span></div><div>\
                     <span>}</span></div></div>";

fn editor(target: &str, at: usize) -> EditorHandle {
    let e = create_editor();
    e.load_html(target);
    e.set_selection(Selection::text(Pos(at), Pos(at)));
    e
}

fn paste(e: &EditorHandle, html: &str) -> bool {
    e.paste(&PasteContent {
        html: Some(html.to_string()),
        text: Some("fn main() {\n    body();\n}".to_string()),
    })
}

/// The lines are one transaction: one undo takes them all back, one redo
/// puts them all in.
#[test]
fn pin_code_lines_pasted_in_a_line_of_text_are_one_undo_step() {
    let e = editor("<p>abcd</p>", 3);
    assert!(paste(&e, LINES));
    let after = "<p>abfn main() {</p><p>    body();</p><p>}cd</p>";
    assert_eq!(node_to_html(&e.doc()), after);
    assert!(e.command("undo"));
    assert_eq!(node_to_html(&e.doc()), "<p>abcd</p>");
    assert!(!e.can_run("undo"), "one step");
    assert!(e.command("redo"));
    assert_eq!(node_to_html(&e.doc()), after);
}

/// In a code block the lines stay code, in the block.
#[test]
fn pin_code_lines_pasted_in_code_stay_code() {
    let e = editor("<pre>abcd</pre>", 3);
    assert!(paste(&e, LINES));
    assert_eq!(
        node_to_html(&e.doc()),
        "<pre>abfn main() {\n    body();\n}cd</pre>"
    );
}

/// The paragraphs reach a peer: nothing outside collaboration's scope is
/// made, and both ends hold the same document.
#[cfg(feature = "collaboration")]
#[test]
fn pin_code_lines_pasted_in_a_line_of_text_reach_the_peer() {
    use std::cell::RefCell;
    use std::rc::Rc;
    type Q = Rc<RefCell<Vec<Vec<u8>>>>;
    let host = editor("<p>abcd</p>", 3);
    let to_guest: Q = Rc::default();
    let sink = to_guest.clone();
    let snap = host
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    let guest = create_editor();
    guest.start_collaboration_guest(&snap, |_| {}).unwrap();
    assert!(paste(&host, LINES));
    assert!(host.collab_outbound_stall().is_none());
    for delta in to_guest.borrow_mut().drain(..) {
        assert!(guest.collab_receive(&delta));
    }
    assert_eq!(
        node_to_html(&host.doc()),
        "<p>abfn main() {</p><p>    body();</p><p>}cd</p>"
    );
    assert_eq!(node_to_html(&guest.doc()), node_to_html(&host.doc()));
}

/// A caret at the START of a line that has text: the line becomes the code
/// block and its own text is the end of the last line of code. One position
/// to the right the same paste is a paragraph per line (the PR's rule), and
/// on main (ee5123ba) both positions gave paragraphs, the line's text after
/// the last (`<p>fn main() {</p><p>    body();</p><p>}abcd</p>`): VS Code's
/// HTML read as paragraphs there. The fitting is #1382's (a `<pre>` pasted
/// here did the same on main); this PR is what makes a VS Code paste reach
/// it. Stated as main's result, for the lead to decide.
#[test]
fn defect_code_lines_pasted_at_the_start_of_a_line_do_not_take_its_text_into_code() {
    let e = editor("<p>abcd</p>", 1);
    assert!(paste(&e, LINES));
    assert_eq!(
        node_to_html(&e.doc()),
        "<p>fn main() {</p><p>    body();</p><p>}abcd</p>"
    );
}

/// The padding bound through an editor: 1.3 KB of bare table parts pasted
/// at a caret. At b5ef069b the paste is accepted and the document holds
/// 65,001 cells (and a collaborating editor sends them to every peer); on
/// main (ee5123ba) the same clipboard fell back to its text flavour.
#[test]
fn defect_a_small_paste_of_bare_table_parts_does_not_make_a_huge_table() {
    let html = format!(
        "<td colspan=1000>a</td>{}",
        "<tr><td>b</td></tr>".repeat(65)
    );
    let e = editor("<p>abcd</p>", 3);
    let t = std::time::Instant::now();
    assert!(e.paste(&PasteContent {
        html: Some(html.clone()),
        text: Some("plain".to_string()),
    }));
    let cells = node_to_html(&e.doc()).matches("<td").count();
    eprintln!(
        "{} bytes pasted: {cells} cells in {:?}",
        html.len(),
        t.elapsed()
    );
    assert!(
        cells <= html.len(),
        "{} bytes made {cells} cells",
        html.len()
    );
}
