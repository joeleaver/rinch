//! Fixtures from the round-2 review of PR #1411 (the HTML reader, #1397; head b5ef069b).
//!
//! `defect_*` fail at b5ef069b and state the behaviour the review asks for.
//! `pin_*` pass at the head; each kills a mutant the PR's suite lets through
//! (named in its doc) or pins a behaviour the PR describes and no test holds.

use rinch_editor_core::serialize::{html_reader_max_depth, node_to_html, slice_from_html};
use rinch_editor_core::transform::Transform;
use rinch_editor_core::{Node, Schema};
use std::time::{Duration, Instant};

fn load(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, html).expect("reads");
    schema.branch("doc", slice.content.clone()).expect("a doc")
}

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

fn count(node: &Node, name: &str) -> usize {
    usize::from(node.type_name() == name)
        + node
            .content()
            .children()
            .iter()
            .map(|c| count(c, name))
            .sum::<usize>()
}

fn fit(target: &str, from: usize, to: usize, html: &str) -> String {
    let schema = Schema::starter_kit();
    let mut tf = Transform::new(&schema, load(target));
    let slice = slice_from_html(&schema, html).unwrap();
    tf.replace_range(from, to, slice).unwrap();
    assert_eq!(tf.steps().len(), 1, "one step");
    node_to_html(&tf.doc)
}

// ── Defects ──────────────────────────────────────────────────────────────────

/// A block flattened past the depth limit hands over a copy of its tag and
/// attributes for every part it is split into (`end_flat_part` clones them
/// before it knows the part is empty), and the mapper then reads each copy.
/// So one `style` of A bytes on a block past the limit that holds N blocks
/// costs A x N: 191 KB of input (a 100 KB `style`, 10,000 `t<p>y</p>`) read
/// in 3.7 s and 381 KB in 23.7 s (release); the same content 10 levels deep,
/// and main at 192, read in 45-60 ms. "Reading is linear in the input" does
/// not hold past the limit.
///
/// Pinned as a ratio against the same content under the limit, with a wide
/// margin (measured 60x and up; 8x allowed).
#[test]
fn defect_a_flattened_block_costs_its_attributes_once_not_once_a_part() {
    let schema = Schema::starter_kit();
    let body = format!(
        "<div style=\"{}\">{}",
        "color:red;".repeat(2_000),
        "t<p>y</p>".repeat(3_000)
    );
    let read = |depth: usize| {
        let src = format!("{}{body}", "<div>".repeat(depth));
        (0..3)
            .map(|_| {
                let t = Instant::now();
                let slice = slice_from_html(&schema, &src).unwrap();
                let took = t.elapsed();
                assert_eq!(slice.content.children().len(), 6_000);
                took
            })
            .min()
            .unwrap()
    };
    let shallow = read(10);
    let deep = read(html_reader_max_depth() + 1);
    assert!(
        deep <= shallow * 8 + Duration::from_millis(20),
        "under the limit {shallow:?}, past it {deep:?}"
    );
}

/// `pad_rows` fills every bare-part table up to 2^16 empty cells, and a paste
/// can hold any number of such tables: 195 bytes ask for 65,000 cells, so a
/// 12.6 KB paste is a 650,000-cell document (35 MB) and 506 KB is 26 million
/// cells (1.28 GB of memory, measured). On main and at c4c22a2c the same
/// input made no cell that the markup did not write.
///
/// What is asked: the cells a paste adds are bounded by the paste (here: no
/// more than a few per byte of input), whatever the bound per table is.
#[test]
fn defect_the_cells_padding_adds_are_bounded_by_the_input() {
    let one = format!(
        "<td colspan=1000>a</td>{}<p>x</p>",
        "<tr><td>b</td></tr>".repeat(65)
    );
    let src = one.repeat(4);
    let doc = load(&src);
    let cells = count(&doc, "table_cell");
    assert!(
        cells <= src.len(),
        "{} bytes of input made {cells} cells",
        src.len()
    );
}

/// A tag gives up looking for the element it closes after `MAX_SCAN` (192)
/// open elements, and "an element further up stays open": so with that many
/// elements left open inside a block, the block's end tag (or the tag that
/// implies it) ends nothing, and the words after it join the words in it.
/// Main (ee5123ba) and Chrome 153 read every one of these as separate lines
/// (measured: `a|b|c|d`, `a|b|c|d`, `head|body|h2`, `l1\nl2|after`); at
/// b5ef069b they are `cd`, `ab`, `headbody` and one code block `l1\nl2after`.
/// Malformed input only (a well-nested document closes the innermost
/// element), but it is words run together where main kept them apart.
#[test]
fn defect_a_block_ends_where_its_end_tag_is_however_many_elements_are_open_in_it() {
    let open = |tag: &str| format!("<{tag}>").repeat(html_reader_max_depth() + 60);
    assert_eq!(
        lines_of(&format!("<ul><li>{}a<li>b<li>c</ul>d", open("i"))),
        ["a", "b", "c", "d"]
    );
    assert_eq!(
        lines_of(&format!("<div>{}a</div>b<div>c</div>d", open("span"))),
        ["a", "b", "c", "d"]
    );
    assert_eq!(
        lines_of(&format!("<h1>{}head</h1>body<h2>h2</h2>", open("u"))),
        ["head", "body", "h2"]
    );
    assert_eq!(
        lines_of(&format!("<pre>{}l1\nl2</pre>after", open("span"))),
        ["l1\nl2", "after"]
    );
}

// ── Pins ─────────────────────────────────────────────────────────────────────

/// What the unfolding does everywhere a caret can be (the PR pins a
/// paragraph, a list item and a heading): the first line continues the line,
/// each further line is a paragraph beside it in the same parent, and the
/// text after the caret joins the last.
#[test]
fn pin_code_lines_in_every_kind_of_line() {
    let code = "<pre>l1\nl2\nl3</pre>";
    for (target, at, want) in [
        (
            "<table><tr><td><p>abcd</p></td><td><p>ef</p></td></tr></table>",
            6,
            "<table><tr><td><p>abl1</p><p>l2</p><p>l3cd</p></td><td><p>ef</p></td></tr></table>",
        ),
        (
            "<blockquote><p>abcd</p></blockquote>",
            4,
            "<blockquote><p>abl1</p><p>l2</p><p>l3cd</p></blockquote>",
        ),
        (
            "<p>a<strong>bc</strong>d</p>",
            3,
            "<p>a<strong>b</strong>l1</p><p>l2</p><p>l3<strong>c</strong>d</p>",
        ),
        (
            "<p style=\"text-align:center\">abcd</p>",
            3,
            "<p style=\"text-align:center\">abl1</p><p>l2</p><p>l3cd</p>",
        ),
        ("<p>abcd</p>", 5, "<p>abcdl1</p><p>l2</p><p>l3</p>"),
    ] {
        assert_eq!(fit(target, at, at, code), want, "{target}");
    }
    // A selection across two blocks.
    assert_eq!(
        fit("<p>abcd</p><ul><li><p>efgh</p></li></ul>", 3, 11, code),
        "<p>abl1</p><p>l2</p><p>l3gh</p>"
    );
}

/// The line ends a source writes: CRLF is one line end (mutant: the
/// `strip_suffix('\r')` removed survives the PR's suite), a trailing line
/// end leaves the text after the caret on a line of its own, and empty lines
/// are empty paragraphs.
#[test]
fn pin_code_line_ends() {
    assert_eq!(
        fit("<p>abcd</p>", 3, 3, "<pre>a\r\nb\r\nc</pre>"),
        "<p>aba</p><p>b</p><p>ccd</p>"
    );
    assert_eq!(
        fit("<p>abcd</p>", 3, 3, "<pre>a\nb\n</pre>"),
        "<p>aba</p><p>b</p><p>cd</p>"
    );
    assert_eq!(
        fit("<p>abcd</p>", 3, 3, "<pre>a\n\n\nb</pre>"),
        "<p>aba</p><p></p><p></p><p>bcd</p>"
    );
}

/// A table written with a `<table>` is not padded, whatever its spans: the
/// PR's fixture for this has rows that differ by one plain cell only.
#[test]
fn pin_a_written_table_is_read_as_written() {
    for src in [
        "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td></tr></table>",
        "<table><tr><td colspan=\"3\">a</td></tr><tr><td>b</td></tr></table>",
        "<table><tr><td rowspan=\"2\">a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
    ] {
        let doc = load(src);
        let text: usize = src.matches("<td").count();
        assert_eq!(count(&doc, "table_cell"), text, "{src}");
    }
}

/// Padding with spans: the bare parts' own cells keep their spans and the
/// table tiles (no hole, no overlap).
#[test]
fn pin_padded_bare_parts_with_spans_tile() {
    for (src, cells) in [
        (
            "<td>A</td><td>B</td><tr><td>C</td></tr><tr><td colspan=3>D</td></tr>",
            7,
        ),
        (
            "<td rowspan=2>A</td><td>B</td><td>C</td><tr><td>D</td></tr>",
            5,
        ),
        ("<td colspan=2 rowspan=2>A</td><tr><td>C</td></tr>", 3),
    ] {
        let doc = load(src);
        assert_eq!(count(&doc, "table_cell"), cells, "{src}");
        let table = doc.content().children().first().unwrap().clone();
        assert_eq!(table.type_name(), "table");
        let holes = rinch_editor_core::tables::row_holes(&table);
        assert!(holes.iter().all(|h| *h == 0), "{src}: {holes:?}");
    }
}

/// Past the cap on padding the parts are left as written: no cell is added
/// (mutant: the `total > MAX_PAD_CELLS` test removed survives the PR's
/// suite, and makes 69,930 cells of this).
#[test]
fn pin_parts_that_would_need_more_padding_than_the_cap_are_left_as_written() {
    let src = format!(
        "<td colspan=1000>a</td>{}",
        "<tr><td>b</td></tr>".repeat(70)
    );
    assert_eq!(count(&load(&src), "table_cell"), 71);
}

/// Everything that walks a document nested to the limit, on the 2 MB a
/// spawned thread has, unoptimized: the reader on shapes the PR's fixture
/// does not hold, the writer, and the fitted paste.
#[test]
fn pin_the_depth_limit_is_walked_on_a_two_megabyte_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let schema = Schema::starter_kit();
            for tag in [
                "blockquote><ul><li",
                "ul><li><blockquote><p",
                "table><tbody><tr><td><div",
                "h1",
                "a href=\"https://e.x/\"",
                "span style=\"color:red\"><b><i",
                "pre",
                "dl><dd",
                "details><summary",
                "center><font",
                "td",
                "li",
            ] {
                let close: String = tag
                    .split('>')
                    .rev()
                    .map(|t| format!("</{}>", t.split(' ').next().unwrap()))
                    .collect();
                let src = format!(
                    "{}deep{}<p>after</p>",
                    format!("<{tag}>").repeat(3000),
                    close.repeat(3000)
                );
                let slice = slice_from_html(&schema, &src).unwrap();
                let doc = schema.branch("doc", slice.content.clone()).unwrap();
                let mut out = Vec::new();
                lines(&doc, &mut out);
                out.retain(|l| !l.is_empty());
                assert_eq!(out, ["deep", "after"], "{tag}");
                let _ = node_to_html(&doc);
                // Pasted in a line of text and on an empty line.
                for target in ["<p>abcd</p>", "<p></p>"] {
                    let mut tf = Transform::new(&schema, load(target));
                    tf.replace_range(1, 1, slice.clone()).unwrap();
                    let mut out = Vec::new();
                    lines(&tf.doc, &mut out);
                    assert!(out.concat().contains("deep"), "{tag} in {target}");
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

// ── Fix round 2 ──────────────────────────────────────────────────────────────

/// However many elements are left open, a tag closes what it closes with
/// three open: the lines are the same at every count. (Round 1 looked no
/// further than 192 open elements, and past them an end tag ended nothing.)
/// Each shape has a stop or a barrier the tag must still respect from far.
#[test]
fn unclosed_elements_change_no_line_however_many() {
    let shapes = [
        "<ul><li>{S}a<li>b<li>c</ul>d",
        "<div>{S}a</div>b<div>c</div>d",
        "<h1>{S}head</h1>body<h2>h2</h2>",
        "<pre>{S}l1\nl2</pre>after",
        // A `<div>` in a cell does not end a `<p>` around the table.
        "<p>o<table><tr><td>{S}x<div>y</div>z</td></tr></table>w",
        // An inline end tag does not reach across a block.
        "<b>k<div>{S}x</b>y</div>z",
        "<table><tr><td>{S}a<td>b<tr><td>c</table>d",
        "<dl><dt>{S}a<dd>b<dt>c</dl>d",
        // An `<li>` in a cell is not the outer list's.
        "<ul><li>{S}a<table><tr><td>b<li>c</td></tr></table><li>d</ul>e",
        "<p>{S}a<svg><desc><p>q</p></desc></svg>b<p>c",
        "<h1>{S}a<h2>b</h2>c",
        "<table><caption>{S}cap<tr><td>a<tbody><tr><td>b</table>c",
        "<div>{S}a<svg><g>{S}<p>b</p>c",
        // Any heading's end tag ends the open heading.
        "<h1>{S}a</h3>b",
        // A row in an inner table closes nothing of the outer one.
        "<table><tr><td>o<table><tr><td>{S}a<tr><td>b</table>c</td><td>e</td></tr></table>d",
        "<ul><li>{S}a<p>b<li>c</ul>d",
    ];
    let limit = html_reader_max_depth();
    for shape in shapes {
        for tag in ["span", "i", "x-y"] {
            let read = |n: usize| lines_of(&shape.replace("{S}", &format!("<{tag}>").repeat(n)));
            let want = read(3);
            assert!(want.len() >= 2, "{shape}: {want:?}");
            for n in [limit - 2, limit, limit + 1, limit + 60, 5 * limit] {
                assert_eq!(read(n), want, "{shape} with {n} <{tag}>");
            }
        }
    }
}

/// Reading stays linear when a block past the depth limit is split into
/// parts: its attributes go with one part, not a copy with each. Both the
/// attribute and the number of parts double here, so a copy a part is four
/// times the steps (the reader counts the attribute bytes it hands over).
#[test]
fn a_flattened_block_is_linear_in_its_attributes_and_parts() {
    use rinch_editor_core::serialize::html_reader_steps;
    let schema = Schema::starter_kit();
    let steps = |n: usize| {
        let src = format!(
            "{}<div style=\"{}\">{}",
            "<div>".repeat(html_reader_max_depth()),
            "color:red;".repeat(n * 8),
            "t<p>y</p>".repeat(n)
        );
        let before = html_reader_steps();
        let slice = slice_from_html(&schema, &src).unwrap();
        assert_eq!(slice.content.children().len(), 2 * n);
        html_reader_steps() - before
    };
    let (small, large) = (steps(1000), steps(2000));
    assert!(small >= 1000, "{small}");
    assert!(
        large <= small * 2 + small / 4,
        "{small} steps for 1000 parts, {large} for 2000"
    );
}

/// The attributes of a block split past the limit go with its first part.
#[test]
fn a_flattened_block_keeps_its_attributes_on_its_first_part() {
    let src = format!(
        "{}<h2 style=\"text-align:center\">a<div>b</div>c</h2>",
        "<div>".repeat(html_reader_max_depth())
    );
    assert_eq!(
        node_to_html(&load(&src)),
        "<h2 style=\"text-align:center\">a</h2><p>b</p><h2>c</h2>"
    );
}

/// What padding adds is in proportion to what was written: at most eight
/// empty cells a cell (sixteen whatever the number), else none.
#[test]
fn padding_is_in_proportion_to_the_cells_written() {
    // 3 cells written, 16 missing: padded.
    let src = "<td colspan=9>a</td><tr><td>b</td></tr><tr><td>c</td></tr>";
    assert_eq!(count(&load(src), "table_cell"), 3 + 16);
    // 3 written, 26 missing (more than 24): as written. 24 missing: padded.
    let src = "<td colspan=14>a</td><tr><td>b</td></tr><tr><td>c</td></tr>";
    assert_eq!(count(&load(src), "table_cell"), 3);
    let src = "<td colspan=13>a</td><tr><td>b</td></tr><tr><td>c</td></tr>";
    assert_eq!(count(&load(src), "table_cell"), 3 + 24);
    // Many small tables in one paste: each is bounded by itself.
    let one = "<td colspan=17>a</td><tr><td>b</td></tr><p>x</p>";
    let src = one.repeat(500);
    assert!(count(&load(&src), "table_cell") <= src.len() / 2);
}

/// Code lines pasted where a line keeps some of its content are paragraphs
/// wherever the caret is in it: at its start (where the fit would otherwise
/// replace the line with the code block and take the line's text into it),
/// in it, at its end, and beside an image alone on a line. A line that keeps
/// nothing (an empty one, or one selected whole) is where the block lands.
#[test]
fn code_lines_are_paragraphs_at_the_start_middle_and_end_of_a_line() {
    let code = "<pre>l1\nl2\nl3</pre>";
    let img = "<img src=\"https://e.x/i.png\">";
    for (target, from, to, want) in [
        (
            "<p>abcd</p>",
            1,
            1,
            "<p>l1</p><p>l2</p><p>l3abcd</p>".to_string(),
        ),
        (
            "<p>abcd</p>",
            3,
            3,
            "<p>abl1</p><p>l2</p><p>l3cd</p>".to_string(),
        ),
        (
            "<p>abcd</p>",
            5,
            5,
            "<p>abcdl1</p><p>l2</p><p>l3</p>".to_string(),
        ),
        // Part of the line selected, from its start.
        (
            "<p>abcd</p>",
            1,
            3,
            "<p>l1</p><p>l2</p><p>l3cd</p>".to_string(),
        ),
        (
            "<ul><li><p>abcd</p></li></ul>",
            3,
            3,
            "<ul><li><p>l1</p><p>l2</p><p>l3abcd</p></li></ul>".to_string(),
        ),
        (
            "<h2>abcd</h2>",
            1,
            1,
            "<h2>l1</h2><p>l2</p><p>l3abcd</p>".to_string(),
        ),
        (
            &format!("<p>{img}</p>"),
            1,
            1,
            format!("<p>l1</p><p>l2</p><p>l3{img}</p>"),
        ),
        (
            &format!("<p>{img}</p>"),
            2,
            2,
            format!("<p>{img}l1</p><p>l2</p><p>l3</p>"),
        ),
        // Nothing of the line is kept: the block.
        ("<p></p>", 1, 1, code.to_string()),
        ("<p>abcd</p>", 1, 5, code.to_string()),
        (
            "<p>x</p><p></p><p>y</p>",
            4,
            4,
            format!("<p>x</p>{code}<p>y</p>"),
        ),
    ] {
        assert_eq!(fit(target, from, to, code), want, "{target} {from}..{to}");
    }
}

/// An inline element that holds a block joins the line it is on, past the
/// limit and under it (Chrome's reading at any depth, by the round-2
/// review's measurement). Under the limit it was a line of its own, the one
/// line break depth changed, until the reader stopped ending a line at an
/// inline element (#1413).
#[test]
fn an_inline_element_around_a_block_joins_its_line_past_the_limit() {
    let read = |depth: usize| lines_of(&format!("{}a<b>d<p>e</p>f</b>g", "<div>".repeat(depth)));
    assert_eq!(read(2), ["ad", "e", "fg"]);
    assert_eq!(read(html_reader_max_depth() + 5), ["ad", "e", "fg"]);
}

/// The same for structure, where what follows the unclosed elements closes
/// back under the depth limit: the document is the one read with three open.
/// (Lines alone do not see an `<li>` left open around the next `<li>`, or a
/// row closed in the wrong table.)
#[test]
fn unclosed_elements_change_no_structure_however_many() {
    let shapes = [
        "<ul><li>{S}a<li>b<li>c</ul>d",
        "<table><tr><td>{S}a<td>b<tr><td>c</table>d",
        "<p>o<table><tr><td>{S}x<div>y</div>z</td></tr></table>w",
        "<table><tr><td>o<table><tr><td>{S}a<tr><td>b</table>c</td><td>e</td></tr></table>d",
        "<dl><dt>{S}a<dd>b<dt>c</dl>d",
        "<table><tr><td>{S}a<tbody><tr><td>b</table>c",
        "<ul><li>k<ol><li>{S}a<li>b</ol><li>c</ul>d",
        // An `<li>` in a cell does not end the item the table is in, and a
        // `<div>` in a `<button>` does not end the `<p>` around it.
        "<ul><li>k<table><tr><td>{S}a<li>b</td></tr></table>m<li>c</ul>d",
        "<p>o<button>{S}x<div>y</div>z</button>w<p>v",
    ];
    for shape in shapes {
        let read = |n: usize| node_to_html(&load(&shape.replace("{S}", &"<span>".repeat(n))));
        let want = read(3);
        for n in [html_reader_max_depth() + 60, 1000] {
            assert_eq!(read(n), want, "{shape} with {n} <span>");
        }
    }
}
