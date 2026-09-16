//! A pixel oracle for glyph paint, and what it says about #461 / #473.
//!
//! Before this file there was **no** pixel-level assertion about text anywhere
//! in the workspace — `paint_tests.rs`'s text case paints and checks only that
//! nothing panicked, and every one of #260's colour assertions rests on the
//! software painter's *fill* path. That absence is why the truncating
//! premultiply in `skia_painter::blit_alpha_mask` could sit there: it is
//! invisible to every existing test and to a whole-screen image diff alike
//! (`reference_visual_regression_gap` — a one-level shift over a couple of
//! thousand edge pixels is far below the noise floor a screenshot comparison
//! runs at).
//!
//! **And it is invisible to a human, which this file is not embarrassed by.**
//! The defect is at most one level of 255 on antialiased channels only — about
//! 0.2% of full scale on a few percent of the frame. It also darkens rather
//! than thinning, since only the colour channels moved and the alpha did not:
//! measured on opaque backgrounds, `#111` on white goes 230.4746 → 230.4434 and
//! `#eee` on black 24.5254 → 24.4937 when the truncation is restored, darker
//! both times. What is worth fixing is a provable one-sided bias; what is worth
//! having is this oracle.
//!
//! # What the oracle is
//!
//! Paint one line of ordinary Latin text in a **known** colour onto an
//! otherwise untouched pixmap. Nothing else is drawn, so the destination under
//! every glyph is transparent black and `SourceOver` reduces to a copy: the
//! bytes in the pixmap are the premultiplied bytes `blit_alpha_mask` wrote.
//! Each painted pixel therefore carries both halves of the arithmetic — its
//! alpha `A` is the glyph's coverage, and its red channel should be
//! `round(199 * A / 255)` for `color: rgb(199, 151, 101)`.
//!
//! Nothing here is a geometry literal measured from text: no assertion names a
//! coordinate, a glyph, a width or an advance, and each pixel is checked against
//! a value derived from its **own** alpha. The counts are the one quantity that
//! depends on what was rasterised, and they are asserted only as generous
//! floors.
//!
//! # Why the face is pinned, which is sharper than it looks
//!
//! An earlier revision of this paragraph claimed the fixture "states the same
//! thing whatever the host's fonts rasterise". **That is false**, and the #794
//! review constructed counter-cases on the host that wrote it, with the code
//! correct: `Brush Script MT` gave 26 offending samples, `Segoe Script` 45,
//! `Vladimir Script` 19, `Algerian` 3, and the default stack at 8-12px gave
//! 1-9.
//!
//! The premise is *"nothing else is drawn, so `SourceOver` reduces to a copy"*,
//! and it fails wherever **two glyphs' antialiased edges land on the same
//! pixel**: the second blit composites over the first and the result leaves the
//! single-blit curve. Forced directly, same string and face: `letter-spacing:
//! -2px` gives 11 offending samples, `-6px` 81, `-12px` 304, and two
//! deliberately overlapped spans 1170.
//!
//! So this fixture pins no coordinate, but the **validity of its assertion is
//! conditioned on a geometric property of the face** — that no two glyphs of
//! this string at this size overlap. That is the session memory's font-set trap
//! in a subtler form. The repair is the one `glyph_y_ydown_tests.rs` already
//! uses: register the bundled `Inter-Regular.ttf` under an override family and
//! name it, so the property belongs to a committed `.ttf` rather than to
//! whatever the host resolves `sans-serif` to. Verified with it pinned: zero
//! offending samples at 16, 24, 40 and 64px.
//!
//! # What it catches
//!
//! - Any premultiply on the mask glyph path that is not round-to-nearest.
//!   Measured: truncation (what #461 is) offends on 3 457 of the 6 822
//!   channel samples, `ceil` on 3 365, `+128` on 9.
//! - A premultiplied channel above its own alpha, which is the invariant
//!   tiny-skia validates and the thing #461 worried a naive fix would break —
//!   including an alpha channel corrupted *consistently* with its colour, which
//!   is the shape a self-referential assertion would miss.
//! - Text painted in the wrong colour entirely, or with the brush alpha
//!   dropped, since both move the whole band off the exact curve.
//!
//! That `+128` figure of 9 is why `TEXT_RGB`'s channels are what they
//! are, and it is worth reading before touching them. `+128` differs from
//! round-to-nearest only where `c * A ≡ 127 (mod 255)`, and `c * A mod 255` only
//! ever takes multiples of `gcd(c, 255)` — so for a channel like `200`
//! (`gcd = 5`) that congruence has **no solution at all** and the mutant is
//! invisible here by construction. The first draft of this file used
//! `200, 150, 100` and passed against it, measured. Even at `199, 151, 101` the
//! margin is 9 samples out of 6 822 and depends on the face
//! happening to produce those alphas, so the *guarantee* against that mutant is
//! the exhaustive unit fixture in `skia_painter.rs`, not this one.
//!
//! # What it does not catch
//!
//! - **The colour-glyph path.** A COLR glyph needs a colour font no CI host is
//!   guaranteed to have, so `blit_color_glyph` is pinned font-free inside
//!   `skia_painter.rs` instead, beside the mask path's own unit fixtures. That
//!   module is also where the `PremultipliedColorU8` validity proof over the
//!   whole 256x256 domain lives. (There is no *bitmap* colour path to cover:
//!   `draw_glyphs` never asks swash for `ColorBitmap`, so CBDT/CBLC and sbix
//!   faces paint nothing at all — measured in the #794 review and filed
//!   separately.)
//! - **A fully opaque pixel.** `A == 255` makes `c * A / 255 == c`, so an
//!   interior pixel separates no rounding rule and is skipped here. It is not
//!   an empty case: `blit_color_glyph` has a whole `sa == 255` branch that
//!   bypasses the premultiply, and only `the_colour_glyph_premultiply_rounds`
//!   covers it.
//! - **Anything on the GPU path.** Vello rasterises glyphs itself in float and
//!   has no `u8` premultiply to get wrong; rinch hands it the brush and the
//!   font. There is no cast of this shape in `vello_painter.rs` — but a
//!   software-only test could not have told you that either way, and this one
//!   does not try.
//! - **Sub-level colour error from anything upstream of the painter.** The
//!   oracle compares against the colour the *brush* carries, so a mistake in
//!   resolving `rgb(199, 151, 101)` to that brush would move both sides of the
//!   comparison together. #260's tests are the ones that cover that half.
//! - **Glyph placement.** Deliberately. A fixture that pinned where a glyph
//!   lands would pin this machine's font set, which is exactly the trap the
//!   session memory records CI falling into.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

/// None of the three is `0` or `255`, and all three are coprime to `255`. See
/// the module doc: the first is what stops the fixture sitting on the fixed
/// point where truncation and rounding agree, the second is what lets it see the
/// `+128` mutant at all.
const TEXT_RGB: [u8; 3] = [199, 151, 101];

/// The bundled face, registered under an override name so the host's own fonts
/// cannot answer instead — the discipline `glyph_y_ydown_tests.rs` and
/// `fonts.rs`'s `claim_generic_families` tests already use.
///
/// It is not decoration here. See the module doc: the oracle's premise fails
/// where two glyphs' antialiased edges land on one pixel, which is a property
/// of the *face*, and several faces on this host break it.
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
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

/// The face, the size and the line box are all declared, so every input to the
/// glyph rasterisation is a value a reader can check rather than one the host
/// chose. `font-size` and `line-height` for the usual reason (session memory: a
/// text-derived literal pins the local font set, and CI's fonts are not this
/// machine's); `font-family` for the sharper reason in the module doc.
fn painted_text() -> TinySkiaPainter {
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

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "color: rgb(199, 151, 101); font-family: ProbeFace; font-size: 40px; \
         line-height: 48px; width: 380px",
    );
    doc.append_child(body, div);
    let text = doc.create_text("Hamburgefonstiv 0123");
    doc.append_child(div, text);
    doc.resolve_layout(VW, VH);

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);
    painter
}

struct EdgeBand {
    /// Signed `channel - exact` for every channel of every partially covered
    /// pixel.
    errors: Vec<f64>,
    /// How many of those channels sit where truncation and rounding disagree.
    /// The positive control: without it a fixture that painted only glyph
    /// interiors, or nothing at all, would pass every assertion vacuously.
    discriminating: usize,
    /// Channels found above their own alpha — the premultiplied invariant.
    invalid: usize,
    partial_pixels: usize,
    opaque_pixels: usize,
}

fn edge_band(painter: &TinySkiaPainter) -> EdgeBand {
    let px = painter.pixels();
    let mut band = EdgeBand {
        errors: Vec::new(),
        discriminating: 0,
        invalid: 0,
        partial_pixels: 0,
        opaque_pixels: 0,
    };

    for i in 0..(painter.width() * painter.height()) as usize {
        let a = px[i * 4 + 3];
        if a == 0 {
            continue;
        }
        if a == 255 {
            band.opaque_pixels += 1;
            // `A == 255` makes `c * A / 255 == c`: no rounding happens, so an
            // interior pixel cannot separate the two rules. Counted, not
            // asserted on.
            continue;
        }
        band.partial_pixels += 1;

        for (ch, &c) in TEXT_RGB.iter().enumerate() {
            let v = px[i * 4 + ch];
            if v > a {
                band.invalid += 1;
            }
            let exact = c as f64 * a as f64 / 255.0;
            band.errors.push(v as f64 - exact);
            if exact.fract() >= 0.5 {
                band.discriminating += 1;
            }
        }
    }
    band
}

/// The oracle. Every partially covered channel is within half a level of the
/// exact premultiplied value, and the band as a whole is unbiased.
///
/// # Against the bug
///
/// On the pinned face this fixture paints 2 274 antialiased pixels, so
/// 6 822 channel samples, of which 3 457 sit where truncation and rounding
/// disagree. With the truncating premultiply restored the signed error runs
/// **-0.996 to -0.004** — one-sided, never above — with a mean of
/// **-0.505**, and all 3 457 of those samples are more than half a level
/// short. With the fix the range is **-0.498 to +0.498** and the mean
/// **+0.002**; 3 457 channel values move up by exactly one, i.e. each of those
/// pixels stops being drawn one level darker than asked for.
///
/// # Against a mutant
///
/// Every mutation below was run, one at a time, against this file and the five
/// unit fixtures in `skia_painter.rs`:
///
/// - **truncation**, in the shared helper or at the mask call site: fails
///   `over_half` (3 457 samples) and `mean` (-0.505).
/// - **`ceil`**: fails `over_half` (3 365) and `mean` (+0.495). Not
///   `invalid` — `ceil(c * A / 255) <= A` still holds for `c <= 255`, which is
///   worth knowing, because it means the premultiplied-validity assertion is
///   not a substitute for the rounding one.
/// - **`+128`**: fails `over_half`, but by 9 samples. See the module
///   doc for why that margin is thin and what actually guarantees it.
/// - **a self-consistent alpha corruption** (the mask site writing
///   `alpha = premultiply_channel(a, a)`, so the channels stay in proportion to
///   the alpha the pixel carries): caught by `invalid` and by `over_half`.
/// - **the colour-glyph site alone**, either branch: passes here, necessarily —
///   no document fixture paints a COLR glyph.
///   `the_colour_glyph_premultiply_rounds` is the only thing in the workspace
///   that kills it.
#[test]
fn an_antialiased_glyph_edge_is_premultiplied_by_rounding_not_truncation() {
    let painter = painted_text();
    let band = edge_band(&painter);

    // ── Positive control ──────────────────────────────────────────────────
    // A filtered run that finds nothing is indistinguishable from a clean one,
    // so say out loud what must have been painted for anything below to mean
    // something. The floors are roughly a tenth of what the pinned face
    // produces (2 274 partial pixels, 3 457 discriminating channels); a build
    // where the face failed to register paints zero and fails here rather than
    // passing. (Verified by construction in the #794 review: giving the div an
    // opaque background makes every pixel opaque, `partial_pixels` drops to 0,
    // and this control fires.)
    assert!(
        band.partial_pixels >= 200,
        "only {} antialiased glyph pixels were painted — the fixture rendered no \
         text, so nothing below is evidence about anything",
        band.partial_pixels
    );
    assert!(
        band.discriminating >= 200,
        "only {} channel samples sit where rounding and truncation differ — this \
         fixture can no longer tell them apart",
        band.discriminating
    );

    // ── The premultiplied invariant (#461 step 2, end to end) ─────────────
    assert_eq!(
        band.invalid, 0,
        "{} channels came out above their own alpha; tiny-skia treats such a \
         pixel as invalid",
        band.invalid
    );

    // ── Round-to-nearest ──────────────────────────────────────────────────
    let worst = band
        .errors
        .iter()
        .cloned()
        .fold(0.0f64, |acc, e| if e.abs() > acc { e.abs() } else { acc });
    let over_half = band.errors.iter().filter(|e| e.abs() > 0.5).count();
    assert_eq!(
        over_half,
        0,
        "{over_half} of {} channel samples are more than half a level from the \
         exact premultiplied value (worst {worst:.3}) — the premultiply is not \
         rounding to nearest",
        band.errors.len()
    );

    // ── No systematic bias ────────────────────────────────────────────────
    // The sharp assertion above is the one a mutant trips first, but this is
    // the one that states the *defect*: truncation is not a rounding-mode
    // preference, it is a one-sided error, and a one-sided error shows up as a
    // mean. Truncation gives about -0.5 here; rounding gives about 0.
    //
    // "Darker", not "thinner": only the colour channels move, the alpha is
    // written through untouched, so a low premultiplied channel composites
    // darker over any background rather than more transparent.
    let mean = band.errors.iter().sum::<f64>() / band.errors.len() as f64;
    assert!(
        mean.abs() <= 0.1,
        "the antialiased edge band is biased by {mean:.3} of a level across {} \
         channel samples — every antialiased glyph pixel is rendering \
         systematically {} than asked for",
        band.errors.len(),
        if mean < 0.0 { "darker" } else { "lighter" }
    );
}
