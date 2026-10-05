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

// ── Defects (failed at c4c22a2c) ─────────────────────────────────────────────

fn limit() -> usize {
    rinch_editor_core::serialize::html_reader_max_depth()
}

/// `depth` elements of `tag` around `inner`, then `after`.
fn nested(tag: &str, depth: usize, inner: &str, after: &str) -> String {
    format!(
        "{}{inner}{}{after}",
        format!("<{tag}>").repeat(depth),
        format!("</{tag}>").repeat(depth)
    )
}

/// The depths worth reading at: around the limit, and far past it.
fn depths() -> Vec<usize> {
    let l = limit();
    vec![10, l - 2, l - 1, l, l + 1, l + 2, l + 50, 3 * l, 5000]
}

/// No depth changes the text or where a line ends. At c4c22a2c a start tag
/// past the limit opened nothing, so this read as `["onetwothreefourfive",
/// "after"]`. Chrome 153 keeps every one of these elements at any depth —
/// measured with 100..2000 `<div>`s: innerText `INNER|PA|PB|LI|TA\tTB|AFTER`.
#[test]
fn defect_blocks_past_the_depth_cap_stay_lines_of_their_own() {
    let inner = "<p>one</p><p>two</p><ul><li>three</li></ul>\
                 <table><tr><td>four</td><td>five</td></tr></table>";
    for tag in ["div", "blockquote", "span", "b", "section", "x-y"] {
        for depth in depths() {
            assert_eq!(
                lines_of(&nested(tag, depth, inner, "after")),
                ["one", "two", "three", "four", "five", "after"],
                "{depth} <{tag}>"
            );
        }
    }
}

/// Past the limit a block is split around the blocks inside it, and inline
/// elements inside a block stay in its line (their marks are what is lost).
#[test]
fn text_between_blocks_past_the_depth_cap_keeps_its_lines_and_order() {
    for depth in depths() {
        assert_eq!(
            lines_of(&nested(
                "div",
                depth,
                "<div>x<p>y1 <b>y2</b> <a href=\"https://e.x/\">y3</a></p>z<hr>w</div>",
                ""
            )),
            ["x", "y1 y2 y3", "z", "w"],
            "{depth}"
        );
        // A quote of two paragraphs: no empty line for the quote itself.
        assert_eq!(
            lines_of(&nested(
                "div",
                depth,
                "<blockquote>\n<p>a</p>\n<p>b</p>\n</blockquote>",
                ""
            )),
            ["a", "b"],
            "{depth}"
        );
        // Empty cells are still cells.
        let doc = load(&nested(
            "div",
            depth,
            "<table><tr><td>a</td><td></td><td>c</td></tr></table>",
            "",
        ));
        let mut cells = 0;
        fn count(n: &Node, cells: &mut usize) {
            if n.type_name() == "table_cell" {
                *cells += 1;
            }
            for c in n.content().children() {
                count(c, cells);
            }
        }
        count(&doc, &mut cells);
        assert_eq!(cells, 3, "{depth}");
    }
}

/// An end tag past the limit ends the element of its name, and one that
/// names nothing open ends nothing (the first cut counted unopened start
/// tags, so any end tag spent one).
#[test]
fn end_tags_past_the_depth_cap_are_matched_by_name() {
    for depth in depths() {
        // `</em>` and `</section>` close nothing; `</b>` closes the `<b>`.
        let src = nested(
            "div",
            depth,
            "<div><p>one</p></em><p>two</p></section>three<b>four</b>five</div>six",
            "seven",
        );
        assert_eq!(
            lines_of(&src),
            ["one", "two", "threefourfive", "six", "seven"],
            "{depth}"
        );
    }
    // The elements at the limit keep their marks; the mark ends at its tag.
    let l = limit();
    let src = format!("{}<b>x</b>y<p>z</p>", "<div>".repeat(l - 1));
    assert_eq!(html(&src), "<p><strong>x</strong>y</p><p>z</p>");
}

/// What the reader drops with its content is dropped past the limit too, and
/// nothing inside it is split out of it.
#[test]
fn dropped_elements_past_the_depth_cap_stay_dropped() {
    for depth in depths() {
        let src = nested(
            "div",
            depth,
            "<p>a<svg><g><text>label</text><desc><p>d</p></desc></g></svg>b</p>\
             <select><option>o1</option><div>o2</div></select><p>c</p>\
             <object><p>fallback</p></object>",
            "",
        );
        assert_eq!(lines_of(&src), ["ab", "c"], "{depth}");
    }
}

/// A valid document nested deeper than the first cut's limit (128) reads
/// back the same: `load_html(node_to_html(doc))`, copy and paste inside one
/// editor. At c4c22a2c this read as one paragraph `ab`.
#[test]
fn defect_a_document_nested_deeper_than_the_cap_reads_back_the_same() {
    let schema = Schema::starter_kit();
    let para = |t: &str| {
        schema
            .branch("paragraph", Fragment::from_node(schema.text(t).unwrap()))
            .unwrap()
    };
    let quotes = |n: usize| {
        let mut inner = schema
            .branch(
                "blockquote",
                Fragment::from_children(vec![para("a"), para("b")]),
            )
            .unwrap();
        for _ in 0..n {
            inner = schema
                .branch("blockquote", Fragment::from_node(inner))
                .unwrap();
        }
        node_to_html(&schema.branch("doc", Fragment::from_node(inner)).unwrap())
    };
    let written = quotes(140);
    assert_eq!(lines_of(&written), ["a", "b"]);
    assert_eq!(html(&written), written);
    // Up to the limit it is the same document; past it, the same lines.
    let written = quotes(limit() - 2);
    assert_eq!(html(&written), written);
    for n in [limit() - 1, limit(), limit() + 1, 4 * limit()] {
        assert_eq!(lines_of(&quotes(n)), ["a", "b"], "{n}");
    }
}

/// The limit is what a small stack reads: every shape, nested without end,
/// on 1.5 MB (an unoptimized build needs 1.25 MB at 192 levels).
#[test]
fn the_depth_limit_reads_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1536 * 1024)
        .spawn(|| {
            for tag in [
                "div",
                "blockquote",
                "ul><li",
                "ol><li><p",
                "table><tr><td",
                "b",
            ] {
                let close: String = tag.split('>').rev().map(|t| format!("</{t}>")).collect();
                let src = format!(
                    "{}deep{}<p>after</p>",
                    format!("<{tag}>").repeat(5000),
                    close.repeat(5000)
                );
                let doc = load(&src);
                // (`<li><p><ol>` is an empty paragraph and a list.)
                let text = |doc: &Node| {
                    let mut out = Vec::new();
                    lines(doc, &mut out);
                    out.retain(|l| !l.is_empty());
                    out
                };
                assert_eq!(text(&doc), ["deep", "after"], "{tag}");
                let written = node_to_html(&doc);
                assert_eq!(text(&load(&written)), ["deep", "after"], "{tag}");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// An `<svg>` or `<math>` that is not closed ends at the first HTML element,
/// as in a browser (Chrome 153 shows `UsB` and `UsC`; c4c22a2c and main
/// dropped everything after the `<svg>`). Inside a `<foreignObject>` or a
/// `<desc>` an HTML element is part of the drawing.
#[test]
fn an_unclosed_svg_ends_at_the_first_html_element() {
    assert_eq!(
        lines_of("<p>UsA</p><svg><path d=\"M0\"><p>UsB</p><p>UsC</p>"),
        ["UsA", "UsB", "UsC"]
    );
    assert_eq!(
        lines_of("<p>a<math><mi>x<span>b</span></p><p>c</p>"),
        ["ab", "c"]
    );
    assert_eq!(
        lines_of(
            "<p>a<svg><foreignObject><div>fo</div></foreignObject>\
             <desc><p>d</p></desc><text>t</text></svg>b</p>"
        ),
        ["ab"]
    );
}

/// A tag looks through a bounded number of open elements: with thousands
/// open, reading stays linear in the input.
#[test]
fn a_tag_among_thousands_of_open_elements_costs_a_bounded_scan() {
    use rinch_editor_core::serialize::html_reader_steps;
    let schema = Schema::starter_kit();
    for shape in [
        "<hr>",
        "</b>",
        "<li>x",
        "<td>x",
        "<p><button>",
        "<div></div>",
    ] {
        let steps = |n: usize| {
            let src = format!("<p><table><td>{}{}", "<span>".repeat(n), shape.repeat(n));
            let before = html_reader_steps();
            slice_from_html(&schema, &src).unwrap();
            html_reader_steps() - before
        };
        let (small, large) = (steps(3000), steps(6000));
        assert!(small >= 3000, "{shape}: {small}");
        assert!(
            large <= small * 2 + small / 4,
            "{shape}: {small} steps for 3000, {large} for 6000"
        );
    }
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
