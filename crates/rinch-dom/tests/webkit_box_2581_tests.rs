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
    assert!(ok, "#{name}: got {got:?}, Chrome {want:?}");
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

// The rest are Chrome 158's numbers on the same markup (the issue's comment
// after #2635). `.wp` boxes are 237px wide; an `<i>` is a 20px-tall block.
const WP: &str = ".wp { display: -webkit-box; width: 237px; } i { display: block; height: 20px; }";

/// A `max-height` or `min-height` on a box between the clamp and its last
/// line holds that box, and the clamp ends after it: a 30px wrapper around
/// the clamped paragraph makes the clamp box 54 (24 + 30, rinch had 72), a
/// `min-height: 90px` one 114, and what follows the wrapper flows on below
/// it.
#[test]
fn a_min_or_max_height_in_the_clamp_chain_holds_issue_2581() {
    const TWELVE: &str = "one two three four five six seven eight nine ten eleven twelve";
    let three = "display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 3";
    let d = page(
        "",
        &format!(
            "<div id=m class=c style=\"{three}\"><p>p1</p>\
             <div id=mw style=\"max-height: 30px\"><p id=m2>{TWELVE}</p></div>\
             <p id=m3>after</p></div>\
             <div id=n class=c style=\"{three}\"><p>p1</p>\
             <div id=nw style=\"min-height: 90px\"><p>{TWELVE}</p></div><p>after</p></div>\
             <div id=q class=c style=\"{three}\"><p>p1</p>\
             <div id=qw style=\"max-height: 30px; padding: 5px 0\"><p>{TWELVE}</p></div>\
             <p id=q3>after</p></div>\
             <div id=r class=c style=\"{three}\"><p>p1</p>\
             <div id=rw style=\"max-height: 30px; padding: 5px 0; box-sizing: border-box\">\
             <p>{TWELVE}</p></div><p>after</p></div>"
        ),
    );
    assert_rect(&d, "m", (0.0, 0.0, 200.0, 54.0));
    assert_rect(&d, "mw", (0.0, 24.0, 200.0, 30.0));
    assert_rect(&d, "m3", (0.0, 54.0, 200.0, 24.0));
    assert_rect(&d, "n", (0.0, 54.0, 200.0, 114.0));
    assert_rect(&d, "nw", (0.0, 78.0, 200.0, 90.0));
    // With padding, the bound is the content box's (40 tall) unless
    // `box-sizing: border-box` (30).
    assert_rect(&d, "q", (0.0, 168.0, 200.0, 64.0));
    assert_rect(&d, "qw", (0.0, 192.0, 200.0, 40.0));
    assert_rect(&d, "q3", (0.0, 232.0, 200.0, 24.0));
    assert_rect(&d, "r", (0.0, 232.0, 200.0, 54.0));
    assert_rect(&d, "rw", (0.0, 256.0, 200.0, 30.0));
}

/// `-webkit-box-pack` places the items on the main axis: `center` (43 and
/// 61 in 237: 66.5 and 109.5), `end`, `justify` (the middle one at 99.5),
/// and `center` around an item wider than the box (300: at -31.5,
/// overflowing both sides). `justify-content` does nothing on a legacy box.
/// rinch's flex items are on whole pixels, so its halves are 67, 110, 100
/// and -31.
#[test]
fn box_pack_places_the_items_issue_2581() {
    let d = page(
        WP,
        "<div class=wp style=\"-webkit-box-pack: center\"><i id=c1 style=\"width: 43px\"></i>\
         <i id=c2 style=\"width: 61px\"></i></div>\
         <div class=wp style=\"-webkit-box-pack: end\"><i id=e1 style=\"width: 43px\"></i>\
         <i id=e2 style=\"width: 61px\"></i></div>\
         <div class=wp style=\"-webkit-box-pack: justify\"><i id=j1 style=\"width: 43px\"></i>\
         <i id=j2 style=\"width: 61px\"></i><i id=j3 style=\"width: 20px\"></i></div>\
         <div class=wp style=\"-webkit-box-pack: center\"><i id=o1 style=\"width: 300px\"></i></div>\
         <div class=wp style=\"justify-content: flex-end\"><i id=f1 style=\"width: 43px\"></i></div>",
    );
    assert_rect(&d, "c1", (67.0, 0.0, 43.0, 20.0));
    assert_rect(&d, "c2", (110.0, 0.0, 61.0, 20.0));
    assert_rect(&d, "e1", (133.0, 20.0, 43.0, 20.0));
    assert_rect(&d, "e2", (176.0, 20.0, 61.0, 20.0));
    assert_rect(&d, "j1", (0.0, 40.0, 43.0, 20.0));
    assert_rect(&d, "j2", (100.0, 40.0, 61.0, 20.0));
    assert_rect(&d, "j3", (217.0, 40.0, 20.0, 20.0));
    assert_rect(&d, "o1", (-31.0, 60.0, 300.0, 20.0));
    assert_rect(&d, "f1", (0.0, 80.0, 43.0, 20.0));
}

/// `-webkit-box-align` aligns the items on the cross axis of a 70px row,
/// in place of `align-items` and each item's `align-self`, which do
/// nothing: `stretch` (the initial value) makes an auto-height item 70 tall
/// whatever `align-items` or `align-self` says; `end` puts a 20px item at
/// 50 though it says `align-self: flex-start`; `start` and `baseline` leave
/// an auto-height item 0 tall; `center` puts a 20px item at 25.
#[test]
fn box_align_aligns_the_items_issue_2581() {
    let d = page(
        ".wp { display: -webkit-box; width: 237px; height: 70px; } i { display: block; }",
        "<div class=wp style=\"align-items: center\"><i id=x1 style=\"width: 43px\"></i></div>\
         <div class=wp><i id=x2 style=\"width: 43px; align-self: center\"></i></div>\
         <div class=wp style=\"-webkit-box-align: end\">\
         <i id=x3 style=\"width: 43px; height: 20px; align-self: flex-start\"></i></div>\
         <div class=wp style=\"-webkit-box-align: start\"><i id=x4 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"-webkit-box-align: baseline\"><i id=x5 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"-webkit-box-align: center\">\
         <i id=x6 style=\"width: 43px; height: 20px; align-self: flex-end\"></i></div>",
    );
    assert_rect(&d, "x1", (0.0, 0.0, 43.0, 70.0));
    assert_rect(&d, "x2", (0.0, 70.0, 43.0, 70.0));
    assert_rect(&d, "x3", (0.0, 190.0, 43.0, 20.0));
    assert_rect(&d, "x4", (0.0, 210.0, 43.0, 0.0));
    assert_rect(&d, "x5", (0.0, 280.0, 43.0, 0.0));
    assert_rect(&d, "x6", (0.0, 375.0, 43.0, 20.0));
}

/// `-webkit-box-direction: reverse` lays the items out from the main axis's
/// end (the first at 194, the second before it at 133), `pack: start` at
/// that end and `pack: end` at the other; a vertical one from the bottom of
/// its 100px. In a column, `pack` is vertical (`center`: 40 down) and
/// `align` horizontal (`center`: 97). `flex-direction` does nothing.
#[test]
fn box_direction_and_a_column_issue_2581() {
    let d = page(
        WP,
        "<div class=wp style=\"-webkit-box-direction: reverse\"><i id=r1 style=\"width: 43px\"></i>\
         <i id=r2 style=\"width: 61px\"></i></div>\
         <div class=wp style=\"-webkit-box-direction: reverse; -webkit-box-pack: start\">\
         <i id=s1 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"-webkit-box-direction: reverse; -webkit-box-pack: end\">\
         <i id=t1 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"-webkit-box-direction: reverse; -webkit-box-orient: vertical; \
         height: 100px\"><i id=v1 style=\"width: 43px\"></i><i id=v2 style=\"width: 61px\"></i></div>\
         <div class=wp style=\"-webkit-box-orient: vertical; -webkit-box-pack: center; \
         height: 100px\"><i id=p1 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"-webkit-box-orient: vertical; -webkit-box-align: center\">\
         <i id=a1 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"flex-direction: row-reverse\"><i id=f1 style=\"width: 43px\"></i></div>",
    );
    assert_rect(&d, "r1", (194.0, 0.0, 43.0, 20.0));
    assert_rect(&d, "r2", (133.0, 0.0, 61.0, 20.0));
    assert_rect(&d, "s1", (194.0, 20.0, 43.0, 20.0));
    assert_rect(&d, "t1", (0.0, 40.0, 43.0, 20.0));
    assert_rect(&d, "v1", (0.0, 140.0, 43.0, 20.0));
    assert_rect(&d, "v2", (0.0, 120.0, 61.0, 20.0));
    assert_rect(&d, "p1", (0.0, 200.0, 43.0, 20.0));
    assert_rect(&d, "a1", (97.0, 260.0, 43.0, 20.0));
    assert_rect(&d, "f1", (0.0, 280.0, 43.0, 20.0));
}

/// The three go through the cascade as Chrome's do: an invalid value is
/// dropped (the `center` before it stands), a `var()` is substituted,
/// `inherit` takes the parent's, and none is inherited otherwise.
#[test]
fn box_pack_through_the_cascade_issue_2581() {
    let d = page(
        WP,
        "<div class=wp style=\"-webkit-box-pack: center; -webkit-box-pack: bogus\">\
         <i id=b1 style=\"width: 43px\"></i></div>\
         <div class=wp style=\"--p: end; -webkit-box-pack: var(--p)\">\
         <i id=v1 style=\"width: 43px\"></i></div>\
         <div style=\"-webkit-box-pack: end\"><div class=wp style=\"-webkit-box-pack: inherit\">\
         <i id=h1 style=\"width: 43px\"></i></div></div>\
         <div style=\"-webkit-box-pack: end\"><div class=wp><i id=n1 style=\"width: 43px\"></i>\
         </div></div>",
    );
    assert_rect(&d, "b1", (97.0, 0.0, 43.0, 20.0));
    assert_rect(&d, "v1", (194.0, 20.0, 43.0, 20.0));
    assert_rect(&d, "h1", (194.0, 40.0, 43.0, 20.0));
    assert_rect(&d, "n1", (0.0, 60.0, 43.0, 20.0));
}
