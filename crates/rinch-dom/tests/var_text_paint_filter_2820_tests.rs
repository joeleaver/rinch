//! `var()` in the properties rinch carries through custom properties
//! (`webkit_box::rewrite`): `-webkit-text-stroke` and its longhands,
//! `background-clip: text`, `-webkit-line-clamp`, `-webkit-box-orient`,
//! `-webkit-box-flex`, `hyphens` and `word-break` (#2820), and a `var()` in a
//! `drop-shadow()` filter, which Stylo substitutes itself.
//!
//! **Against Chrome.** Every expected value below is what Chrome 158 computes
//! for the same element (`getComputedStyle`), on the page [`PAGE`] with
//! [`CSS`], read by
//!
//! ```text
//! google-chrome --headless=new --no-sandbox --disable-gpu --dump-dom file://<the page>
//! ```
//!
//! with a script writing each element's `-webkit-text-stroke-width`,
//! `-webkit-text-stroke-color`, `filter`, `-webkit-line-clamp`,
//! `-webkit-box-orient`, `background-clip`, `hyphens`, `word-break` and
//! `-webkit-box-flex` into the page. Chrome treats a shorthand whose
//! substituted value is no `-webkit-text-stroke` (`h1`: an undefined `var()`
//! with no fallback; `i1`: two widths) as invalid at computed-value time, so
//! its longhands inherit the parent's stroke.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{ComputedStyle, FilterOp, OverflowWrapValue};

const CSS: &str = "
.cls { --ink2: rgb(1, 2, 3); }
.cls span.s { -webkit-text-stroke: 4px var(--ink2); }
";

const PAGE: &str = r#"<div id="root" style="--ink: #3A1E08; --w: 5px; --s: 3px blue; --clip: text; --n: 2; --o: vertical; --f: 2; --h: none; --wb: break-word; font-size: 20px; color: gold">
  <span id="a" style="-webkit-text-stroke: 8px var(--ink)">7</span>
  <span id="b" style="-webkit-text-stroke: var(--missing, #123456) 2px">7</span>
  <span id="c" style="-webkit-text-stroke: var(--w) red">7</span>
  <span id="d" style="-webkit-text-stroke: var(--s)">7</span>
  <span id="e" style="-webkit-text-stroke: 8px var(--ink); -webkit-text-stroke-color: red">7</span>
  <span id="f" style="-webkit-text-stroke: 8px var(--missing)">7</span>
  <span id="g" style="-webkit-text-stroke: 8px var(--ink)"><b id="g1" style="--ink: red">7</b></span>
  <span id="h" style="-webkit-text-stroke: 6px green"><b id="h1" style="-webkit-text-stroke: 8px var(--missing)">7</b></span>
  <span id="i" style="-webkit-text-stroke: 6px green"><b id="i1" style="-webkit-text-stroke: 8px var(--w) red">7</b></span>
  <span id="j" style="-webkit-text-stroke-color: var(--ink); -webkit-text-stroke-width: var(--w)">7</span>
  <div class="cls"><span class="s" id="k">7</span></div>
  <span id="l" style="-webkit-text-stroke: var(--w)">7</span>
  <span id="v" style="-webkit-text-stroke: 6px green"><b id="v1" style="-webkit-text-stroke-width: initial">7</b></span>
  <div id="m" style="filter: drop-shadow(0 6px 0 var(--ink))">x</div>
  <div id="n" style="filter: drop-shadow(1px 2px 3px var(--missing, rgb(4, 5, 6)))">x</div>
  <div id="o" style="display: -webkit-box; -webkit-box-orient: var(--o); -webkit-line-clamp: var(--n); overflow: hidden">x</div>
  <div id="p" style="-webkit-line-clamp: var(--missing)">x</div>
  <div id="q" style="background-clip: var(--clip); -webkit-background-clip: var(--clip)">x</div>
  <div id="q2" style="background-clip: var(--missing, padding-box)">x</div>
  <div id="r" style="-webkit-hyphens: var(--h); hyphens: var(--h)">x</div>
  <div id="t" style="-webkit-box-flex: var(--f)">x</div>
  <div id="u" style="word-break: var(--wb)">x</div>
</div>"#;

fn doc(css: &str, html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(css);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

fn style<'a>(d: &'a RinchDocument, id: &str) -> &'a ComputedStyle {
    let n = d.query_selector_all(&format!("#{id}"))[0];
    &d.tree.get(n.0).unwrap().computed_style
}

fn rgb(c: peniko::Color) -> [u8; 3] {
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    [r, g, b]
}

/// An element's id and its [`stroke`].
type Row = (&'static str, (f32, Option<[u8; 3]>));

/// `id`'s stroke: its width and, where it has one, its colour (`color` for
/// `currentcolor`). Chrome's colour where the width is `0` is not drawn, and
/// rinch keeps none.
fn stroke(d: &RinchDocument, id: &str) -> (f32, Option<[u8; 3]>) {
    let p = &style(d, id).text_paint;
    (p.stroke_width, p.stroke_color.map(rgb))
}

const GOLD: [u8; 3] = [255, 215, 0];
const INK: [u8; 3] = [58, 30, 8];
const GREEN: [u8; 3] = [0, 128, 0];

#[test]
fn a_var_in_the_text_stroke_shorthand_is_substituted_as_chrome_does() {
    let d = doc(CSS, PAGE);
    let expect: &[Row] = &[
        ("a", (8.0, Some(INK))),
        // The fallback, the colour written before the width.
        ("b", (2.0, Some([18, 52, 86]))),
        // The width substituted.
        ("c", (5.0, Some([255, 0, 0]))),
        // The whole shorthand substituted.
        ("d", (3.0, Some([0, 0, 255]))),
        // A later longhand beats the shorthand's half.
        ("e", (8.0, Some([255, 0, 0]))),
        // An undefined `var()` with no fallback, under no stroke.
        ("f", (0.0, None)),
        // The child inherits the colour substituted on the parent, not its
        // own `--ink`.
        ("g", (8.0, Some(INK))),
        ("g1", (8.0, Some(INK))),
        // A rule's `var()` from a class.
        ("k", (4.0, Some([1, 2, 3]))),
        // The whole shorthand a `var()` holding the width alone (the colour
        // is `currentcolor`).
        ("l", (5.0, Some(GOLD))),
        // The longhands, which carried a `var()` before #2820 too.
        ("j", (5.0, Some(INK))),
    ];
    let got: Vec<_> = expect.iter().map(|(id, _)| (*id, stroke(&d, id))).collect();
    assert_eq!(got, expect.to_vec());
}

/// A shorthand invalid at computed-value time inherits the parent's stroke
/// (`h1`, `i1`); a longhand set to `initial` does not (`v1`).
#[test]
fn a_text_stroke_left_invalid_by_its_var_inherits_as_chrome_does() {
    let d = doc(CSS, PAGE);
    let expect: &[Row] = &[
        ("h", (6.0, Some(GREEN))),
        ("h1", (6.0, Some(GREEN))),
        ("i", (6.0, Some(GREEN))),
        ("i1", (6.0, Some(GREEN))),
        ("v1", (0.0, None)),
    ];
    let got: Vec<_> = expect.iter().map(|(id, _)| (*id, stroke(&d, id))).collect();
    assert_eq!(got, expect.to_vec());
}

#[test]
fn a_var_in_a_drop_shadow_is_substituted_as_chrome_does() {
    let d = doc(CSS, PAGE);
    let shadow = |id| match style(&d, id).filter.as_slice() {
        [
            FilterOp::DropShadow {
                offset_x,
                offset_y,
                std_dev,
                color,
            },
        ] => (*offset_x, *offset_y, *std_dev, color.map(rgb)),
        other => panic!("{id}: {other:?}"),
    };
    // Chrome: `drop-shadow(rgb(58, 30, 8) 0px 6px 0px)` and
    // `drop-shadow(rgb(4, 5, 6) 1px 2px 3px)`.
    assert_eq!(shadow("m"), (0.0, 6.0, 0.0, Some(INK)));
    assert_eq!(shadow("n"), (1.0, 2.0, 3.0, Some([4, 5, 6])));
}

#[test]
fn a_var_in_the_other_rewritten_properties_is_substituted_as_chrome_does() {
    let d = doc(CSS, PAGE);
    // Chrome: `-webkit-line-clamp: 2`, `-webkit-box-orient: vertical`.
    let o = style(&d, "o");
    assert_eq!(
        (o.webkit_line_clamp, o.webkit_box_vertical, o.line_clamp),
        (Some(2), true, Some(2))
    );
    // Chrome: `none` for an undefined `var()`.
    assert_eq!(style(&d, "p").webkit_line_clamp, None);
    // Chrome: `background-clip: text`, and the fallback `padding-box`.
    assert_eq!(style(&d, "q").background_clip_text, vec![true]);
    assert!(style(&d, "q2").background_clip_text.is_empty());
    // Chrome: `hyphens: none`, `-webkit-box-flex: 2`, `word-break:
    // break-word`.
    assert!(style(&d, "r").hyphens_none);
    assert_eq!(style(&d, "t").webkit_box_flex, 2.0);
    assert_eq!(style(&d, "u").overflow_wrap, OverflowWrapValue::Anywhere);
}

/// The issue's numeral: a stroke through `var()` paints exactly what the
/// colour written out paints, and that is not plain text.
#[cfg(feature = "software-renderer")]
#[test]
fn a_stroke_through_var_paints_as_the_colour_written_out() {
    fn paint(decl: &str) -> Vec<u8> {
        let mut d = doc(
            "",
            &format!(
                r#"<div style="--ink: #3A1E08; font-size: 96px; line-height: 120px"><span style="color: gold; {decl}">7</span></div>"#
            ),
        );
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
    let through_var = paint("-webkit-text-stroke: 8px var(--ink)");
    let written_out = paint("-webkit-text-stroke: 8px #3A1E08");
    assert!(written_out != paint(""), "the stroke paints nothing");
    assert!(
        through_var == written_out,
        "the stroke through var() differs"
    );
}
