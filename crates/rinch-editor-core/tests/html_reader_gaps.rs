//! The HTML reader against Chrome 157 on the shapes of #1407, #1462, #1446,
//! #1445 and #1417.
//!
//! Every expectation is what Chrome 157 shows for the same markup set as a
//! `<div>`'s `innerHTML`: its `innerText` lines, and each text node's
//! computed `font-weight` (600 and up is bold), `font-style`, the
//! `text-decoration-line` of it and its ancestors, `vertical-align`, `color`,
//! `background-color` and the `href` of the `<a>` around it.

use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{html_reader_steps, node_to_html, slice_from_html};

fn html(src: &str) -> String {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, src).expect("reads");
    slice.content.children().iter().map(node_to_html).collect()
}

/// Each textblock's text runs, `text[mark,mark]`, the marks sorted by name
/// (the order `Mark::add_to_set` keeps them in): a run with several marks
/// reads as its set, whatever tags the writer nests them as.
fn runs(src: &str) -> String {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, src).expect("reads");
    let mut out = String::new();
    fn walk(n: &rinch_editor_core::Node, out: &mut String) {
        if n.is_textblock() {
            for c in n.content().children() {
                let mut marks: Vec<&str> = c.marks().iter().map(|m| m.type_name()).collect();
                marks.sort();
                out.push_str(&format!("{}[{}]", c.text().unwrap_or("?"), marks.join(",")));
            }
            out.push('|');
        } else {
            for c in n.content().children() {
                walk(c, out);
            }
        }
    }
    for n in slice.content.children() {
        walk(n, &mut out);
    }
    out
}

fn steps(src: &str) -> u64 {
    let schema = Schema::starter_kit();
    let before = html_reader_steps();
    slice_from_html(&schema, src).expect("reads");
    html_reader_steps() - before
}

// ── #1407: character formatting said as CSS on a <span> ─────────────────────

/// Google Docs writes bold, italic, underline and strikethrough as CSS on a
/// `<span>`; Chrome shows each.
#[test]
fn a_spans_style_gives_the_marks_it_shows() {
    assert_eq!(
        html(
            "<p><span style=\"font-weight:700\">bold</span> \
             <span style=\"font-style:italic\">it</span> \
             <span style=\"text-decoration:underline\">u</span> \
             <span style=\"text-decoration-line:line-through\">s</span> \
             <span style=\"vertical-align:sub\">lo</span> \
             <span style=\"vertical-align:super\">hi</span></p>"
        ),
        "<p><strong>bold</strong> <em>it</em> <u>u</u> <s>s</s> <sub>lo</sub> <sup>hi</sup></p>"
    );
}

/// A weight below 600 or `font-style: normal` takes the mark off, as it does
/// in Chrome; `bolder` and 600 are bold, 500 is not.
#[test]
fn a_spans_style_can_take_bold_and_italic_off() {
    assert_eq!(
        html(
            "<p><b>a<span style=\"font-weight:400\">n</span></b>\
             <i>c<span style=\"font-style:normal\">d</span></i>\
             <span style=\"font-weight:bolder\">e</span>\
             <span style=\"font-weight:600\">f</span>\
             <span style=\"font-weight:500\">g</span></p>"
        ),
        "<p><strong>a</strong>n<em>c</em>d<strong>ef</strong>g</p>"
    );
}

/// What Google Docs copies, and what Chrome's own copy wraps an inline
/// selection in: the `text-decoration-*` longhands that are not the line
/// say nothing, and `font-weight: 400` at the top is no mark.
#[test]
fn clipboard_spans_read_as_they_show() {
    assert_eq!(
        runs(
            "<p><span style=\"font-size:11pt;font-family:Arial,sans-serif;\
             background-color:transparent;font-weight:700;font-style:italic;\
             text-decoration:underline;white-space:pre-wrap;\">biu</span></p>"
        ),
        "biu[bold,italic,underline]|"
    );
    assert_eq!(
        html(
            "<span style=\"font-style: normal; font-weight: 400; \
             text-decoration-thickness: initial; text-decoration-style: initial; \
             text-decoration-color: initial;\">plain</span>"
        ),
        "<p>plain</p>"
    );
}

// ── #1462: a <span style> around a block ────────────────────────────────────

/// Chrome shows each of these in red (and the background as a highlight).
#[test]
fn a_styled_span_around_a_block_carries_its_colour() {
    let red = |t: &str| format!("<p><span style=\"color:red\">{t}</span></p>");
    assert_eq!(html("<span style=\"color:red\"><p>x</p></span>"), red("x"));
    assert_eq!(
        html("<span style=\"color:red\">a<div>x</div>b</span>"),
        [red("a"), red("x"), red("b")].concat()
    );
    assert_eq!(
        html("<span style=\"color:red\">a<div></div>b</span>"),
        [red("a"), red("b")].concat()
    );
    assert_eq!(
        html("<span style=\"color:red\">a</p>b</span>"),
        [red("a"), red("b")].concat()
    );
    assert_eq!(
        html("<span style=\"background-color:yellow\">a</p>b</span>"),
        "<p><mark style=\"background-color:yellow\">a</mark></p>\
         <p><mark style=\"background-color:yellow\">b</mark></p>"
    );
    assert_eq!(
        html("<span style=\"font-weight:700\"><p>x</p></span>"),
        "<p><strong>x</strong></p>"
    );
}

// ── #1446: inline content written directly in a list ────────────────────────

/// Chrome lays a run of a list's children that are not items out as lines
/// of their own; an inline element holding a block in that run ends no line.
#[test]
fn a_lists_loose_content_is_split_into_items_where_its_lines_end() {
    assert_eq!(
        html("<ul>a<b>b<p>c"),
        "<ul><li><p>a<strong>b</strong></p></li><li><p><strong>c</strong></p></li></ul>"
    );
    // The `<li>` is inside the `<span>`: a list of its own, in an item of
    // its own, so `c` is a line of its own as in Chrome, one level down.
    assert_eq!(
        html("<ul>a<span>b<li></div>c"),
        "<ul><li><p>ab</p></li><li><ul><li><p>c</p></li></ul></li></ul>"
    );
    assert_eq!(
        html("<ul>a</blockquote>b<b>c<p>d"),
        "<ul><li><p>ab<strong>c</strong></p></li><li><p><strong>d</strong></p></li></ul>"
    );
}

// ── #1445: formatting elements reopened after a block's end ─────────────────

#[test]
fn a_formatting_element_closed_by_a_blocks_end_is_reopened() {
    // a, b, c bold.
    assert_eq!(
        html("<p><b>a</p>b<p>c</p>"),
        "<p><strong>a</strong></p><p><strong>b</strong></p><p><strong>c</strong></p>"
    );
    // b, c italic.
    assert_eq!(html("<p>a<i>b</p>c"), "<p>a<em>b</em></p><p><em>c</em></p>");
    // a bold italic, b italic.
    assert_eq!(runs("<b><i>a</b>b"), "a[bold,italic]b[italic]|");
    // Reopened before a start tag too: b bold italic.
    assert_eq!(runs("<p><b>a</p><i>b</i>"), "a[bold]|b[bold,italic]|");
    // a bold; b bold italic; c italic.
    assert_eq!(
        runs("<b>a<div><i>b</b></div>c"),
        "a[bold]|b[bold,italic]|c[italic]|"
    );
    // Lines a, bcd, ef: c and f plain.
    assert_eq!(
        html("<b>a<div>b</b>c<b>d</div>e</b>f"),
        "<p><strong>a</strong></p><p><strong>b</strong>c<strong>d</strong></p>\
         <p><strong>e</strong>f</p>"
    );
}

/// A second `<a>` ends the first, across a block as anywhere: `d` and `e`
/// are no link.
#[test]
fn a_second_link_ends_the_first() {
    assert_eq!(
        html("<a href=\"x\">a<div>b<a href=\"y\">c</a>d</a>e</div>"),
        "<p><a href=\"x\">a</a></p><p><a href=\"x\">b</a><a href=\"y\">c</a>de</p>"
    );
    assert_eq!(
        html("<p><a href=\"x\">a<a href=\"y\">b</a>c</p>"),
        "<p><a href=\"x\">a</a><a href=\"y\">b</a>c</p>"
    );
}

/// A well-formed document reopens nothing, a table cell does not take a
/// formatting element opened outside its table, and at most three alike are
/// reopened (the HTML parser's Noah's Ark clause).
#[test]
fn reopening_is_bounded_as_a_browsers_is() {
    assert_eq!(
        html("<p><b>a</b></p><p>b</p>"),
        "<p><strong>a</strong></p><p>b</p>"
    );
    // x bold; y and z not.
    assert_eq!(
        html("<table><tr><td><b>x</td><td>y</td></tr></table>z"),
        "<table><tr><td><p><strong>x</strong></p></td><td><p>y</p></td></tr></table><p>z</p>"
    );
    // The cell's own formatting ends with the cell (its marker is cleared):
    // z plain.
    assert_eq!(
        html("<table><tr><td><b>x</td></tr></table>z"),
        "<table><tr><td><p><strong>x</strong></p></td></tr></table><p>z</p>"
    );
    // An end tag with its element closed already ends its reopening: c plain.
    assert_eq!(
        html("<p><b>a</p></b>c"),
        "<p><strong>a</strong></p><p>c</p>"
    );
    // Five `<b>`s: three come back. They are one mark either way, so the
    // count shows as the steps a text after them takes, not in the output.
    assert_eq!(
        html("<p><b><b><b><b><b>a</p>b"),
        "<p><strong>a</strong></p><p><strong>b</strong></p>"
    );
}

// ── #1417: marks around blocks are applied once ─────────────────────────────

/// `<b><div>` nested `levels` deep around a table of `rows` rows.
fn marked_table(levels: usize, rows: usize) -> String {
    let mut s = "<b><div>".repeat(levels);
    s.push_str("<table>");
    for _ in 0..rows {
        s.push_str("<tr><td>a</td><td>b</td></tr>");
    }
    s
}

/// The mark walk is counted, and it is one walk however many levels of
/// mark-and-block hold the table: 90 levels cost no more per row than one.
#[test]
fn marks_around_nested_blocks_are_put_on_once() {
    let per_row = |levels: usize| {
        (steps(&marked_table(levels, 400)) - steps(&marked_table(levels, 200))) as f64 / 200.0
    };
    let one = per_row(1);
    let ninety = per_row(90);
    assert!(one > 0.0);
    assert!(
        ninety <= one * 1.5,
        "a row costs {one} steps under one level and {ninety} under 90"
    );
    assert_eq!(
        runs("<b><div><i><div><p>x</p></div></i></div></b>"),
        "x[bold,italic]|"
    );
}
