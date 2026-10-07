//! Round-2 review fixtures for PR #1434 (issue #631), at head `d8e7bdbe`.
//!
//! Scaffold and oracle as `review_1434_tests.rs`: Chrome 153 headless, the
//! bundled Inter through `@font-face`, rects relative to the container's
//! border box.
//!
//! Three pass at the head and pin something the PR's suite does not (each
//! names what it guards). One is `#[ignore]`d: the round-1 fix for the
//! `vertical-align` shift reads the shift of the span's first and last
//! **byte**, which belongs to a child when the span starts or ends with a
//! shifted child.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
const REL: &str = "position: relative";

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> usize {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "[data-m={m}] names exactly one node");
    found[0]
}

fn build(c_extra: &str, inner: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(r#"<div data-m="c" style="{C}{c_extra}">{inner}</div>"#),
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn rect(doc: &RinchDocument, m: &str) -> [f32; 4] {
    let (id, c) = (one(doc, m), one(doc, "c"));
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, c, 1.0);
    let l = doc.tree.get(id).unwrap().layout;
    [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
}

#[track_caller]
fn assert_rect(got: [f32; 4], want: [f32; 4], what: &str) {
    let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55);
    assert!(close, "{what}: got {got:?}, want {want:?}");
}

fn abs(style: &str) -> String {
    format!(r#"<div data-m="abs" style="position: absolute; {style}"></div>"#)
}

// ── Pins (pass at d8e7bdbe) ─────────────────────────────────────────────────

/// The cached fragments belong to one set of lines. A span with **no text of
/// its own** (empty, or holding only an atomic inline) is the case where the
/// only input that changes is the span's font, which is not in any glyph:
/// the lines must still be built again, or the box keeps the old font box.
/// Chrome 153 after the change: `47.64, -3, 0 x 39` and `…, 30 x 39` — its
/// line grows for the 32px strut; rinch's line does not (no strut for an
/// inline box without text of its own), so the fragment starts 5px higher.
/// The height is the pin: 39 is the 32px font box, 20 the 16px one.
#[test]
fn a_font_change_on_a_span_with_no_text_of_its_own_is_measured_again() {
    for (inner, width) in [
        ("", 0.0),
        (
            r#"<span style="display: inline-block; width: 30px; height: 8px"></span>"#,
            30.0,
        ),
    ] {
        let mut doc = build(
            "",
            &format!(
                r#"lead <span data-m="t" style="{REL}">{inner}{}</span>tail"#,
                abs("inset: 0")
            ),
        );
        let before = rect(&doc, "abs");
        assert_eq!([before[2], before[3]], [width, 20.0], "16px font box");
        let t = NodeId(one(&doc, "t"));
        doc.set_attribute(t, "style", &format!("{REL}; font-size: 32px"));
        doc.resolve_layout(800.0, 600.0);
        let after = rect(&doc, "abs");
        assert_eq!([after[2], after[3]], [width, 39.0], "32px font box");
        // And it is what a fresh layout of that state gives.
        doc.resolve_layout(800.0, 640.0);
        assert_eq!(rect(&doc, "abs"), after);
    }
}

/// A box that newly hangs from a second span of lines that are **not**
/// rebuilt: the first span's answers are already cached beside those lines,
/// and the second must still be measured. (`display: none` → shown changes
/// no line: an absolute box is not in them.) Chrome 153: `abs2` at
/// `112.70, 7, 27.88 x 20`.
#[test]
fn a_box_shown_later_in_another_span_of_the_same_lines_is_measured() {
    let mut doc = build(
        "",
        &format!(
            r#"lead <span style="{REL}">one{}</span> mid <span style="{REL}">two<div data-m="abs2" style="position: absolute; inset: 0; display: none"></div></span> end"#,
            abs("inset: 0")
        ),
    );
    let first = rect(&doc, "abs");
    let b = NodeId(one(&doc, "abs2"));
    doc.set_attribute(b, "style", "position: absolute; inset: 0");
    doc.resolve_layout(800.0, 600.0);
    assert_rect(rect(&doc, "abs2"), [112.70, 7.0, 27.88, 20.0], "abs2");
    assert_eq!(rect(&doc, "abs"), first, "the first span's box stays");
}

/// A span that is **part of** a longer right-to-left run (text of the same
/// run before and after it). The clusters of such a run come out in visual
/// order, which is the reverse of byte order, and a span's are found by
/// byte: the PR's own right-to-left fixtures hold a span that is the whole
/// run (or one end of it) and pass with the clusters left unsorted
/// (`clusters.sort_by_key` removed — the review's surviving mutant); these
/// two fail against it. Latin under U+202E in the bundled face; Chrome 153:
/// `38.66, 7, 24.58 x 20` and `32.03, 7, 48.20 x 20`.
#[test]
fn a_span_inside_a_longer_right_to_left_run() {
    let doc = build(
        "",
        &format!(
            "\u{202e}abc <span style=\"{REL}; color: red\">def{}</span> ghi\u{202c} end",
            abs("inset: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [38.66, 7.0, 24.58, 20.0], "one colour");
    let doc = build(
        "",
        &format!(
            "\u{202e}abc <span style=\"{REL}\">de<b style=\"color: red\">fg</b>hi{}</span> jkl\u{202c} end",
            abs("inset: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [32.03, 7.0, 48.20, 20.0], "bold child");
}

/// An **empty** span inside a right-to-left run sits at the start edge of
/// the character after it, which for a right-to-left character is its
/// *right* edge. Chrome 153: `35.58, 7`. Fails with the anchor taken as the
/// next cluster's left edge whatever its direction (the review's mutant
/// `c_empty_rtl_next`, which the PR's suite does not kill).
#[test]
fn an_empty_span_inside_a_right_to_left_run() {
    let doc = build(
        "",
        &format!(
            "\u{202e}abc <span style=\"{REL}\">{}</span>def\u{202c} end",
            abs("width: 40px; height: 30px; top: 0; left: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [35.58, 7.0, 40.0, 30.0], "empty");
}

// ── Finding (fails at d8e7bdbe) ─────────────────────────────────────────────

/// A span's fragment is the **span's** font box at the span's baseline. A
/// child with its own `vertical-align` moves the child's glyphs, not the
/// span's box — but the fragment's top takes the shift of the span's first
/// byte and its bottom the shift of its last, which is the child's when the
/// span starts or ends with one.
///
/// Head: `<span rel><i style="vertical-align: 7px">a</i>bc</span>` gives
/// `48, 0, 28 x 27` (top raised 7px with the child); child last: `48, 7,
/// 28 x 13`; `<sub>` first: `48, 11, 26 x 16`; `<sub>` last: `48, 7, 26 x
/// 24`. Chrome 153 gives a 20px-tall fragment in all four (at y 14 for the
/// raised child, where its line grows by 7px and rinch's does not, #1357;
/// at y 7 for `<sub>`). Asserted in rinch's own frame: the span's text `bc`
/// / `ab` is drawn on the unshifted baseline, so y 7, height 20.
#[test]
#[ignore = "review2 of #1434: the fragment takes a first/last CHILD's vertical-align shift"]
fn a_shifted_child_does_not_move_the_spans_fragment() {
    let i = r#"<i style="vertical-align: 7px; font-style: normal">"#;
    for (name, inner, width) in [
        ("raised child first", format!("{i}a</i>bc"), 27.92),
        ("raised child last", format!("ab{i}c</i>"), 27.92),
        ("sub first", "<sub>a</sub>bc".to_string(), 26.42),
        ("sub last", "ab<sub>c</sub>".to_string(), 26.41),
        // The whole content in one shifted child: both ends move (head:
        // `48, 11, 23 x 20`).
        ("sub whole", "<sub>abc</sub>".to_string(), 23.27),
    ] {
        let doc = build(
            "",
            &format!(
                r#"lead <span style="{REL}">{inner}{}</span> tail"#,
                abs("inset: 0")
            ),
        );
        assert_rect(rect(&doc, "abs"), [47.64, 7.0, width, 20.0], name);
    }
}
