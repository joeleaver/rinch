//! `text-indent` (#2628), `word-break: break-word` (#2628) and `overflow-wrap`
//! set on an inline span (#2625).
//!
//! - `text-indent` indents the first formatted line of a block container (with
//!   `each-line`, every line after a forced break too; with `hanging`, the
//!   other lines instead). rinch ignored it.
//! - `word-break: break-word` is the legacy spelling of `overflow-wrap:
//!   anywhere` with `word-break: normal`. Stylo's servo build parses it only
//!   under Gecko, so the declaration was dropped and did nothing.
//! - `overflow-wrap` on a span was pushed only as the paragraph's default, so
//!   a span's own value was ignored.
//!
//! Every number is Chrome 157's for the same markup (bundled Inter, 16px/20px,
//! in a 560px column), measured with `getBoundingClientRect` and a `Range`'s
//! client rects. Widths are chosen off round numbers (203, 13%, 37px, -17px):
//! an indent of 0 or a percentage of 0 is where a broken implementation and a
//! correct one agree. `Supercalifragilistic` is 139.70 wide, `alpha` 41.11,
//! `beta` 33.42, `gamma` 55.81, `alpha beta` 79.03.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const WORD: &str = "Supercalifragilistic";
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta";

fn doc_with(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css(CSS);
    if !css.is_empty() {
        d.load_css(css);
    }
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

fn doc(html: &str) -> RinchDocument {
    doc_with("", html)
}

fn node(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

/// The border box size of `#id`.
fn w_h(d: &RinchDocument, id: &str) -> (f32, f32) {
    let l = &d.tree.get(node(d, id).0).unwrap().layout;
    (l.width, l.height)
}

/// The border box of `#id` on the page: x, y, width, height.
fn rect(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = node(d, id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (x, y) = rinch_dom::paint::compute_absolute_position(&d.tree, n.0, 1.0);
    (x as f32, y as f32, l.width, l.height)
}

/// Within half a pixel: Chrome lays out in fractions, rinch rounds.
#[track_caller]
fn near(d: &RinchDocument, id: &str, chrome: (f32, f32)) {
    let got = w_h(d, id);
    assert!(
        (got.0 - chrome.0).abs() <= 0.5 && (got.1 - chrome.1).abs() <= 0.5,
        "#{id}: rinch (w, h) {got:?}, Chrome {chrome:?}"
    );
}

/// Where each line's glyphs start, from the IFC root's content box: the left
/// edge of its leftmost glyph run that is not only white space, as Chrome's
/// `Range` client rects put it. `#id` is the root, or a node in it (an
/// anonymous block box's run).
fn line_lefts(d: &RinchDocument, id: &str) -> Vec<f32> {
    let mut n = d.tree.get(node(d, id).0).unwrap();
    if n.text_layout.is_none() {
        let root = n.ifc_root.expect("in an IFC");
        n = d.tree.get(root).unwrap();
    }
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|line| {
            let mut left = f32::INFINITY;
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                    let run = g.run();
                    let text = &il.text_content[run.text_range()];
                    if text.trim().is_empty() {
                        continue;
                    }
                    // Skip the white space at the visual start of the run.
                    let mut x = g.offset();
                    let clusters: Vec<_> = run.visual_clusters().collect();
                    for c in clusters {
                        let t = &il.text_content[c.text_range()];
                        if t.trim().is_empty() {
                            x += c.advance();
                        } else {
                            break;
                        }
                    }
                    left = left.min(x);
                }
            }
            left
        })
        .collect()
}

/// A text leaf's paint layout (a flex or grid container's own text).
fn leaf_layout(d: &RinchDocument, text: u64) -> &parley::Layout<peniko::Brush> {
    d.tree.nodes[text]
        .cached_text_parley
        .as_deref()
        .expect("the text leaf's layout")
}

#[track_caller]
fn lefts_near(d: &RinchDocument, id: &str, chrome: &[f32]) {
    let got = line_lefts(d, id);
    assert_eq!(
        got.len(),
        chrome.len(),
        "#{id}: rinch lines {got:?}, Chrome {chrome:?}"
    );
    for (g, c) in got.iter().zip(chrome) {
        assert!(
            (g - c).abs() <= 0.5,
            "#{id}: rinch line starts {got:?}, Chrome {chrome:?}"
        );
    }
}

fn indented(style: &str, inner: &str) -> String {
    format!("<div id=t style='width:203px;{style}'>{inner}</div>")
}

// --- text-indent: lengths, percentages, negative values -------------------

/// The first line starts 37px in, and has 37px less room: `alpha beta gamma`
/// (139.34) on it, where 166 is left; the rest wrap at the full width.
#[test]
fn a_length_indents_the_first_line_only() {
    let d = doc(&indented("text-indent:37px", TEXT));
    lefts_near(&d, "t", &[37.0, 0.0, 0.0]);
    near(&d, "t", (203.0, 60.0));
}

/// A percentage is of the block's own content width: 13% of 203 is 26.39.
#[test]
fn a_percentage_is_of_the_content_width() {
    let d = doc(&indented("text-indent:13%", TEXT));
    lefts_near(&d, "t", &[26.38, 0.0, 0.0]);
}

/// `calc()` mixing both: 10% of 203 plus 7px.
#[test]
fn a_calc_mixes_a_length_and_a_percentage() {
    let d = doc(&indented("text-indent:calc(10% + 7px)", TEXT));
    lefts_near(&d, "t", &[27.30, 0.0, 0.0]);
}

/// An `em` is of the element's own font size: 2em at 13px is 26.
#[test]
fn an_em_is_of_the_font_size() {
    let d = doc(&indented("text-indent:2em;font-size:13px", TEXT));
    let first = line_lefts(&d, "t")[0];
    assert!(
        (first - 26.0).abs() <= 0.5,
        "first line at {first}, Chrome 26"
    );
}

/// A negative indent pulls the first line out past the start edge, into the
/// padding, and gives it more room: `alpha beta gamma delta` (181.38) fits.
#[test]
fn a_negative_indent_pulls_the_first_line_out() {
    let d = doc(&indented("text-indent:-23px;padding-left:30px", TEXT));
    lefts_near(&d, "t", &[-23.0, 0.0]);
    near(&d, "t", (233.0, 40.0));
}

// --- each-line and hanging ---------------------------------------------------

/// `hanging` indents every line but the first.
#[test]
fn hanging_indents_the_other_lines() {
    let d = doc(&indented("text-indent:37px hanging", TEXT));
    lefts_near(&d, "t", &[0.0, 37.0]);
    near(&d, "t", (203.0, 40.0));
}

/// Without `each-line` a line after a `<br>` is not indented; with it, it is.
#[test]
fn a_line_after_a_forced_break_is_indented_only_with_each_line() {
    let d = doc(&indented(
        "text-indent:37px",
        "alpha beta<br>gamma delta epsilon zeta eta theta",
    ));
    lefts_near(&d, "t", &[37.0, 0.0, 0.0]);
    let d = doc(&indented(
        "text-indent:37px each-line",
        "alpha beta<br>gamma delta epsilon zeta eta theta",
    ));
    lefts_near(&d, "t", &[37.0, 37.0, 0.0]);
}

// --- Direction and alignment ---------------------------------------------------

/// In a right-to-left block the indent is at the right: the first line ends
/// 37px from the right edge (`alpha beta gamma`, 139.34, starts at 26.66).
#[test]
fn rtl_indents_from_the_right() {
    let d = doc(&format!(
        "<div id=t dir=rtl style='width:203px;text-indent:37px'>{TEXT}</div>"
    ));
    lefts_near(&d, "t", &[26.66, 41.75, 164.67]);
}

/// Centred, the first line is centred in the room the indent leaves.
#[test]
fn a_centred_first_line_is_centred_after_the_indent() {
    let d = doc(&indented("text-indent:23px;text-align:center", TEXT));
    lefts_near(&d, "t", &[43.33, 20.88, 82.33]);
}

/// Right-aligned, the indent only narrows the first line's room.
#[test]
fn a_right_aligned_first_line_ends_at_the_right_edge() {
    let d = doc(&indented("text-indent:23px;text-align:right", TEXT));
    lefts_near(&d, "t", &[63.66, 41.75, 164.67]);
}

/// Justified, the first line starts at the indent.
#[test]
fn a_justified_first_line_spans_from_the_indent() {
    let d = doc(&indented("text-indent:23px;text-align:justify", TEXT));
    lefts_near(&d, "t", &[23.0, 0.0, 0.0]);
}

// --- Intrinsic sizes ---------------------------------------------------------

/// The issue's float: its max-content width counts the indent. It was 140.
#[test]
fn a_floats_width_counts_the_indent() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left;text-indent:20px'>{WORD}</div></div>"
    ));
    near(&d, "f", (159.70, 20.0));
}

/// A negative indent takes its length off the max-content width.
#[test]
fn a_negative_indent_narrows_a_float() {
    let d = doc(
        "<div style='width:300px'><div id=f style='float:left;text-indent:-17px'>alpha beta</div></div>",
    );
    near(&d, "f", (62.03, 20.0));
    // And off the min-content width: `alpha` less 17 is narrower than `beta`.
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:-17px;width:min-content'>alpha beta</div></div>");
    near(&d, "f", (33.42, 40.0));
}

/// A percentage is cyclic in an intrinsic size and counts 0 there; the box
/// then lays its lines out with it resolved against its own width (13% of
/// 79.03 is 10.27), which no longer fits `alpha beta` on one line.
#[test]
fn a_percentage_counts_zero_in_a_floats_width() {
    let d = doc(
        "<div style='width:300px'><div id=f style='float:left;text-indent:13%'>alpha beta</div></div>",
    );
    near(&d, "f", (79.03, 40.0));
    lefts_near(&d, "f", &[10.27, 0.0]);
}

/// The same in an inline-block, which inherits its block's 10%.
#[test]
fn an_inline_block_resolves_an_inherited_percentage_against_itself() {
    let d = doc(
        "<div style='width:300px;text-indent:10%'>x<span id=b style='display:inline-block'>alpha beta</span></div>",
    );
    near(&d, "b", (79.03, 40.0));
    lefts_near(&d, "b", &[7.90, 0.0]);
}

/// The min-content width counts the indent on the first line.
#[test]
fn the_min_content_width_counts_the_first_lines_indent() {
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px;width:min-content'>alpha beta</div></div>");
    near(&d, "f", (58.11, 40.0));
}

/// `hanging` with a forced break: the first line is not indented, the line
/// after the break is (`gamma` + 17 = 72.81), and `each-line` turns that off.
#[test]
fn hanging_min_and_max_content_widths() {
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px hanging'>alpha beta<br>gamma</div></div>");
    near(&d, "f", (79.03, 40.0));
    lefts_near(&d, "f", &[0.0, 17.0]);
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px hanging;width:min-content'>alpha beta<br>gamma</div></div>");
    near(&d, "f", (72.81, 60.0));
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px hanging each-line;width:min-content'>alpha beta<br>gamma</div></div>");
    near(&d, "f", (55.81, 60.0));
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px each-line'>alpha beta<br>gamma</div></div>");
    near(&d, "f", (96.03, 40.0));
}

/// Chrome's min-content width does not count a `hanging` indent on a line
/// after a soft wrap (`beta` + 17 = 50.42 is not the 41.11 it answers), and
/// rinch does: a known difference, kept off the other pins.
#[test]
#[ignore = "known difference: Chrome's min-content width leaves out a hanging indent after a soft wrap"]
fn hanging_indent_after_a_soft_wrap_in_min_content() {
    let d = doc("<div style='width:300px'>\
         <div id=f style='float:left;text-indent:17px hanging;width:min-content'>alpha beta</div></div>");
    near(&d, "f", (41.11, 40.0));
}

// --- Flex and grid items ---------------------------------------------------

/// A flex item's width counts its indent.
#[test]
fn a_flex_items_width_counts_its_indent() {
    let d = doc(
        "<div style='width:300px;display:flex'><div id=i style='text-indent:19px'>alpha beta</div></div>",
    );
    near(&d, "i", (98.03, 20.0));
}

/// A flex container's own text (an anonymous item) inherits the indent and
/// is indented by it.
#[test]
fn a_flex_containers_own_text_is_indented() {
    let d = doc("<div id=c style='width:300px;display:flex;text-indent:19px'>alpha beta</div>");
    let c = d.tree.get(node(&d, "c").0).unwrap();
    let text = c.children[0];
    let (x, _) = rinch_dom::paint::compute_absolute_position(&d.tree, text, 1.0);
    let layout = leaf_layout(&d, text);
    let line = layout.lines().next().unwrap();
    let m = line.metrics();
    let left = x as f32 + m.inline_min_coord + m.offset;
    assert!((left - 19.0).abs() <= 0.5, "text at {left}, Chrome 19");
}

/// A grid container's own text, in a 150px track: 10% is of the item's 150.
#[test]
fn a_grid_items_percentage_is_of_its_track() {
    let d = doc(
        "<div id=c style='width:300px;display:grid;text-indent:10%;grid-template-columns:150px'>\
         alpha beta gamma delta epsilon</div>",
    );
    let c = d.tree.get(node(&d, "c").0).unwrap();
    let text = c.children[0];
    let layout = match d.tree.nodes[text].ifc_root {
        Some(root) => &d.tree.nodes[root].text_layout.as_ref().unwrap().layout,
        None => leaf_layout(&d, text),
    };
    assert_eq!(layout.len(), 3, "three lines");
    let line = layout.lines().next().unwrap();
    let m = line.metrics();
    let left = m.inline_min_coord + m.offset;
    assert!((left - 15.0).abs() <= 0.5, "text at {left}, Chrome 15");
    near(&d, "c", (300.0, 60.0));
}

// --- Floats beside the lines -----------------------------------------------

/// Beside a float the indent is measured from the line's start: 47 + 23.
#[test]
fn the_indent_starts_after_a_float() {
    let d = doc(&indented(
        "text-indent:23px",
        &format!("<div style='float:left;width:47px;height:30px'></div>{TEXT}"),
    ));
    lefts_near(&d, "t", &[70.0, 47.0, 0.0]);
}

/// The same from the right in a right-to-left block.
#[test]
fn rtl_indent_starts_after_a_right_float() {
    let d = doc(&format!(
        "<div id=t dir=rtl style='width:203px;text-indent:23px'>\
         <div style='float:right;width:47px;height:30px'></div>{TEXT}</div>"
    ));
    lefts_near(&d, "t", &[53.97, 58.16, 40.95]);
}

/// A negative indent beside a float pulls the line back over the float.
#[test]
fn a_negative_indent_beside_a_float() {
    let d = doc(&indented(
        "text-indent:-17px",
        &format!("<div style='float:left;width:47px;height:30px'></div>{TEXT}"),
    ));
    lefts_near(&d, "t", &[30.0, 47.0, 0.0]);
}

/// A float met after `alpha ` goes beside the first line when it fits there
/// with the indent (11 + 45.61 + 113 <= 203), and below it when the indent
/// leaves too little room (53 + 45.61 + 113 > 203).
#[test]
fn a_float_in_the_first_line_fits_beside_it_only_with_room_for_the_indent() {
    let markup = |indent: u32| {
        indented(
            &format!("text-indent:{indent}px"),
            "alpha <span id=fl style='float:left;width:113px;height:10px'></span>\
             beta gamma delta epsilon zeta eta theta",
        )
    };
    let d = doc(&markup(11));
    let (t, fl) = (rect(&d, "t"), rect(&d, "fl"));
    assert_eq!(
        (fl.0 - t.0, fl.1 - t.1),
        (0.0, 0.0),
        "indent 11: beside the first line"
    );
    assert_eq!(
        line_lefts(&d, "t")[0].round(),
        124.0,
        "indent 11: alpha after the float"
    );
    let d = doc(&markup(53));
    let (t, fl) = (rect(&d, "t"), rect(&d, "fl"));
    assert_eq!(
        (fl.0 - t.0, fl.1 - t.1),
        (0.0, 20.0),
        "indent 53: below the first line"
    );
    assert_eq!(line_lefts(&d, "t")[0].round(), 53.0, "indent 53");
}

// --- Which lines a block's indent reaches --------------------------------

/// Text after a block child is in an anonymous block that does not hold the
/// block container's first line: not indented. Text before one is.
#[test]
fn only_the_first_formatted_line_is_indented() {
    let d = doc(&indented(
        "text-indent:37px",
        "<span id=before>gamma</span><div>q</div><span id=after>beta</span>",
    ));
    lefts_near(&d, "before", &[37.0]);
    lefts_near(&d, "after", &[0.0]);
    // An empty block child before the text is still a block before it.
    let d = doc(&indented(
        "text-indent:37px",
        "<div></div><span id=after>gamma</span>",
    ));
    lefts_near(&d, "after", &[0.0]);
    // An absolutely positioned child is not: the text is indented.
    let d = doc(&indented(
        "text-indent:37px",
        "<div style='position:absolute'>x</div><span id=after>gamma</span>",
    ));
    lefts_near(&d, "after", &[37.0]);
    // Nor is a float.
    let d = doc(&indented(
        "text-indent:37px",
        "<div style='float:right'>x</div><span id=after>gamma</span>",
    ));
    lefts_near(&d, "after", &[37.0]);
    // Block-in-inline: the text after the block is not indented either.
    let d = doc(&indented(
        "text-indent:37px",
        "<span><span id=before>gamma</span> <div>q</div> <span id=after>delta</span></span>",
    ));
    lefts_near(&d, "before", &[37.0]);
    lefts_near(&d, "after", &[0.0]);
    // A child block inherits the indent and indents its own first line.
    let d = doc(&indented(
        "text-indent:37px",
        &format!("<div id=c>{TEXT}</div>"),
    ));
    lefts_near(&d, "c", &[37.0, 0.0, 0.0]);
}

/// Text after a block child is indented once that child stops being in flow
/// (`display: none`), and not again once it is back.
#[test]
fn the_first_formatted_line_follows_a_block_leaving_the_flow() {
    let mut d = doc(&indented(
        "text-indent:37px",
        "<div id=x>q</div><span id=after>gamma</span><div>z</div>",
    ));
    lefts_near(&d, "after", &[0.0]);
    let x = node(&d, "x");
    d.set_attribute(x, "style", "display:none");
    d.resolve_layout(800.0, 790.0);
    lefts_near(&d, "after", &[37.0]);
    d.set_attribute(x, "style", "float:left;width:10px");
    d.resolve_layout(800.0, 780.0);
    lefts_near(&d, "after", &[47.0]);
    d.set_attribute(x, "style", "");
    d.resolve_layout(800.0, 770.0);
    lefts_near(&d, "after", &[0.0]);
}

/// `text-indent` does not apply to `::first-line` (CSS Pseudo 4 §2.3.1); a
/// rule there changes nothing.
#[test]
fn a_first_line_rule_does_not_change_the_indent() {
    let d = doc_with(
        "#t::first-line { text-indent: 80px; color: red }",
        &indented("text-indent:37px", TEXT),
    );
    lefts_near(&d, "t", &[37.0, 0.0, 0.0]);
}

/// A restyle of `text-indent` alone lays the lines out again.
#[test]
fn a_restyle_of_the_indent_alone_moves_the_lines() {
    let mut d = doc(&indented("text-indent:37px", TEXT));
    lefts_near(&d, "t", &[37.0, 0.0, 0.0]);
    let t = node(&d, "t");
    d.set_attribute(t, "style", "width:203px;text-indent:13%");
    d.resolve_layout(800.0, 800.0);
    lefts_near(&d, "t", &[26.38, 0.0, 0.0]);
}

// --- word-break: break-word ----------------------------------------------

/// The issue's float: `break-word` is `overflow-wrap: anywhere`, which counts
/// in the min-content width, so the float takes the room and breaks the word.
/// It was 140 x 20.
#[test]
fn a_break_word_float_takes_the_room() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left;word-break:break-word'>{WORD}</div></div>"
    ));
    near(&d, "f", (100.0, 40.0));
}

/// In a block of given width the word breaks; from a style sheet with
/// `!important` too, over an inline `word-break: normal`.
#[test]
fn a_break_word_block_breaks_the_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px;word-break:break-word'>{WORD}</div>"
    ));
    near(&d, "b", (100.0, 40.0));
    let d = doc_with(
        ".bw { word-break: break-word !important }",
        &format!("<div id=b class=bw style='width:100px;word-break:normal'>{WORD}</div>"),
    );
    near(&d, "b", (100.0, 40.0));
}

/// Its min-content width is one grapheme at a time: the widest, `S`, 10.27
/// (Chrome 157: 10.27 x 340). rinch lays the box out at 10, a pixel
/// boundary, where fewer pairs of narrow letters share a line (20 lines,
/// not 17) — as it does under `overflow-wrap: anywhere`, which is what this
/// compares with for the height.
#[test]
fn a_break_word_min_content_width_is_the_widest_grapheme() {
    let markup = |style: &str| {
        format!(
            "<div style='width:300px'><div id=f style='float:left;width:min-content;{style}'>{WORD}</div></div>"
        )
    };
    let d = doc(&markup("word-break:break-word"));
    let anywhere = doc(&markup("overflow-wrap:anywhere"));
    let (w, h) = w_h(&d, "f");
    assert!((w - 10.27).abs() <= 0.5, "width {w}, Chrome 10.27");
    assert_eq!((w, h), w_h(&anywhere, "f"), "as overflow-wrap: anywhere");
    assert!(h > 100.0, "one grapheme or two a line: {h}");
}

/// It wins over the element's own `overflow-wrap: normal`, and over a span's.
#[test]
fn break_word_wins_over_overflow_wrap_normal() {
    let d = doc(&format!(
        "<div id=b style='width:100px;word-break:break-word'>{WORD} <span style='overflow-wrap:normal'>{WORD}</span></div>"
    ));
    near(&d, "b", (100.0, 80.0));
}

/// A span's own `word-break` replaces the inherited `break-word`: `normal`
/// and `keep-all` keep the word whole.
#[test]
fn a_spans_own_word_break_replaces_break_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px;word-break:break-word'>{WORD} <span style='word-break:normal'>{WORD}</span></div>"
    ));
    near(&d, "b", (100.0, 60.0));
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left;word-break:break-word'><span style='word-break:keep-all'>{WORD}</span></div></div>"
    ));
    near(&d, "f", (139.70, 20.0));
}

/// `break-word` on a span breaks the span's word only.
#[test]
fn a_break_word_span_breaks_its_own_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px'>{WORD} <span style='word-break:break-word'>{WORD}</span></div>"
    ));
    near(&d, "b", (100.0, 60.0));
}

/// A flex item's text, and a flex container's own text (a text leaf).
#[test]
fn break_word_in_flex_items() {
    let d = doc(&format!(
        "<div style='width:100px;display:flex'><div id=i style='word-break:break-word'>{WORD}</div></div>"
    ));
    near(&d, "i", (100.0, 40.0));
    let d = doc(&format!(
        "<div id=c style='width:100px;display:flex;word-break:break-word'>{WORD}</div>"
    ));
    near(&d, "c", (100.0, 40.0));
}

/// `nowrap` keeps the word whole whatever `word-break` says.
#[test]
fn break_word_does_not_wrap_nowrap_text() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left;word-break:break-word;white-space:nowrap'>{WORD}</div></div>"
    ));
    near(&d, "f", (139.70, 20.0));
}

// --- overflow-wrap on a span (#2625) ---------------------------------------

/// The issue's block: a `break-word` span breaks its word. It was 100 x 20.
#[test]
fn a_break_word_span_breaks_its_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px'><span style='overflow-wrap:break-word'>{WORD}</span></div>"
    ));
    near(&d, "b", (100.0, 40.0));
}

#[test]
fn an_anywhere_span_breaks_its_word() {
    let d = doc(&format!(
        "<div id=b style='width:100px'><span style='overflow-wrap:anywhere'>{WORD}</span></div>"
    ));
    near(&d, "b", (100.0, 40.0));
}

/// In a float, the span's `anywhere` counts in the float's min-content
/// width: the float takes the room.
#[test]
fn an_anywhere_span_in_a_float_counts_in_its_width() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left'><span style='overflow-wrap:anywhere'>{WORD}</span></div></div>"
    ));
    near(&d, "f", (100.0, 40.0));
}

/// Mixed spans: the `anywhere` span's word breaks, the plain word after it
/// overflows on a line of its own (`aaa`, two broken lines, the word).
#[test]
fn only_the_spans_word_breaks() {
    let d = doc(&format!(
        "<div id=b style='width:100px'>aaa <span style='overflow-wrap:anywhere'>{WORD}</span> {WORD}</div>"
    ));
    near(&d, "b", (100.0, 80.0));
}

/// The other way round: a `normal` span in an `anywhere` float keeps its
/// word whole, so the float is the word's width; the float's own word,
/// before it, breaks at nothing then (the line is as wide as it).
#[test]
fn a_normal_span_in_an_anywhere_float_keeps_its_word() {
    let d = doc(&format!(
        "<div style='width:100px'><div id=f style='float:left;overflow-wrap:anywhere'>{WORD} <span style='overflow-wrap:normal'>{WORD}</span></div></div>"
    ));
    near(&d, "f", (139.70, 40.0));
}

/// A word whose second half is a `break-word` span breaks inside the span.
#[test]
fn a_word_breaks_inside_its_break_word_part() {
    let d = doc(
        "<div id=b style='width:100px'>Supercali<span style='overflow-wrap:break-word'>fragilistic</span></div>",
    );
    near(&d, "b", (100.0, 40.0));
}

/// Chrome also breaks a word at the end of its `break-word` part, before the
/// plain rest (`Supercali` / `fragilistic`); parley offers an emergency break
/// only before a character whose own style allows it. A known difference.
#[test]
#[ignore = "known difference: no emergency break after the end of a break-word span"]
fn a_word_breaks_after_its_break_word_part() {
    let d = doc(
        "<div id=b style='width:100px'><span style='overflow-wrap:break-word'>Supercali</span>fragilistic</div>",
    );
    near(&d, "b", (100.0, 40.0));
}
