//! `background-clip: text`, `-webkit-text-fill-color`, `-webkit-text-stroke`
//! and `paint-order` on HTML text, on the software painter (#2719).
//!
//! **Against Chrome.** `tests/fixtures/text_clip_stroke_2719.html` is the
//! page; Chrome 157's screenshots of it are
//! `tests/fixtures/text_clip_stroke_2719_chrome_{1,2}x.png`, the top-left
//! 600x410 CSS px of
//!
//! ```text
//! google-chrome --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
//!     --force-device-scale-factor=S --window-size=600,510 \
//!     --user-data-dir=<a fresh dir> --screenshot=out.png file://<the page>
//! ```
//!
//! for `S` in 1 and 2, kept as RGB. The test paints the page's `<body>`
//! (its script, which selects the last line in Chrome, left out) with the
//! page's style sheet and the same bundled Inter, and compares each feature's
//! region. Two engines' glyphs differ at their edges by hinting and sub-pixel
//! placement, so each region is compared through a 3x3 box blur: its mean
//! difference and how many pixels differ by more than a quarter of full scale.
//! Each tolerance sits well under what a broken feature costs (the mutants in
//! the PR: no clip, no stroke, the paint order swapped).
//!
//! **Selection.** Chrome draws a selection's text in the `::selection`
//! style, and since highlight inheritance that style's `-webkit-text-stroke`
//! is its own initial `0`: under an author `::selection` rule the selected
//! text loses its stroke (the fixture's "Sel 42"), while under Chrome's
//! default selection it keeps it, filled in the highlight text colour. rinch
//! has no `::selection` styling: its selection is a wash under the text,
//! which is drawn as it is unselected, stroke included. The selection test
//! pins the wash's colour against Chrome's and the stroke kept.

#![cfg(feature = "software-renderer")]

#[path = "support/text_clip_stroke_2719.rs"]
mod twin;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use twin::*;

fn paint(d: &mut RinchDocument, w: u32, h: u32, scale: f64) -> Image {
    let (pw, ph) = (
        (w as f64 * scale).round() as u32,
        (h as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        scale,
        (w as f32, h as f32),
        &mut d.font_cx,
        &mut cx,
    );
    Image::from_rgba(painter.pixels().as_chunks::<4>().0, pw, ph)
}

fn render(html: &str, w: u32, h: u32, scale: f64) -> Image {
    let mut d = doc(html, w as f32, h as f32);
    paint(&mut d, w, h, scale)
}

fn chrome(png: &[u8]) -> Image {
    let img = image::load_from_memory(png).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    Image {
        px: img.pixels().map(|p| p.0).collect(),
        w,
        h,
    }
}

/// Paint the page at `scale` and compare `regions` with Chrome's screenshot.
#[track_caller]
fn against_chrome(scale: u32, regions: &[Region]) {
    let ours = render(&page_body(), W, H, scale as f64);
    let theirs = chrome(if scale == 1 { CHROME_1X } else { CHROME_2X });
    let bad = unlike_chrome(&ours, &theirs, scale, regions);
    assert!(bad.is_empty(), "regions unlike Chrome at {scale}x: {bad:?}");
}

#[test]
fn gradients_clipped_to_text_match_chrome() {
    for scale in [1, 2] {
        against_chrome(scale, &[LINEAR, RADIAL, PADDING]);
    }
}

#[test]
fn stroke_widths_and_paint_order_match_chrome() {
    for scale in [1, 2] {
        against_chrome(scale, &[WIDTHS, STROKE_FIRST]);
    }
}

#[test]
fn a_hud_numeral_and_shadows_with_a_stroke_match_chrome() {
    for scale in [1, 2] {
        against_chrome(scale, &[HUD, FILL_COLOR_SHADOW, TRANSPARENT_SHADOW]);
    }
}

#[test]
fn a_paragraph_and_inline_spans_match_chrome() {
    for scale in [1, 2] {
        against_chrome(scale, &[PARAGRAPH, INLINE, MIXED]);
    }
}

/// Chrome's selection background (the fixture's `::selection` colour, rinch's
/// wash) is where rinch draws its wash, and the selected text keeps its
/// stroke, as under Chrome's default selection (see the module docs).
#[test]
fn a_selection_over_stroked_text_is_the_wash_under_the_stroked_glyphs() {
    let mut d = doc(&page_body(), W as f32, H as f32);
    let unselected = paint(&mut d, W, H, 1.0);
    let sel = d.query_selector("#sel").unwrap();
    d.set_attribute(sel, "data-text-sel", "true");
    d.set_attribute(sel, "data-text-sel-start", "0");
    d.set_attribute(sel, "data-text-sel-end", "6");
    d.resolve_layout(W as f32, H as f32);
    let selected = paint(&mut d, W, H, 1.0);
    let theirs = chrome(CHROME_1X);
    // Between the glyphs, inside the line: the wash over white.
    for (x, y) in [(9, 352), (127, 352), (70, 396)] {
        let (a, b) = (selected.at(x, y), theirs.at(x, y));
        assert!(
            (0..3).all(|c| a[c].abs_diff(b[c]) <= 3),
            "({x}, {y}): rinch {a:?}, Chrome {b:?}"
        );
        assert_eq!(unselected.at(x, y), [255, 255, 255]);
    }
    // The stroke is drawn over the wash: where it is opaque, the pixel is
    // the stroke's whether or not the text is selected.
    let navy = [0x11, 0x22, 0x33];
    let stroked: Vec<(u32, u32)> = (351..398)
        .flat_map(|y| (7..140).map(move |x| (x, y)))
        .filter(|&(x, y)| unselected.at(x, y) == navy)
        .collect();
    assert!(
        stroked.len() > 200,
        "{} opaque stroke pixels",
        stroked.len()
    );
    for &(x, y) in &stroked {
        assert_eq!(selected.at(x, y), navy, "({x}, {y})");
    }
}

/// The line cull (#2314) counts a stroke's reach: in a partial repaint whose
/// damage ends above a line, the line whose glyphs' face box (Inter's `head`
/// box, 1.108 em above the baseline, plus the cull's 2 px) misses the damage
/// and whose 40px stroke (20px past the cap height) reaches into it is drawn
/// there; the lines below it, which even the stroke cannot reach from, are
/// still culled. The third line (baseline 85.9) is longer than the second,
/// so past the second's end only its stroke can ink the damage's last rows.
#[test]
fn a_stroke_reaching_into_the_damage_keeps_its_line_from_the_cull() {
    use rinch_dom::perf::Counter;
    let html = "<p style='position:absolute; left:10px; top:0; margin:0; font-size:30px; \
                line-height:30px; -webkit-text-stroke:40px #000; color:#000'>H<br>H<br>\
                HHHHHHHH<br>H<br>H<br>H<br>H<br>H</p>";
    let mut d = doc(html, 400.0, 300.0);
    let damage = peniko::kurbo::Rect::new(0.0, 0.0, 400.0, 48.0);
    let mut frame = |cull: bool| {
        let was = rinch_dom::paint::set_text_line_cull(cull);
        rinch_dom::paint::set_dirty_region(Some(damage));
        d.tree.perf.reset();
        let img = paint(&mut d, 400, 300, 1.0);
        rinch_dom::paint::set_dirty_region(None);
        rinch_dom::paint::set_text_line_cull(was);
        (img, d.tree.perf.end_frame().get(Counter::TextLinesCulled))
    };
    let (culled, n) = frame(true);
    let (drawn, _) = frame(false);
    assert!(n > 0, "the lines past the stroke's reach are culled");
    let inked = (100..180)
        .filter(|&x| drawn.at(x, 46) != [255, 255, 255])
        .count();
    assert!(
        inked > 40,
        "{inked} pixels of the third line's stroke at y = 46"
    );
    let differ: Vec<(u32, u32)> = (0..48)
        .flat_map(|y| (0..400).map(move |x| (x, y)))
        .filter(|&(x, y)| culled.at(x, y) != drawn.at(x, y))
        .collect();
    assert!(
        differ.is_empty(),
        "{} pixels the cull changed, first {:?}",
        differ.len(),
        differ.first()
    );
}

/// Restyle `#t` with `style`, then repaint only the damage the frame names
/// over the old paint: it must be the full paint of the new state. A stroke
/// widens the text's ink past its box (#2719), so the damage of a change to
/// stroked text must reach as far.
fn restyle_repaints_the_stroke(html: &str, style: &str) {
    use peniko::Fill;
    use peniko::kurbo::{Affine, Rect};
    use rinch_dom::paint::painter::{PaintShape, Painter};
    const VP: (f32, f32) = (300.0, 200.0);
    let mut d = doc(html, VP.0, VP.1);
    let full = |d: &mut RinchDocument, p: &mut TinySkiaPainter| {
        p.fill_white();
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&d.tree, p, 1.0, VP, &mut d.font_cx, &mut cx);
    };
    let mut p = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    full(&mut d, &mut p);
    d.tree.consume_paint_dirty();
    let t = d.query_selector("#t").unwrap();
    d.set_attribute(t, "style", style);
    d.resolve_layout(VP.0, VP.1);
    let damage = rinch_dom::paint::compute_damage(&d.tree, 1.0, VP.0 as f64, VP.1 as f64);
    assert!(!damage.is_full(), "precondition: a partial repaint");
    let rects = damage.rects().to_vec();
    for r in &rects {
        let r = r.intersect(Rect::new(0.0, 0.0, VP.0 as f64, VP.1 as f64));
        p.clear_rect_white(
            r.x0 as u32,
            r.y0 as u32,
            r.width().ceil() as u32,
            r.height().ceil() as u32,
        );
    }
    rinch_dom::paint::set_dirty_rects(Some(&rects));
    for r in &rects {
        p.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(*r));
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&d.tree, &mut p, 1.0, VP, &mut d.font_cx, &mut cx);
        p.pop_layer();
    }
    rinch_dom::paint::set_dirty_region(None);
    let partial = p.pixels().to_vec();
    d.tree.consume_paint_dirty();
    full(&mut d, &mut p);
    // A stroke drawn through the damage's clip mask rounds an edge pixel a
    // level apart from one drawn without it: a missed repaint is the old
    // colour, far more than that.
    let missed: Vec<(usize, usize)> = partial
        .as_chunks::<4>()
        .0
        .iter()
        .zip(p.pixels().as_chunks::<4>().0)
        .enumerate()
        .filter(|(_, (a, b))| (0..4).any(|c| a[c].abs_diff(b[c]) > 2))
        .map(|(i, _)| (i % VP.0 as usize, i / VP.0 as usize))
        .collect();
    assert!(
        missed.is_empty(),
        "{} px of the restyled stroke were not repainted ({rects:?}), first {:?}",
        missed.len(),
        missed.first()
    );
}

/// A block's own stroke, 9px past the glyphs, over a line box that holds
/// them tightly: recoloured, it is repainted where it reaches past the box.
#[test]
fn recolouring_a_stroke_past_its_box_repaints_all_of_it() {
    restyle_repaints_the_stroke(
        "<div id=t style='position:absolute; left:40px; top:50px; font-size:40px; \
         line-height:30px; -webkit-text-stroke:18px #000'>WA</div>",
        "position:absolute; left:40px; top:50px; font-size:40px; line-height:30px; \
         -webkit-text-stroke:18px #c00",
    );
}

/// An inline element's own stroke, in a paragraph that strokes nothing: its
/// root's ink takes it (its members are walked, as for a `text-shadow`).
#[test]
fn recolouring_an_inline_stroke_past_its_root_repaints_all_of_it() {
    restyle_repaints_the_stroke(
        "<p style='position:absolute; left:40px; top:50px; margin:0; font-size:40px; \
         line-height:30px'>x<span id=t style='-webkit-text-stroke:18px #000'>WA</span></p>",
        "-webkit-text-stroke:18px #c00",
    );
}

/// A shrink-to-fit `overflow: hidden` box whose 12px stroke reaches past
/// its edges clips it there, as Chrome does: the clip is not elided as if
/// its text fitted (`layer_bounds`' walk counts the stroke's reach, review
/// of #2729). Without the clip the same text strokes past the box.
#[test]
fn an_overflow_clip_cuts_a_stroke_reaching_past_the_box() {
    let page = |overflow: &str| {
        format!(
            "<div id=t style='position:absolute; left:50px; top:50px; font-size:40px; \
             line-height:60px; -webkit-text-stroke:12px #000; color:#000; {overflow}'>HH</div>"
        )
    };
    let ink_right_of_box = |overflow: &str| {
        let mut d = doc(&page(overflow), 300.0, 200.0);
        let l = d
            .tree
            .get(d.query_selector("#t").unwrap().0)
            .unwrap()
            .layout;
        let right = (50.0 + l.width).ceil() as u32;
        let img = paint(&mut d, 300, 200, 1.0);
        let outside = (60..100)
            .flat_map(|y| (right + 1..right + 8).map(move |x| (x, y)))
            .filter(|&(x, y)| img.at(x, y) != [255, 255, 255])
            .count();
        let inside = (60..100)
            .flat_map(|y| (right - 4..right).map(move |x| (x, y)))
            .filter(|&(x, y)| img.at(x, y) != [255, 255, 255])
            .count();
        (outside, inside)
    };
    let (open_out, _) = ink_right_of_box("");
    assert!(
        open_out > 20,
        "precondition: the stroke reaches past the box ({open_out} px)"
    );
    let (out, inside) = ink_right_of_box("overflow:hidden");
    assert!(
        inside > 20,
        "the stroke is drawn up to the edge ({inside} px)"
    );
    assert_eq!(out, 0, "px of stroke drawn past an overflow: hidden box");
}

/// The layer an `opacity` opens holds the whole stroke, `2 x width` past the
/// glyphs' line box (a mitred tip), on the element and on an inline member.
#[test]
fn an_opacity_layer_holds_the_stroke() {
    for html in [
        "<div id=t style='position:absolute; left:50px; top:50px; font-size:40px; \
         line-height:40px; opacity:0.5; -webkit-text-stroke:12px #000'>HH</div>",
        "<div id=t style='position:absolute; left:50px; top:50px; font-size:40px; \
         line-height:40px; opacity:0.5'><span style='-webkit-text-stroke:12px #000'>HH</span></div>",
    ] {
        let d = doc(html, 300.0, 200.0);
        let id = d.query_selector("#t").unwrap().0;
        let l = d.tree.get(id).unwrap().layout;
        let b = rinch_dom::paint::opacity_layer_bounds(&d.tree, id, 1.0, 50.0, 50.0);
        // Two stroke widths (24px) past the box on every side.
        assert!(b.x0 <= 50.0 - 24.0 && b.y0 <= 50.0 - 24.0, "{html}: {b:?}");
        assert!(
            b.x1 >= 50.0 + l.width as f64 + 24.0 && b.y1 >= 50.0 + l.height as f64 + 24.0,
            "{html}: {b:?} for a {}x{} box",
            l.width,
            l.height
        );
    }
}

/// `background-clip: text` takes the text of an inline run beside a block
/// child, which lives in an anonymous block box (the box tree, not the DOM
/// children; review of #2729): Chrome paints "ab" in the gradient.
#[test]
fn a_clip_takes_inline_text_beside_a_block_child() {
    let mut d = doc(
        "<div style='position:absolute; left:10px; top:10px; width:200px; font-size:40px; \
         line-height:50px; background:#e00; background-clip:text; color:transparent'>\
         HH<div>HH</div></div>",
        300.0,
        200.0,
    );
    let img = paint(&mut d, 300, 200, 1.0);
    let red = |y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (10..120).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = img.at(x, y);
                p[0] > 200 && p[1] < 60
            })
            .count()
    };
    assert!(
        red(10, 60) > 100,
        "the anonymous run's \"HH\" is clipped in ({} px)",
        red(10, 60)
    );
    assert!(
        red(60, 110) > 100,
        "the block child's \"HH\" too ({} px)",
        red(60, 110)
    );
}

/// The stroke is mitred, as Skia strokes text: an `A`'s apex, 25px wide,
/// carries a tip far above where a round or bevel join would stop.
#[test]
fn the_stroke_is_mitred() {
    let mut d = doc(
        "<div style='position:absolute; left:40px; top:60px; font-size:60px; line-height:60px; \
         -webkit-text-stroke:24px #000; color:#000'>A</div>",
        300.0,
        200.0,
    );
    let img = paint(&mut d, 300, 200, 1.0);
    let top = (0..200u32)
        .find(|&y| (0..300).any(|x| img.at(x, y) != [255, 255, 255]))
        .unwrap();
    // The apex of Inter's A sits at the cap height (0.727 em, 43.6px) above
    // a baseline at about 108: a round join reaches 12px above it (y 52), a
    // mitre at Skia's limit about 35 px (y 29).
    assert!(top < 40, "the stroke's top row is {top}: not mitred");
}

/// An `<input>`'s value is stroked and filled as its style says, as Chrome
/// draws it: a red stroke around transparent glyphs.
#[test]
fn an_input_value_is_stroked() {
    let mut d = doc(
        "<input value='88' style='position:absolute; left:10px; top:10px; width:150px; \
         font-family:InterT; font-size:40px; border:0; padding:0; color:#00f; \
         -webkit-text-fill-color:transparent; -webkit-text-stroke:3px #f00'>",
        300.0,
        200.0,
    );
    let img = paint(&mut d, 300, 200, 1.0);
    let (mut red, mut blue) = (0, 0);
    for y in 10..70 {
        for x in 10..160 {
            let p = img.at(x, y);
            red += usize::from(p[0] > 200 && p[1] < 80 && p[2] < 80);
            blue += usize::from(p[2] > 200 && p[0] < 80);
        }
    }
    assert!(red > 50, "{red} stroke px");
    assert_eq!(blue, 0, "the transparent fill draws no glyph in `color`");
}

// ── Computed values ─────────────────────────────────────────────────────────

fn style_of<'d>(d: &'d RinchDocument, sel: &str) -> &'d rinch_dom::computed_style::ComputedStyle {
    let NodeId(id) = d.query_selector(sel).unwrap_or_else(|| panic!("{sel}"));
    &d.tree.get(id).unwrap().computed_style
}

#[test]
fn the_properties_reach_the_computed_style_from_a_style_sheet() {
    let d = doc_with(
        "#a { color: #f00; -webkit-text-stroke: 3.5px #00f; paint-order: stroke fill; \
              -webkit-text-fill-color: transparent; \
              background: linear-gradient(red, blue); -webkit-background-clip: text; } \
         #c { -webkit-text-stroke-width: 0.1em; font-size: 30px; -webkit-text-stroke-color: currentcolor; color: #0f0; \
              paint-order: markers; background-clip: text, border-box; } \
         #d { -webkit-text-stroke: thick; -webkit-text-fill-color: currentcolor; }",
        "<div id=a>A<span id=b>B</span></div><div id=c>C</div><div id=d>D</div>",
        400.0,
        300.0,
    );
    let a = style_of(&d, "#a");
    assert_eq!(a.text_paint.stroke_width, 3.5);
    assert_eq!(
        a.text_paint.stroke_color.unwrap().to_rgba8().to_u8_array(),
        [0, 0, 255, 255]
    );
    assert!(a.text_paint.stroke_first);
    assert_eq!(a.text_paint.fill.unwrap().components[3], 0.0);
    assert_eq!(a.background_clip_text, vec![true]);
    // Inherited, all four; `background-clip` is not.
    let b = style_of(&d, "#b");
    assert_eq!(b.text_paint, a.text_paint);
    assert!(b.background_clip_text.is_empty());
    // An `em` width, `currentcolor`, a list without `stroke` before `fill`,
    // and a per-layer clip list.
    let c = style_of(&d, "#c");
    assert_eq!(c.text_paint.stroke_width, 3.0);
    assert_eq!(
        c.text_paint.stroke_color.unwrap().to_rgba8().to_u8_array(),
        [0, 255, 0, 255]
    );
    assert!(!c.text_paint.stroke_first);
    assert_eq!(c.background_clip_text, vec![true, false]);
    // `thick` is 5px in the text's colour; a `currentcolor` fill is none.
    let dd = style_of(&d, "#d");
    assert_eq!(dd.text_paint.stroke_width, 5.0);
    assert_eq!(dd.text_paint.fill, None);
    assert!(d.tree.text_paint_effects);
}

#[test]
fn the_cascade_decides_background_clip_text() {
    let d = doc_with(
        // A `background` shorthand after the clip resets it, one before it
        // does not; a later rule's `padding-box` beats an earlier `text`;
        // `!important` beats a later rule.
        ".g { background: linear-gradient(red, blue); } \
         #a { -webkit-background-clip: text; background: red; } \
         #b { background: red; -webkit-background-clip: text; } \
         #c { background-clip: text; } #c.late { background-clip: padding-box; } \
         #e { background-clip: text !important; } #e.late { background-clip: padding-box; } \
         #f { background-clip: text; } #f.late { background-clip: nonsense; } \
         #g { background-clip: inherit; } #h { background-clip: initial; }",
        "<div id=a>a</div><div id=b>b</div><div id=c class=late>c</div>\
         <div id=e class=late>e</div><div id=f class=late>f<p id=g>g</p><p id=h>h</p></div>",
        400.0,
        300.0,
    );
    assert!(style_of(&d, "#a").background_clip_text.is_empty());
    assert_eq!(style_of(&d, "#b").background_clip_text, vec![true]);
    assert!(style_of(&d, "#c").background_clip_text.is_empty());
    assert_eq!(style_of(&d, "#e").background_clip_text, vec![true]);
    // An invalid declaration is dropped: the earlier `text` stands.
    assert_eq!(style_of(&d, "#f").background_clip_text, vec![true]);
    // `inherit` takes the parent's `text`; `initial` is `border-box`.
    assert_eq!(style_of(&d, "#g").background_clip_text, vec![true]);
    assert!(style_of(&d, "#h").background_clip_text.is_empty());
}

#[test]
fn invalid_values_are_dropped_and_inherited_ones_stand() {
    let d = doc_with(
        "#p { -webkit-text-stroke: 2px #123; -webkit-text-fill-color: #0f0; paint-order: stroke; } \
         #k { -webkit-text-stroke-width: -3px; -webkit-text-fill-color: banana; paint-order: stroke stroke; } \
         #v { --w: 4px; -webkit-text-stroke-width: var(--w); }",
        "<div id=p><span id=k>k</span><span id=v>v</span></div>",
        400.0,
        300.0,
    );
    let k = style_of(&d, "#k");
    assert_eq!(k.text_paint, style_of(&d, "#p").text_paint);
    assert_eq!(style_of(&d, "#v").text_paint.stroke_width, 4.0);
}

#[test]
fn plain_text_turns_nothing_on() {
    let d = doc(
        "<div style='color: red; background: linear-gradient(red, blue)'>plain <b>text</b></div>",
        400.0,
        300.0,
    );
    assert!(!d.tree.text_paint_effects);
}

// ── The text is still text ──────────────────────────────────────────────────

/// A stroke, a fill colour and a clip paint differently and lay out and hit
/// test exactly as the plain text does: the same lines, the same glyph
/// under the same point.
#[test]
fn a_stroked_clipped_text_lays_out_and_hit_tests_as_plain_text() {
    let html = |extra: &str| {
        format!(
            "<div id=t style='position:absolute; left:11px; top:7px; width:173px; \
             font-size:29px; line-height:35px; {extra}'>Lap 3 of 12, best 1:23.456</div>"
        )
    };
    let plain = doc(&html(""), 400.0, 300.0);
    let fancy = doc(
        &html(
            "-webkit-text-stroke: 4px #000; paint-order: stroke fill; color: transparent; \
             background: linear-gradient(red, blue); background-clip: text",
        ),
        400.0,
        300.0,
    );
    fn node(d: &RinchDocument) -> &rinch_dom::Node {
        d.tree.get(d.query_selector("#t").unwrap().0).unwrap()
    }
    let (p, f) = (node(&plain), node(&fancy));
    assert_eq!(p.layout.width, f.layout.width);
    assert_eq!(p.layout.height, f.layout.height);
    let (lp, lf) = (
        &p.text_layout.as_ref().unwrap().layout,
        &f.text_layout.as_ref().unwrap().layout,
    );
    assert_eq!(lp.lines().count(), lf.lines().count());
    assert!(lp.lines().count() > 1, "the text wraps");
    for (x, y) in [(3.0, 10.0), (57.0, 20.0), (101.0, 50.0), (20.0, 80.0)] {
        assert_eq!(
            rinch_dom::text_query::glyph_cluster_at_point(lp, x, y),
            rinch_dom::text_query::glyph_cluster_at_point(lf, x, y),
            "({x}, {y})"
        );
    }
}

/// A probe, run by hand in release: the wall time of a software repaint of
/// 24 HUD numerals whose text changes every frame, at 1x and 2x.
#[test]
#[ignore]
fn hud_frame_time_probe() {
    let css = format!(
        "{CSS} .n {{ display: inline-block; margin: 6px 10px; font-family: InterT; \
         font-size: 41px; line-height: 48px; \
         background: linear-gradient(#fff6b0 8%, #ffb000 46%, #e04000 54%, #7a1000 93%); \
         background-clip: text; color: transparent; -webkit-text-stroke: 4.5px #1a1a2e; \
         paint-order: stroke fill; text-shadow: 2px 3px 0 #000; }}"
    );
    let html: String = (0..24)
        .map(|i| format!("<span class=n id=n{i}>1:23.4{i:02}</span>"))
        .collect();
    for scale in [1.0, 2.0] {
        let mut d = doc_with(&css, &html, 800.0, 600.0);
        let ids: Vec<_> = (0..24)
            .map(|i| d.query_selector(&format!("#n{i}")).unwrap())
            .collect();
        let (pw, ph) = ((800.0 * scale) as u32, (600.0 * scale) as u32);
        let mut painter = TinySkiaPainter::new(pw, ph);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        let frames = 30;
        let mut total = std::time::Duration::ZERO;
        for f in 0..frames + 5 {
            for (i, id) in ids.iter().enumerate() {
                d.set_text_content(
                    *id,
                    &format!("{}:{:02}.{:03}", f % 9, i, (f * 37 + i) % 1000),
                );
            }
            d.resolve_layout(800.0, 600.0);
            painter.fill_white();
            let t = std::time::Instant::now();
            rinch_dom::paint::paint_document(
                &d.tree,
                &mut painter,
                scale,
                (800.0, 600.0),
                &mut d.font_cx,
                &mut cx,
            );
            if f >= 5 {
                total += t.elapsed();
            }
        }
        eprintln!(
            "HUD repaint at {scale}x: {:.2} ms a frame",
            total.as_secs_f64() * 1000.0 / frames as f64
        );
    }
}

/// Write the page's render at `$RINCH_2719_DUMP/rinch.png` (a probe, run by
/// hand: `page.html` there is the body painted, `W`, `H`, `S` its size).
#[test]
#[ignore]
fn dump() {
    let Ok(dir) = std::env::var("RINCH_2719_DUMP") else {
        return;
    };
    let page = std::fs::read_to_string(format!("{dir}/page.html")).unwrap();
    let (w, h, s): (u32, u32, f64) = (
        std::env::var("W").map_or(460, |v| v.parse().unwrap()),
        std::env::var("H").map_or(170, |v| v.parse().unwrap()),
        std::env::var("S").map_or(1.0, |v| v.parse().unwrap()),
    );
    let img = render(&page, w, h, s);
    let rgb: Vec<u8> = img.px.concat();
    image::RgbImage::from_raw(img.w, img.h, rgb)
        .unwrap()
        .save(format!("{dir}/rinch.png"))
        .unwrap();
}
