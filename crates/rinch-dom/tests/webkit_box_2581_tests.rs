//! `-webkit-line-clamp` across block children, and the legacy
//! `display: -webkit-box` laid out as Chrome lays it out (#2581).
//!
//! Every number is Chrome 155's on the same markup: the bundled Inter
//! (`Inter-Regular.ttf`, loaded with `@font-face` there and registered as
//! `ProbeFace` here), `body { margin: 0; font: 16px/24px ProbeFace }`,
//! `p { margin: 0 }` unless a fixture says otherwise. Boxes are from
//! `getBoundingClientRect`, a "…"'s x from a `Range` around the text before
//! it.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const TOL: f32 = 0.05;
const TEXT: &str =
    "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
const CSS: &str = "
    body { margin: 0; font: 16px/24px ProbeFace; }
    p { margin: 0; }
    .c { width: 200px; }
    .clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
    .wb { display: -webkit-box; width: 230px; }
    .v { -webkit-box-orient: vertical; }
";

fn page(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css(&format!("{CSS}{css}"));
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(400.0, 800.0);
    d
}

fn id(d: &RinchDocument, name: &str) -> u64 {
    rinch_dom::testing::query_selector(&d.tree, &format!("#{name}"))[0]
}

/// `(x, y, width, height)` from the page's origin.
fn rect(d: &RinchDocument, node: u64) -> (f32, f32, f32, f32) {
    let n = d.tree.get(node).unwrap();
    let (mut x, mut y) = (n.layout.x, n.layout.y);
    let mut p = n.parent;
    while let Some(pid) = p {
        let pn = d.tree.get(pid).unwrap();
        x += pn.layout.x;
        y += pn.layout.y;
        p = pn.parent;
    }
    (x, y, n.layout.width, n.layout.height)
}

#[track_caller]
fn assert_rect(d: &RinchDocument, name: &str, want: (f32, f32, f32, f32)) {
    let got = rect(d, id(d, name));
    let ok = [
        (got.0, want.0),
        (got.1, want.1),
        (got.2, want.2),
        (got.3, want.3),
    ]
    .iter()
    .all(|(g, w)| (g - w).abs() <= TOL);
    assert!(ok, "#{name}: got {got:?}, Chrome 155 {want:?}");
}

/// The "…" marks of an IFC root's lines: `(line, x)`.
fn marks(d: &RinchDocument, name: &str) -> Vec<(usize, f32)> {
    let il = d
        .tree
        .get(id(d, name))
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap();
    il.ellipsis
        .as_ref()
        .map(|e| e.marks.iter().map(|m| (m.line, m.x)).collect())
        .unwrap_or_default()
}

#[track_caller]
fn assert_mark(d: &RinchDocument, name: &str, line: usize, x: f32) {
    let m = marks(d, name);
    assert!(
        m.len() == 1 && m[0].0 == line && (m[0].1 - x).abs() <= 0.1,
        "#{name}: {m:?}, Chrome 155 line {line} at {x}"
    );
}

/// The clamp counts lines through the box's block children: `p1 short` is
/// line 1, the second paragraph's first line is line 2 and ends the box
/// (48), its own box ends there too (24 tall, its later lines overflow it),
/// and the line is cut for the "…" after `five s` (184.64). A paragraph
/// after it flows on from there (`a3` at 48).
#[test]
fn a_clamp_counts_lines_across_block_children_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=a class=\"c clamp\"><p id=a1>p1 short</p><p id=a2>{TEXT}</p></div>\
             <div id=b class=\"c clamp\"><p>one</p><p id=b2>{TEXT}</p><p id=b3>x</p></div>"
        ),
    );
    assert_rect(&d, "a", (0.0, 0.0, 200.0, 48.0));
    assert_rect(&d, "a1", (0.0, 0.0, 200.0, 24.0));
    assert_rect(&d, "a2", (0.0, 24.0, 200.0, 24.0));
    assert_mark(&d, "a2", 0, 184.64);
    assert!(marks(&d, "a1").is_empty());
    assert_rect(&d, "b", (0.0, 48.0, 200.0, 48.0));
    assert_rect(&d, "b2", (0.0, 72.0, 200.0, 24.0));
    assert_rect(&d, "b3", (0.0, 96.0, 200.0, 24.0));
}

/// Margins and padding on the way: `p { margin: 5px 0; padding: 3px 0 }`
/// gives 70 (the clamped paragraph keeps its padding, its bottom margin is
/// dropped); padding 7/11 on the box and a 9px padding and 2px border under
/// the paragraph give 77 (the paragraph 35 tall).
#[test]
fn the_clamped_boxes_keep_their_padding_and_border_issue_2581() {
    let d = page(
        ".m p { margin: 5px 0; padding: 3px 0; }",
        &format!(
            "<div id=b class=\"c clamp m\"><p id=b1>p1 short</p><p id=b2>{TEXT}</p></div>\
             <div id=m class=\"c clamp\" style=\"padding: 7px 0 11px\"><p>p1</p>\
             <p id=m2 style=\"padding-bottom: 9px; border-bottom: 2px solid\">{TEXT}</p></div>"
        ),
    );
    assert_rect(&d, "b", (0.0, 0.0, 200.0, 70.0));
    assert_rect(&d, "b1", (0.0, 5.0, 200.0, 30.0));
    assert_rect(&d, "b2", (0.0, 40.0, 200.0, 30.0));
    assert_rect(&d, "m", (0.0, 70.0, 200.0, 77.0));
    assert_rect(&d, "m2", (0.0, 101.0, 200.0, 35.0));
}

/// `-webkit-line-clamp: 3` through nested blocks: `alpha` in a `div`,
/// `beta gamma` two levels down a `section`, the third line the first of
/// the last paragraph (72). The clamp falls inside the third paragraph's
/// second line (`n2` keeps two lines, 48) when the first child takes one
/// (82, with its 4/6 padding).
#[test]
fn a_clamp_of_three_counts_through_nested_blocks_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=c class=\"c clamp\" style=\"-webkit-line-clamp: 3\"><div id=c1><p>alpha</p>\
             </div><section><div><p id=c2>beta gamma</p></div></section><p id=c3>{TEXT}</p></div>\
             <div id=n class=\"c clamp\" style=\"-webkit-line-clamp: 3\">\
             <p id=n1 style=\"padding: 4px 0 6px\">aa</p><p id=n2>{TEXT}</p></div>"
        ),
    );
    assert_rect(&d, "c", (0.0, 0.0, 200.0, 72.0));
    assert_rect(&d, "c1", (0.0, 0.0, 200.0, 24.0));
    assert_rect(&d, "c2", (0.0, 24.0, 200.0, 24.0));
    assert_rect(&d, "c3", (0.0, 48.0, 200.0, 24.0));
    assert_mark(&d, "c3", 0, 184.64);
    assert_rect(&d, "n", (0.0, 72.0, 200.0, 82.0));
    assert_rect(&d, "n1", (0.0, 72.0, 200.0, 34.0));
    assert_rect(&d, "n2", (0.0, 106.0, 200.0, 48.0));
}

/// A clamp that falls at a child's last line: the box ends there (48), and
/// the line ends in "…" (after `two`, 27.88) only when content follows it
/// (`e3`, which flows on below); with nothing after it, no "…". Collapsed
/// margins before the line count (`e1`'s 9 and `e2`'s 4: 57).
#[test]
fn a_clamp_at_a_child_boundary_issue_2581() {
    let d = page(
        "",
        "<div id=e class=\"c clamp\"><p>one</p><p id=e2>two</p><p id=e3>three</p></div>\
         <div id=f class=\"c clamp\"><p>one</p><p id=f2>two</p></div>\
         <div id=g class=\"c clamp\"><p style=\"margin-bottom: 9px\">one</p>\
         <p id=g2 style=\"margin-top: 4px\">two</p><p>three</p></div>",
    );
    assert_rect(&d, "e", (0.0, 0.0, 200.0, 48.0));
    assert_rect(&d, "e2", (0.0, 24.0, 200.0, 24.0));
    assert_rect(&d, "e3", (0.0, 48.0, 200.0, 24.0));
    assert_mark(&d, "e2", 0, 27.88);
    assert_rect(&d, "f", (0.0, 48.0, 200.0, 48.0));
    assert!(marks(&d, "f2").is_empty());
    assert_rect(&d, "g", (0.0, 96.0, 200.0, 57.0));
    assert_rect(&d, "g2", (0.0, 129.0, 200.0, 24.0));
}

/// The box between the clamp and the clamped line ends after it too: `w`
/// (padding 5, border 3, margin 13 under it) is 63 tall; the clamped
/// paragraph's 7px margin is dropped from it, but the paragraphs after the
/// clamp flow on at their places (`b3` at 62, `b4` after `w`'s margin, 76).
#[test]
fn every_box_around_the_clamped_line_ends_with_it_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=b class=\"c clamp\"><div id=w style=\"padding-bottom: 5px; \
             margin-bottom: 13px; border-bottom: 3px solid\"><p style=\"margin-bottom: 7px\">one</p>\
             <p id=b2 style=\"margin-bottom: 7px\">{TEXT}</p><p id=b3>x</p></div>\
             <p id=b4>after</p></div>"
        ),
    );
    assert_rect(&d, "b", (0.0, 0.0, 200.0, 63.0));
    assert_rect(&d, "w", (0.0, 0.0, 200.0, 63.0));
    assert_rect(&d, "b2", (0.0, 31.0, 200.0, 24.0));
    assert_rect(&d, "b3", (0.0, 62.0, 200.0, 24.0));
    assert_rect(&d, "b4", (0.0, 76.0, 200.0, 24.0));
}

/// Inline content beside the blocks is a line like any other: `lead bold`
/// (an anonymous block) is line 1 and the `div` after it holds the "…"
/// (48). A `flow-root` child is a formatting context of its own, whose two
/// lines are not counted (96); an `inline-block`'s line is one line (72).
#[test]
fn inline_runs_count_and_formatting_contexts_do_not_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=g class=\"c clamp\">lead <b>bold</b><div id=g2>{TEXT}</div>tail</div>\
             <div id=c class=\"c clamp\"><div style=\"display: flow-root\">one<br>two</div>\
             <p id=c2>{TEXT}</p></div>\
             <div id=d class=\"c clamp\"><div style=\"display: inline-block; width: 100%\">one<br>\
             two</div><p id=d2>{TEXT}</p></div>"
        ),
    );
    assert_rect(&d, "g", (0.0, 0.0, 200.0, 48.0));
    assert_rect(&d, "g2", (0.0, 24.0, 200.0, 24.0));
    assert_mark(&d, "g2", 0, 184.64);
    assert_rect(&d, "c", (0.0, 48.0, 200.0, 96.0));
    assert_rect(&d, "c2", (0.0, 96.0, 200.0, 48.0));
    assert_rect(&d, "d", (0.0, 144.0, 200.0, 72.0));
    assert_rect(&d, "d2", (0.0, 192.0, 200.0, 24.0));
}

/// A clamp change lays the blocks out again.
#[test]
fn a_clamp_change_relays_the_blocks_out_issue_2581() {
    let mut d = page(
        "",
        &format!("<div id=a class=\"c clamp\"><p>p1 short</p><p id=a2>{TEXT}</p></div>"),
    );
    let a = rinch_core::dom::NodeId(id(&d, "a"));
    d.set_attribute(a, "style", "-webkit-line-clamp: 3");
    d.resolve_layout(400.0, 800.0);
    assert_rect(&d, "a", (0.0, 0.0, 200.0, 72.0));
    assert_rect(&d, "a2", (0.0, 24.0, 200.0, 48.0));
    d.set_attribute(a, "style", "display: block");
    d.resolve_layout(400.0, 800.0);
    assert_rect(&d, "a", (0.0, 0.0, 200.0, 120.0));
    assert_rect(&d, "a2", (0.0, 24.0, 200.0, 96.0));
    assert!(marks(&d, "a2").is_empty());
}

/// A legacy box's items do not shrink unless `-webkit-box-flex` says so,
/// and the flex grows them in proportion: two 150px items in a 230px box
/// keep 150 (the second at 150); with `-webkit-box-flex: 1` the second
/// shrinks to 80; 50 and 70 at flexes 1 and 2 grow to 86.67 and 143.33
/// (rinch's flex items are whole pixels wide, as with `display: flex`: 87
/// and 143).
#[test]
fn legacy_items_flex_only_by_box_flex_issue_2581() {
    let d = page(
        "",
        "<div id=h class=wb><div id=h1 style=\"width: 150px\">a</div>\
         <div id=h2 style=\"width: 150px\">b</div></div>\
         <div id=i class=wb><div id=i1 style=\"width: 150px\">a</div>\
         <div id=i2 style=\"width: 150px; -webkit-box-flex: 1\">b</div></div>\
         <div id=j class=wb><div id=j1 style=\"width: 50px; -webkit-box-flex: 1\">a</div>\
         <div id=j2 style=\"width: 70px; -webkit-box-flex: 2\">b</div></div>",
    );
    assert_rect(&d, "h", (0.0, 0.0, 230.0, 24.0));
    assert_rect(&d, "h1", (0.0, 0.0, 150.0, 24.0));
    assert_rect(&d, "h2", (150.0, 0.0, 150.0, 24.0));
    assert_rect(&d, "i1", (0.0, 24.0, 150.0, 24.0));
    assert_rect(&d, "i2", (150.0, 24.0, 80.0, 24.0));
    assert_rect(&d, "j1", (0.0, 48.0, 87.0, 24.0));
    assert_rect(&d, "j2", (87.0, 48.0, 143.0, 24.0));
}

/// A vertical legacy box is a column whose items' margins do not collapse
/// (76); a horizontal one a row: an auto-width item is as wide as its
/// content up to the box (`short` 39.02, the long text 230, three lines,
/// both stretched to 72).
#[test]
fn a_legacy_box_lays_its_blocks_out_as_flex_items_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=k class=\"wb v\"><p id=k1 style=\"margin: 7px 0 9px\">a</p>\
             <p id=k2 style=\"margin: 6px 0\">b</p></div>\
             <div id=o class=wb><div id=o1>short</div><div id=o2>{TEXT}</div></div>"
        ),
    );
    assert_rect(&d, "k", (0.0, 0.0, 230.0, 76.0));
    assert_rect(&d, "k1", (0.0, 7.0, 230.0, 24.0));
    assert_rect(&d, "k2", (0.0, 46.0, 230.0, 24.0));
    assert_rect(&d, "o", (0.0, 76.0, 230.0, 72.0));
    assert_rect(&d, "o1", (0.0, 76.0, 39.02, 72.0));
    assert_rect(&d, "o2", (39.02, 76.0, 230.0, 72.0));
}

/// Inline children mixed with blocks: each run is one anonymous item. In a
/// row, `lead <span>aa</span>` (54.61 wide; 55 in rinch, whose flex items
/// are whole pixels) then the 60px block at 54.61 (55),
/// then `tail` (24 tall); in a column the run, the block (60 wide, at 24),
/// then `tail` (72). Spans alone stay one line (`yy` at 8.73).
#[test]
fn inline_runs_of_a_legacy_box_are_items_issue_2581() {
    let d = page(
        "",
        "<div id=l class=wb>lead <span>aa</span><div id=l2 style=\"width: 60px\">B</div>tail</div>\
         <div id=m class=\"wb v\">lead <span>aa</span><div id=m2 style=\"width: 60px\">B</div>tail</div>\
         <div id=q class=wb><span>x</span><span id=q2>yy</span></div>\
         <div id=r class=wb>Hello <b>x</b><span style=\"position: absolute\">abs</span></div>",
    );
    assert_rect(&d, "l", (0.0, 0.0, 230.0, 24.0));
    assert_rect(&d, "l2", (55.0, 0.0, 60.0, 24.0));
    assert_rect(&d, "m", (0.0, 24.0, 230.0, 72.0));
    assert_rect(&d, "m2", (0.0, 48.0, 60.0, 24.0));
    assert_rect(&d, "q", (0.0, 96.0, 230.0, 24.0));
    assert_rect(&d, "r", (0.0, 120.0, 230.0, 24.0));
}

/// Markup as it is written, with white space between the blocks: the same
/// boxes as without (Chrome 155), and the "…" after `two` only where a
/// paragraph follows.
#[test]
fn white_space_between_the_blocks_counts_no_line_issue_2581() {
    let d = page(
        "",
        &format!(
            "<div id=a class=\"c clamp\">\n  <p>p1 short</p>\n  <p id=a2>{TEXT}</p>\n</div>\n\
             <div id=e class=\"c clamp\">\n  <p>one</p>\n  <p id=e2>two</p>\n  <p>three</p>\n</div>\n\
             <div id=f class=\"c clamp\">\n  <p>one</p>\n  <p id=f2>two</p>\n</div>\n"
        ),
    );
    assert_rect(&d, "a", (0.0, 0.0, 200.0, 48.0));
    assert_rect(&d, "a2", (0.0, 24.0, 200.0, 24.0));
    assert_mark(&d, "a2", 0, 184.64);
    assert_rect(&d, "e", (0.0, 48.0, 200.0, 48.0));
    assert_mark(&d, "e2", 0, 27.88);
    assert_rect(&d, "f", (0.0, 96.0, 200.0, 48.0));
    assert!(marks(&d, "f2").is_empty());
}
