//! #2314 — paint skips a text node's lines whose ink cannot be seen.
//!
//! The paint cull works per node, and a long paragraph or `<pre>` is one node,
//! so every one of its lines used to be drawn — a glyph-atlas lookup per glyph
//! on both painters — however much of it the window, a scroller's clip or a
//! partial repaint's damage showed. `paint::text::draw_text` now skips a line
//! whose ink (each glyph's face bounding box at the place it is drawn, its
//! straight decorations, faux bold, a relative inline element's offset)
//! misses all of that, through the paint transform.
//!
//! Each fixture paints a page twice, with the line cull and without it
//! (`set_text_line_cull`), and asserts the pixels are byte-identical and that
//! the cull skipped lines (`text_lines_culled`, the positive control). The
//! pages sit lines just outside what can be seen whose ink, or whose shadow
//! or decoration, reaches in: a mutant that tests the line box instead of the
//! ink, drops the decorations, or tests a shadow's lines where its text is,
//! fails one of them.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::Rect;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 400.0;
const VH: f32 = 300.0;

fn doc_with(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css(&format!(
        "body {{ margin: 0; font: 14px/18px ProbeFace; color: rgb(0, 0, 0); }} \
         p {{ margin: 0; width: 380px; }} {css}"
    ));
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    doc
}

/// The page painted at `scale`, and how many lines the paint culled.
fn paint(doc: &mut RinchDocument, scale: f64, damage: Option<Rect>, cull: bool) -> (Vec<u8>, u64) {
    let (w, h) = ((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut painter = TinySkiaPainter::new(w, h);
    painter.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::clear_text_shadow_cache();
    let was = rinch_dom::paint::set_text_line_cull(cull);
    rinch_dom::paint::set_dirty_region(damage);
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    rinch_dom::paint::set_dirty_region(None);
    rinch_dom::paint::set_text_line_cull(was);
    let culled = doc.tree.perf.end_frame().get(Counter::TextLinesCulled);
    let mut px = painter.pixels().to_vec();
    // A partial repaint shows only its damage: the shell clips to it.
    if let Some(d) = damage {
        let (x0, y0, x1, y1) = (
            (d.x0 * scale) as u32,
            (d.y0 * scale) as u32,
            (d.x1 * scale) as u32,
            (d.y1 * scale) as u32,
        );
        for y in 0..h {
            for x in 0..w {
                if !(x0..x1).contains(&x) || !(y0..y1).contains(&y) {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
    }
    (px, culled)
}

/// The same page with and without the line cull: identical pixels, and the
/// cull skipped at least `min_culled` lines. Returns the pixels.
fn same_pixels(
    name: &str,
    doc: &mut RinchDocument,
    scale: f64,
    damage: Option<Rect>,
    min_culled: u64,
) -> Vec<u8> {
    let (all, none) = paint(doc, scale, damage, false);
    assert_eq!(none, 0, "{name}: nothing culled with the cull off");
    let (culled, n) = paint(doc, scale, damage, true);
    assert!(
        n >= min_culled,
        "{name}: positive control: {n} lines culled, expected at least {min_culled}"
    );
    let differ = all
        .as_chunks::<4>()
        .0
        .iter()
        .zip(culled.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differ, 0,
        "{name}: {differ} pixels differ with the line cull"
    );
    // And the page drew text at all (not a blank page compared with itself).
    assert!(
        all.as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] < 128 && p[3] == 255),
        "{name}: positive control: no dark pixel painted"
    );
    all
}

/// [`same_pixels`] at 1x on a damage band, which must show some dark ink.
fn same_pixels_in(name: &str, doc: &mut RinchDocument, damage: Option<Rect>, min_culled: u64) {
    let px = same_pixels(name, doc, 1.0, damage, min_culled);
    let d = damage.unwrap();
    let dark = px
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            let (x, y) = ((i % VW as usize) as f64, (i / VW as usize) as f64);
            d.contains(peniko::kurbo::Point::new(x + 0.5, y + 0.5)) && p[0] < 200 && p[3] == 255
        })
        .count();
    assert!(
        dark > 0,
        "{name}: positive control: the damage band holds no ink"
    );
}

fn words(n: usize) -> String {
    (0..n).map(|i| format!("word{i} ")).collect()
}

fn scroller(doc: &mut RinchDocument, top: f64) {
    let s = doc.query_selector(".s").unwrap();
    doc.set_scroll_top(s, top);
    doc.resolve_layout(VW, VH);
}

/// A long paragraph in a scroller, scrolled to a place that cuts a line in
/// two at each edge, at 1x and 2x; and one running past the bottom of the
/// window.
///
/// Kills: the cull tested without the scroll offset or the scale.
#[test]
fn a_scrolled_paragraph_draws_what_it_drew() {
    let css = ".s { margin: 50px 0; height: 100px; overflow-y: auto }";
    let html = format!("<div class=s><p>{}</p></div>", words(600));
    let mut doc = doc_with(css, &html);
    scroller(&mut doc, 709.0);
    same_pixels("scrolled, 1x", &mut doc, 1.0, None, 40);
    same_pixels("scrolled, 2x", &mut doc, 2.0, None, 40);

    let mut doc = doc_with("", &format!("<p>{}</p>", words(900)));
    same_pixels("past the window", &mut doc, 1.0, None, 20);
}

/// Ink outside the line box: a 40px font on 4px lines (each line's glyphs
/// reach far past its box, into the lines around it), and marks stacked over
/// a letter, in a scroller whose clip falls between lines.
///
/// Kills: the line box (`block_min_coord..block_max_coord`) tested instead of
/// the ink, and the face's bounding box left out (the run's ascent alone).
#[test]
fn glyphs_whose_ink_leaves_their_line_are_drawn() {
    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .s p { font-size: 40px; line-height: 4px }";
    let html = format!("<div class=s><p>{}</p></div>", words(400));
    let mut doc = doc_with(css, &html);
    scroller(&mut doc, 200.0);
    same_pixels("40px on 4px lines", &mut doc, 1.0, None, 10);

    let stacked: String = (0..300)
        .map(|i| format!("e\u{301}\u{301}\u{301}\u{308}{i} "))
        .collect();
    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .s p { font-size: 20px; line-height: 12px }";
    let mut doc = doc_with(css, &format!("<div class=s><p>{stacked}</p></div>"));
    scroller(&mut doc, 300.0);
    same_pixels("stacked marks", &mut doc, 1.0, None, 10);
}

/// The same, exactly: a partial repaint's damage is not grown by an ink
/// margin, as the window and clip culls are. A 60px face on 10px lines, line
/// 10 holding `Ǻ`, Inter's tallest glyph (1.108 em, where its ascent is
/// 0.969 em), every line around it empty but the first and some far below,
/// and a damage band above line 10's box that only the ring of its `Ǻ`
/// reaches. Line 10's box misses the band, so its ink is measured.
///
/// Kills: the run's ascent taken for the ink's top (the face's bounding box
/// left out).
#[test]
fn a_glyph_taller_than_its_ascent_is_drawn_in_the_damage() {
    let css = "p { font-size: 60px; line-height: 10px }";
    let html = format!(
        "<p>a{}ǺǺǺ{}{}</p>",
        "<br>".repeat(10),
        "<br>".repeat(25),
        words(30)
    );
    let mut doc = doc_with(css, &html);
    // Line 10's box is 100..110 and its baseline ~127 (the 72.6px content
    // area centred on it), so its ascent ends at ~69 (~67 with the 2px
    // margin) and `Ǻ`'s ring at ~60: the band holds that ring and no other
    // ink (line 0's `a` ends by ~27).
    let damage = Some(Rect::new(0.0, 61.5, 400.0, 65.5));
    same_pixels_in("tall glyph, damage", &mut doc, damage, 4);
}

/// An underline moved below its line box (`text-underline-offset`, set on
/// the computed style: no CSS reaches the field yet, #580), the lines after
/// it empty, and a damage band that holds only that underline. The line's box
/// misses the band, so its ink is measured.
///
/// Kills: the decorations left out of a line's ink.
#[test]
fn an_underline_in_the_damage_is_drawn() {
    let css = "p { line-height: 20px; text-decoration: underline }";
    let html = format!("<p>{}{}{}</p>", words(4), "<br>".repeat(25), words(60));
    let mut doc = doc_with(css, &html);
    let p = doc.query_selector("p").unwrap();
    doc.tree.nodes[p.0].computed_style.text_underline_offset = Some(40.0);
    doc.tree.nodes[p.0].text_layout = None;
    doc.tree.layout_dirty = true;
    doc.resolve_layout(VW, VH);
    // Line 0's box is 0..20 and its baseline ~15; its glyphs' ink ends by
    // ~22, and the underline, 40px under the baseline, is at ~55: the band
    // from 45 to 65 holds it alone (lines 1 to 25 are empty).
    let damage = Some(Rect::new(0.0, 45.0, 400.0, 65.0));
    same_pixels_in("underline, damage", &mut doc, damage, 4);
}

/// Underlined, struck-through and overlined text in a scroller cut between
/// lines.
#[test]
fn decorated_lines_draw_what_they_drew() {
    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .s p { text-decoration: underline }";
    let html = format!("<div class=s><p>{}</p></div>", words(500));
    let mut doc = doc_with(css, &html);
    scroller(&mut doc, 401.0);
    same_pixels("underline", &mut doc, 1.0, None, 20);

    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .s p { text-decoration: line-through overline; }";
    let mut doc = doc_with(css, &html);
    scroller(&mut doc, 401.0);
    same_pixels("line-through and overline", &mut doc, 1.0, None, 20);
}

/// `text-shadow`s that land far from their text: a hard one 120px up (the
/// shadow of a line below the clip is inside it), blurred ones through the
/// mask (in the mask's own pixels) and through the kernel of copies (the
/// Vello-only build's path), at 1x and 2x.
///
/// Kills: a shadow pass's lines tested where its text is, and the mask's
/// copy culled against the surface instead of the mask.
#[test]
fn shadows_of_lines_out_of_sight_are_drawn() {
    let html = format!("<div class=s><p>{}</p></div>", words(500));
    for (name, shadow) in [
        ("hard, 120px up", "0 -120px 0 rgb(200, 0, 0)"),
        ("blurred, 50px down", "0 50px 4px rgb(0, 0, 200)"),
        (
            "two, either way",
            "0 -70px 2px rgb(0, 150, 0), 30px 70px 0 rgb(0, 0, 200)",
        ),
    ] {
        let css = format!(
            ".s {{ margin: 80px 0; height: 80px; overflow-y: auto }} \
             .s p {{ text-shadow: {shadow} }}"
        );
        let mut doc = doc_with(&css, &html);
        scroller(&mut doc, 500.0);
        same_pixels(name, &mut doc, 1.0, None, 20);
        same_pixels(&format!("{name}, 2x"), &mut doc, 2.0, None, 20);
        let was = rinch_dom::paint::force_tapped_text_shadows(true);
        same_pixels(&format!("{name}, tapped"), &mut doc, 1.0, None, 20);
        rinch_dom::paint::force_tapped_text_shadows(was);
    }
}

/// A transformed paragraph: rotated, scaled and skewed, partly off the
/// window, with a shadow.
///
/// Kills: the line's ink taken without the paint transform.
#[test]
fn a_transformed_paragraph_draws_what_it_drew() {
    let html = format!("<p class=t>{}</p>", words(900));
    for transform in [
        "rotate(30deg)",
        "rotate(-90deg) translate(-200px, 0)",
        "scale(2.5)",
        "translateY(-300px) scale(0.5, 1.5)",
        "skewY(20deg)",
    ] {
        let css = format!(
            ".t {{ transform: {transform}; transform-origin: 0 0; \
                   text-shadow: 0 30px 3px rgb(0, 0, 200) }}"
        );
        let mut doc = doc_with(&css, &html);
        same_pixels(transform, &mut doc, 1.0, None, 5);
    }
}

/// A read-only selection over many lines, and faux bold and oblique text:
/// each drawn as before.
///
/// Kills: a selection pass (its own mask) culled by the wrong lines, and
/// faux bold's stroke left out of the ink.
#[test]
fn selection_and_synthesised_faces_draw_what_they_drew() {
    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto }";
    let text = words(500);
    let mut doc = doc_with(css, &format!("<div class=s><p>{text}</p></div>"));
    scroller(&mut doc, 300.0);
    let p = doc.query_selector("p").unwrap();
    doc.set_read_only_selection(p.0, &[(40, text.len() - 40)]);
    doc.resolve_layout(VW, VH);
    same_pixels("selection", &mut doc, 1.0, None, 20);

    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .s p { font-weight: 900; font-style: italic; font-size: 30px; line-height: 14px }";
    let mut doc = doc_with(css, &format!("<div class=s><p>{text}</p></div>"));
    scroller(&mut doc, 300.0);
    same_pixels("faux bold italic", &mut doc, 1.0, None, 10);
}

/// A relatively positioned span moved far off its line.
///
/// Kills: the relative offset left out of a line's ink.
#[test]
fn a_relative_span_moved_off_its_line_is_drawn() {
    let css = ".s { margin: 60px 0; height: 60px; overflow-y: auto } \
               .r { position: relative; top: -90px; color: rgb(200, 0, 0) }";
    let mut html = String::from("<div class=s><p>");
    for i in 0..60 {
        html.push_str(&format!("{} <span class=r>moved{i}</span> ", words(6)));
    }
    html.push_str("</p></div>");
    let mut doc = doc_with(css, &html);
    scroller(&mut doc, 400.0);
    same_pixels("relative span", &mut doc, 1.0, None, 10);
}

/// A partial repaint: the damage is a band across the middle of a long
/// paragraph, with and without a blurred shadow; the lines outside the band
/// are culled and the band is drawn as before.
///
/// Kills: the damage left out of the cull rect (nothing culled: the positive
/// control), and the damage used without the shadow's offset.
#[test]
fn a_partial_repaint_draws_its_damage_as_before() {
    let damage = Some(Rect::new(0.0, 101.0, 400.0, 141.0));
    let mut doc = doc_with("", &format!("<p>{}</p>", words(500)));
    same_pixels("damage band", &mut doc, 1.0, damage, 10);
    let css = "p { text-shadow: 0 -60px 3px rgb(0, 0, 200), 0 60px 0 rgb(200, 0, 0) }";
    let mut doc = doc_with(css, &format!("<p>{}</p>", words(500)));
    same_pixels("damage band, shadows", &mut doc, 1.0, damage, 10);
}

/// The cull is on by default, and a paint outside `paint_document` (no window
/// known) culls nothing.
#[test]
fn the_cull_is_on_by_default() {
    let mut doc = doc_with("", &format!("<p>{}</p>", words(900)));
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    doc.tree.perf.reset();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    assert!(doc.tree.perf.end_frame().get(Counter::TextLinesCulled) > 10);
    let _ = NodeId(0);
}
