//! #2286 stage 2, review of #2301: the glyph-mask atlas on the Vello painter,
//! frame after frame, counted (no GPU needed: the painter's glyph stats say
//! what each frame rasterised, uploaded, evicted and drew as outlines).
//!
//! - A static page whose masks are bigger than the atlas's budget is not
//!   rasterised again on every repaint: a frame's own masks are never evicted,
//!   and what the last frames drew is kept.
//! - Text whose size animates (a `font-size` transition, a `scale()`
//!   transition) draws as outlines while it animates, with no mask made or
//!   uploaded, and goes back to the atlas once it settles; text that does not
//!   change size (a still page, a `translateX` animation) never falls back.
//! - A colour face is drawn as outlines without being rasterised first.
//! - A font swash cannot read is drawn as outlines, not as nothing.
#![cfg(feature = "glyph-atlas")]

use peniko::{Blob, Brush, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::glyph_atlas::{GlyphAtlas, SharedAtlas};
use rinch_dom::paint::painter::{PaintGlyph, Painter};
use rinch_dom::paint::vello_painter::{GlyphStats, VelloPainter};

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const EMOJI: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji-U1F600.ttf");
const VW: f32 = 640.0;
const VH: f32 = 400.0;
const T0: f64 = 1_000_000.0;

fn doc_with(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(T0));
    for (bytes, name) in [(INTER, "ProbeFace"), (EMOJI, "ProbeEmoji")] {
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(bytes)),
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
    }
    doc.load_css("html, body { margin: 0; background: #fff; } p { margin: 0; }");
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    doc
}

/// A persistent painter, as the shell keeps one per window, on its own atlas.
fn painter(atlas: &SharedAtlas) -> VelloPainter {
    let mut p = VelloPainter::new();
    p.set_glyph_atlas(atlas.clone());
    p
}

/// One frame at clock `t`: transitions ticked, laid out, painted.
fn frame(doc: &mut RinchDocument, p: &mut VelloPainter, t: f64) -> GlyphStats {
    doc.set_animation_clock_ms(Some(t));
    doc.tick_transitions();
    doc.tick_animations();
    doc.resolve_layout(VW, VH);
    p.reset();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, p, 1.0, (VW, VH), &mut doc.font_cx, &mut cx);
    p.take_glyph_stats()
}

fn atlas(budget: usize) -> SharedAtlas {
    std::rc::Rc::new(std::cell::RefCell::new(GlyphAtlas::with_budget(budget)))
}

fn by_id(doc: &RinchDocument, id: &str) -> NodeId {
    NodeId(
        doc.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").is_some_and(|v| v == id))
            .map(|(i, _)| i)
            .expect("the element"),
    )
}

/// Review of #2301, item 1 (CJK at 2x re-rasterised ~2,550 glyphs a static
/// frame): a page whose masks are several times the budget, painted again
/// unchanged, rasterises, uploads and evicts nothing. Red when eviction may
/// drop masks the current or last frame drew.
#[test]
fn a_static_page_bigger_than_the_budget_is_not_rasterised_again() {
    let mut d = doc_with(
        "p { font: 40px/48px ProbeFace; color: #111; }",
        "<p>ABCDEFGHIJKLM</p><p>NOPQRSTUVWXYZ</p><p>abcdefghijklm</p><p>nopqrstuvwxyz</p>",
    );
    let full = atlas(usize::MAX);
    frame(&mut d, &mut painter(&full), T0);
    let need = full.borrow().bytes();
    let budget = need / 3;
    let a = atlas(budget);
    let mut p = painter(&a);
    let first = frame(&mut d, &mut p, T0);
    assert!(first.atlas.misses >= 52, "{first:?}");
    for n in 0..3 {
        let again = frame(&mut d, &mut p, T0);
        assert_eq!(
            (
                again.atlas.misses,
                again.atlas.uploads,
                again.atlas.evictions
            ),
            (0, 0, 0),
            "repaint {n}: {again:?}"
        );
        assert_eq!(again.atlas.hits, first.atlas.hits + first.atlas.misses);
    }
    // What no recent frame drew still goes: a page of other glyphs, drawn for
    // a few frames, pushes the letters out down to the budget's working set.
    let mut other = doc_with(
        "p { font: 40px/48px ProbeFace; color: #111; }",
        "<p>0123456789</p><p>9876543210</p>",
    );
    let mut evicted = 0;
    for _ in 0..4 {
        evicted += frame(&mut other, &mut p, T0).atlas.evictions;
    }
    assert!(evicted > 0, "the letters' masks were evicted");
}

const SIZE_CSS: &str = "#t { font: 13px/40px ProbeFace; color: #111; \
                        transition: font-size 1s linear, transform 1s linear; \
                        transform-origin: 0 0; } \
                        #t.big { font-size: 30px; } #t.scaled { transform: scale(2); } \
                        #t.moved { transform: translateX(37px); }";
const SIZE_HTML: &str = "<p id=\"t\">The quick brown fox jumps over the lazy dog</p>";

/// Run `class` as a 1 s transition from the settled page: frames every 16 ms
/// while it runs, then two frames after it has ended. Answers each frame's
/// stats, the first being the settled page's.
fn animate(class: &str) -> Vec<GlyphStats> {
    let mut d = doc_with(SIZE_CSS, SIZE_HTML);
    let a = atlas(usize::MAX);
    let mut p = painter(&a);
    let mut out = vec![frame(&mut d, &mut p, T0)];
    let t = by_id(&d, "t");
    d.set_attribute(t, "class", class);
    d.resolve_layout(VW, VH);
    for i in 1..=20 {
        out.push(frame(&mut d, &mut p, T0 + 16.0 * i as f64));
    }
    out.push(frame(&mut d, &mut p, T0 + 2_000.0));
    out.push(frame(&mut d, &mut p, T0 + 2_016.0));
    out.push(frame(&mut d, &mut p, T0 + 2_032.0));
    out
}

fn check_settles(class: &str) {
    let f = animate(class);
    let glyphs = f[0].atlas.hits + f[0].atlas.misses;
    assert!(glyphs > 30, "{class}: the line is drawn: {:?}", f[0]);
    assert_eq!(
        f[0].outline_fallbacks, 0,
        "{class}: settled, from the atlas"
    );
    // Mid-animation (from the third animated frame on): outlines, no mask.
    for (i, s) in f.iter().enumerate().take(21).skip(3) {
        assert_eq!(
            (s.atlas.misses, s.atlas.uploads, s.atlas.hits),
            (0, 0, 0),
            "{class} frame {i}: nothing from the atlas mid-animation: {s:?}"
        );
        assert_eq!(s.outline_fallbacks, glyphs, "{class} frame {i}: {s:?}");
    }
    // Settled again: back to the atlas, and then nothing new.
    let last = f.last().unwrap();
    assert_eq!(last.outline_fallbacks, 0, "{class}: settled: {last:?}");
    assert_eq!(
        (last.atlas.misses, last.atlas.uploads),
        (0, 0),
        "{class}: settled: {last:?}"
    );
    assert_eq!(last.atlas.hits, glyphs, "{class}: {last:?}");
}

/// Review of #2301, item 2: a `font-size` transition made new masks every
/// frame (154-168 misses a frame, about twice the CPU of outlines).
#[test]
fn a_font_size_transition_draws_outlines_until_it_settles() {
    check_settles("big");
}

/// The same for a `scale()` transition: a new device size every frame.
#[test]
fn a_scale_transition_draws_outlines_until_it_settles() {
    check_settles("scaled");
}

/// No false positive: a translation moves glyphs but not their size, so a
/// `translateX` transition stays on the atlas throughout.
#[test]
fn a_translate_transition_stays_on_the_atlas() {
    let f = animate("moved");
    for (i, s) in f.iter().enumerate() {
        assert_eq!(s.outline_fallbacks, 0, "frame {i}: {s:?}");
    }
}

/// Review of #2301, item 4: a colour face (the bundled emoji's bitmap
/// strike) is drawn as outlines straight away, with no mask rasterised and
/// thrown away first.
#[test]
fn a_colour_face_is_not_rasterised() {
    let mut d = doc_with(
        "p { font: 20px/28px ProbeEmoji; }",
        "<p>&#x1F600;&#x1F600;</p>",
    );
    let a = atlas(usize::MAX);
    let mut p = painter(&a);
    let s = frame(&mut d, &mut p, T0);
    assert_eq!(s.outline_fallbacks, 2, "{s:?}");
    assert_eq!((s.atlas.misses, s.atlas.hits), (0, 0), "{s:?}");
    assert!(a.borrow().is_empty(), "nothing kept");
}

/// Review of #2301, item 3: a glyph swash cannot render (here, an id past
/// the face's glyph count) was cached as empty and drew nothing on the GPU; it
/// is now handed to Vello's outlines, counted as a fallback, every frame. Its
/// neighbours stay on the atlas.
#[test]
fn a_glyph_swash_cannot_render_draws_as_an_outline() {
    let inter = FontData::new(Blob::new(std::sync::Arc::new(INTER)), 0);
    let a = atlas(usize::MAX);
    let mut p = painter(&a);
    let glyphs = [
        PaintGlyph {
            id: 3,
            x: 10.0,
            y: 20.0,
        },
        PaintGlyph {
            id: 60_000,
            x: 20.0,
            y: 20.0,
        },
        PaintGlyph {
            id: 5,
            x: 30.0,
            y: 20.0,
        },
    ];
    for n in 0..2 {
        p.reset();
        p.draw_glyphs(
            &inter,
            13.0,
            peniko::kurbo::Affine::IDENTITY,
            None,
            &Brush::Solid(peniko::color::palette::css::BLACK),
            true,
            &[],
            &glyphs,
        );
        let s = p.take_glyph_stats();
        assert_eq!(s.outline_fallbacks, 1, "frame {n}: {s:?}");
        assert_eq!(s.atlas.hits + s.atlas.misses, 3, "frame {n}: {s:?}");
    }
}

/// A font no reader can parse draws nothing and does not panic (Vello's own
/// `draw_glyphs` unwraps the font, so it is not handed one), and nothing is
/// cached.
#[test]
fn an_unreadable_font_draws_nothing_and_does_not_panic() {
    let junk = FontData::new(Blob::new(std::sync::Arc::new(vec![0_u8; 64])), 0);
    let a = atlas(usize::MAX);
    let mut p = painter(&a);
    let glyphs = [PaintGlyph {
        id: 3,
        x: 10.0,
        y: 20.0,
    }];
    p.draw_glyphs(
        &junk,
        13.0,
        peniko::kurbo::Affine::IDENTITY,
        None,
        &Brush::Solid(peniko::color::palette::css::BLACK),
        true,
        &[],
        &glyphs,
    );
    let s = p.take_glyph_stats();
    assert_eq!(s, GlyphStats::default(), "{s:?}");
    assert!(a.borrow().is_empty());
}

// ── Round 2 of the #2301 review ────────────────────────────────────────────

/// Item 1: two windows sharing the thread's atlas, each page bigger than
/// half the budget, painted in turn. Every repaint after the first of each
/// rasterises, uploads and evicts nothing: what each painter's last frame drew
/// is protected, not just the largest one's.
#[test]
fn two_windows_sharing_the_atlas_do_not_evict_each_other() {
    let css = "p { font: 40px/48px ProbeFace; color: #111; }";
    let mut a_doc = doc_with(css, "<p>ABCDEFGHIJKLM</p><p>NOPQRSTUVWXYZ</p>");
    let mut b_doc = doc_with(css, "<p>abcdefghijklm</p><p>nopqrstuvwxyz</p>");
    let full = atlas(usize::MAX);
    frame(&mut a_doc, &mut painter(&full), T0);
    let one = full.borrow().bytes();
    // Room for one page, not both.
    let shared = atlas(one * 3 / 2);
    let mut a = painter(&shared);
    let mut b = painter(&shared);
    frame(&mut a_doc, &mut a, T0);
    frame(&mut b_doc, &mut b, T0);
    for n in 0..3 {
        for (name, d, p) in [("a", &mut a_doc, &mut a), ("b", &mut b_doc, &mut b)] {
            let s = frame(d, p, T0);
            assert_eq!(
                (s.atlas.misses, s.atlas.uploads, s.atlas.evictions),
                (0, 0, 0),
                "round {n}, window {name}: {s:?}"
            );
        }
    }
}

/// Items 2 and 3: GPU partial frames draw only their damage. A 14px line and
/// a 20px line in one face, repainted on alternate frames (a caret blink
/// beside a clock; here the other line is hidden, so the frame draws only the
/// one), stay on the atlas; so does a size that comes back after one frame
/// away (A -> B -> A).
#[test]
fn alternating_damage_in_one_face_stays_on_the_atlas() {
    let mut d = doc_with(
        "#s { font: 14px/20px ProbeFace; color: #111; } \
         #b { font: 20px/28px ProbeFace; color: #111; } .h { visibility: hidden; }",
        "<p id=\"s\">caret line</p><p id=\"b\">12:34:56</p>",
    );
    let (sn, bn) = (by_id(&d, "s"), by_id(&d, "b"));
    let only = |d: &mut RinchDocument, small: bool| {
        d.set_attribute(sn, "class", if small { "" } else { "h" });
        d.set_attribute(bn, "class", if small { "h" } else { "" });
    };
    let a = atlas(usize::MAX);
    let mut p = painter(&a);
    for n in 0..8 {
        only(&mut d, n % 2 == 0);
        let s = frame(&mut d, &mut p, T0);
        assert!(
            s.atlas.hits + s.atlas.misses > 0,
            "frame {n} draws text: {s:?}"
        );
        assert_eq!(s.outline_fallbacks, 0, "frame {n}: {s:?}");
    }
    // A -> B -> A after a still stretch: the return is not "new".
    only(&mut d, true);
    for _ in 0..3 {
        frame(&mut d, &mut p, T0);
    }
    only(&mut d, false);
    let to_b = frame(&mut d, &mut p, T0);
    only(&mut d, true);
    let back = frame(&mut d, &mut p, T0);
    assert_eq!(to_b.outline_fallbacks, 0, "{to_b:?}");
    assert_eq!(back.outline_fallbacks, 0, "{back:?}");
}

/// Item 5: past the budget, the atlas comes back within it on the frames
/// after the big text stops being drawn, whether or not anything else draws
/// text: the trim runs as a painter starts a frame.
#[test]
fn the_overshoot_is_trimmed_on_the_next_frames_even_with_no_text() {
    let mut big = doc_with(
        "p { font: 40px/48px ProbeFace; color: #111; }",
        "<p>ABCDEFGHIJKLM</p><p>NOPQRSTUVWXYZ</p><p>abcdefghijklm</p>",
    );
    let mut blank = doc_with("", "<div style=\"height: 10px\"></div>");
    let full = atlas(usize::MAX);
    frame(&mut big, &mut painter(&full), T0);
    let budget = full.borrow().bytes() / 4;
    let a = atlas(budget);
    let mut p = painter(&a);
    frame(&mut big, &mut p, T0);
    assert!(a.borrow().bytes() > budget * 3, "overshoots while drawn");
    for _ in 0..3 {
        frame(&mut blank, &mut p, T0);
    }
    assert!(
        a.borrow().bytes() <= budget,
        "{} > {budget} after three text-free frames",
        a.borrow().bytes()
    );
}
