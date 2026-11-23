//! The rest of the `var()` family in the properties rinch rewrites or reads by
//! hand (#2870, after #2820): `display: var(--d)` naming `-webkit-box`,
//! `text-indent` keywords through a `var()`, a `var()` left invalid in
//! `hyphens`, `word-break` and `text-indent` (which then inherit), the same
//! for the text stroke on `::before` / `::after`, a `calc()` stroke width, a
//! CSS-wide keyword from a `var()` fallback, and a `currentcolor` stroke a
//! child inherits.
//!
//! **Against Chrome.** Every expected value is what Chrome 158 computes for
//! the same element (`getComputedStyle`) on [`PAGE`] with [`CSS`], read by
//!
//! ```text
//! google-chrome --headless=new --no-sandbox --disable-gpu --dump-dom file://<the page>
//! ```
//!
//! with a script writing each element's `display`, `text-indent`, `hyphens`,
//! `word-break`, `overflow-wrap`, `-webkit-text-stroke-width`,
//! `-webkit-text-stroke-color`, `-webkit-line-clamp` and `height` into the
//! page (and the two stroke longhands of `#s5::before` and `#s6::after`). A
//! value written through a `var()` is compared with the same value written
//! out (the `*x` twins), which Chrome computes identically.
//!
//! Not pinned: `text-indent: initial` (written out, or from a `var()`
//! fallback) under a `hanging` parent, where Chrome 158 keeps the parent's
//! keyword (`0px hanging`) and rinch resets both, as the property's initial
//! value is.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{
    ComputedStyle, DisplayValue, LengthPercentageValue, OverflowWrapValue, WordBreakValue,
};

const CSS: &str = "
.pb::before { content: \"x\"; -webkit-text-stroke: 8px var(--missing); }
.pa::after { content: \"y\"; -webkit-text-stroke-width: var(--missing); }
";

const PAGE: &str = r#"<div id="root" style="--d: -webkit-box; --di: -webkit-inline-box; --t: 2em hanging; --te: 10px each-line; --bad: foo; --c: calc(2px + 1px); font-size: 20px; line-height: 23px; color: gold">
 <div id="d1" style="display: var(--d)">a</div>
 <div id="d1x" style="display: -webkit-box">a</div>
 <span id="d2" style="display: var(--di)">a</span>
 <span id="d2x" style="display: -webkit-inline-box">a</span>
 <div id="d3" style="display: var(--d); -webkit-box-orient: vertical; -webkit-line-clamp: 1; width: 40px">aaa bbb ccc ddd</div>
 <div id="d4" style="--d: block; display: var(--d)">a</div>
 <div id="t1" style="text-indent: var(--t)">a<div id="t1c" style="font-size: 10px">a</div></div>
 <div id="t2" style="text-indent: var(--te)">a</div>
 <div id="t0" style="text-indent: 7px hanging"><div id="t3" style="text-indent: var(--bad)">a</div><div id="t4" style="text-indent: var(--missing)">a</div></div>
 <div id="h0" style="hyphens: none"><div id="h1" style="hyphens: var(--missing)">a</div><div id="h2" style="hyphens: var(--bad)">a</div><div id="h3" style="hyphens: var(--x, initial)">a</div><div id="h4" style="hyphens: var(--x, var(--y, initial))">a</div></div>
 <div id="w0" style="word-break: break-all"><div id="w1" style="word-break: var(--missing)">a</div><div id="w2" style="word-break: var(--bad)">a</div><div id="w3" style="word-break: var(--wb, break-word)">a</div></div>
 <div id="wb0" style="word-break: break-word"><div id="wb1" style="word-break: var(--missing)">a</div></div>
 <div id="s0" style="-webkit-text-stroke: 6px green">
   <b id="s1" style="-webkit-text-stroke: var(--c) red">7</b>
   <b id="s2" style="-webkit-text-stroke: var(--x, initial)">7</b>
   <b id="s3" style="-webkit-text-stroke-width: var(--x, initial)">7</b>
   <b id="s4" style="-webkit-text-stroke-width: calc(1px + 0.1em)">7</b>
   <b id="s7" style="-webkit-text-stroke: var(--x, inherit)">7</b>
   <b id="s5" class="pb">7</b>
   <b id="s6" class="pa">7</b>
 </div>
 <div id="cc0" style="-webkit-text-stroke: 6px currentcolor; color: blue"><b id="cc1" style="color: red; -webkit-text-stroke: 8px var(--missing)">7</b></div>
</div>"#;

fn doc() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, PAGE);
    d.resolve_layout(800.0, 600.0);
    d
}

fn id_of(d: &RinchDocument, id: &str) -> rinch_dom::node::RawNodeId {
    d.query_selector_all(&format!("#{id}"))[0].0
}

fn style<'a>(d: &'a RinchDocument, id: &str) -> &'a ComputedStyle {
    &d.tree.get(id_of(d, id)).unwrap().computed_style
}

/// The style of `id`'s generated box (`::before` or `::after`).
fn pseudo<'a>(d: &'a RinchDocument, id: &str) -> &'a ComputedStyle {
    let node = d.tree.get(id_of(d, id)).unwrap();
    let g = node
        .children
        .iter()
        .find(|&&c| d.tree.get(c).is_some_and(|n| n.is_pseudo_element))
        .unwrap_or_else(|| panic!("{id} has no generated box"));
    &d.tree.get(*g).unwrap().computed_style
}

fn rgb(c: peniko::Color) -> [u8; 3] {
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    [r, g, b]
}

fn stroke(s: &ComputedStyle) -> (f32, Option<[u8; 3]>) {
    (
        s.text_paint.stroke_width,
        s.text_paint.stroke_color.map(rgb),
    )
}

fn display(s: &ComputedStyle) -> (DisplayValue, bool, bool, Option<u32>) {
    (s.display, s.flow_root, s.webkit_box, s.line_clamp)
}

fn height(d: &RinchDocument, id: &str) -> f32 {
    d.tree.get(id_of(d, id)).unwrap().layout.height
}

const GREEN: [u8; 3] = [0, 128, 0];
const RED: [u8; 3] = [255, 0, 0];

/// Chrome: `display: -webkit-box` and `-webkit-inline-box` through a `var()`,
/// as written out; a clamping one computes `flow-root`, one line tall (23px).
#[test]
fn display_through_var_makes_the_legacy_box_chrome_makes() {
    let d = doc();
    assert_eq!(display(style(&d, "d1")), display(style(&d, "d1x")));
    assert!(style(&d, "d1").webkit_box);
    assert_eq!(display(style(&d, "d2")), display(style(&d, "d2x")));
    assert_eq!(style(&d, "d2").display, DisplayValue::InlineBlock);
    let d3 = style(&d, "d3");
    assert_eq!(
        (d3.display, d3.line_clamp, height(&d, "d3")),
        (DisplayValue::Block, Some(1), 23.0)
    );
    // A `var()` naming any other display is Stylo's own.
    assert_eq!(
        (style(&d, "d4").display, style(&d, "d4").webkit_box),
        (DisplayValue::Block, false)
    );
}

/// Chrome: `40px hanging`, `10px each-line`; a child at another font size
/// inherits the `40px`; an invalid `var()` inherits the parent's `7px
/// hanging`.
#[test]
fn text_indent_keywords_through_var_are_chrome_s() {
    let d = doc();
    let ti = |id| {
        let t = &style(&d, id).text_indent;
        (t.length, t.hanging, t.each_line)
    };
    assert_eq!(ti("t1"), (LengthPercentageValue::Length(40.0), true, false));
    assert_eq!(
        ti("t1c"),
        (LengthPercentageValue::Length(40.0), true, false)
    );
    assert_eq!(ti("t2"), (LengthPercentageValue::Length(10.0), false, true));
    assert_eq!(ti("t3"), (LengthPercentageValue::Length(7.0), true, false));
    assert_eq!(ti("t4"), (LengthPercentageValue::Length(7.0), true, false));
}

/// Chrome: an invalid `var()` in `hyphens` gives the parent's `none`; one
/// whose fallback is `initial` (directly or through another `var()`) gives
/// `manual`.
#[test]
fn hyphens_left_invalid_by_its_var_inherits_as_chrome_does() {
    let d = doc();
    let got: Vec<_> = ["h1", "h2", "h3", "h4"]
        .iter()
        .map(|id| style(&d, id).hyphens_none)
        .collect();
    assert_eq!(got, [true, true, false, false]);
}

/// Chrome: an invalid `var()` in `word-break` gives the parent's (`break-all`,
/// and `break-word`, which is `overflow-wrap: anywhere`); a `break-word`
/// fallback under `break-all` is `break-word`, not the parent's `break-all`.
#[test]
fn word_break_left_invalid_by_its_var_inherits_as_chrome_does() {
    let d = doc();
    let wb = |id| (style(&d, id).word_break, style(&d, id).overflow_wrap);
    use OverflowWrapValue as O;
    use WordBreakValue as W;
    assert_eq!(wb("w1"), (W::BreakAll, O::Normal));
    assert_eq!(wb("w2"), (W::BreakAll, O::Normal));
    assert_eq!(wb("w3"), (W::Normal, O::Anywhere));
    assert_eq!(wb("wb1"), (W::Normal, O::Anywhere));
}

/// Chrome: `calc(2px + 1px)` through a `var()` is 3px; `var(--x, initial)`
/// is the initial stroke (the shorthand: 0px, own colour; the width alone:
/// 0px), and `var(--x, inherit)` the parent's; a `calc()` width written out
/// is 3px at 20px font size.
#[test]
fn stroke_calc_and_keyword_fallback_are_chrome_s() {
    let d = doc();
    assert_eq!(stroke(style(&d, "s1")), (3.0, Some(RED)));
    assert_eq!(stroke(style(&d, "s2")).0, 0.0);
    assert_eq!(stroke(style(&d, "s3")).0, 0.0);
    assert_eq!(stroke(style(&d, "s4")), (3.0, Some(GREEN)));
    assert_eq!(stroke(style(&d, "s7")), (6.0, Some(GREEN)));
}

/// Chrome: an invalid stroke on `::before` / `::after` is the originating
/// element's (6px green).
#[test]
fn a_pseudo_element_s_invalid_stroke_inherits_as_chrome_does() {
    let d = doc();
    assert_eq!(stroke(pseudo(&d, "s5")), (6.0, Some(GREEN)));
    assert_eq!(stroke(pseudo(&d, "s6")), (6.0, Some(GREEN)));
}

/// Chrome: a parent's `currentcolor` stroke, inherited through an invalid
/// `var()`, is the child's own `color` (red), not the parent's (blue).
#[test]
fn an_inherited_currentcolor_stroke_is_the_child_s_color() {
    let d = doc();
    assert_eq!(stroke(style(&d, "cc0")), (6.0, Some([0, 0, 255])));
    assert_eq!(stroke(style(&d, "cc1")), (6.0, Some(RED)));
}

/// What a value through a `var()` paints is what it written out paints, and
/// that differs from the box without it: the `calc()` stroke, the hanging
/// indent and the clamping legacy box.
#[cfg(feature = "software-renderer")]
#[test]
fn values_through_var_paint_as_written_out() {
    fn paint(decl: &str) -> Vec<u8> {
        let mut d = RinchDocument::new();
        let body = d.body();
        d.set_inner_html(
            body,
            &format!(
                r#"<div style="--c: calc(2px + 1px); --t: 2em hanging; --d: -webkit-box; font-size: 24px; line-height: 30px; width: 130px; color: gold; {decl}">77 77 77 77 77 77</div>"#
            ),
        );
        d.resolve_layout(200.0, 140.0);
        let mut painter = rinch_dom::paint::skia_painter::TinySkiaPainter::new(200, 140);
        painter.fill_white();
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &d.tree,
            &mut painter,
            1.0,
            (200.0, 140.0),
            &mut d.font_cx,
            &mut cx,
        );
        painter.pixels().to_vec()
    }
    let plain = paint("");
    for (through_var, written_out) in [
        (
            "-webkit-text-stroke: var(--c) red",
            "-webkit-text-stroke: 3px red",
        ),
        ("text-indent: var(--t)", "text-indent: 2em hanging"),
        (
            "display: var(--d); -webkit-box-orient: vertical; -webkit-line-clamp: 1",
            "display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 1",
        ),
    ] {
        let want = paint(written_out);
        assert!(want != plain, "{written_out} paints as the plain box");
        assert!(paint(through_var) == want, "{through_var} differs");
    }
}
