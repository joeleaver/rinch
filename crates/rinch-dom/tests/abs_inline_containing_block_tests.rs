//! A `position: relative` (or `sticky`) **inline** element is the containing
//! block of an absolute box inside it (issue #631; CSS 2.1 §10.1).
//!
//! The containing block is the box from the top-left of the inline's
//! **first** fragment's padding box to the bottom-right of its **last**
//! fragment's. Every rect below is Chrome 153's (`--headless=new`, standards
//! mode, `* { box-sizing: border-box }`, 800x600), measured with
//! `getBoundingClientRect` and reported **relative to the block container's
//! border box**, with this crate's bundled Inter loaded through `@font-face`
//! under the name the fixtures register it with, so the advances are the same
//! face's. The scaffold:
//!
//! ```html
//! <div C="position: relative; width: 400px; font: 16px/20px ProbeFace;
//!         margin: 13px 0 0 17px; padding: 7px 0 0 11px">
//!   lead <span style="position: relative">text<div ABS></div>tail</span>
//! ```
//!
//! so the span's one fragment is `36.641, 0, 50.375x20` inside the content
//! box, which starts at `(11, 7)` — and every expectation is off the fixed
//! point where the span and the container coincide: container-resolved
//! `top: 0; left: 0` is `(0, 0)`, the span's corner `(47.641, 7)`.
//!
//! Inter at 16px has a rounded ascent + descent of 20px, the declared
//! line-height, so the fragment is the line box and the vertical numbers are
//! declarations. Where a fixture uses another size the fragment is that
//! font's own ascent + descent around the baseline and is said so there.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
const REL: &str = "position: relative";
const BOX: &str = "width: 40px; height: 30px;";

struct Case {
    doc: RinchDocument,
    c: usize,
    relayouts: u32,
}

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
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

impl Case {
    /// `inner` as the content of the container `C` (with `c_extra` appended
    /// to its style).
    fn with(c_extra: &str, inner: &str) -> Self {
        let mut doc = document();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(r#"<div data-m="c" style="{C}{c_extra}">{inner}</div>"#),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let c = one(&doc, "[data-m=c]");
        Case {
            doc,
            c,
            relayouts: 0,
        }
    }

    fn new(inner: &str) -> Self {
        Self::with("", inner)
    }

    /// `lead <span rel>text{abs}tail</span>`.
    fn lead(abs_style: &str) -> Self {
        Self::new(&format!(
            r#"lead <span data-m="span" style="{REL}">text{}tail</span>"#,
            abs(abs_style, "")
        ))
    }

    fn id(&self, m: &str) -> usize {
        one(&self.doc, &format!("[data-m={m}]"))
    }

    /// `m`'s on-screen box relative to the container's border box.
    fn rect(&self, m: &str) -> [f32; 4] {
        let id = self.id(m);
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, id, 1.0);
        let (cx, cy) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, self.c, 1.0);
        let l = self.doc.tree.get(id).unwrap().layout;
        [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
    }

    /// Lay out again at a viewport height no earlier pass used, so the pass
    /// is not skipped while no width in the fixture moves.
    fn relayout(&mut self) {
        self.relayouts += 1;
        self.doc
            .resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0 * self.relayouts as f32);
    }

    fn set_style(&mut self, m: &str, style: &str) {
        let id = self.id(m);
        self.doc
            .set_attribute(rinch_core::dom::NodeId(id), "style", style);
    }

    fn frame(&mut self) -> rinch_dom::perf::FrameStats {
        self.doc.tree.perf.end_frame();
        self.relayout();
        self.doc.tree.perf.frame()
    }
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

fn abs(style: &str, inner: &str) -> String {
    format!(r#"<div data-m="abs" style="position: absolute; {style}">{inner}</div>"#)
}

/// `want` is Chrome's layout rect, which is fractional wherever text decides
/// it (`lead ` is 36.641px). rinch lays every box out on the pixel grid — a
/// box hung from a fragment is snapped like the boxes Taffy places, to the
/// pixel Chrome *paints* its edge on — so the position asserted is Chrome's
/// to the nearest pixel, and the size Chrome's to within the half pixel
/// Taffy's own rounding moves it.
#[track_caller]
fn assert_rect(got: [f32; 4], want: [f32; 4], what: &str) {
    let on_grid = got.iter().all(|g| g.fract() == 0.0);
    let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55);
    assert!(
        on_grid && close,
        "{what}: got {got:?}, Chrome 153 gives {want:?}"
    );
}

// ── The issue's table ───────────────────────────────────────────────────────

/// `top: 0; left: 0` lands on the span's fragment, not on the container.
/// Before the fix: `(0, 0)`, the container's padding-box corner.
#[test]
fn insets_resolve_against_the_spans_fragment() {
    let c = Case::lead(&format!("{BOX} top: 0; left: 0"));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "top/left");
}

/// With nothing before the span its fragment starts at the content box's
/// corner, `(11, 7)` — not the padding box's `(0, 0)`. (A container with no
/// padding is the fixed point where the two agree.)
#[test]
fn a_span_at_the_line_start_is_at_the_content_corner() {
    let c = Case::new(&format!(
        r#"<span style="{REL}">text{}tail</span>"#,
        abs(&format!("{BOX} top: 0; left: 0"), "")
    ));
    assert_rect(c.rect("abs"), [11.0, 7.0, 40.0, 30.0], "no lead");
}

/// A percentage size is of the fragment: half of 50.375 x 20. Before: half
/// the container's 400 x 27.
#[test]
fn a_percentage_size_is_of_the_fragment() {
    let c = Case::lead("left: 0; top: 0; width: 50%; height: 50%");
    assert_rect(c.rect("abs"), [47.641, 7.0, 25.188, 10.0], "50% x 50%");
}

/// `right: 0; bottom: 0` hangs the box from the fragment's far corner:
/// `47.641 + 50.375 - 40`, `7 + 20 - 30`.
#[test]
fn right_and_bottom_hang_from_the_fragments_far_corner() {
    let c = Case::lead(&format!("{BOX} right: 0; bottom: 0"));
    assert_rect(c.rect("abs"), [58.016, -3.0, 40.0, 30.0], "right/bottom");
}

/// Auto insets keep the static position, which the containing block has no
/// part in. A control: this is `main`'s answer too.
#[test]
fn auto_insets_keep_the_static_position() {
    let c = Case::lead(BOX);
    assert_rect(c.rect("abs"), [11.0, 27.0, 40.0, 30.0], "static");
}

/// `inset: 0` is the fragment itself, and insets come off it.
#[test]
fn paired_insets_fill_the_fragment() {
    let c = Case::lead("inset: 0");
    assert_rect(c.rect("abs"), [47.641, 7.0, 50.375, 20.0], "inset: 0");
    let c = Case::lead("inset: 2px 3px 4px 5px");
    assert_rect(c.rect("abs"), [52.641, 9.0, 42.375, 14.0], "four insets");
}

/// Percentage insets, padding and margins are of the fragment's width
/// (107.031 for `texttexttexttail`): `left: 10%` is 10.7, `top: 50%` is 10.
#[test]
fn percentage_insets_padding_and_margins_are_of_the_fragment() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">texttexttext{}tail</span>"#,
        abs(&format!("{BOX} left: 10%; top: 50%"), "")
    ));
    assert_rect(
        c.rect("abs"),
        [58.344, 17.0, 40.0, 30.0],
        "percentage insets",
    );
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">texttexttext{}tail</span>"#,
        abs(
            "left: 0; top: 0; padding: 10%; margin-left: 10%; margin-top: 10%",
            r#"<div style="width: 10px; height: 10px"></div>"#
        )
    ));
    assert_rect(
        c.rect("abs"),
        [58.344, 17.703, 31.406, 31.406],
        "percentage padding and margins",
    );
}

/// The box's own content lays out inside the fragment-sized box: a
/// `50% x 50%` child of an `inset: 0` box is half of 78.703 x 20.
#[test]
fn the_boxs_children_lay_out_inside_the_fragment_sized_box() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">texttext{}tail</span>"#,
        abs(
            "inset: 0",
            r#"<div data-m="kid" style="width: 50%; height: 50%"></div>"#
        )
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 78.703, 20.0], "box");
    assert_rect(c.rect("kid"), [47.641, 7.0, 39.344, 10.0], "child");
}

// ── Which span, which fragment ──────────────────────────────────────────────

/// The containing block is the nearest positioned ancestor, through static
/// inline elements: `rel > b > abs` and `rel > span > abs`.
#[test]
fn the_span_need_not_be_the_boxs_parent() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">te<span>xt{}</span>tail</span> more"#,
        abs("inset: 0", "")
    ));
    assert_rect(
        c.rect("abs"),
        [47.641, 7.0, 50.375, 20.0],
        "static span between",
    );
}

/// An inline-level absolute (`<span style="position: absolute">`) is
/// blockified and resolves the same way.
#[test]
fn an_absolute_span_resolves_the_same_way() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text<span data-m="abs" style="position: absolute; left: 0; top: 0; width: 50%; height: 10px"></span>tail</span>"#
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 25.188, 10.0], "absolute span");
}

/// `position: sticky` positions a span too.
#[test]
fn a_sticky_span_is_a_containing_block() {
    let c = Case::new(&format!(
        r#"lead <span style="position: sticky; top: 0">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 50.375, 20.0], "sticky");
}

/// A `transform` on a non-atomic inline does not apply (#1080), so it
/// contains nothing: the box fills the positioned container, 400 x 27.
/// Kills a fix that takes every boxless ancestor for a containing block.
#[test]
fn a_transformed_span_is_not_a_containing_block() {
    let c = Case::new(&format!(
        r#"lead <span style="transform: translateX(0)">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 27.0], "transformed span");
}

/// An empty span has a zero-width fragment where it sits in the line.
#[test]
fn an_empty_span_is_a_zero_width_fragment_in_the_line() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">{}</span>tail"#,
        abs(&format!("{BOX} top: 0; left: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "empty span");
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">{}</span>tail"#,
        abs(
            "top: 0; left: 0; width: 50%; height: 50%; min-width: 3px; min-height: 3px",
            ""
        )
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 3.0, 10.0], "half of nothing");
}

/// A span holding only an atomic inline is as wide as that box, and its
/// fragment is still the font's line, not the box: lead 30px, box 50x10.
/// Every horizontal number here is a declaration. (The `x` after the span
/// gives the line a strut's worth of text: rinch's line has no strut (#624,
/// #1258), so on a line of nothing but atomic inlines the baseline is the
/// boxes' bottom edge, and the fragment hangs from that.)
#[test]
fn a_span_of_one_inline_block_is_that_wide() {
    let c = Case::new(&format!(
        r#"<span style="display: inline-block; width: 30px; height: 10px"></span><span style="{REL}"><span style="display: inline-block; width: 50px; height: 10px"></span>{}</span> x"#,
        abs("inset: 0", "")
    ));
    assert_eq!(c.rect("abs"), [41.0, 7.0, 50.0, 20.0]);
}

/// A span over three lines: the block is from the first fragment's top-left
/// (315.5, 7) to the last fragment's bottom-right (152.25, 67). The right
/// edge is left of the left edge, so the width is zero — `right: 0` hangs the
/// box from x = 315.5 — and the height is the three lines.
#[test]
fn a_wrapped_span_runs_from_its_first_fragment_to_its_last() {
    let ml = |style: &str| {
        Case::new(&format!(
            r#"<span style="display: inline-block; width: 300px; height: 10px"></span> <span data-m="span" style="{REL}">aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm{} nnnn oooo</span>"#,
            abs(style, "")
        ))
    };
    let c = ml(&format!("{BOX} top: 0; left: 0"));
    assert_rect(c.rect("abs"), [315.5, 7.0, 40.0, 30.0], "top/left");
    let c = ml(&format!("{BOX} right: 0; bottom: 0"));
    assert_rect(c.rect("abs"), [275.5, 37.0, 40.0, 30.0], "right/bottom");
    let c = ml("inset: 0");
    assert_rect(c.rect("abs"), [315.5, 7.0, 0.0, 60.0], "inset: 0");
    let c = ml("left: 0; top: 0; width: 50%; height: 50%");
    assert_rect(c.rect("abs"), [315.5, 7.0, 0.0, 30.0], "50% x 50%");
}

/// The fragment is the span's **own** font's ascent and descent around the
/// baseline, whatever the line box: a 32px span in a 16px line is 39 tall
/// and starts 3px above the container; a 10px one is 12 tall; a span with
/// its own `line-height: 40px` is still 20 tall, 10px into its line.
#[test]
fn the_fragment_is_the_spans_own_font_box() {
    let k = |span: &str| {
        Case::new(&format!(
            r#"lead <span style="{REL}; {span}">text{}tail</span>"#,
            abs("inset: 0", "")
        ))
    };
    assert_rect(
        k("font-size: 32px").rect("abs"),
        [47.641, -3.0, 100.75, 39.0],
        "32px span",
    );
    assert_rect(
        k("font-size: 10px").rect("abs"),
        [47.641, 13.0, 31.5, 12.0],
        "10px span",
    );
    assert_rect(
        k("line-height: 40px").rect("abs"),
        [47.641, 17.0, 50.375, 20.0],
        "tall line",
    );
}

/// The line follows `text-align`, and the box with it. (The aligned block
/// declares the 389px it would fill anyway: rinch aligns the lines of an
/// auto-width block in a box one pixel wider than it, which puts a
/// right-aligned line a pixel right of Chrome's whatever is in it.)
#[test]
fn the_box_follows_an_aligned_line() {
    let c = Case::new(&format!(
        r#"<div style="text-align: center; width: 389px">lead <span style="{REL}">text{}tail</span></div>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [198.625, 7.0, 40.0, 30.0], "centred");
    let c = Case::new(&format!(
        r#"<div style="text-align: right; width: 389px">lead <span style="{REL}">text{}tail</span></div>"#,
        abs(&format!("{BOX} right: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [360.0, 7.0, 40.0, 30.0], "right");
}

/// Text beside a block child is laid out by an anonymous block box (#566);
/// the span's line is 25px down, in that box.
#[test]
fn a_span_in_an_anonymous_block_box() {
    let m = |style: &str| {
        Case::new(&format!(
            r#"<div style="height: 25px"></div>lead <span style="{REL}">text{}tail</span>"#,
            abs(style, "")
        ))
    };
    assert_rect(
        m("inset: 0").rect("abs"),
        [47.641, 32.0, 50.375, 20.0],
        "inset: 0",
    );
    assert_rect(
        m(&format!("{BOX} left: 0; top: 0")).rect("abs"),
        [47.641, 32.0, 40.0, 30.0],
        "top/left",
    );
}

// ── A box between the absolute one and the span ─────────────────────────────

/// The absolute box's layout parent is an `inline-block` inside the span:
/// the span is still the containing block, 110.375 wide with the 60px box in
/// it. (The box is 10px tall so that it sits inside the line's ascent and
/// the line stays 20px; the box-resolved answer would be 60x10 at the
/// inline-block.)
#[test]
fn a_box_inside_an_inline_block_inside_the_span() {
    let j = |style: &str| {
        Case::new(&format!(
            r#"lead <span style="{REL}">text<span style="display: inline-block; width: 60px; height: 10px">{}</span>tail</span>"#,
            abs(style, "")
        ))
    };
    assert_rect(
        j("inset: 0").rect("abs"),
        [47.641, 7.0, 110.375, 20.0],
        "inset: 0",
    );
    assert_rect(
        j(&format!("{BOX} right: 0; bottom: 0")).rect("abs"),
        [118.016, -3.0, 40.0, 30.0],
        "right/bottom",
    );
}

/// A scroller between the box and the span does not carry the box (it is
/// the span's content, not the scroller's): `top: 0; left: 0` stays on the
/// span's corner at `scrollTop = 10`.
#[test]
fn a_scroller_between_the_box_and_the_span_does_not_carry_it() {
    let mut c = Case::new(&format!(
        r#"lead <span style="{REL}">text<span data-m="sc" style="display: inline-block; width: 60px; height: 14px; overflow: auto">{}<div style="height: 100px"></div></span>tail</span>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "unscrolled");
    let sc = c.id("sc");
    c.doc.set_scroll_top(rinch_core::dom::NodeId(sc), 10.0);
    assert_eq!(
        c.doc.tree.get(sc).unwrap().scroll_offset.1,
        10.0,
        "the scroller did scroll"
    );
    assert_rect(
        c.rect("abs"),
        [47.641, 7.0, 40.0, 30.0],
        "scrolled, no layout",
    );
    c.relayout();
    assert_rect(
        c.rect("abs"),
        [47.641, 7.0, 40.0, 30.0],
        "scrolled, laid out",
    );
}

/// The container's own scroll does carry it, with the line.
#[test]
fn the_containers_own_scroll_carries_the_box() {
    let mut c = Case::with(
        "height: 40px; overflow: auto;",
        &format!(
            r#"lead <span style="{REL}">text{}tail</span><div style="height: 200px"></div>"#,
            abs(&format!("{BOX} top: 0; left: 0"), "")
        ),
    );
    let id = c.c;
    c.doc.set_scroll_top(rinch_core::dom::NodeId(id), 10.0);
    assert_rect(c.rect("abs"), [47.641, -3.0, 40.0, 30.0], "scrolled");
    c.relayout();
    assert_rect(c.rect("abs"), [47.641, -3.0, 40.0, 30.0], "laid out");
}

// ── Changes ─────────────────────────────────────────────────────────────────

/// The fragment grows with its text, and a box sized from it follows in the
/// same layout: `texttext` + `tail` is 78.703 wide.
#[test]
fn a_text_change_resizes_the_box_in_the_same_layout() {
    let mut c = Case::new(&format!(
        r#"lead <span style="{REL}"><span data-m="t">text</span>{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 50.375, 20.0], "before");
    let t = c.id("t");
    c.doc
        .set_text_content(rinch_core::dom::NodeId(t), "texttext");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [47.641, 7.0, 78.703, 20.0], "after");
}

/// Text added **before** the span moves the fragment and a position-only
/// box with it (no size of the box depends on the fragment, so nothing but
/// the placement can carry it).
#[test]
fn text_before_the_span_moves_a_position_only_box() {
    let mut c = Case::new(&format!(
        r#"<span data-m="t">lead </span><span style="{REL}">text{}tail</span>"#,
        abs(&format!("{BOX} top: 0; left: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "before");
    let t = c.id("t");
    c.doc.set_text_content(rinch_core::dom::NodeId(t), "");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [11.0, 7.0, 40.0, 30.0], "after");
}

/// A span that becomes positioned takes the box over from the container,
/// and hands it back when it stops.
#[test]
fn a_span_that_starts_or_stops_being_positioned() {
    let mut c = Case::new(&format!(
        r#"lead <span data-m="span">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 27.0], "static span");
    c.set_style("span", REL);
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [47.641, 7.0, 50.375, 20.0], "relative span");
    c.set_style("span", "");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 27.0], "static again");
}

/// A `text-align` change moves the line with no Taffy compute (the text-only
/// path of `resolve_layout`); the box moves with its span.
#[test]
fn a_text_align_change_moves_the_box_with_no_compute() {
    let mut c = Case::new(&format!(
        r#"<div data-m="line">lead <span style="{REL}">text{}tail</span></div>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "start");
    c.set_style("line", "text-align: center");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [198.625, 7.0, 40.0, 30.0], "centred");
}

/// A resize of the container re-wraps the line; the box follows the span to
/// its new line. At 120px wide `lead` fills the first line's start and the
/// span stays on it; the oracle is the rule (the fragment's corner), read
/// from the span's own first glyph.
#[test]
fn the_box_follows_the_span_through_a_relayout() {
    let mut c = Case::lead(&format!("{BOX} top: 0; left: 0"));
    c.relayout();
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "same place");
}

/// A span that becomes a **split** inline (a block-level child arrives) can
/// no longer be measured, and the box goes back to the block container —
/// size and place, not the fragment-sized box it was baked as.
#[test]
fn a_span_that_becomes_split_hands_the_box_back() {
    let mut c = Case::new(&format!(
        r#"lead <span data-m="span" style="{REL}">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 50.375, 20.0], "unsplit");
    let span = rinch_core::dom::NodeId(c.id("span"));
    let block = c.doc.create_element("div");
    c.doc.set_attribute(block, "style", "height: 25px");
    let first = c.doc.tree.get(span.0).unwrap().children[1];
    c.doc
        .insert_before(span, block, rinch_core::dom::NodeId(first));
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(c.rect("abs"), [0.0, 0.0, 400.0, 72.0], "split");
}

/// A second span in the same lines becomes positioned: the lines are not
/// rebuilt, and already hold an answer for the first span, so they have to
/// be measured again for both. The second box then fills `tail` (22.047
/// wide, after `lead `, `text` and a space: 36.641 + 28.328 + 4.297 into the
/// content box — Chrome's own advances, summed).
#[test]
fn a_second_span_in_measured_lines_is_measured_when_it_becomes_positioned() {
    let mut c = Case::new(&format!(
        r#"lead <span style="{REL}">text{}</span> <span data-m="b">tail<div data-m="second" style="position: absolute; inset: 0"></div></span>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "first span");
    assert_rect(
        c.rect("second"),
        [0.0, 0.0, 400.0, 27.0],
        "static second span",
    );
    c.set_style("b", REL);
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("second"), [80.266, 7.0, 22.047, 20.0], "second span");
    assert_rect(
        c.rect("abs"),
        [47.641, 7.0, 40.0, 30.0],
        "first span, still",
    );
}

// ── Cost ────────────────────────────────────────────────────────────────────

/// A position-only box costs one compute per layout, like any other; a box
/// sized from the fragment costs a second compute only in the layout where
/// the fragment's size is new to it (the first one here), and none after.
#[test]
fn a_fragment_sized_box_costs_a_second_compute_only_when_the_fragment_changes() {
    let mut c = Case::lead(&format!("{BOX} top: 0; left: 0"));
    let f = c.frame();
    assert_eq!(f.get(Counter::TaffyRootComputes), 1, "position-only");
    assert_eq!(f.get(Counter::AbsContainingBlockPasses), 0);

    let mut c = Case::lead("inset: 0");
    assert_eq!(
        c.doc
            .tree
            .perf
            .total()
            .get(Counter::AbsContainingBlockPasses),
        1,
        "the first layout learns the fragment's size after its lines are built"
    );
    let f = c.frame();
    assert_eq!(
        f.get(Counter::TaffyRootComputes),
        1,
        "sized, fragment unchanged"
    );
    assert_eq!(f.get(Counter::AbsContainingBlockPasses), 0);
}

// ── More shapes, each from the same Chrome 153 run ──────────────────────────

/// Two positioned spans: the nearest one contains the box (`xt`, 14.281
/// wide, 14.047 into the outer span).
#[test]
fn the_nearest_positioned_span_wins() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">te<span style="{REL}">xt{}</span>tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [61.688, 7.0, 14.281, 20.0], "inner span");
}

/// A `display: contents` wrapper between the box and the span changes
/// nothing.
#[test]
fn a_contents_wrapper_between_the_box_and_the_span() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text<div style="display: contents">{}</div>tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(
        c.rect("abs"),
        [47.641, 7.0, 50.375, 20.0],
        "contents wrapper",
    );
}

/// Vertical padding grows the fragment's padding box (3px above, 4 below);
/// it takes no room in the line.
#[test]
fn vertical_padding_is_part_of_the_containing_block() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}; padding: 3px 0 4px 0">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 4.0, 50.375, 27.0], "padded span");
}

/// A span broken by a `<br>`: first fragment on line one, last on line two,
/// whose right edge (22.047 into the content box) is left of the first's
/// left edge — zero wide, two lines tall.
#[test]
fn a_span_holding_a_forced_break() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text<br>ta{}il</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 0.0, 40.0], "br");
}

/// An empty span right after an atomic inline, and one that opens the line.
#[test]
fn an_empty_span_after_an_inline_block_and_at_the_line_start() {
    let c = Case::new(&format!(
        r#"<span style="display: inline-block; width: 30px; height: 10px"></span><span style="{REL}">{}</span>tail"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_eq!(c.rect("abs"), [41.0, 7.0, 40.0, 30.0], "after a box");
    let c = Case::new(&format!(
        r#"<span style="{REL}">{}</span>tail"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_eq!(c.rect("abs"), [11.0, 7.0, 40.0, 30.0], "first in the line");
    // Text, then the box, then the span: the span sits after the box
    // (18.781 + 30 into the content box), not where the text ends.
    let c = Case::new(&format!(
        r#"ab<span style="display: inline-block; width: 30px; height: 10px"></span><span style="{REL}">{}</span>tail"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(
        c.rect("abs"),
        [59.781, 7.0, 40.0, 30.0],
        "after text and a box",
    );
    // The same with nothing after the span: there is no next character to
    // stand before, and the box, not the text before it, is what it follows.
    let c = Case::new(&format!(
        r#"ab<span style="display: inline-block; width: 30px; height: 10px"></span><span style="{REL}">{}</span>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(
        c.rect("abs"),
        [59.781, 7.0, 40.0, 30.0],
        "after a box, last in the line",
    );
}

/// A span whose only text is a collapsed space is empty too.
#[test]
fn a_span_of_collapsed_white_space_is_empty() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}"> {}</span>tail"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_rect(c.rect("abs"), [47.641, 7.0, 40.0, 30.0], "white space");
}

/// A fragment-sized box that itself holds a positioned span with a box in
/// it: the outer box is placed 10px right of and 25px below the outer
/// span's corner, and the inner one fills `side` (31.453 wide, after `in `).
/// Each level's size is learnt from lines built after the level above was
/// sized.
#[test]
fn a_box_in_a_span_in_a_box_in_a_span() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text<div data-m="outer" style="position: absolute; left: 10px; top: 25px; width: 200px">in <span style="{REL}">side{}</span></div>tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("outer"), [57.641, 32.0, 200.0, 20.0], "outer");
    assert_rect(c.rect("abs"), [75.469, 32.0, 31.453, 20.0], "inner");
}

/// A span that is a flex item is blockified: it has a box, and is Taffy's
/// own containing block. A control (this is `main`'s answer).
#[test]
fn a_blockified_span_is_an_ordinary_containing_block() {
    let c = Case::new(&format!(
        r#"<div style="display: flex"><span style="{REL}">text{}tail</span></div>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [11.0, 7.0, 50.375, 20.0], "flex item");
}

/// The check that follows the lines looks only at boxes hung from a span. One
/// box resolved against a boxed grandparent (#386) beside one hung from a
/// span: a layout looks at the first three times (size check, read-back,
/// second placement) and at the second four times (the same, and the check
/// after the lines) — seven, not eight.
#[test]
fn the_check_after_the_lines_skips_boxes_with_a_boxed_containing_block() {
    let mut c = Case::new(&format!(
        r#"<div><div style="position: absolute; right: 3px; top: 3px; width: 5px; height: 5px"></div></div>lead <span style="{REL}">text{}tail</span>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    let f = c.frame();
    assert_eq!(f.get(Counter::AbsBoxesVisited), 7);
}

// ── Direction, mixed fonts, vertical-align (the review of #1434) ────────────

/// A fragment's left and right are the **visual** extent of its content: a
/// span of right-to-left text is as wide as its text and starts where the
/// text is drawn, not at its first byte's caret (which is the right edge).
/// Latin text under a U+202E override is right-to-left in the bundled face,
/// so the numbers are Chrome's: `text` reversed is 28.172 wide.
#[test]
fn a_span_of_right_to_left_text() {
    let rlo = |inner: &str, after: &str, style: &str| {
        Case::new(&format!(
            "lead <span style=\"{REL}\">{inner}{}</span> {after}",
            abs(style, "")
        ))
    };
    assert_rect(
        rlo("\u{202E}text\u{202C}", "tail", "inset: 0").rect("abs"),
        [47.641, 7.0, 28.172, 20.0],
        "all right-to-left",
    );
    assert_rect(
        rlo(
            "\u{202E}text\u{202C}",
            "tail",
            &format!("{BOX} left: 0; top: 0"),
        )
        .rect("abs"),
        [47.641, 7.0, 40.0, 30.0],
        "top/left",
    );
    // Right-to-left then left-to-right, and the other way about.
    assert_rect(
        rlo("\u{202E}text\u{202C}tail", "end", "inset: 0").rect("abs"),
        [47.641, 7.0, 50.219, 20.0],
        "rtl then ltr",
    );
    assert_rect(
        rlo("te\u{202E}xtta\u{202C}", "end", "inset: 0").rect("abs"),
        [47.641, 7.0, 42.594, 20.0],
        "ltr then rtl",
    );
}

/// The fragment is the span's own font box when its content ends in a child
/// set larger: `sm <b 32px>big</b>` in a 16px span is 20 tall, 5px into a
/// line the child made 25px tall — not the child's 39.
#[test]
fn the_fragment_is_the_spans_font_box_beside_a_larger_child() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">sm <b style="font-size: 32px; font-weight: normal">big</b>{}</span> tail"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 12.0, 73.938, 20.0], "larger child");
}

/// A `<sup>`'s fragment is raised with its glyphs (`vertical-align: super`,
/// 13.33px: 16 tall), so the box sits on the text as rinch draws it. That is
/// 5px above Chrome's `(47.641, 9)`: Chrome grows the line for the raised
/// text and rinch does not (#1357), so rinch's superscript itself is drawn
/// that much higher. A `<sub>` lowers into room both give it, and matches
/// (`review_1434_tests::a_sub_spans_fragment_is_where_its_text_is_drawn`).
#[test]
fn a_sup_spans_fragment_is_raised_with_its_text() {
    let c = Case::new(&format!(
        r#"lead <sup style="{REL}">text{}tail</sup>"#,
        abs("inset: 0", "")
    ));
    assert_eq!(c.rect("abs"), [48.0, 4.0, 42.0, 16.0]);
}

/// A wrapped span whose last content is an atomic inline alone on a fourth
/// line: the first fragment is still the first line's text (left 315.5), and
/// the last is the box's line (bottom 87), so `right: 0; bottom: 0` hangs
/// 30px above that.
#[test]
fn a_wrapped_span_ending_in_an_inline_block_on_its_own_line() {
    let w = |style: &str| {
        Case::new(&format!(
            r#"<span style="display: inline-block; width: 300px; height: 10px"></span> <span style="{REL}">aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm nnnn oooo<span style="display: inline-block; width: 300px; height: 10px"></span>{}</span> x"#,
            abs(style, "")
        ))
    };
    assert_rect(
        w(&format!("{BOX} left: 0; top: 0")).rect("abs"),
        [315.5, 7.0, 40.0, 30.0],
        "top/left",
    );
    assert_rect(
        w(&format!("{BOX} right: 0; bottom: 0")).rect("abs"),
        [275.5, 57.0, 40.0, 30.0],
        "right/bottom",
    );
}

/// Two spans set in different sizes in one paragraph, a box in each: each
/// fragment is its own span's font box (20 tall at 16px, 39 at 32px).
#[test]
fn two_spans_of_different_sizes_in_one_paragraph() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text{}</span> <span style="{REL}; font-size: 32px">big<div data-m="big" style="position: absolute; inset: 0"></div></span>"#,
        abs("inset: 0", "")
    ));
    assert_rect(c.rect("abs"), [47.641, 12.0, 28.328, 20.0], "16px span");
    assert_rect(c.rect("big"), [80.469, -3.0, 46.969, 39.0], "32px span");
}

/// A percentage of the fragment's height is of its **rounded** font box:
/// ten times 20, not ten times Inter's 19.36.
#[test]
fn a_percentage_height_is_of_the_rounded_font_box() {
    let c = Case::lead("left: 0; top: 0; width: 10px; height: 1000%");
    assert_rect(c.rect("abs"), [47.641, 7.0, 10.0, 200.0], "1000%");
}

/// A span whose text holds an emoji, which takes a family of its own
/// (#1204): the fragment covers the whole text. The emoji's width is the
/// host face's, so only "wider than `text` and `tail` together" (50.375) is
/// asserted.
#[test]
fn a_span_holding_an_emoji_covers_its_whole_text() {
    let c = Case::new(&format!(
        "lead <span style=\"{REL}\">text\u{1F600}tail{}</span>",
        abs("inset: 0", "")
    ));
    let r = c.rect("abs");
    assert_eq!((r[0], r[1], r[3]), (48.0, 7.0, 20.0));
    assert!(r[2] > 52.0, "width {}: the whole text", r[2]);
}

// ── Stated divergences (none introduced here) ───────────────────────────────

/// A **split** inline — one holding a block-level child (#513) — has its
/// fragments in several anonymous boxes, and is not measured: the box keeps
/// the answer it had before #631, the positioned block container. Chrome
/// 153: `top: 0; left: 0` at the first fragment's corner `(47.641, 7)`, and
/// `inset: 0` a zero-wide box 65px tall there. Tracked in #1424.
#[test]
fn a_split_inline_is_not_yet_a_containing_block() {
    let split = |style: &str| {
        Case::new(&format!(
            r#"lead <span style="{REL}">text<div style="height: 25px">block</div>{}tail</span>"#,
            abs(style, "")
        ))
    };
    assert_eq!(
        split(&format!("{BOX} left: 0; top: 0")).rect("abs"),
        [0.0, 0.0, 40.0, 30.0],
        "the container's corner"
    );
    assert_eq!(
        split("inset: 0").rect("abs"),
        [0.0, 0.0, 400.0, 72.0],
        "the container"
    );
}

/// rinch does not move a flowed inline by its `left`/`top` (the text of a
/// `position: relative; left: 5px; top: 3px` span is drawn where it flows),
/// so the box is hung from the unmoved fragment — where rinch draws the
/// span. Chrome 153 moves both: `(52.641, 10)`. Tracked in #1425.
#[test]
fn a_relative_spans_own_offset_moves_nothing() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}; left: 5px; top: 3px">text{}tail</span>"#,
        abs(&format!("{BOX} left: 0; top: 0"), "")
    ));
    assert_eq!(c.rect("abs"), [48.0, 7.0, 40.0, 30.0]);
}

/// rinch gives an inline element's horizontal padding no room on its line:
/// it paints the background 6px left of and 5px right of the glyphs, over
/// the neighbours. The containing block is that painted padding box, so it
/// is as wide as Chrome's (61.375) and starts 6px further left than
/// Chrome's `(47.641, 4)`.
#[test]
fn horizontal_padding_takes_no_room_in_the_line() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}; padding: 3px 5px 4px 6px">text{}tail</span>"#,
        abs("inset: 0", "")
    ));
    assert_eq!(c.rect("abs"), [42.0, 4.0, 61.0, 27.0]);
}

/// A span that **ends** in a forced break has, in Chrome 153, an empty last
/// fragment at the start of the next line, which makes the block zero wide
/// and two lines tall (`47.641, 7, 0 x 40`). rinch gives the span no
/// fragment on a line it has no content on: the block is `text`, 28.328
/// wide, with no width for the break itself.
#[test]
fn a_span_ending_in_a_forced_break_has_no_empty_last_fragment() {
    let c = Case::new(&format!(
        r#"lead <span style="{REL}">text<br>{}</span>tail"#,
        abs("inset: 0", "")
    ));
    assert_eq!(c.rect("abs"), [48.0, 7.0, 28.0, 20.0]);
}
