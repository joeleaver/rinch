//! Soft wrap opportunities next to atomic inlines follow `white-space`
//! (#1238).
//!
//! A `nowrap`/`pre` IFC root was broken unconstrained unless some *text* in
//! it belonged to a wrapping element (`wraps_anywhere`, #1212), so a
//! `white-space: normal` span holding only inline-blocks never wrapped. And
//! where the paragraph was broken at a width, parley 0.11.1 allows a break
//! after every inline box and before one whenever the text before it wraps,
//! whatever the `white-space` around the box: a chip in `nowrap` text wrapped
//! after itself, and boxes in a `nowrap` span inside a wrapping root wrapped
//! between each other.
//!
//! Chrome 155 decides the opportunity after a box (or a character) by the
//! `white-space` of the element the box is in, or of any element whose end
//! lies between it and what follows: `aa<span normal>[1][2][3]</span>cc` in a
//! `nowrap` root breaks after `[1]` and after `[3]` but not between `aa` and
//! `[1]`; `[1]` then `<span nowrap>[2][3]</span>` then `[4]` in a `normal`
//! span breaks after `[3]`. An inline-block's own `white-space` (which styles
//! its contents) plays no part.
//!
//! Every expectation is Chrome 155's on the same markup (bundled Inter as
//! `ProbeFace`, 16px/20px), read item by item: each line is its characters
//! and `[id]` for each 30x10 inline-block, with the spaces at its ends left
//! out (Chrome's per-character rects put a hanging space on either line).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d
}

/// A 30x10 inline-block with `id`.
fn b(id: &str) -> String {
    format!("<b id=\"{id}\" style=\"display:inline-block;width:30px;height:10px\"></b>")
}

/// The same, with its own `white-space: nowrap`.
fn b_nowrap(id: &str) -> String {
    format!(
        "<b id=\"{id}\" style=\"display:inline-block;width:30px;height:10px;white-space:nowrap\"></b>"
    )
}

fn normal(inner: &str) -> String {
    format!("<span style=\"white-space:normal\">{inner}</span>")
}

fn nowrap(inner: &str) -> String {
    format!("<span style=\"white-space:nowrap\">{inner}</span>")
}

fn bbb() -> String {
    format!("{}{}{}", b("1"), b("2"), b("3"))
}

/// One laid-out paragraph: its lines (characters, `_` for a space, `[id]`
/// for a box, a newline left out, in visual order, the spaces at either end left out), each
/// line's offset, and the root's width. (Heights are not compared: a line
/// holding only boxes is their height in rinch and a full strut line in
/// Chrome, #624/#1258.)
struct Laid {
    lines: Vec<String>,
    /// The lines with the spaces at their ends.
    raw: Vec<String>,
    offsets: Vec<f32>,
    width: f32,
}

/// `<div style="font:16px/20px ProbeFace;{style}">{inner}</div>`, laid out.
fn lay_out(style: &str, inner: &str) -> Laid {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font:16px/20px ProbeFace;{style}"));
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    read(&d, c)
}

fn read(d: &RinchDocument, c: NodeId) -> Laid {
    use parley::PositionedLayoutItem;
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    let mut lines = Vec::new();
    let mut raw = Vec::new();
    let mut offsets = Vec::new();
    for line in il.layout.lines() {
        let mut s = String::new();
        // A shaped run is drawn as one glyph run per style in it: read
        // each run's clusters once.
        let mut last_run = None;
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(g) => {
                    let range = g.run().text_range();
                    if last_run.replace(range.clone()) == Some(range) {
                        continue;
                    }
                    for cluster in g.run().clusters() {
                        let t = &il.text_content[cluster.text_range()];
                        s.push_str(&t.replace(' ', "_").replace('\n', ""));
                    }
                }
                PositionedLayoutItem::InlineBox(ib) => {
                    last_run = None;
                    if let Some(id) = d.get_attribute(NodeId(ib.id as usize), "id") {
                        s.push_str(&format!("[{id}]"));
                    }
                }
            }
        }
        lines.push(s.trim_matches('_').to_string());
        raw.push(s);
        offsets.push(line.metrics().offset);
    }
    Laid {
        lines,
        raw,
        offsets,
        width: n.layout.width,
    }
}

fn check(name: &str, style: &str, inner: &str, want: &[&str]) -> Laid {
    let laid = lay_out(style, inner);
    assert_eq!(laid.lines, want, "{name}: {style} / {inner}");
    laid
}

fn near(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 0.05, "{what}: {got} (Chrome {want})");
}

/// The issue (review of #1228, c8): a `normal` span holding only
/// inline-blocks wraps between them inside a `nowrap` or `pre` root, at
/// two widths. rinch kept each on one line.
#[test]
fn a_wrapping_span_of_inline_blocks_wraps_in_a_nowrap_or_pre_root() {
    for root in ["white-space:nowrap", "white-space:pre"] {
        check(
            "k1/k2",
            &format!("{root};width:70px"),
            &normal(&bbb()),
            &["[1][2]", "[3]"],
        );
    }
    check(
        "k3",
        "white-space:nowrap;width:50px",
        &normal(&bbb()),
        &["[1]", "[2]", "[3]"],
    );
}

/// The root's own `nowrap` text glues to the box after it, but the span's
/// last box may still wrap before the root's text after it: there is an
/// opportunity after a box whose element wraps.
#[test]
fn the_roots_text_glues_to_a_span_box_after_it_not_before_it() {
    let cases: &[(&str, &str, String, &[&str])] = &[
        (
            "k4",
            "white-space:nowrap;width:70px",
            format!("aa{}cc", normal(&bbb())),
            &["aa[1]", "[2][3]", "cc"],
        ),
        (
            "k20",
            "white-space:nowrap;width:70px",
            format!("aa{}", normal(&format!("{}{}", b("1"), b("2")))),
            &["aa[1]", "[2]"],
        ),
        (
            "k32",
            "white-space:nowrap;width:70px",
            format!("a b c{}", normal(&format!("{}{}", b("1"), b("2")))),
            &["a_b_c[1]", "[2]"],
        ),
        (
            "k17",
            "white-space:nowrap;width:70px",
            format!("{}{}{}", normal("xx"), b("1"), b("2")),
            &["xx", "[1][2]"],
        ),
        (
            "k18",
            "white-space:nowrap;width:70px",
            format!("{}{}{}", b("1"), normal("xx"), b("2")),
            &["[1]xx", "[2]"],
        ),
        (
            "k5",
            "white-space:nowrap;width:70px",
            format!(
                "{}{}{}",
                b("0"),
                normal(&format!("{}{}", b("1"), b("2"))),
                b("3")
            ),
            &["[0][1]", "[2][3]"],
        ),
        (
            "k30",
            "white-space:nowrap;width:60px",
            normal(&format!("aa{}bb{}cc{}", b("1"), b("2"), b("3"))),
            &["aa[1]", "bb[2]", "cc[3]"],
        ),
    ];
    for (name, style, inner, want) in cases {
        check(name, style, inner, want);
    }
}

/// Where an element ends between two boxes, the opportunity is the
/// wrapping ancestor's: `[1] <nowrap>[2][3]</nowrap> [4]` in a `normal`
/// span breaks after `[1]` and after `[3]`, and three sibling `normal`
/// spans break between each other.
#[test]
fn an_element_ending_between_two_boxes_lends_its_parent_s_opportunity() {
    check(
        "k9",
        "white-space:nowrap;width:70px",
        &normal(&format!(
            "{}{}{}",
            b("1"),
            nowrap(&format!("{}{}", b("2"), b("3"))),
            b("4")
        )),
        &["[1]", "[2][3]", "[4]"],
    );
    check(
        "k10",
        "white-space:nowrap;width:70px",
        &format!("{}{}{}", normal(&b("1")), normal(&b("2")), normal(&b("3"))),
        &["[1][2]", "[3]"],
    );
    // Text of a `nowrap` span that ends inside a `normal` one may wrap
    // before the box after it; in k39 the box's own `nowrap` text then
    // overflows up to the root's newline.
    check(
        "k40",
        "white-space:nowrap;width:40px",
        &normal(&format!("{}{}", nowrap("aa"), b("1"))),
        &["aa", "[1]"],
    );
    check(
        "k39",
        "white-space:pre;width:40px",
        &format!(
            "{}\nyy",
            normal(&format!(
                "{}{}",
                nowrap("aa"),
                nowrap(&format!("{}xx", b("1")))
            ))
        ),
        &["aa", "[1]xx", "yy"],
    );
    check(
        "k24",
        "white-space:normal;width:70px",
        &format!("aa{}{}{}", nowrap(&b("1")), b("2"), b("3")),
        &["aa[1]", "[2][3]"],
    );
}

/// The hazard #1237 pinned for an unwrapped paragraph, in one that is
/// broken at a width: a chip in the `nowrap` root's text does not wrap
/// after itself. rinch put `y` (and `z`) on a line of its own.
#[test]
fn a_chip_in_nowrap_text_does_not_wrap_after_itself() {
    check(
        "k6c",
        "white-space:nowrap;width:40px",
        &format!("{}x{}y", normal("bbb ccc "), b("1")),
        &["bbb", "ccc", "x[1]y"],
    );
    check(
        "k6e",
        "white-space:nowrap;width:60px",
        &format!("{}x{}y{}z", normal("aa bb "), b("1"), b("2")),
        &["aa_bb", "x[1]y[2]z"],
    );
}

/// Boxes in a `nowrap` span inside a wrapping root stay together.
#[test]
fn boxes_in_a_nowrap_span_stay_together_in_a_wrapping_root() {
    let cases: &[(&str, &str, String, &[&str])] = &[
        (
            "k7",
            "white-space:normal;width:70px",
            format!("aa {}", nowrap(&bbb())),
            &["aa", "[1][2][3]"],
        ),
        (
            "k16",
            "white-space:pre-wrap;width:70px",
            format!("aa {}", nowrap(&bbb())),
            &["aa", "[1][2][3]"],
        ),
        (
            "k15",
            "white-space:normal;width:70px",
            format!("{} cc", nowrap(&format!("aa{}bb", b("1")))),
            &["aa[1]bb", "cc"],
        ),
        (
            "k36",
            "white-space:normal;width:70px",
            format!("aa {} cc", nowrap(&format!("bb{}{}", b("1"), b("2")))),
            &["aa", "bb[1][2]", "cc"],
        ),
    ];
    for (name, style, inner, want) in cases {
        check(name, style, inner, want);
    }
}

/// With padding on the spans (their edge boxes, #1426), the same breaks.
#[test]
fn padded_spans_break_the_same_way() {
    check(
        "k25",
        "white-space:nowrap;width:70px",
        &format!(
            "aa<span style=\"white-space:normal;padding:0 5px\">{}</span>cc",
            bbb()
        ),
        &["aa[1]", "[2][3]", "cc"],
    );
    let laid = check(
        "k26",
        "white-space:normal;width:70px",
        &format!(
            "aa <span style=\"white-space:nowrap;padding:0 5px\">{}</span>",
            bbb()
        ),
        &["aa", "[1][2][3]"],
    );
    assert_eq!(laid.offsets[1], 0.0, "an overflowing line starts at 0");
}

/// A `pre` root: its forced break and its preserved spaces between two
/// wrapping spans.
#[test]
fn a_pre_roots_forced_break_and_spaces_between_wrapping_spans() {
    check(
        "k28",
        "white-space:pre;width:70px",
        &format!(
            "{}\n{}",
            normal(&bbb()),
            normal(&format!("{}{}{}", b("4"), b("5"), b("6")))
        ),
        &["[1][2]", "[3]", "[4][5]", "[6]"],
    );
    // The root's `xx` overflows before its newline: the line ends after the
    // span's last box, not at the newline.
    check(
        "k38",
        "white-space:pre;width:70px",
        &format!("{}xx\nyy", normal(&format!("{}{}", b("1"), b("2")))),
        &["[1][2]", "xx", "yy"],
    );
    // The root's `pre` spaces start the second line.
    let laid = check(
        "k22",
        "white-space:pre;width:70px",
        &format!(
            "{}  {}",
            normal(&format!("{}{}", b("1"), b("2"))),
            normal(&format!("{}{}", b("3"), b("4")))
        ),
        &["[1][2]", "[3][4]"],
    );
    assert_eq!(laid.raw[1], "__[3][4]", "k22");
}

/// The wrapped lines are aligned within the box.
#[test]
fn the_wrapped_lines_are_aligned_within_the_box() {
    let laid = check(
        "k14",
        "white-space:nowrap;width:70px;text-align:right",
        &normal(&bbb()),
        &["[1][2]", "[3]"],
    );
    near(laid.offsets[0], 10.0, "k14 line 1");
    near(laid.offsets[1], 40.0, "k14 line 2");
    let laid = check(
        "k21",
        "white-space:nowrap;width:70px;text-align:center",
        &normal(&bbb()),
        &["[1][2]", "[3]"],
    );
    near(laid.offsets[0], 5.0, "k21 line 1");
    near(laid.offsets[1], 20.0, "k21 line 2");
    let laid = check(
        "k37",
        "white-space:nowrap;width:70px;text-align:right",
        &format!("aa{}", normal(&format!("{}{}", b("1"), b("2")))),
        &["aa[1]", "[2]"],
    );
    near(laid.offsets[0], 22.03, "k37 line 1");
    near(laid.offsets[1], 40.0, "k37 line 2");
}

/// The box's min-content width counts the opportunities between the
/// span's boxes: `width: min-content` is one box wide.
#[test]
fn min_content_counts_the_opportunities_between_boxes() {
    let laid = check(
        "k33",
        "white-space:nowrap;width:min-content",
        &normal(&bbb()),
        &["[1]", "[2]", "[3]"],
    );
    assert_eq!(laid.width, 30.0, "k33 width");
    let laid = check(
        "k35",
        "white-space:nowrap;width:min-content",
        &format!("{}aa", normal(&bbb())),
        &["[1]", "[2]", "[3]", "aa"],
    );
    assert_eq!(laid.width, 30.0, "k35 width");
    check(
        "k34",
        "white-space:nowrap;width:min-content",
        &format!("aa{}", normal(&bbb())),
        &["aa[1]", "[2]", "[3]"],
    );
}

/// The inline-block's own `white-space` (which styles its contents) plays
/// no part, and spaces or text in the span break as usual. rinch already
/// matched k12, k8, k13 and k23 (the paragraph was broken at a width there,
/// and parley's breaks happened to be Chrome's); k11 and k19 stayed on one
/// line.
#[test]
fn the_boxes_own_white_space_and_the_spans_spaces_and_text() {
    let own = format!("{}{}{}", b_nowrap("1"), b_nowrap("2"), b_nowrap("3"));
    check(
        "k11",
        "white-space:nowrap;width:70px",
        &normal(&own),
        &["[1][2]", "[3]"],
    );
    check(
        "k12",
        "white-space:normal;width:70px",
        &own,
        &["[1][2]", "[3]"],
    );
    check(
        "k8",
        "white-space:nowrap;width:70px",
        &normal(&format!("{} {} {}", b("1"), b("2"), b("3"))),
        &["[1]_[2]", "[3]"],
    );
    check(
        "k13",
        "white-space:nowrap;width:70px",
        &normal(&format!("aa {}", bbb())),
        &["aa_[1]", "[2][3]"],
    );
    check(
        "k19",
        "white-space:nowrap;width:70px",
        &format!("{}aa", normal(&bbb())),
        &["[1][2]", "[3]aa"],
    );
    check(
        "k23",
        "white-space:nowrap;width:70px",
        &format!("{}{}", normal("aa"), normal(&bbb())),
        &["aa[1]", "[2][3]"],
    );
}
