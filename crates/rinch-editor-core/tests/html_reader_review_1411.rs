//! Fixtures from the review of PR #1411 (the HTML reader, #1397).
//!
//! `defect_*` fail at the PR head c4c22a2c and state the behaviour the review
//! asks for. `pin_*` pass at the head and each kill a mutant that the PR's own
//! suite lets through (named in each doc).

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{Fragment, Node, Schema};

fn load(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, html).expect("reads");
    schema.branch("doc", slice.content.clone()).expect("a doc")
}

fn html(src: &str) -> String {
    node_to_html(&load(src))
}

/// The text of every textblock, in order.
fn lines(node: &Node, out: &mut Vec<String>) {
    if node.is_textblock() {
        let mut s = String::new();
        for c in node.content().children() {
            s.push_str(c.text().unwrap_or(""));
        }
        out.push(s);
        return;
    }
    for c in node.content().children() {
        lines(c, out);
    }
}

fn lines_of(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    lines(&load(src), &mut out);
    out
}

// ── Defects ──────────────────────────────────────────────────────────────────

/// Past `MAX_DEPTH` a start tag opens nothing, so a block element stops being
/// a line of its own and the words of neighbouring blocks run together:
/// at the head this reads as `["onetwothreefourfive", "after"]`. Chrome 153
/// (cap 512, not 128) keeps every one of these elements at any depth — measured
/// with 100..2000 `<div>`s: innerText `INNER|PA|PB|LI|TA\tTB|AFTER`, 2 `<p>`,
/// 1 `<li>`, 2 `<td>` each time. main read 200 levels as five lines.
#[test]
fn defect_blocks_past_the_depth_cap_stay_lines_of_their_own() {
    let src = format!(
        "{}<p>one</p><p>two</p><ul><li>three</li></ul><table><tr><td>four</td><td>five</td></tr></table>{}after",
        "<div>".repeat(200),
        "</div>".repeat(200)
    );
    assert_eq!(
        lines_of(&src),
        ["one", "two", "three", "four", "five", "after"]
    );
}

/// The same loss for a document the editor itself wrote: `node_to_html` of a
/// valid document nested deeper than the cap does not read back
/// (`load_html(node_to_html(doc))`, copy and paste inside one editor).
#[test]
fn defect_a_document_nested_deeper_than_the_cap_reads_back_the_same() {
    let schema = Schema::starter_kit();
    let para = |t: &str| {
        schema
            .branch("paragraph", Fragment::from_node(schema.text(t).unwrap()))
            .unwrap()
    };
    let mut inner = schema
        .branch(
            "blockquote",
            Fragment::from_children(vec![para("a"), para("b")]),
        )
        .unwrap();
    for _ in 0..140 {
        inner = schema
            .branch("blockquote", Fragment::from_node(inner))
            .unwrap();
    }
    let doc = schema.branch("doc", Fragment::from_node(inner)).unwrap();
    let written = node_to_html(&doc);
    assert_eq!(lines_of(&written), ["a", "b"]);
    assert_eq!(html(&written), written);
}

/// `<img src>` / `<img src="">` makes an image whose `src` is empty; the writer
/// writes `<img>` for it, which reads as nothing: not a fixed point (the PR's
/// "reading back the HTML written from it gives the same document"). Same on
/// main; an empty `src` should be no image, as a missing one is.
#[test]
fn defect_an_image_with_an_empty_src_reads_back_the_same() {
    let once = html("<p>a<img src>b<img src=\"\">c</p>");
    assert_eq!(html(&once), once);
}

// ── Pins for surviving mutants (pass at the head) ────────────────────────────

/// Kills T4 (`if false && closes_p(tag)`: no start tag ends a `<p>`), which the
/// PR reports as surviving: with it the `<div>` and the text after it are read
/// inside the centred `<p>` and take its alignment.
#[test]
fn pin_a_block_ends_the_p_before_it_so_what_follows_is_not_in_it() {
    assert_eq!(
        html("<p style=\"text-align:center\">Aa<div>Bb</div>Cc"),
        "<p style=\"text-align:center\">Aa</p><p>Bb</p><p>Cc</p>"
    );
}

/// Kills review mutant A (an `</h3>` no longer closes an open `<h1>`).
/// Chrome 153: `Aa` is the heading, `Bb` a line after it.
#[test]
fn pin_any_heading_end_tag_ends_the_open_heading() {
    assert_eq!(html("<h1>Aa</h3>Bb"), "<h1>Aa</h1><p>Bb</p>");
}

/// Kills review mutant B (`"ul" | "ol" => tag == "li"`: an inline end tag
/// reaches across a list and closes it).
#[test]
fn pin_an_inline_end_tag_does_not_close_the_list_it_is_in() {
    let doc = load("<b>x<ul>Aa</b>Bb<li>Cc</li></ul>");
    let kinds: Vec<&str> = doc
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    assert_eq!(kinds, ["paragraph", "bullet_list"]);
    assert_eq!(doc.content().children()[1].content().children().len(), 2);
}

/// Kills review mutant C (raw text ends at any end tag that *starts with* the
/// element's name).
#[test]
fn pin_raw_text_ends_at_its_own_end_tag_only() {
    assert_eq!(html("<script>Aa</scriptx>Bb</script>Cc"), "<p>Cc</p>");
    assert_eq!(html("<style>Aa</styles>Bb</style>Cc"), "<p>Cc</p>");
}

/// Kills review mutant Q (`svg` taken out of `is_dropped`): nothing in the
/// PR's suite reads an `<svg>` that holds text.
#[test]
fn pin_svg_and_math_are_not_read() {
    assert_eq!(
        html("a<svg><title>icon</title><text>label</text></svg>b<math><mi>x</mi></math>c"),
        "<p>abc</p>"
    );
}

/// Kills review mutant R (bare `<li>` runs no longer split where task items
/// and plain items meet).
#[test]
fn pin_bare_plain_items_and_bare_task_items_are_two_lists() {
    let doc = load("<li>Cc</li><li data-type=\"taskItem\" data-checked=\"true\">Dd</li>");
    let kinds: Vec<&str> = doc
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    assert_eq!(kinds, ["bullet_list", "task_list"]);
}

/// Kills review mutant V (a non-breaking space between two inline elements
/// read as a plain space).
#[test]
fn pin_a_non_breaking_space_between_inline_elements_is_kept() {
    assert_eq!(lines_of("<b>a</b>&nbsp;<i>b</i>"), ["a\u{a0}b"]);
}

/// Kills review mutant K (text before `</body>` trimmed whether or not a line
/// end follows it): a trailing space with no line end is content.
#[test]
fn pin_only_a_line_end_before_the_body_end_tag_is_layout() {
    assert_eq!(lines_of("<body><b>a</b> b </body>"), ["a b "]);
    assert_eq!(lines_of("<body><b>a</b> b \n</body>"), ["a b"]);
}
