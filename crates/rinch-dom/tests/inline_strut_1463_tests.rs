//! Every inline box on a line gives it its strut, glyphs or not (#1463), and
//! a text span's `top`/`middle`/`bottom`/`text-top`/`text-bottom` places it
//! as Chrome does (#1357).
//!
//! CSS 2.1 §10.8.1: each inline box contributes its font's ascent, descent
//! and half-leading to the line box. parley added a span's box to a line only
//! from a text atom, so a span holding only an atomic inline, or nothing at
//! all, added nothing (parley fork `4e872e97`; rinch hands an empty inline
//! element a zero-size out-of-flow box, `ifc::EMPTY_INLINE_BOX`).
//!
//! Every number is Chrome 157's, from synthetic pages serving the bundled
//! Inter as `@font-face` in a 400px block at `font: 16px/20px`: the block's
//! `getBoundingClientRect().height`, and the top of a `Range` over a glyph
//! (its content area, so a fixed distance above its baseline). rinch's
//! baselines are on the pixel grid (a quantized layout), so a fractional
//! Chrome offset is compared within half a pixel, or three quarters where
//! parley rounds two terms (`middle`).
//!
//! | block content | Chrome 157 height | first glyph's top |
//! |---|---|---|
//! | `<span lh60><img 0x0></span>` | 60 | |
//! | `<span lh60><inline-block 5x5></span>` | 60 | |
//! | `x <span lh60><img 0x0></span>` | 60 | 20 |
//! | `x <span lh60></span>` | 60 | 20 |
//! | `x <span lh60><abspos></span>` | 60 | 20 |
//! | `x <span style="font-size: 32px"></span>y` | 25 | 5 |
//! | `x <span lh60; vertical-align: 10px><img 0x0></span>` | 60 | 30 |
//! | `<span lh60></span>` alone | 0 | |
//! | `x` (plain line) | 20 | 0 |

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BLOCK: &str = "margin: 0; width: 400px; font-family: ProbeFace; font-size: 16px; \
                     line-height: 20px";

fn doc() -> RinchDocument {
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

/// A block holding `html`, laid out: the document and the block.
fn block(html: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    let p = d.create_element("div");
    d.set_attribute(p, "style", BLOCK);
    d.append_child(body, p);
    d.set_inner_html(p, html);
    d.resolve_layout(800.0, 600.0);
    (d, p)
}

fn height(html: &str) -> f32 {
    let (d, p) = block(html);
    d.tree.get(p.0).unwrap().layout.height
}

/// The baseline of the glyph run holding layout byte `byte`, from the top
/// of the first line. A glyph run is one style's stretch of a shaped run,
/// so the byte's cluster names the style.
fn baseline_at(html: &str, byte: usize) -> f32 {
    let (d, p) = block(html);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    let top = il.layout.lines().next().unwrap().metrics().block_min_coord;
    let cluster = parley::layout::Cluster::from_byte_index(&il.layout, byte).unwrap();
    for item in cluster.line().items() {
        if let parley::layout::PositionedLayoutItem::GlyphRun(g) = item
            && g.style_index() == cluster.style_index()
            && g.run().text_range() == cluster.run().text_range()
        {
            return g.baseline() - top;
        }
    }
    panic!("no glyph run holds byte {byte} of {html:?}")
}

/// How far the first glyph's baseline is below the plain line's.
fn first_baseline_drop(html: &str) -> f32 {
    baseline_at(html, 0) - baseline_at("x", 0)
}

#[track_caller]
fn assert_chrome(got: f32, chrome: f32, tolerance: f32, what: &str) {
    assert!(
        (got - chrome).abs() <= tolerance,
        "{what}: rinch {got}, Chrome 157 {chrome}"
    );
}

const IMG0: &str = r#"<img style="width: 0; height: 0">"#;

#[test]
fn a_span_holding_only_an_atomic_inline_gives_its_line_its_line_height() {
    let alone = format!(r#"<span style="line-height: 60px">{IMG0}</span>"#);
    assert_chrome(height(&alone), 60.0, 0.5, "alone, 0x0 img");
    let ib = r#"<span style="line-height: 60px"><span style="display: inline-block; width: 5px; height: 5px"></span></span>"#;
    assert_chrome(height(ib), 60.0, 0.5, "alone, 5x5 inline-block");
    let after_text = format!(r#"x <span style="line-height: 60px">{IMG0}</span>"#);
    assert_chrome(height(&after_text), 60.0, 0.5, "after text");
    assert_chrome(
        first_baseline_drop(&after_text),
        20.0,
        0.5,
        "after text: the baseline",
    );
}

#[test]
fn an_empty_span_gives_its_line_its_strut() {
    let tall = r#"x <span style="line-height: 60px"></span>"#;
    assert_chrome(height(tall), 60.0, 0.5, "empty, line-height 60px");
    assert_chrome(
        first_baseline_drop(tall),
        20.0,
        0.5,
        "empty, 60px: baseline",
    );
    let big = r#"x <span style="font-size: 32px"></span>y"#;
    assert_chrome(height(big), 25.0, 0.5, "empty, 32px font");
    assert_chrome(first_baseline_drop(big), 5.0, 0.5, "empty, 32px: baseline");
    let abspos =
        r#"x <span style="line-height: 60px"><span style="position: absolute"></span></span>"#;
    assert_chrome(height(abspos), 60.0, 0.5, "holding only an abspos box");
}

/// The control: an empty span alone makes no line (CSS 2.1 §9.4.2).
#[test]
fn a_line_of_only_an_empty_span_stays_empty() {
    assert_eq!(height(r#"<span style="line-height: 60px"></span>"#), 0.0);
}

#[test]
fn a_raised_span_holding_only_an_image_raises_its_strut() {
    let html = format!(r#"x <span style="line-height: 60px; vertical-align: 10px">{IMG0}</span>"#);
    assert_chrome(height(&html), 60.0, 0.5, "raised");
    assert_chrome(first_baseline_drop(&html), 30.0, 0.5, "raised: baseline");
}

/// `alpha <span style="vertical-align: K; line-height: 40px">1</span> beta`:
/// the tall span is placed by its keyword and the root's text moves (Chrome
/// 157, the root text's top against the same line with a `baseline` span).
#[test]
fn a_tall_text_span_aligned_by_a_keyword_moves_the_roots_text() {
    let line = |va: &str| format!(r#"alpha <span style="{va}line-height: 40px">1</span> beta"#);
    let control = baseline_at(&line(""), 0);
    for (va, chrome) in [
        ("top", -10.0),
        ("middle", -1.625),
        ("bottom", 10.0),
        ("text-top", -10.0),
        ("text-bottom", 10.0),
    ] {
        let html = line(&format!("vertical-align: {va}; "));
        assert_chrome(height(&html), 40.0, 0.5, va);
        assert_chrome(baseline_at(&html, 0) - control, chrome, 0.75, va);
        // The span's own text stays where the baseline span's is.
        assert_chrome(
            baseline_at(&html, 6) - baseline_at(&line(""), 6),
            0.0,
            0.5,
            va,
        );
    }
}

/// `alpha <span style="vertical-align: K; font-size: 8px; line-height: 8px">1</span> beta`:
/// the small span moves (Chrome 157, its glyph's top against the same span
/// on the baseline).
#[test]
fn a_small_text_span_aligned_by_a_keyword_moves() {
    let line = |va: &str| {
        format!(r#"alpha <span style="{va}font-size: 8px; line-height: 8px">1</span> beta"#)
    };
    let control = baseline_at(&line(""), 6);
    // `chrome_top`: the top of a `Range` over the `1` in the block.
    for (va, chrome, chrome_top) in [
        ("top", -9.0, -1.0),
        ("middle", -1.375, 6.625),
        ("bottom", 3.0, 11.0),
        ("text-top", -9.0, -1.0),
        ("text-bottom", 3.0, 11.0),
    ] {
        let html = line(&format!("vertical-align: {va}; "));
        assert_chrome(height(&html), 20.0, 0.5, va);
        assert_chrome(baseline_at(&html, 6) - control, chrome, 0.75, va);
        // The glyph's bounds are its own content area where parley put it.
        let (d, p) = block(&html);
        let bounds = rinch_dom::text_query::glyph_bounds_for_offset(&d, p.0, 6).unwrap();
        assert_chrome(
            bounds.y,
            chrome_top,
            0.75,
            &format!("{va}: glyph bounds top"),
        );
    }
}
