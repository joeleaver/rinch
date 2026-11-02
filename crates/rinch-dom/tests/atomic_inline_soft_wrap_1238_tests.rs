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
                    if let Some(id) = d.get_attribute(NodeId(ib.id), "id") {
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

/// A `w`px-wide inline-block with `id`.
fn bw(id: &str, w: f32) -> String {
    format!("<b id=\"{id}\" style=\"display:inline-block;width:{w}px;height:10px\"></b>")
}

/// Review of #1878, class A: a space at the end of a `nowrap`/`pre`
/// element. Chrome puts the opportunity of a run of spaces before the run,
/// so it is the white space of the text before the spaces that decides it,
/// and that text does not wrap: no break between the spaces and what
/// follows, even though the element closes into a wrapping one.
#[test]
fn a_space_ending_a_nowrap_element_offers_no_wrap() {
    let cases: &[(&str, &str, String, &[&str])] = &[
        (
            "v1",
            "white-space:normal;width:55px",
            format!("{}{}", nowrap("abc def "), bw("B3", 60.0)),
            &["abc_def_[B3]"],
        ),
        (
            "v2",
            "white-space:normal;width:55px",
            format!(
                "<span style=\"white-space:pre\">abc def </span>{}",
                bw("B3", 60.0)
            ),
            &["abc_def_[B3]"],
        ),
        (
            "v3",
            "white-space:normal;width:55px",
            format!("{}xyz{}", nowrap("abc def "), bw("B3", 10.0)),
            &["abc_def_xyz", "[B3]"],
        ),
        (
            "v4",
            "white-space:normal;width:55px",
            format!(
                "{}{}",
                nowrap(&format!("{} ", bw("B2", 45.0))),
                bw("B3", 30.0)
            ),
            &["[B2]_[B3]"],
        ),
        (
            "v7",
            "white-space:normal;width:120px",
            format!(
                "lorem ipsum {}{} amet",
                nowrap("dolor sit "),
                bw("B3", 60.0)
            ),
            &["lorem_ipsum", "dolor_sit_[B3]", "amet"],
        ),
    ];
    for (name, style, inner, want) in cases {
        check(name, style, inner, want);
    }
}

/// Review of #1878, class B: the root's `nowrap` spaces after a wrapping
/// span's text. The opportunity before them is the span's, as parley has
/// it: `aaddddhello` ends the first line.
#[test]
fn nowrap_spaces_after_wrapping_text_still_break_before_them() {
    for root in ["white-space:nowrap", "white-space:pre"] {
        for w in [55, 80] {
            check(
                "g1/g3",
                &format!("{root};width:{w}px"),
                &format!("aa{} ccc ccc{}", normal("ddddhello"), b("B2")),
                &["aaddddhello", "ccc_ccc[B2]"],
            );
        }
    }
}

/// Review of #1878, class C: a `pre-wrap` space hangs at a forced break
/// even with the span's end padding after it: `[B1]q-r` fits 71.5px. A
/// `break-spaces` space does not hang, and the line breaks after `q-`.
#[test]
fn a_hanging_space_before_end_padding_and_a_forced_break_still_hangs() {
    let inner = format!(
        "<span style=\"padding:0 3px\">{}q-r </span><br>xy",
        bw("B1", 45.0)
    );
    check(
        "y2_71.5",
        "white-space:pre-wrap;width:71.5px",
        &inner,
        &["[B1]q-r", "xy"],
    );
    check(
        "y1_71.5",
        "white-space:pre-wrap;width:71.5px",
        &format!(
            "<span style=\"padding:0 3px\">{}q-r </span>\nxy",
            bw("B1", 45.0)
        ),
        &["[B1]q-r", "xy"],
    );
    check(
        "y3_71.5",
        "white-space:break-spaces;width:71.5px",
        &inner,
        &["[B1]q-", "r", "xy"],
    );
}

/// Review of #1878: a re-broken line asks the same segmenter parley does
/// (with the complex-script dictionaries), so there is no break between
/// the end of a Thai or Khmer run and the Latin letter after it (Chrome 155
/// has none either). Only whether `w` starts a line is compared, which no
/// host font changes: everything before it overflows 55px.
#[test]
fn no_break_between_a_complex_script_run_and_the_latin_after_it() {
    for (name, text) in [("t1", "ภาษาไทยง่ายมาก"), ("t4", "ខ្មែរភាសា")]
    {
        let laid = lay_out(
            "white-space:normal;width:55px",
            &format!("{}w", nowrap(&format!("{}{text}", bw("B1", 20.0)))),
        );
        assert_eq!(laid.lines.len(), 1, "{name}: {:?}", laid.lines);
    }
}

/// A `nowrap` root whose only wrapping element holds no text of its own
/// (its text is in a `nowrap` span inside it) still breaks at the width:
/// the span closes into the `normal` one before the box (Chrome 155).
#[test]
fn a_wrapping_element_around_nowrap_text_reaches_the_width() {
    check(
        "o1",
        "white-space:nowrap;width:40px",
        &format!("{}{}", normal(&nowrap("xx")), b("B1")),
        &["xx", "[B1]"],
    );
}

/// A box wider than the line at its start overflows, and what is glued to
/// it stays with it: parley's emergency break after the box is not taken
/// inside `nowrap` content (Chrome 155: `[B1]cc` on one line).
#[test]
fn a_too_wide_box_keeps_the_nowrap_text_after_it() {
    check(
        "o4",
        "white-space:normal;width:60px",
        &format!("aa {} dd", nowrap(&format!("{}cc", bw("B1", 80.0)))),
        &["aa", "[B1]cc", "dd"],
    );
    check(
        "o3",
        "white-space:nowrap;width:60px",
        &format!("aa{}cc", normal(&bw("B1", 80.0))),
        &["aa[B1]", "cc"],
    );
}

/// `break-spaces` breaks between two spaces: trailing spaces that overflow
/// before a forced break go to a line of their own (Chrome 155), in a
/// paragraph re-broken by CSS's rules (a `nowrap` span gives it seams).
#[test]
fn break_spaces_trailing_spaces_take_a_line_of_their_own() {
    let after = format!("\n{}", nowrap("ab"));
    check(
        "q1_66",
        "white-space:pre-wrap;width:66px",
        &format!(
            "<span style=\"white-space:break-spaces\">{}xy z  </span>{after}",
            b("B1")
        ),
        &["[B1]xy_z", "", "ab"],
    );
    check(
        "q2_62",
        "white-space:break-spaces;width:62px",
        &format!("{}xy z   {after}", b("B1")),
        &["[B1]xy", "z", "ab"],
    );
}

/// A line re-broken around a padded span keeps the dictionary breaks of the
/// Thai before it (Chrome 155: `ภาษา` / `ไทย` / `ง่าย` / `มากw`). The widths
/// are the host's Thai font's, so only where the lines break is checked:
/// at word boundaries, more than once, `w` glued to the last word.
#[test]
fn a_re_broken_line_keeps_thai_word_breaks() {
    let laid = lay_out(
        "white-space:normal;width:30px",
        "ภาษาไทยง่ายมาก<span style=\"padding:0 3px\">w</span>",
    );
    let words = ["ภาษา", "ไทย", "ง่าย", "มาก"];
    assert!(laid.lines.len() >= 2, "{:?}", laid.lines);
    assert!(
        laid.lines.last().unwrap().ends_with("มากw"),
        "{:?}",
        laid.lines
    );
    let mut rest = laid.lines.concat();
    for line in &laid.lines {
        let mut l = line.as_str();
        while let Some(w) = words.iter().find(|w| l.starts_with(**w)) {
            l = &l[w.len()..];
        }
        assert!(
            l.is_empty() || l == "w",
            "line {line:?} not whole words: {:?}",
            laid.lines
        );
    }
    rest.retain(|c| c != 'w');
    assert_eq!(rest, words.concat());
}
