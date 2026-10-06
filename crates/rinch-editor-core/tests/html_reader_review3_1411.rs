//! Fixtures from the round-3 review of PR #1411 (head 26dc14e1).
//!
//! `defect_*` fail at 26dc14e1 and state what main (`ee5123ba`) does with the
//! same clipboard; `pin_*` pass.

use rinch_editor_core::serialize::{html_reader_steps, node_to_html, slice_from_html};
use rinch_editor_core::transform::Transform;
use rinch_editor_core::{Node, Schema};

fn load(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, html).expect("reads");
    schema.branch("doc", slice.content.clone()).expect("a doc")
}

fn fit(target: &str, from: usize, to: usize, html: &str) -> String {
    let schema = Schema::starter_kit();
    let mut tf = Transform::new(&schema, load(target));
    let slice = slice_from_html(&schema, html).unwrap();
    tf.replace_range(from, to, slice).unwrap();
    assert_eq!(tf.steps().len(), 1, "one step");
    node_to_html(&tf.doc)
}

/// One line copied from VS Code (the sample in `html_reader_clipboard_samples.rs`).
const VS_CODE_LINE: &str = "<meta charset='utf-8'><div style=\"color: #cccccc;background-color: #1f1f1f;\
font-family: Consolas, monospace;font-size: 14px;white-space: pre;\"><div><span style=\"color: \
#9cdcfe;\">count</span><span style=\"color: #cccccc;\"> </span><span style=\"color: #d4d4d4;\">+=</span><span \
style=\"color: #cccccc;\"> </span><span style=\"color: #b5cea8;\">1</span></div></div>";

/// Three lines copied from VS Code.
const VS_CODE: &str = "<meta charset='utf-8'><div style=\"font-family: Consolas;white-space: pre;\"><div>\
<span>fn main() {</span></div><div><span>    let</span></div><div><span>}</span></div></div>";

/// One line copied from VS Code and pasted at the start of a line that has
/// text makes the whole line a code block, the line's own text included.
/// Round 2's fix unfolds a code block only when it holds a line end, so the
/// one-line copy still takes the fit that replaces the textblock. Main reads
/// the same clipboard as text and gives the paragraph asserted here. One
/// position to the right the head gives the paragraph too.
#[test]
fn defect_one_line_of_code_at_the_start_of_a_line_does_not_make_the_line_code() {
    let img = "<img src=\"https://e.x/i.png\">";
    for (target, at, want) in [
        ("<p>abcd</p>", 1, "<p>count += 1abcd</p>".to_string()),
        (
            "<ul><li><p>abcd</p></li></ul>",
            3,
            "<ul><li><p>count += 1abcd</p></li></ul>".to_string(),
        ),
        (
            "<blockquote><p>ab</p></blockquote>",
            2,
            "<blockquote><p>count += 1ab</p></blockquote>".to_string(),
        ),
        (
            "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr></table>",
            4,
            "<table><tr><td><p>count += 1ab</p></td><td><p>cd</p></td></tr></table>".to_string(),
        ),
        (
            &format!("<p>{img}</p>"),
            1,
            format!("<p>count += 1{img}</p>"),
        ),
    ] {
        assert_eq!(fit(target, at, at, VS_CODE_LINE), want, "{target} at {at}");
    }
    // Control, passes at the head: the same paste one position in.
    assert_eq!(
        fit("<p>abcd</p>", 2, 2, VS_CODE_LINE),
        "<p>acount += 1bcd</p>"
    );
}

/// Code pasted over a selection that starts at a line's start and ends inside
/// a later line: "the range covers its textblock" is tested with `to >=`, so
/// the code block lands whole and the text kept from the later line joins the
/// code. In a heading the code is one heading holding line ends. Main gives a
/// line each, with the kept text after the last.
#[test]
fn defect_code_over_a_selection_ending_in_a_later_line_keeps_that_line_out_of_the_code() {
    for (target, from, to, want) in [
        (
            "<p>ab</p><p>cd</p>",
            1,
            6,
            "<p>fn main() {</p><p>    let</p><p>}d</p>",
        ),
        (
            "<p>ab</p><p>cd</p>",
            1,
            5,
            "<p>fn main() {</p><p>    let</p><p>}cd</p>",
        ),
        (
            "<p></p><p>cd</p>",
            1,
            4,
            "<p>fn main() {</p><p>    let</p><p>}d</p>",
        ),
        (
            "<h2>ab</h2><p>cd</p>",
            1,
            6,
            "<h2>fn main() {</h2><p>    let</p><p>}d</p>",
        ),
        (
            "<ul><li><p>ab</p></li><li><p>cd</p></li></ul>",
            3,
            10,
            "<ul><li><p>fn main() {</p><p>    let</p><p>}d</p></li></ul>",
        ),
    ] {
        assert_eq!(
            fit(target, from, to, VS_CODE),
            want,
            "{target} {from}..{to}"
        );
    }
}

/// No textblock that is not code holds a line end after a code paste,
/// wherever the range is.
#[test]
fn defect_no_line_that_is_not_code_holds_a_line_end_after_a_code_paste() {
    fn check(node: &Node, bad: &mut Vec<String>) {
        if node.is_textblock() && !node.node_type().spec().code {
            let text: String = node
                .content()
                .children()
                .iter()
                .filter_map(|c| c.text())
                .collect();
            if text.contains('\n') {
                bad.push(text);
            }
        }
        for c in node.content().children() {
            check(c, bad);
        }
    }
    let schema = Schema::starter_kit();
    let mut bad = Vec::new();
    for target in [
        "<h2>ab</h2><p>cd</p>",
        "<p>ab</p><h2>cd</h2>",
        "<h1>ab</h1><ul><li><p>cd</p></li></ul>",
    ] {
        let doc = load(target);
        let size = doc.content().size();
        for from in 0..=size {
            for to in from..=size {
                let mut tf = Transform::new(&schema, doc.clone());
                let slice = slice_from_html(&schema, VS_CODE).unwrap();
                if tf.replace_range(from, to, slice).is_ok() {
                    let mut found = Vec::new();
                    check(&tf.doc, &mut found);
                    if !found.is_empty() {
                        bad.push(format!("{target} {from}..{to}: {}", node_to_html(&tf.doc)));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The cases the round-2 fix was written for still hold (controls for the
/// two defects above; pass at the head).
#[test]
fn pin_code_lands_whole_only_where_nothing_of_a_line_is_kept() {
    let code = "<pre>fn main() {\n    let\n}</pre>";
    assert_eq!(fit("<p></p>", 1, 1, VS_CODE), code);
    assert_eq!(fit("<p>abcd</p>", 1, 5, VS_CODE), code);
    assert_eq!(
        fit("<p>abcd</p>", 1, 1, VS_CODE),
        "<p>fn main() {</p><p>    let</p><p>}abcd</p>"
    );
    assert_eq!(fit("<p></p>", 1, 1, VS_CODE_LINE), "<pre>count += 1</pre>");
}

/// A tag far from the element it closes costs a fixed number of steps: every
/// index path (an end tag, an implied `<p>`/`<li>`/`<dd>` end, a cell, a row,
/// a row group, a heading, foreign content) at twice the tags is twice the
/// steps, with 250 elements left open above.
#[test]
fn pin_every_index_path_is_linear_in_the_tags() {
    let schema = Schema::starter_kit();
    let open = |inner: &str| format!("{inner}{}", "<i>".repeat(250));
    for (opener, unit) in [
        (open(""), "<p>x</p>"),
        (open("<ul><li>"), "<li>x"),
        (open("<table><tr><td>"), "<td>x"),
        (open("<table><tr><td>"), "<tr><td>x"),
        (open("<table><tr><td>"), "<tbody><tr><td>x"),
        (open(""), "x</b>"),
        (open(""), "x</h3>"),
        (open(""), "<h2>x"),
        (open(""), "<svg><g><p>x"),
        (open("<dl><dt>"), "<dd>x<dt>y"),
    ] {
        let steps = |n: usize| {
            let src = format!("{opener}{}", unit.repeat(n));
            let before = html_reader_steps();
            slice_from_html(&schema, &src).unwrap();
            html_reader_steps() - before
        };
        let (small, large) = (steps(500), steps(1000));
        assert!(
            large <= small * 2 + small / 8,
            "{unit}: {small} steps for 500, {large} for 1000"
        );
    }
}

/// Bare table parts whose spans ask for more padding than eight cells a cell
/// written are left as written, and what they cost stays in proportion.
#[test]
fn pin_padding_stays_in_proportion_under_a_covering_rowspan() {
    let schema = Schema::starter_kit();
    let steps = |n: usize| {
        let src = format!(
            "<td colspan=1000 rowspan={n}>a</td>{}",
            "<tr><td>b</td></tr>".repeat(n)
        );
        let slice = slice_from_html(&schema, &src).unwrap();
        slice.content.size()
    };
    let (small, large) = (steps(500), steps(1000));
    assert!(large <= small * 2 + 64, "{small} then {large}");
}
