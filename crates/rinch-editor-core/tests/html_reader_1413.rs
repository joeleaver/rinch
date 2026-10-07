//! The HTML reader against Chrome 153 on three shapes (#1413, #1410, #1415).
//!
//! Every expectation here is what Chrome 153 shows for the same markup set
//! as a `<div>`'s `innerHTML`: where its `innerText` puts a line end, and
//! which text nodes compute to bold or italic. The reader's blank lines are
//! its own (an empty `<p>` is an empty paragraph), so each fixture states
//! lines and marks, not margins.

use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{
    html_reader_max_depth, html_reader_steps, node_to_html, slice_from_html,
};

fn html(src: &str) -> String {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, src).expect("reads");
    slice.content.children().iter().map(node_to_html).collect()
}

fn steps(src: &str) -> u64 {
    let schema = Schema::starter_kit();
    let before = html_reader_steps();
    slice_from_html(&schema, src).expect("reads");
    html_reader_steps() - before
}

/// `make(n)` costs steps in proportion to `n`: each doubling of `n` from
/// `from` adds at most twice the steps the doubling before it added (and a
/// little). Increments, because the first elements open are cheaper than the
/// ones past the depth the reader scans.
fn assert_linear(what: &str, from: usize, make: impl Fn(usize) -> String) {
    let mut n = from;
    let mut last = steps(&make(n));
    let mut added = steps(&make(2 * n)) - last;
    assert!(last > 0 && added > 0, "{what}: the counter counts");
    last += added;
    n *= 2;
    for _ in 0..4 {
        n *= 2;
        let now = steps(&make(n));
        assert!(
            (now - last) as f64 <= added as f64 * 2.2,
            "{what}: {added} steps added, then {} at n = {n}",
            now - last
        );
        added = now - last;
        last = now;
    }
}

// ── #1413: an empty block element ends the line before it ───────────────────

/// A block-level element with nothing in it makes no block and still ends
/// the line: Chrome shows `a` and `b` on two lines for each of these. They
/// read as one paragraph `ab` before #1413.
#[test]
fn an_empty_block_element_ends_the_line_before_it() {
    for empty in [
        "<div></div>",
        "<div><span></span></div>",
        "<div><div></div></div>",
        "<section></section>",
        "<address></address>",
        "<form></form>",
        "<center></center>",
        "<dl></dl>",
        "<details></details>",
        "<legend></legend>",
        "<figure></figure>",
        "<div></div><div></div>",
        // An inline element around an empty block.
        "<span><div></div></span>",
        "<b><div></div></b>",
    ] {
        assert_eq!(
            html(&format!("Aa{empty}Bb")),
            "<p>Aa</p><p>Bb</p>",
            "{empty}"
        );
    }
    assert_eq!(
        html("<span>Aa</span><div></div><span>Bb</span>"),
        "<p>Aa</p><p>Bb</p>"
    );
    // The whitespace around it is the source's layout.
    assert_eq!(
        html("<span>Aa</span> <div></div> <span>Bb</span>"),
        "<p>Aa</p><p>Bb</p>"
    );
    // At an edge there is no line to end.
    assert_eq!(html("<div></div>Aa"), "<p>Aa</p>");
    assert_eq!(html("Aa<div></div>"), "<p>Aa</p>");
}

/// Positive control for the fixture above: an empty inline element ends no
/// line, in Chrome and here.
#[test]
fn an_empty_inline_element_ends_no_line() {
    for empty in ["<span></span>", "<b></b>", "<o:p></o:p>", "<x-y></x-y>", ""] {
        assert_eq!(html(&format!("Aa{empty}Bb")), "<p>AaBb</p>", "{empty}");
    }
}

/// A block element that holds only collapsible whitespace is no line either
/// (Chrome: `EbFoo`, `EbBar`, `EbBaz`), where one that holds a non-breaking
/// space is a line of its own (Chrome: `a`, U+00A0, `b`).
#[test]
fn a_block_of_whitespace_is_a_boundary_and_one_of_nbsp_is_a_line() {
    assert_eq!(
        html("EbFoo<div></div>EbBar<div> </div>EbBaz"),
        "<p>EbFoo</p><p>EbBar</p><p>EbBaz</p>"
    );
    assert_eq!(
        html("<div>Aa</div><div> \n </div><div>Bb</div>"),
        "<p>Aa</p><p>Bb</p>"
    );
    assert_eq!(
        html("Aa<div>&nbsp;</div>Bb"),
        "<p>Aa</p><p>\u{a0}</p><p>Bb</p>"
    );
    assert_eq!(html("<div>&nbsp;</div>Bb"), "<p>\u{a0}</p><p>Bb</p>");
    assert_eq!(html("Aa<div>&nbsp;</div>"), "<p>Aa</p><p>\u{a0}</p>");
    assert_eq!(
        html("Aa<div><div>&nbsp;</div></div>Bb"),
        "<p>Aa</p><p>\u{a0}</p><p>Bb</p>"
    );
    // Unchanged: a line that is only a non-breaking space, between blocks.
    assert_eq!(
        html("<div>Aa</div><div>&nbsp;</div><div>Bb</div>"),
        "<p>Aa</p><p>\u{a0}</p><p>Bb</p>"
    );
}

/// The same boundary inside every kind of container the reader builds.
#[test]
fn an_empty_block_ends_the_line_inside_a_container() {
    assert_eq!(html("<div>Aa<div></div>Bb</div>"), "<p>Aa</p><p>Bb</p>");
    assert_eq!(
        html("<blockquote>Aa<div></div>Bb</blockquote>"),
        "<blockquote><p>Aa</p><p>Bb</p></blockquote>"
    );
    assert_eq!(
        html("<ul><li>Aa<div></div>Bb</li></ul>"),
        "<ul><li><p>Aa</p><p>Bb</p></li></ul>"
    );
    assert_eq!(
        html("<table><tr><td>Aa<div></div>Bb</td></tr></table>"),
        "<table><tr><td><p>Aa</p><p>Bb</p></td></tr></table>"
    );
    // A heading around a block is a heading a line (#1397).
    assert_eq!(html("<h1>Aa<div></div>Bb</h1>"), "<h1>Aa</h1><h1>Bb</h1>");
    assert_eq!(
        html("<h1>Aa<div>&nbsp;</div>Bb</h1>"),
        "<h1>Aa</h1><h1>\u{a0}</h1><h1>Bb</h1>"
    );
    assert_eq!(
        html("<b>Aa<div></div>Bb</b>"),
        "<p><strong>Aa</strong></p><p><strong>Bb</strong></p>"
    );
    // Preformatted text had the line end already.
    assert_eq!(html("<pre>Aa<div></div>Bb</pre>"), "<pre>Aa\nBb</pre>");
}

#[test]
fn empty_blocks_cost_steps_in_proportion() {
    assert_linear("empty divs", 500, |n| "x<div></div>".repeat(n));
    assert_linear("nested empty divs", 500, |n| {
        format!("{}{}", "x<div>".repeat(n), "</div>".repeat(n))
    });
}

// ── #1410: a misnested formatting end tag, and a stray </p> ─────────────────

/// `</b>` with a block open between it and its `<b>`: Chrome ends the bold
/// at the end tag and keeps it on what the block already holds (the adoption
/// agency algorithm). Before #1410 the end tag was skipped and the bold ran
/// to the end of the input.
#[test]
fn a_formatting_end_tag_across_a_block_ends_its_mark_there() {
    assert_eq!(
        html("<b>a<div>b</b>c</div>d"),
        "<p><strong>a</strong></p><p><strong>b</strong>c</p><p>d</p>"
    );
    assert_eq!(
        html("<b>a<p>b</b>c</p>d"),
        "<p><strong>a</strong></p><p><strong>b</strong>c</p><p>d</p>"
    );
    assert_eq!(
        html("<strong>a<div>b</strong>c</div>d"),
        "<p><strong>a</strong></p><p><strong>b</strong>c</p><p>d</p>"
    );
    assert_eq!(
        html("<i>a<div>b</i>c</div>d<p>e</p>"),
        "<p><em>a</em></p><p><em>b</em>c</p><p>d</p><p>e</p>"
    );
    assert_eq!(
        html("<a href=\"https://e.x/\">a<div>b</a>c</div>d"),
        "<p><a href=\"https://e.x/\">a</a></p><p><a href=\"https://e.x/\">b</a>c</p><p>d</p>"
    );
    assert_eq!(
        html("<code>a<div>b</code>c</div>d"),
        "<p><code>a</code></p><p><code>b</code>c</p><p>d</p>"
    );
    // The block is a heading, a quote, a list item.
    assert_eq!(
        html("<em>a<h1>b</em>c</h1>d"),
        "<p><em>a</em></p><h1><em>b</em>c</h1><p>d</p>"
    );
    assert_eq!(
        html("<b>a<blockquote>b</b>c</blockquote>d"),
        "<p><strong>a</strong></p><blockquote><p><strong>b</strong>c</p></blockquote><p>d</p>"
    );
    assert_eq!(
        html("<b>a<ul><li>b</b>c</li></ul>d"),
        "<p><strong>a</strong></p><ul><li><p><strong>b</strong>c</p></li></ul><p>d</p>"
    );
    // Nothing follows the end tag inside the block.
    assert_eq!(
        html("<b>a<div>b</b></div>c"),
        "<p><strong>a</strong></p><p><strong>b</strong></p><p>c</p>"
    );
    // Inside a container.
    assert_eq!(
        html("<div><b>a<div>b</b>c</div>d</div>e"),
        "<p><strong>a</strong></p><p><strong>b</strong>c</p><p>d</p><p>e</p>"
    );
}

/// More than one block open, a block closed before the end tag, and two
/// formatting elements: what each piece of text is in Chrome.
#[test]
fn a_formatting_end_tag_across_several_open_elements() {
    // Chrome: a, b, c bold; d, e, f not.
    assert_eq!(
        html("<b>a<div>b<div>c</b>d</div>e</div>f"),
        "<p><strong>a</strong></p><p><strong>b</strong></p><p><strong>c</strong>d</p>\
         <p>e</p><p>f</p>"
    );
    assert_eq!(
        html("<b>a<blockquote>b<p>c</b>d</p>e</blockquote>f"),
        "<p><strong>a</strong></p><blockquote><p><strong>b</strong></p>\
         <p><strong>c</strong>d</p><p>e</p></blockquote><p>f</p>"
    );
    // A block that was closed stays as it was.
    assert_eq!(
        html("<b>a<div>b</div><div>c</b>d</div>e"),
        "<p><strong>a</strong></p><p><strong>b</strong></p><p><strong>c</strong>d</p><p>e</p>"
    );
    // Chrome: a and b bold italic, c italic, d and e plain.
    assert_eq!(
        html("<b><i>a<div>b</b>c</i>d</div>e"),
        "<p><em><strong>a</strong></em></p><p><em><strong>b</strong></em><em>c</em>d</p><p>e</p>"
    );
    // The same with the italic opened inside the block: Chrome reopens it
    // after the `</b>`, so c is italic.
    assert_eq!(
        html("<b>a<div><i>b</b>c</i>d</div>e"),
        "<p><strong>a</strong></p><p><em><strong>b</strong></em><em>c</em>d</p><p>e</p>"
    );
    // Two `<b>`: a and b bold, c bold (the outer one), d and e plain.
    assert_eq!(
        html("<b><b>a<div>b</b>c</b>d</div>e"),
        "<p><strong>a</strong></p><p><strong>bc</strong>d</p><p>e</p>"
    );
    // Twice in a row.
    assert_eq!(
        html("<b>1<p>2</b>3</p><i>4<div>5</i>6</div>7"),
        "<p><strong>1</strong></p><p><strong>2</strong>3</p><p><em>4</em></p>\
         <p><em>5</em>6</p><p>7</p>"
    );
}

/// What is left as it was, because Chrome leaves it: an end tag of an
/// element that is no formatting element is ignored across a block, and no
/// end tag reaches out of a table cell.
#[test]
fn an_end_tag_chrome_ignores_is_still_ignored() {
    // `<mark>`, `<sub>`, `<del>` and `<span>` are not formatting elements.
    assert_eq!(
        html("<mark>a<div>b</mark>c</div>d"),
        "<p><mark>a</mark></p><p><mark>bc</mark></p><p><mark>d</mark></p>"
    );
    assert_eq!(
        html("<sub>a<div>b</sub>c</div>d"),
        "<p><sub>a</sub></p><p><sub>bc</sub></p><p><sub>d</sub></p>"
    );
    // Chrome: every letter bold.
    assert_eq!(
        html("<b>a<table><tr><td>b</b>c</td></tr></table>d"),
        "<p><strong>a</strong></p><table><tr><td><p><strong>bc</strong></p></td></tr></table>\
         <p><strong>d</strong></p>"
    );
    // Well nested: nothing to do. (Chrome: `a`, `b`, `cd`.)
    assert_eq!(
        html("<b>a<div>b</div>c</b>d"),
        "<p><strong>a</strong></p><p><strong>b</strong></p><p><strong>c</strong>d</p>"
    );
}

/// A formatting end tag across a block does not reach out of the part of
/// an `<svg>` that holds HTML: it is skipped, as before and as in Chrome
/// (`a` and `z`, both bold), however many elements are open in between. An
/// HTML element that is only named like such a part is none (Chrome: `y`
/// is not bold).
#[test]
fn a_formatting_end_tag_across_a_block_stays_inside_a_drawing() {
    for depth in [0, 1, 4, 5, 6, html_reader_max_depth() + 40] {
        let src = format!(
            "<b>a<svg><desc><div>{}x</b>y{}</div></desc></svg>z",
            "<span>".repeat(depth),
            "</span>".repeat(depth)
        );
        assert_eq!(html(&src), "<p><strong>az</strong></p>", "{depth}");
    }
    assert_eq!(html("<b>a<desc>x</b>y"), "<p><strong>ax</strong>y</p>");
    assert_eq!(
        html("<b>a<desc><div>x</b>y</div></desc>z"),
        "<p><strong>a</strong></p><p><strong>x</strong>y</p><p>z</p>"
    );
    // With no block open the end tag closes the drawing with its element,
    // as it did: what follows is read. (Chrome reads `y` into the drawing,
    // where it is not shown, and keeps `z` bold.)
    assert_eq!(
        html("<b>a<svg><desc>x</b>y</desc></svg>z"),
        "<p><strong>a</strong>yz</p>"
    );
}

/// An inline element that is not a formatting element, open inside the
/// block when the end tag comes, ends with it (Chrome: `c` is neither bold
/// nor red). A formatting element there goes on.
#[test]
fn an_inline_element_open_in_the_block_ends_with_the_formatting_element() {
    let src = "<span style=\"color: rgb(255, 0, 0)\">";
    let red = "<span style=\"color:rgb(255, 0, 0)\">";
    assert_eq!(
        html(&format!("<b>a<div>{src}b</b>c</span>d</div>e")),
        format!("<p><strong>a</strong></p><p>{red}<strong>b</strong></span>cd</p><p>e</p>")
    );
    assert_eq!(
        html(&format!("<b>a<div>{src}<i>b</b>c</i>d</span>e</div>f")),
        format!(
            "<p><strong>a</strong></p>\
             <p>{red}<em><strong>b</strong></em></span><em>c</em>de</p><p>f</p>"
        )
    );
    assert_eq!(
        html(&format!("<b>a<div><i>{src}b</b>c</span>d</i>e</div>f")),
        format!(
            "<p><strong>a</strong></p>\
             <p>{red}<em><strong>b</strong></em></span><em>cd</em>e</p><p>f</p>"
        )
    );
}

/// The element that was ended leaves nothing behind in the tree: text read
/// before and after the block it was open across is one run, and a heading
/// right after it is that block's sibling.
#[test]
fn an_ended_formatting_element_leaves_the_tree_as_it_would_be_without_it() {
    assert_eq!(
        html("<div>x<b><div>y</b>z</div>w v</div>"),
        "<p>x</p><p><strong>y</strong>z</p><p>w v</p>"
    );
    // The `<h2>` ends the `<h1>` it follows (it would nest in it otherwise).
    assert_eq!(
        html("<h1>t<b>a<div>b</b>c</div><h2>u</h2>"),
        "<h1>t<strong>a</strong></h1><h1><strong>b</strong>c</h1><h2>u</h2>"
    );
    assert_eq!(
        html("<i>x<b>a<div>b</b>c</div>d</i>e"),
        "<p><em>x</em><em><strong>a</strong></em></p><p><em><strong>b</strong></em><em>c</em></p>\
         <p><em>d</em>e</p>"
    );
}

/// An inline element that holds a block ends no line itself: what it holds
/// beside the block is on the line of what is beside the element. Chrome:
/// `xy`, `z`, `vw`. (Each was a line of its own before #1413, which an
/// empty block or a stray `</p>` inside an inline element would have made
/// of every such element.)
#[test]
fn an_inline_element_around_a_block_ends_no_line() {
    assert_eq!(
        html("x<b>y<div>z</div>v</b>w"),
        "<p>x<strong>y</strong></p><p><strong>z</strong></p><p><strong>v</strong>w</p>"
    );
    assert_eq!(
        html("x<span>y<div>z</div>v</span>w"),
        "<p>xy</p><p>z</p><p>vw</p>"
    );
    assert_eq!(
        html("x<a href=\"https://e.x/\">y<div>z</div>v</a>w"),
        "<p>x<a href=\"https://e.x/\">y</a></p><p><a href=\"https://e.x/\">z</a></p>\
         <p><a href=\"https://e.x/\">v</a>w</p>"
    );
    assert_eq!(
        html("x<o:p>y<div>z</div>v</o:p>w"),
        "<p>xy</p><p>z</p><p>vw</p>"
    );
    // Chrome: `ab`, `cd`.
    assert_eq!(
        html("a<b>b<div></div>c</b>d"),
        "<p>a<strong>b</strong></p><p><strong>c</strong>d</p>"
    );
    assert_eq!(
        html("<div>a<b>b</p>c</b>d</div>"),
        "<p>a<strong>b</strong></p><p><strong>c</strong>d</p>"
    );
    // Marks one inside the other, and a block of blocks.
    assert_eq!(
        html("x<b>y<i>z<blockquote><p>q</p><ul><li>r</li></ul></blockquote>v</i></b>w"),
        "<p>x<strong>y</strong><em><strong>z</strong></em></p>\
         <blockquote><p><em><strong>q</strong></em></p>\
         <ul><li><p><em><strong>r</strong></em></p></li></ul></blockquote>\
         <p><em><strong>v</strong></em>w</p>"
    );
    // The inner link is the text's.
    assert_eq!(
        html("<a href=\"https://e.x/1\">y<div>z<a href=\"https://e.x/2\">q</a></div></a>"),
        "<p><a href=\"https://e.x/1\">y</a></p>\
         <p><a href=\"https://e.x/1\">z</a><a href=\"https://e.x/2\">q</a></p>"
    );
}

/// Inline children of a list side by side are one item, as they are one
/// line in a browser (`AaBb`, then `Cc`): a misnested `</b>` there must not
/// make two items of one line.
#[test]
fn inline_children_of_a_list_side_by_side_are_one_item() {
    assert_eq!(
        html("<ul><b>Aa</b>Bb<li>Cc</li></ul>"),
        "<ul><li><p><strong>Aa</strong>Bb</p></li><li><p>Cc</p></li></ul>"
    );
    assert_eq!(
        html("<b>x<ul>Aa</b>Bb<li>Cc</li></ul>"),
        "<p><strong>x</strong></p>\
         <ul><li><p><strong>Aa</strong>Bb</p></li><li><p>Cc</p></li></ul>"
    );
    assert_eq!(
        html("<ul>Aa<span>Bb</span> Cc<li>Dd</li><i>Ee</i>Ff</ul>"),
        "<ul><li><p>AaBb Cc</p></li><li><p>Dd</p></li><li><p><em>Ee</em>Ff</p></li></ul>"
    );
    // Blocks keep an item each.
    assert_eq!(
        html("<ul><div>Aa</div><div>Bb</div></ul>"),
        "<ul><li><p>Aa</p></li><li><p>Bb</p></li></ul>"
    );
}

/// A `</p>` with no `<p>` open is an empty `<p>` in a browser: the line
/// ends there. It is no line of its own here (an empty paragraph would be a
/// blank line, which a browser does not show for it).
#[test]
fn a_stray_p_end_tag_ends_the_line() {
    assert_eq!(
        html("<p>a<h1>b</h1>c</p>d"),
        "<p>a</p><h1>b</h1><p>c</p><p>d</p>"
    );
    assert_eq!(html("a</p>b"), "<p>a</p><p>b</p>");
    assert_eq!(html("a</p></p>b"), "<p>a</p><p>b</p>");
    assert_eq!(html("<div>a</p>b</div>c"), "<p>a</p><p>b</p><p>c</p>");
    // The commonest way to write one: a list or a `<div>` inside a `<p>`.
    assert_eq!(
        html("<p>a<ul><li>b</li></ul>c</p>d"),
        "<p>a</p><ul><li><p>b</p></li></ul><p>c</p><p>d</p>"
    );
    assert_eq!(
        html("<p>a<div>b</div>c</p>d"),
        "<p>a</p><p>b</p><p>c</p><p>d</p>"
    );
    assert_eq!(
        html("<ul><li>a</p>b</li></ul>"),
        "<ul><li><p>a</p><p>b</p></li></ul>"
    );
    assert_eq!(
        html("<table><tr><td>a</p>b</td></tr></table>"),
        "<table><tr><td><p>a</p><p>b</p></td></tr></table>"
    );
    assert_eq!(
        html("<b>x</p>y</b>"),
        "<p><strong>x</strong></p><p><strong>y</strong></p>"
    );
    // A `<p>` outside the cell is not the cell's to close.
    assert_eq!(
        html("<p>a<table><tr><td>b</p>c</td></tr></table>"),
        "<p>a</p><table><tr><td><p>b</p><p>c</p></td></tr></table>"
    );
    // Controls: a `</p>` that has its `<p>` is that paragraph's end.
    assert_eq!(html("<p>a</p>b"), "<p>a</p><p>b</p>");
    assert_eq!(html("<p>a<b>b</p>c"), "<p>a<strong>b</strong></p><p>c</p>");
    // Both shapes of the issue together.
    assert_eq!(
        html("<b>a<p>b</b>c</p></p>d"),
        "<p><strong>a</strong></p><p><strong>b</strong>c</p><p>d</p>"
    );
}

/// With more elements open above the formatting element than a browser
/// walks, the end tag is skipped as before: nothing is lost, and the mark
/// runs on (Chrome 153 shows every letter of this bold too).
#[test]
fn a_formatting_end_tag_under_many_open_elements_is_skipped() {
    let src = format!("<b>a{}x</b>y{}z", "<div>".repeat(10), "</div>".repeat(10));
    assert_eq!(
        html(&src),
        "<p><strong>a</strong></p><p><strong>xy</strong></p><p><strong>z</strong></p>"
    );
}

/// No text is lost and no line fused whatever is misnested, at any depth:
/// the tokens come out in order, one line each where a block separates them.
#[test]
fn misnested_end_tags_lose_no_text_at_any_depth() {
    let limit = html_reader_max_depth();
    for depth in [0, 1, 7, 8, 9, limit - 9, limit - 2, limit, limit + 3, 400] {
        for tag in ["b", "a href=\"https://e.x/\"", "i"] {
            let name = tag.split(' ').next().unwrap();
            let src = format!(
                "{}<{tag}>Aa<div>Bb<div>Cc</{name}>Dd</div>Ee</div>Ff</p>Gg",
                "<div>".repeat(depth)
            );
            let out = html(&src);
            // One `|` where a paragraph ends, and no other tag.
            let mut lines = String::new();
            let mut rest = out.as_str();
            while let Some(at) = rest.find('<') {
                lines.push_str(&rest[..at]);
                let end = at + rest[at..].find('>').expect("a tag ends");
                if &rest[at..=end] == "</p>" {
                    lines.push('|');
                }
                rest = &rest[end + 1..];
            }
            assert_eq!(lines, "Aa|Bb|CcDd|Ee|Ff|Gg|", "{depth} {tag}: {out}");
        }
    }
}

/// The tree stays within the depth limit: a document of misnested end tags
/// at every depth reads on a small stack.
#[test]
fn misnested_end_tags_read_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1536 * 1024)
        .spawn(|| {
            let schema = Schema::starter_kit();
            for src in [
                "<b>x<div>y</b>z".repeat(3000),
                format!("{}<div>x{}", "<b>".repeat(3000), "</b>y".repeat(3000)),
                format!(
                    "{}{}",
                    "<b><i>x<div>".repeat(2000),
                    "</b>y</i>z</div>".repeat(2000)
                ),
                "x</p>".repeat(3000),
            ] {
                slice_from_html(&schema, &src).expect("reads");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn misnested_end_tags_cost_steps_in_proportion() {
    assert_linear("a misnested end tag a block", 250, |n| {
        "<b>x<div>y</b>z</div>".repeat(n)
    });
    assert_linear("misnested, blocks left open", 250, |n| {
        "<b>x<div>y</b>z".repeat(n)
    });
    assert_linear("many <b>, one block, many </b>", 250, |n| {
        format!("{}<div>x{}", "<b>".repeat(n), "</b>y".repeat(n))
    });
    assert_linear("many <b>, many blocks, many </b>", 250, |n| {
        format!(
            "{}{}{}",
            "<b>".repeat(n),
            "<div>x".repeat(n),
            "</b>y".repeat(n)
        )
    });
    assert_linear("eight blocks under each <b>", 100, |n| {
        format!("<b>{}</b>y", "<div>x".repeat(8)).repeat(n)
    });
    assert_linear("a long link, eight blocks", 100, |n| {
        format!(
            "<a href=\"https://e.x/{}\">{}</a>y",
            "p".repeat(200),
            "<div>x".repeat(8)
        )
        .repeat(n)
    });
    assert_linear("stray </p>", 500, |n| "x</p>".repeat(n));
    assert_linear("stray </p> under open elements", 250, |n| {
        format!("{}{}", "<div>".repeat(n), "x</p>".repeat(n))
    });
}

// ── #1415: the HTML standard's named character references ──────────────────

/// Names outside HTML 4's table, upper-case ones, and names that stand for
/// two characters. Each was left as written before #1415.
#[test]
fn named_references_of_the_html_standard_decode() {
    assert_eq!(
        html("&plus; &num; &lpar; &comma; &check; &half;"),
        "<p>+ # ( , \u{2713} \u{bd}</p>"
    );
    assert_eq!(html("&AMP; &LT; &GT; &QUOT;"), "<p>&amp; &lt; &gt; \"</p>");
    assert_eq!(
        html("&CounterClockwiseContourIntegral;&langle;&angst;"),
        "<p>\u{2233}\u{27e8}\u{c5}</p>"
    );
    assert_eq!(
        html("&NotEqualTilde;|&bne;|&fjlig;"),
        "<p>\u{2242}\u{338}|=\u{20e5}|fj</p>"
    );
}

/// The 106 references HTML decodes with no `;`, in text: the longest name
/// that matches, whatever follows it.
#[test]
fn legacy_references_decode_without_a_semicolon_in_text() {
    assert_eq!(
        html("a&amp b&lt c&gt d&quot e&copy f&reg"),
        "<p>a&amp; b&lt; c&gt; d\" e\u{a9} f\u{ae}</p>"
    );
    assert_eq!(html("x&nbspy"), "<p>x\u{a0}y</p>");
    // Chrome: `¬it; ∉ ¬in ¬x`.
    assert_eq!(
        html("&notit; &notin; &not;in &notx"),
        "<p>\u{ac}it; \u{2209} \u{ac}in \u{ac}x</p>"
    );
    assert_eq!(
        html("&copy=2 &copy2 &copyx &ampx &amp="),
        "<p>\u{a9}=2 \u{a9}2 \u{a9}x &amp;x &amp;=</p>"
    );
    assert_eq!(html("&ampamp; &ltb&gtc"), "<p>&amp;amp; &lt;b&gt;c</p>");
    assert_eq!(html("&Aring &aring &AElig"), "<p>\u{c5} \u{e5} \u{c6}</p>");
    // A `<textarea>` holds text.
    assert_eq!(
        html("<p><textarea>&copy &notit; &plus;</textarea></p>"),
        html("<p>&copy &notit; &plus;</p>")
    );
}

/// What is no reference stays as written, as Chrome shows it: a name that
/// needs its `;` without one, an unknown name, a bare `&`.
#[test]
fn what_is_no_reference_stays_as_written() {
    assert_eq!(
        html("R&D AT&T &lang &ang &nosuch; &am; &a; &; & &x &Amp; &plus"),
        "<p>R&amp;D AT&amp;T &amp;lang &amp;ang &amp;nosuch; &amp;am; &amp;a; &amp;; &amp; \
         &amp;x &amp;Amp; &amp;plus</p>"
    );
    // One pass: what a reference decodes to is not decoded again.
    assert_eq!(html("&amp;copy; &amp;amp;"), "<p>&amp;copy; &amp;amp;</p>");
}

fn href(src: &str) -> String {
    let out = html(&format!("<a href=\"{src}\">l</a>"));
    let start = out.find("href=\"").expect("a link") + 6;
    let end = start + out[start..].find('"').unwrap();
    out[start..end].replace("&amp;", "&").replace("&lt;", "<")
}

/// In an attribute value a reference with no `;` is left alone when `=` or
/// a letter or digit follows it, so the `&copy=` of a query string is not a
/// copyright sign. Chrome 153's `href` for each of these.
#[test]
fn a_legacy_reference_in_an_attribute_needs_what_follows_to_end_it() {
    assert_eq!(
        href("https://e.x/?a=1&copy=2&reg2&amp;x&lt&gt=3&amp"),
        "https://e.x/?a=1&copy=2&reg2&x<&gt=3&"
    );
    assert_eq!(href("https://e.x/x&copy"), "https://e.x/x\u{a9}");
    assert_eq!(href("https://e.x/y&copy;z"), "https://e.x/y\u{a9}z");
    assert_eq!(
        href("https://e.x/z&copy z&notit;"),
        "https://e.x/z\u{a9} z&notit;"
    );
    assert_eq!(href("https://e.x/?q=&plus;&AMP;"), "https://e.x/?q=+&");
}

#[test]
fn references_cost_steps_in_proportion() {
    assert_linear("legacy references", 500, |n| "&notit; ".repeat(n));
    assert_linear("long names that are none", 500, |n| {
        format!("&{};", "a".repeat(40)).repeat(n)
    });
    assert_linear("ampersands", 500, |n| "&".repeat(n * 8));
}
