//! A pixel-diff oracle for the software painter's caches and pools.
//!
//! `TinySkiaPainter` keeps three things across draws and frames that it did
//! not keep before: a cache of rasterised glyphs, a pool of surface-sized clip
//! masks and layer pixmaps whose used area it tracks, and (on the image cache
//! entry) a premultiplied copy of each image. Each is only correct if it draws
//! **exactly** the pixels the painter drew without it, and a cache that is
//! subtly wrong — a key missing a field, a pooled buffer not fully cleaned, a
//! layer composited over too small a rect — draws *plausible* text and boxes
//! that no visual inspection would flag.
//!
//! So the oracle is the painter itself with every one of them switched off
//! ([`TinySkiaPainter::set_reference_mode`]), in the same process, over the
//! same document: every glyph rasterised on every draw, every clip and layer
//! a fresh surface-sized buffer composited whole, every image premultiplied
//! per draw — the code as it was. Each fixture demands the two agree **byte
//! for byte**, with no tolerance, cold (the first paint) and warm (after the
//! caches and pools hold everything, and after other documents have been
//! through the same painter and left their buffers in its pools).
//!
//! # What it catches (measured, one mutant at a time — see the PR)
//!
//! | mutant | caught by |
//! |---|---|
//! | glyph key drops the font size | `text_sizes_and_colours`, `partial_repaints`, `one_painter_many_documents` |
//! | glyph key drops the font id | `scripts_and_fallback_faces`, `one_painter_many_documents` |
//! | glyph key drops the face index | `faces_of_one_collection` **only** — a `.ttc` built in the test, since a host's CJK collections share their glyph images across faces and CI's fonts are not this machine's |
//! | glyph key drops the variation coords | `text_sizes_and_colours` (1 872 px: the vendored Space Grotesk at five weights), `partial_repaints`, `one_painter_many_documents` |
//! | a pooled mask is not zeroed on release | `clips_and_scrollers`, `translucent_layers`, `partial_repaints`, `one_painter_many_documents` |
//! | a clip's recorded bounds are empty (so release zeroes nothing) | `partial_repaints` |
//! | a pooled layer is not cleared on release | `translucent_layers`, `images`, `partial_repaints`, `one_painter_many_documents` |
//! | a fill is not counted in a layer's drawn area | `translucent_layers`, `clip_and_layer_counts`, `partial_repaints`, `one_painter_many_documents` |
//! | a stroke is not counted in a layer's drawn area | `translucent_layers` (an underline running past its glyphs under no-break spaces), `partial_repaints`, `one_painter_many_documents` |
//! | a glyph is not counted in a layer's drawn area | `translucent_layers` (a layer holding only text), `partial_repaints`, `one_painter_many_documents` |
//! | an image is not counted in a layer's drawn area | `images`, `one_painter_many_documents` |
//! | a nested layer's drawn area is not added to its parent layer's | `translucent_layers` (1 155 px: an outer layer that draws nothing itself, around an inner one holding text), `partial_repaints`, `one_painter_many_documents` |
//! | the intersect walk is narrowed to the parent's bounds, so a child clip wider than its parent keeps coverage outside it | `clips_and_scrollers` (439 px, first paint), `partial_repaints`, `one_painter_many_documents` |
//! | the end-of-frame pool trim releases nothing | `the_pool_is_trimmed_to_recent_use` |
//! | the bookkeeping pad is negative | six of the oracle tests (measured when there were eleven) |
//! | the cached premultiply rounds with `+128` | `images`, `one_painter_many_documents` — which needed reference mode to keep its *own* copy of the old premultiply; sharing the function let this mutant through |
//!
//! Two mutants survive. A pad of **0** instead of 2: in every fixture the
//! floor/ceil of a draw's geometric bounds already covered every pixel it
//! wrote, so the pad is a safety margin rather than something a fixture here
//! proves necessary (it is cheap, and a narrower answer is the one way to get
//! the bookkeeping wrong). And a clip whose recorded bounds are its
//! **parent's** rather than the intersection is equivalent: it only
//! over-states where the mask can be non-zero, which costs zeroing and never
//! a pixel.
//!
//! There is no sub-pixel offset in the glyph key and deliberately none to
//! drop: this painter rasterises every glyph at the origin and places the
//! finished image by nearest-neighbour sampling, so glyphs at `x = 10.25` and
//! `x = 10.75` are the same bytes. `text_sizes_and_colours` puts glyphs at
//! many fractional positions and is what fails if that ever stops being true
//! (a change that renders at the fractional offset without keying on it).

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::{Affine, Rect};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 600.0;
const VH: f32 = 400.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A variable face (a `wght` axis, 300-700), vendored under the OFL (see
/// `SpaceGrotesk-OFL.txt` beside it), so the variation-coords field of the
/// glyph key is pinned on every host rather than only on one that happens to
/// have a variable font installed.
const VARIABLE_FACE: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");

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
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(VARIABLE_FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeVar"),
            ..Default::default()
        }),
    );
    doc.load_css(
        "body { margin: 0; font-family: ProbeFace, sans-serif; font-size: 14px; \
         line-height: 18px; color: rgb(30, 30, 40); }",
    );
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    doc
}

fn full_paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
    painter.reset();
    painter.fill_white();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

/// What `RinchApp::build_pixels` does for a partial frame.
fn partial_paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, region: Rect) {
    painter.clear_rect_white(
        region.x0 as u32,
        region.y0 as u32,
        region.width() as u32,
        region.height() as u32,
    );
    rinch_dom::paint::set_dirty_region(Some(region));
    painter.push_clip(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        &PaintShape::Rect(region),
    );
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pop_layer();
    rinch_dom::paint::set_dirty_region(None);
}

fn reference_painter() -> TinySkiaPainter {
    let mut p = TinySkiaPainter::new(VW as u32, VH as u32);
    p.set_reference_mode(true);
    p
}

/// Assert two pixel buffers are identical, naming where they are not.
#[track_caller]
fn assert_same(label: &str, reference: &[u8], got: &[u8]) {
    assert_eq!(reference.len(), got.len());
    let mut differing = 0usize;
    let mut first = None;
    let mut max_delta = 0u8;
    for (i, (a, b)) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(got.as_chunks::<4>().0)
        .enumerate()
    {
        if a != b {
            differing += 1;
            first.get_or_insert((i % VW as usize, i / VW as usize, *a, *b));
            for c in 0..4 {
                max_delta = max_delta.max(a[c].abs_diff(b[c]));
            }
        }
    }
    assert_eq!(
        differing, 0,
        "{label}: {differing} pixels differ from the reference painter (max channel \
         delta {max_delta}); first at {first:?} (x, y, reference, cached)"
    );
}

/// The positive control every fixture needs: it painted *something*, so an
/// equality between two blank buffers is not mistaken for agreement.
#[track_caller]
fn assert_inked(label: &str, px: &[u8]) {
    let inked = px
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| **p != [255, 255, 255, 255])
        .count();
    assert!(
        inked > 500,
        "{label}: only {inked} non-white pixels painted"
    );
}

/// Paint `doc` with a fresh reference painter and with `painter` twice
/// (cold, then warm), and demand all three agree.
fn check(label: &str, doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
    let mut reference = reference_painter();
    full_paint(doc, &mut reference);
    assert_inked(label, reference.pixels());
    full_paint(doc, painter);
    assert_same(
        &format!("{label} (first paint)"),
        reference.pixels(),
        painter.pixels(),
    );
    full_paint(doc, painter);
    assert_same(
        &format!("{label} (warm)"),
        reference.pixels(),
        painter.pixels(),
    );
}

const LOREM: &str = "Sphinx of black quartz, judge my vow — 0123456789 &@%!";

fn text_sizes_doc() -> RinchDocument {
    let mut html = String::new();
    // Several sizes of one face: a glyph key without the size draws one
    // size's coverage at every other size.
    for (i, size) in [9.0, 11.0, 13.5, 14.0, 14.25, 20.0, 31.7]
        .iter()
        .enumerate()
    {
        // Fractional x positions everywhere: a fractional padding and a
        // fractional letter-spacing put every glyph at a different sub-pixel
        // offset.
        html.push_str(&format!(
            "<p style=\"margin: 0; font-size: {size}px; line-height: {lh}px; \
             padding-left: {pad}px; letter-spacing: 0.{i}7px; \
             color: rgba({r}, 60, 120, {a})\">{LOREM}</p>",
            lh = size * 1.3,
            pad = 0.3 * i as f32,
            r = 20 * i,
            a = if i % 2 == 0 { "1" } else { "0.55" },
        ));
    }
    // Synthesis: the face has no bold or italic, so both are synthesised.
    html.push_str("<p style=\"margin: 0; font-weight: bold\">Bold synthesised text</p>");
    html.push_str("<p style=\"margin: 0; font-style: italic\">Italic synthesised text</p>");
    // The vendored variable face at five weights on one line: the same glyph
    // ids, one font blob, one size, five sets of variation coordinates. A
    // key without the coords draws the first weight's glyphs at all five.
    html.push_str(
        "<p style=\"margin: 0; font-family: ProbeVar; font-size: 18px; \
          line-height: 24px\">\
         <span style=\"font-weight: 300\">Weight </span>\
         <span style=\"font-weight: 400\">Weight </span>\
         <span style=\"font-weight: 500\">Weight </span>\
         <span style=\"font-weight: 600\">Weight </span>\
         <span style=\"font-weight: 700\">Weight</span></p>",
    );
    doc_with("", &html)
}

#[test]
fn text_sizes_and_colours() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = text_sizes_doc();
    check("text sizes", &mut doc, &mut painter);
    assert!(
        painter.stats().glyph_cache_hits > 0,
        "the warm paint drew nothing from the cache"
    );
}

fn scripts_doc() -> RinchDocument {
    doc_with(
        "p { margin: 0 0 4px 0; font-size: 16px; line-height: 22px; }",
        "<p style=\"font-family: sans-serif\">漢字かな交じり文 한국어 中文字体</p>\
         <p style=\"font-family: serif\">漢字かな交じり文 中文字体 serif</p>\
         <p>ภาษาไทยเป็นภาษาที่มีระดับเสียงของคำแน่นอน</p>\
         <p>مرحبا بالعالم — עברית</p>\
         <p>Combining: e\u{301} n\u{303} u\u{308}\u{30c}  Ligatures: ffi fl</p>\
         <p style=\"font-family: monospace\">monospace() { return 0; }</p>\
         <p style=\"font-family: 'Segoe UI Emoji', sans-serif; font-size: 24px; \
          line-height: 30px\">Emoji 😀🎉✅ ✓</p>",
    )
}

#[test]
fn scripts_and_fallback_faces() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = scripts_doc();
    check("scripts", &mut doc, &mut painter);
}

fn decorations_doc() -> RinchDocument {
    doc_with(
        "p { margin: 0 0 6px 0; font-size: 18px; line-height: 24px; }",
        "<p style=\"text-decoration: underline\">Underlined text here</p>\
         <p style=\"text-decoration: line-through; text-decoration-color: red\">Struck</p>\
         <p style=\"text-decoration: underline wavy rgb(200, 0, 0)\">Wavy spell error</p>\
         <p style=\"text-shadow: 1px 2px 2px rgba(0, 0, 0, 0.5)\">Shadowed text</p>\
         <p style=\"text-shadow: 0 0 3px rgb(0, 120, 255); color: white\">Glow</p>",
    )
}

#[test]
fn decorations_and_shadows() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = decorations_doc();
    check("decorations", &mut doc, &mut painter);
}

fn clips_doc() -> RinchDocument {
    let mut html = String::from("<div style=\"display: flex; flex-wrap: wrap; gap: 5px\">");
    for i in 0..12 {
        html.push_str(&format!(
            "<div class=\"sc\" style=\"width: 90px; height: 70px; overflow: auto; \
             border-radius: {r}px; background: rgb(230, 235, {b})\">",
            r = i * 3,
            b = 200 + i * 4,
        ));
        for j in 0..8 {
            html.push_str(&format!("<div>row {i}.{j} text</div>"));
        }
        html.push_str("</div>");
    }
    // Nested rounded clips, a rotated child inside a clip, an absolute child
    // escaping a static clip, and a zero-height clip (the degenerate branch).
    html.push_str(
        "<div style=\"width: 200px; height: 90px; overflow: hidden; border-radius: 20px; \
          background: rgb(250, 220, 200)\">\
           <div style=\"margin: 10px; width: 150px; height: 60px; overflow: hidden; \
            border-radius: 12px; background: rgb(200, 220, 250)\">\
             <div style=\"transform: rotate(12deg); width: 180px\">Rotated in two clips</div>\
           </div>\
         </div>\
         <div style=\"width: 120px; height: 40px; overflow: hidden\">\
           <span style=\"position: absolute; left: 400px; top: 300px\">escapes</span>\
           clipped content that is long enough to be cut off</div>\
         <div style=\"height: 0; overflow: hidden\"><div>hidden rows</div></div>\
         <div style=\"width: 100px; height: 40px; overflow: hidden; border-radius: 6px; \
          background: rgb(238, 238, 238)\">\
           <div style=\"margin-left: 50px; width: 300px; height: 30px; overflow: hidden; \
            border-radius: 6px; background: rgb(40, 120, 200)\">a child clip wider than its parent</div>\
         </div>",
    );
    html.push_str("</div>");
    let mut doc = doc_with("", &html);
    // Scroll every scroller by a different amount, so the clip is not at the
    // content's origin.
    for (i, id) in doc.query_selector_all(".sc").into_iter().enumerate() {
        doc.tree.nodes[id.0].scroll_offset = (0.0, 7.0 * i as f64);
    }
    doc.tree.hit_cache.invalidate();
    doc
}

#[test]
fn clips_and_scrollers() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = clips_doc();
    check("clips", &mut doc, &mut painter);
}

fn layers_doc() -> RinchDocument {
    doc_with(
        ".l { width: 150px; height: 70px; margin: 4px; display: inline-block; \
              background: rgb(200, 80, 40); }",
        "<div style=\"opacity: 0.6\"><div style=\"opacity: 0.5\">\
            <span style=\"font-size: 20px; color: red\">inner-only layer text</span></div></div>\
         <div style=\"opacity: 0.5; font-size: 22px; line-height: 28px\">Only glyphs in this layer</div>\
         <div style=\"opacity: 0.5; font-size: 22px; line-height: 28px; \
         text-decoration: underline\">\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}ab\
         \u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}</div>\
        <div class=\"l\" style=\"opacity: 0.5\">Half opaque with text</div>\
         <div class=\"l\" style=\"opacity: 0.8\">\
            <div style=\"opacity: 0.5; background: blue; width: 60px; height: 30px\">nest</div>\
         </div>\
         <div class=\"l\" style=\"opacity: 0.6; border: 3px solid black; \
            text-decoration: underline\">Bordered, underlined</div>\
         <div style=\"width: 160px; height: 60px; overflow: hidden; border-radius: 16px\">\
            <div style=\"opacity: 0.7; background: green; width: 200px; height: 80px\">\
              layer in a clip</div>\
         </div>\
         <div class=\"l\" style=\"opacity: 0.6\">\
            <div style=\"transform: rotate(-20deg); background: purple; width: 80px\">spun</div>\
         </div>\
         <div class=\"l\" style=\"filter: grayscale(0.5)\">Greyscaled</div>\
         <div class=\"l\" style=\"opacity: 0.4; box-shadow: 4px 4px 8px black\">Shadow</div>\
         <div class=\"l\" style=\"opacity: 0.5; position: relative\">\
            <div style=\"position: absolute; left: 170px; top: 50px; background: teal; \
             width: 40px; height: 40px\">out</div>\
         </div>",
    )
}

#[test]
fn translucent_layers() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = layers_doc();
    check("layers", &mut doc, &mut painter);
    // Nothing a layer drew is on the surface outside it, so the bounded
    // composite must be well short of one surface per layer.
    let s = painter.take_stats();
    assert!(s.layers >= 7, "{s:?}");
    assert!(
        s.layer_px < (VW * VH) as u64,
        "the layers composited {} px — a whole surface's worth or more: {s:?}",
        s.layer_px
    );
}

fn png_data_uri(seed: u8, w: u32, h: u32) -> String {
    use base64::Engine;
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([
            (x as u8).wrapping_mul(7).wrapping_add(seed),
            (y as u8).wrapping_mul(5),
            seed.wrapping_mul(3),
            // Every alpha, so the premultiply is exercised off its fixed
            // points (0 and 255).
            (x.wrapping_mul(13) ^ y.wrapping_mul(7)) as u8,
        ]);
    }
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .unwrap();
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn images_doc() -> RinchDocument {
    let a = png_data_uri(17, 64, 48);
    let b = png_data_uri(90, 33, 71);
    doc_with(
        "img { margin: 3px; }",
        &format!(
            "<img src=\"{a}\" style=\"width: 64px; height: 48px\">\
             <img src=\"{a}\" style=\"width: 128px; height: 96px\">\
             <img src=\"{b}\" style=\"width: 90px; height: 90px; object-fit: cover\">\
             <img src=\"{b}\" style=\"width: 90px; height: 90px; object-fit: contain\">\
             <img src=\"{a}\" style=\"width: 50px; height: 30px; object-fit: none\">\
             <div style=\"opacity: 0.5; display: inline-block\"><img src=\"{b}\"></div>\
             <img src=\"{a}\" style=\"transform: rotate(30deg)\">\
             <div style=\"width: 150px; height: 100px; background-image: url({b})\"></div>"
        ),
    )
}

#[test]
fn images() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut doc = images_doc();
    check("images", &mut doc, &mut painter);
    // Two paints, and every image came from the cache: none premultiplied
    // at draw time. The reference painter premultiplies each draw.
    assert_eq!(painter.stats().image_premultiplies, 0);
    let mut reference = reference_painter();
    full_paint(&mut doc, &mut reference);
    assert!(reference.stats().image_premultiplies >= 8);
}

#[test]
fn partial_repaints() {
    let regions = [
        Rect::new(10.0, 10.0, 210.0, 60.0),
        Rect::new(95.0, 0.0, 305.0, 400.0),
        Rect::new(0.0, 150.0, 600.0, 190.0),
        Rect::new(333.0, 222.0, 377.0, 290.0),
    ];
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    for (name, mut doc) in [
        ("text", text_sizes_doc()),
        ("clips", clips_doc()),
        ("layers", layers_doc()),
    ] {
        let mut reference = reference_painter();
        full_paint(&mut doc, &mut reference);
        full_paint(&mut doc, &mut painter);
        for r in regions {
            partial_paint(&mut doc, &mut reference, r);
            partial_paint(&mut doc, &mut painter, r);
            assert_same(
                &format!("{name}: partial {r:?}"),
                reference.pixels(),
                painter.pixels(),
            );
        }
    }
}

/// One painter through every document in turn, twice, so each fixture meets
/// a pool that another one's clips and layers have used and a glyph cache
/// full of other documents' runs.
#[test]
fn one_painter_many_documents() {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut docs = [
        ("layers", layers_doc()),
        ("text", text_sizes_doc()),
        ("clips", clips_doc()),
        ("images", images_doc()),
        ("scripts", scripts_doc()),
        ("decorations", decorations_doc()),
    ];
    for round in 0..2 {
        painter.take_stats();
        for (name, doc) in docs.iter_mut() {
            let mut reference = reference_painter();
            full_paint(doc, &mut reference);
            full_paint(doc, &mut painter);
            assert_same(
                &format!("{name}, round {round}"),
                reference.pixels(),
                painter.pixels(),
            );
        }
    }
    // The second round found every buffer it needed in the pool.
    let s = painter.stats();
    assert_eq!(s.surface_allocs, 0, "{s:?}");
    assert_eq!(s.glyph_cache_misses, 0, "{s:?}");
}

// ── Counters ───────────────────────────────────────────────────────────────

/// The glyph cache's counts on a text whose glyph set is known: "aaaa bbbb"
/// in the pinned face is nine glyphs of three ids (`a`, space, `b` — Inter
/// has no contextual alternate for either letter). The first paint
/// rasterises each id once and draws the rest from the cache; the second
/// rasterises nothing.
#[test]
fn glyph_cache_counts() {
    let mut doc = doc_with("", "<p style=\"margin: 0\">aaaa bbbb</p>");
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    full_paint(&mut doc, &mut painter);
    let first = painter.take_stats();
    assert_eq!(
        (first.glyph_cache_misses, first.glyph_cache_hits),
        (3, 6),
        "{first:?}"
    );
    full_paint(&mut doc, &mut painter);
    let second = painter.take_stats();
    assert_eq!(
        (second.glyph_cache_misses, second.glyph_cache_hits),
        (0, 9),
        "{second:?}"
    );
    assert!(painter.glyph_cache_bytes() > 0);
    painter.clear_glyph_cache();
    full_paint(&mut doc, &mut painter);
    assert_eq!(painter.take_stats().glyph_cache_misses, 3);
}

/// The pool keeps what recent frames needed at once, and gives the rest
/// back: a frame nesting four translucent layers leaves four pixmaps pooled,
/// and after a window of frames that need one, three of them are released.
#[test]
fn the_pool_is_trimmed_to_recent_use() {
    let mut deep = doc_with(
        "",
        "<div style=\"opacity: 0.9\"><div style=\"opacity: 0.8\">\
           <div style=\"opacity: 0.7\"><div style=\"opacity: 0.6\">deep</div></div>\
         </div></div>",
    );
    let mut shallow = doc_with("", "<div style=\"opacity: 0.5\">shallow</div>");
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    full_paint(&mut deep, &mut painter);
    painter.end_frame();
    assert_eq!(painter.pooled_buffers().1, 4, "four layers open at once");
    painter.take_stats();
    // Seven more frames still remember the deep one.
    for _ in 0..7 {
        full_paint(&mut shallow, &mut painter);
        painter.end_frame();
        assert_eq!(painter.pooled_buffers().1, 4);
    }
    assert_eq!(painter.take_stats().surface_trims, 0);
    // The eighth shallow frame pushes it out of the window.
    full_paint(&mut shallow, &mut painter);
    painter.end_frame();
    assert_eq!(painter.pooled_buffers().1, 1);
    let s = painter.take_stats();
    assert_eq!(s.surface_trims, 3, "{s:?}");
    assert_eq!(
        s.surface_allocs, 0,
        "the kept pixmap serves the shallow frame"
    );
    // Pixels are unaffected.
    let mut reference = reference_painter();
    full_paint(&mut shallow, &mut reference);
    assert_same("after a trim", reference.pixels(), painter.pixels());
}

/// A clip costs its own area, not the surface's; a layer composites what
/// was drawn into it; and a second frame allocates no surface-sized buffer.
#[test]
fn clip_and_layer_counts() {
    let mut doc = doc_with(
        "",
        "<div style=\"margin: 20px; width: 100px; height: 50px; overflow: hidden; \
          background: red\"><div style=\"width: 300px; height: 300px\"></div></div>\
         <div style=\"margin: 20px; width: 100px; height: 50px; opacity: 0.5; \
          background: blue\"></div>",
    );
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    full_paint(&mut doc, &mut painter);
    let first = painter.take_stats();
    assert_eq!(first.clip_masks, 1, "{first:?}");
    assert_eq!(first.layers, 1, "{first:?}");
    // 100x50, padded by two device pixels a side for anti-aliasing: at most
    // 105x55. The whole surface is 240 000.
    assert!(
        (5_000..=105 * 55).contains(&first.clip_mask_px),
        "{first:?}"
    );
    assert!((5_000..=105 * 55).contains(&first.layer_px), "{first:?}");
    assert_eq!(
        first.surface_allocs, 2,
        "one mask and one layer, first frame"
    );

    full_paint(&mut doc, &mut painter);
    let second = painter.take_stats();
    assert_eq!(second.surface_allocs, 0, "the pool serves the second frame");
    assert_eq!(second.clip_mask_px, first.clip_mask_px);
    assert_eq!(second.layer_px, first.layer_px);

    // The reference painter does what the painter did before: every push a
    // fresh surface, every clip and layer the whole surface.
    let mut reference = reference_painter();
    full_paint(&mut doc, &mut reference);
    full_paint(&mut doc, &mut reference);
    let r = reference.take_stats();
    assert_eq!(r.surface_allocs, 4, "{r:?}");
    assert_eq!(r.layer_px, 2 * (VW * VH) as u64, "{r:?}");
}

// ── A font collection, built here ─────────────────────────────────────────

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The byte offset of table `tag` in a single-font file.
fn table_offset(ttf: &[u8], tag: &[u8; 4]) -> usize {
    let n = be16(ttf, 4) as usize;
    (0..n)
        .map(|i| 12 + 16 * i)
        .find(|&rec| &ttf[rec..rec + 4] == tag)
        .map(|rec| be32(ttf, rec + 8) as usize)
        .unwrap_or_else(|| panic!("no {tag:?} table"))
}

/// A two-face `.ttc` made of the pinned face and a copy of it that claims
/// weight 700 and half the units per em — so it draws every glyph, under the
/// same glyph id, at twice the size.
///
/// That is the one input a glyph key can lose without any other field
/// noticing: both faces come out of one font blob (so one `Blob::id`), at one
/// CSS size, with the same glyph ids; only the face index tells them apart.
/// A host's own `.ttc` cannot be relied on for it — CI's fonts are not this
/// machine's, and a CJK collection's faces share their glyph images anyway.
fn two_face_collection() -> Vec<u8> {
    let mut bold = FACE.to_vec();
    let head = table_offset(&bold, b"head");
    let upem = be16(&bold, head + 18);
    bold[head + 18..head + 20].copy_from_slice(&(upem / 2).to_be_bytes());
    let os2 = table_offset(&bold, b"OS/2");
    bold[os2 + 4..os2 + 6].copy_from_slice(&700u16.to_be_bytes());

    let header: usize = 12 + 4 * 2;
    let base_a = header.next_multiple_of(4);
    let base_b = (base_a + FACE.len()).next_multiple_of(4);
    let mut out = vec![0u8; base_b + bold.len()];
    out[0..4].copy_from_slice(b"ttcf");
    out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    out[8..12].copy_from_slice(&2u32.to_be_bytes());
    out[12..16].copy_from_slice(&(base_a as u32).to_be_bytes());
    out[16..20].copy_from_slice(&(base_b as u32).to_be_bytes());
    for (base, font) in [(base_a, FACE), (base_b, &bold[..])] {
        out[base..base + font.len()].copy_from_slice(font);
        // Table offsets in a collection count from the start of the file.
        let n = be16(font, 4) as usize;
        for i in 0..n {
            let at = base + 12 + 16 * i + 8;
            let off = be32(&out, at) + base as u32;
            out[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
    }
    out
}

/// The same text in both faces of one collection, at one CSS size.
#[test]
fn faces_of_one_collection() {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(two_face_collection())),
        Some(FontInfoOverride {
            family_name: Some("ProbeCollection"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one family");
    assert_eq!(registered[0].1.len(), 2, "two faces in it");
    doc.load_css(
        "body { margin: 0; font-family: ProbeCollection; font-size: 16px; \
         line-height: 40px; color: black; } p { margin: 0 }",
    );
    let body = doc.body();
    doc.set_inner_html(
        body,
        "<p>Regular face one</p><p style=\"font-weight: 700\">Regular face one</p>",
    );
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    check("collection faces", &mut doc, &mut painter);
}
