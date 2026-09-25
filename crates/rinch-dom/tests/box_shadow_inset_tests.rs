//! #974: an `inset` `box-shadow` is painted, inside the padding box.
//!
//! css-backgrounds-3 §7.1: an inner shadow is drawn inside the **padding**
//! box, as if everything outside the padding edge were opaque; its hole is
//! the padding box moved by the offset and shrunk by the spread, with the
//! padding box's radii shrunk by the spread; it paints above the background
//! and below the border; the first shadow in the list is on top; and a blur
//! is a Gaussian with a standard deviation of half the blur radius.
//!
//! Every expected value here is also what Chrome 153 paints, measured on a
//! headless screenshot of the same boxes over a white page (the probe page is
//! quoted in the PR). rinch used to skip every inset shadow outright, on both
//! painters, so each fixture here saw nothing at all before the fix.
//!
//! The fixtures stay off the fixed points: the offset is `10px 20px`, not a
//! symmetric one, so an x/y swap shows; the border is 5px and the spread 10px,
//! so the padding box and the border box disagree; and the blurred profile is
//! sampled at depths where a Gaussian and a linear ramp differ.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const BOX: i64 = 100;
const LEFT: i64 = 150;
const TOP: i64 = 150;

fn painted(style: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: {BOX}px; height: {BOX}px; {style}"
        ),
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
    painter
}

/// `[r, g, b, a]` of one pixel, each 0..=1 (premultiplied, as tiny-skia
/// stores it — exact for the opaque pixels these fixtures read colour from).
fn rgba(p: &TinySkiaPainter, x: i64, y: i64) -> [f64; 4] {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    let px = &p.pixels()[idx..idx + 4];
    [
        px[0] as f64 / 255.0,
        px[1] as f64 / 255.0,
        px[2] as f64 / 255.0,
        px[3] as f64 / 255.0,
    ]
}

fn alpha(p: &TinySkiaPainter, x: i64, y: i64) -> f64 {
    rgba(p, x, y)[3]
}

fn assert_alpha(p: &TinySkiaPainter, x: i64, y: i64, want: f64, what: &str) {
    let got = alpha(p, x, y);
    assert!(
        (got - want).abs() <= 0.02,
        "{what}: alpha at ({x}, {y}) is {got:.3}, want {want:.3}"
    );
}

/// `inset 10px 20px`: the hole is the box moved right 10 and down 20, so the
/// shadow is a 10px strip down the left edge and a 20px strip across the top.
/// Chrome 153: exactly those strips, solid, and nothing else.
#[test]
fn an_offset_inset_shadow_paints_the_uncovered_strips() {
    let p = painted("box-shadow: inset 10px 20px 0 0 rgb(0,0,0)");
    let mid = TOP + 70;
    // The left strip, 10px wide.
    assert_alpha(&p, LEFT, mid, 1.0, "left strip, first column");
    assert_alpha(&p, LEFT + 9, mid, 1.0, "left strip, last column");
    assert_alpha(&p, LEFT + 10, mid, 0.0, "just right of the left strip");
    // The top strip, 20px tall.
    assert_alpha(&p, LEFT + 70, TOP, 1.0, "top strip, first row");
    assert_alpha(&p, LEFT + 70, TOP + 19, 1.0, "top strip, last row");
    assert_alpha(&p, LEFT + 70, TOP + 20, 0.0, "just below the top strip");
    // Nothing on the far sides, and nothing outside the box.
    assert_alpha(&p, LEFT + BOX - 1, mid, 0.0, "right edge");
    assert_alpha(&p, LEFT + 70, TOP + BOX - 1, 0.0, "bottom edge");
    assert_alpha(&p, LEFT - 1, mid, 0.0, "outside the box");
    assert_alpha(&p, LEFT + 70, TOP - 1, 0.0, "outside the box");
}

/// The shadow is drawn inside the **padding** box, and its hole takes the
/// padding box's radii shrunk by the spread. Border 5px (transparent),
/// radius 30px, so the padding box is inset 5px with radius 25; spread 10px
/// makes the hole inset 15px with radius 15. Chrome 153, row through the
/// middle: transparent for 5px, black for 10px, clear after. Along the
/// diagonal from the corner: clear to 11px in, black 13..=18, clear from 20.
#[test]
fn an_inset_shadow_fills_the_padding_box_and_its_hole_is_rounded_by_the_spread() {
    let p = painted(
        "border: 5px solid transparent; border-radius: 30px; \
         box-shadow: inset 0 0 0 10px rgb(0,0,0)",
    );
    let mid = TOP + 55;
    for i in 0..5 {
        assert_alpha(&p, LEFT + i, mid, 0.0, "border area");
    }
    for i in 5..15 {
        assert_alpha(&p, LEFT + i, mid, 1.0, "the 10px ring");
    }
    assert_alpha(&p, LEFT + 15, mid, 0.0, "inside the hole");
    // Outside the padding box's rounded corner (radius 25 about (30, 30)).
    assert_alpha(&p, LEFT + 8, TOP + 8, 0.0, "outside the padding corner");
    for i in 13..=18 {
        assert_alpha(&p, LEFT + i, TOP + i, 1.0, "the ring across the corner");
    }
    // Inside the hole's corner (radius 15 about (30, 30)). A hole that kept
    // the padding radius 25 would put its arc's centre at (40, 40) and leave
    // this pixel in the shadow.
    assert_alpha(&p, LEFT + 22, TOP + 22, 0.0, "inside the hole's corner");
}

/// A blurred inset shadow is a Gaussian of `sigma = blur / 2` about the
/// hole's edge. `inset 0 0 20px`, row through the middle, alpha at depth `d`
/// from the left edge (pixel centre `d + 0.5`). Chrome 153 measures
/// 0.467 / 0.290 / 0.141 / 0.055 / 0.012; the Gaussian gives
/// 0.480 / 0.291 / 0.147 / 0.061 / 0.020.
#[test]
fn a_blurred_inset_shadow_falls_off_as_a_gaussian() {
    let p = painted("box-shadow: inset 0 0 20px rgb(0,0,0)");
    let mid = TOP + 50;
    for (d, chrome) in [
        (0, 0.467),
        (5, 0.290),
        (10, 0.141),
        (15, 0.055),
        (20, 0.012),
    ] {
        let got = alpha(&p, LEFT + d, mid);
        assert!(
            (got - chrome).abs() <= 0.04,
            "blurred inset shadow at depth {d}: alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
    assert!(
        alpha(&p, LEFT + 50, mid) <= 0.01,
        "the middle of the box is 2.5 sigma from every edge"
    );
    assert_alpha(&p, LEFT - 1, mid, 0.0, "outside the box");
    // The corner, where two edges' shadows meet. Chrome 153 is separable
    // there — the hole's blurred coverage is the product of the two axes' —
    // so the corner is darker than either edge: 0.706 at the corner pixel,
    // 0.494 five pixels in along the diagonal. A model that asks only for the
    // nearest edge (#1014's first cut) gave 0.435 and 0.314.
    for (d, chrome) in [(0, 0.706), (5, 0.494)] {
        let got = alpha(&p, LEFT + d, TOP + d);
        assert!(
            (got - chrome).abs() <= 0.04,
            "blurred inset shadow's corner at ({d}, {d}): alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
}

/// A small blur (`inset 0 0 4px`, sigma 2) against Chrome 153, mid-edge and
/// at the corner.
#[test]
fn a_small_blurred_inset_shadow_matches_chrome() {
    let p = painted("box-shadow: inset 0 0 4px rgb(0,0,0)");
    for ((x, y), chrome) in [
        ((0, 50), 0.400),
        ((1, 50), 0.251),
        ((2, 50), 0.125),
        ((0, 0), 0.631),
        ((1, 1), 0.439),
        ((3, 3), 0.098),
    ] {
        let got = alpha(&p, LEFT + x, TOP + y);
        assert!(
            (got - chrome).abs() <= 0.05,
            "inset 0 0 4px at ({x}, {y}): alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
}

/// A rounded hole blurs as a rounded hole. `border-radius: 30px; inset 0 0
/// 15px`, along the diagonal from the corner. Chrome 153: clear until the
/// padding corner's arc (about 8.8px in), then 0.427 at 10px, 0.125 at 15px
/// and nearly clear at 20px. A hole blurred as if it were square would give
/// the diagonal a square hole's corner and leave these too dark.
#[test]
fn a_blurred_rounded_inset_shadow_follows_the_rounded_hole() {
    let p = painted("border-radius: 30px; box-shadow: inset 0 0 15px rgb(0,0,0)");
    assert_alpha(&p, LEFT + 6, TOP + 6, 0.0, "outside the padding corner");
    for (d, chrome) in [(10, 0.427), (12, 0.286), (15, 0.125), (20, 0.012)] {
        let got = alpha(&p, LEFT + d, TOP + d);
        assert!(
            (got - chrome).abs() <= 0.04,
            "rounded blurred inset shadow at ({d}, {d}): alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
}

/// Review of #1014, F1: on the software painter the shadow reaches the
/// rounded clip's anti-aliased edge **once**. Drawn as a stack of translucent
/// fills each clipped on its own, the edge pixel compounded the clip's
/// coverage into itself and came out 0.80 — a dark rim along the curve.
/// Chrome 153 paints 0.357 there; one draw through the clip gives the
/// unblurred shape's own edge coverage, 0.435.
#[test]
fn a_blurred_rounded_inset_shadow_has_no_dark_rim() {
    let p = painted("border-radius: 30px; box-shadow: inset 10px 20px 15px 5px rgb(0,0,0)");
    for (x, y) in [(20, 1), (1, 20)] {
        let got = alpha(&p, LEFT + x, TOP + y);
        assert!(
            (got - 0.357).abs() <= 0.1,
            "the clip edge at ({x}, {y}): alpha {got:.3}, Chrome 153 paints 0.357"
        );
    }
}

/// A translucent inset shadow keeps its alpha, unblurred and blurred.
/// `rgba(0,0,255,0.5)`: the unblurred ring is 0.5 deep; `inset 0 0 10px 5px`
/// at depths 0 / 5 / 12, Chrome 153: 0.416 / 0.231 / 0.024.
#[test]
fn a_translucent_inset_shadow_keeps_its_alpha() {
    let p = painted("box-shadow: inset 0 0 0 10px rgba(0,0,255,0.5)");
    assert_alpha(&p, LEFT + 5, TOP + 50, 0.5, "the unblurred ring");
    let p = painted("box-shadow: inset 0 0 10px 5px rgba(0,0,255,0.5)");
    for (d, chrome) in [(0, 0.416), (5, 0.231), (12, 0.024)] {
        let got = alpha(&p, LEFT + d, TOP + 50);
        assert!(
            (got - chrome).abs() <= 0.03,
            "translucent blurred inset shadow at depth {d}: alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
}

/// Unequal borders at a corner: `border-width: 5px 30px 5px 5px;
/// border-radius: 40px`. CSS makes the padding corner at the top right an
/// ellipse (10 wide, 35 tall); rinch's radii are circular and take the larger
/// border, giving a radius of 10 — which agrees with Chrome 153 at this pixel
/// (black, in the ring under the corner). Taking the smaller border instead
/// (radius 35) would cut it away.
#[test]
fn an_unequal_border_corner_takes_the_larger_border() {
    let p = painted(
        "border-style: solid; border-color: rgba(0,0,0,0); border-width: 5px 30px 5px 5px; \
         border-radius: 40px; box-shadow: inset 0 0 0 10px rgb(0,0,0)",
    );
    assert_alpha(&p, LEFT + 60, TOP + 10, 1.0, "under the top-right corner");
}

/// Review of #1014, F2: a `data-viewport` hole in a clipping container shows
/// the layer beneath through the container's inset shadow, exactly as it does
/// through its background (a browser paints the child above its parent's
/// shadow) — and whether or not the container paints a background, since the
/// holes used to be looked for only when it did.
#[test]
fn an_inset_shadow_is_cut_for_a_viewport_hole() {
    for background in ["background: rgb(255,255,255);", ""] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!(
                "position: absolute; left: {LEFT}px; top: {TOP}px; width: 200px; height: 200px; \
                 overflow: hidden; {background} box-shadow: inset 0 0 0 30px rgb(0,0,0)"
            ),
        );
        let hole = doc.create_element("div");
        doc.set_attribute(hole, "data-viewport", "game");
        doc.set_attribute(hole, "style", "width: 100px; height: 100px");
        doc.append_child(div, hole);
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);
        let mut p = TinySkiaPainter::new(800, 600);
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut p,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut lcx,
        );
        let what = |w: &str| format!("{w} ({background:?})");
        assert_alpha(
            &p,
            LEFT + 5,
            TOP + 50,
            0.0,
            &what("the ring inside the viewport hole"),
        );
        assert_alpha(&p, LEFT + 50, TOP + 50, 0.0, &what("the hole itself"));
        assert_eq!(
            rgba(&p, LEFT + 5, TOP + 150),
            [0.0, 0.0, 0.0, 1.0],
            "{}",
            what("the ring outside the hole")
        );
    }
}

/// An inset shadow paints **above** the element's background (an outer one
/// paints below it, where the background hides it).
#[test]
fn an_inset_shadow_paints_above_the_background() {
    let p = painted("background: rgb(255,255,255); box-shadow: inset 10px 20px 0 0 rgb(0,0,0)");
    let [r, g, b, a] = rgba(&p, LEFT + 5, TOP + 70);
    assert_eq!(
        (r, g, b, a),
        (0.0, 0.0, 0.0, 1.0),
        "the background painted over the inset shadow"
    );
    let [r, g, b, _] = rgba(&p, LEFT + 50, TOP + 70);
    assert_eq!((r, g, b), (1.0, 1.0, 1.0), "the background itself");
}

/// The first shadow in the list is on top. A 10px black ring over a 20px red
/// one: black for the first 10px, red for the next 10.
#[test]
fn the_first_inset_shadow_is_on_top() {
    let p = painted("box-shadow: inset 0 0 0 10px rgb(0,0,0), inset 0 0 0 20px rgb(255,0,0)");
    let mid = TOP + 50;
    let [r, g, b, a] = rgba(&p, LEFT + 5, mid);
    assert_eq!(
        (r, g, b, a),
        (0.0, 0.0, 0.0, 1.0),
        "depth 5: the first shadow"
    );
    let [r, g, b, a] = rgba(&p, LEFT + 15, mid);
    assert_eq!(
        (r, g, b, a),
        (1.0, 0.0, 0.0, 1.0),
        "depth 15: the second shadow"
    );
    assert_alpha(&p, LEFT + 25, mid, 0.0, "depth 25: neither");
}

/// A negative spread grows the hole past the padding box, which leaves no
/// shadow at all (Chrome 153: nothing painted).
#[test]
fn a_negative_spread_inset_shadow_paints_nothing() {
    let p = painted("box-shadow: inset 0 0 0 -10px rgb(0,0,0)");
    for (x, y) in [(LEFT, TOP), (LEFT + 5, TOP + 50), (LEFT + 50, TOP + 50)] {
        assert_alpha(&p, x, y, 0.0, "negative-spread inset shadow");
    }
}

/// A spread larger than half the box leaves no hole: the whole padding box is
/// shadow, including its middle.
#[test]
fn an_inset_spread_past_the_middle_fills_the_whole_padding_box() {
    let p = painted("box-shadow: inset 0 0 0 60px rgb(0,0,0)");
    assert_alpha(&p, LEFT + 50, TOP + 50, 1.0, "the middle");
    assert_alpha(&p, LEFT - 1, TOP + 50, 0.0, "outside the box");
}

// ── The crop (round 2 of #1014's review) ───────────────────────────────────

const B: &str = "position: absolute; box-sizing: border-box;";

/// `html` inside a margin-less body, laid out and painted at `w`x`h` over
/// white.
fn painted_at(html: &str, w: u32, h: u32) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(w as f32, h as f32);
    let mut p = TinySkiaPainter::new(w, h);
    p.fill_white();
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        1.0,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    p
}

/// The blurred shadow's image is cropped to what the frame can show, and the
/// crop must change no pixel. So the same document painted into a 500x450
/// window and into a 1600x1400 one agrees wherever the small one can see.
/// Each case puts a shadow edge or rounded corner just past the small
/// window's cull edge, within the blur's reach — where a crop that forgot the
/// translation, grew the corner patch short of the margin, or pulled the
/// visible rect back wrongly would show. Rotated and scaled boxes differ by a
/// few anti-aliasing bytes between window sizes with no shadow at all (the
/// review measured the controls), so those are held to a bound, not zero.
#[test]
fn the_crop_changes_no_pixel() {
    let cases = [
        (
            "corner past the right edge",
            format!(
                "<div style=\"{B} left:200px; top:100px; width:400px; height:200px; border-radius: 30px; box-shadow: inset 0 0 90px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "translated, corner past the right edge",
            format!(
                "<div style=\"{B} left:0; top:0; width:400px; height:200px; transform: translate(200px, 100px); border-radius: 30px; box-shadow: inset 0 0 90px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "translated off the left",
            format!(
                "<div style=\"{B} left:0; top:0; width:400px; height:200px; transform: translate(-150px, 100px); border-radius: 30px; box-shadow: inset 0 0 40px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "placed right, translated back left",
            format!(
                "<div style=\"{B} left:700px; top:100px; width:400px; height:200px; transform: translate(-400px, 0); border-radius: 30px; box-shadow: inset 0 0 40px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "a big corner sliver just past the crop",
            format!(
                "<div style=\"{B} left:220px; top:60px; width:400px; height:330px; border-radius: 100px; box-shadow: inset 0 0 80px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "rotated off the right",
            format!(
                "<div style=\"{B} left:350px; top:120px; width:700px; height:200px; transform: rotate(10deg); box-shadow: inset 0 0 30px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "rotated, big",
            format!(
                "<div style=\"{B} left:100px; top:50px; width:600px; height:300px; transform: rotate(20deg); border-radius: 30px; box-shadow: inset 0 0 40px rgb(0,0,0)\"></div>"
            ),
        ),
        (
            "scaled",
            format!(
                "<div style=\"{B} left:100px; top:50px; width:300px; height:150px; transform: scale(2); border-radius: 30px; box-shadow: inset 0 0 40px rgb(0,0,0)\"></div>"
            ),
        ),
    ];
    let mut bad = Vec::new();
    for (name, html) in cases {
        let small = painted_at(&html, 500, 450);
        let large = painted_at(&html, 1600, 1400);
        let mut differ = 0;
        for y in 0..450u32 {
            for x in 0..500u32 {
                let i = ((y * 500 + x) * 4) as usize;
                let j = ((y * 1600 + x) * 4) as usize;
                differ += (0..4)
                    .filter(|k| small.pixels()[i + k] != large.pixels()[j + k])
                    .count();
            }
        }
        let aa_noise = name.contains("rotated") || name.contains("scaled");
        if (differ > 0 && !aa_noise) || differ > 200 {
            bad.push((name, differ));
        }
    }
    assert!(
        bad.is_empty(),
        "the crop changed pixels (bytes differing): {bad:?}"
    );
}

/// A large radius and a small blur: the rounded hole's shadow along the arc
/// lies inside the region where the *rect's* blur leaves no shadow (the arc
/// is ~17.6px in along the diagonal, the blur reaches ~6px). The image is cut
/// into rings around that clear middle, and the middle has to be shrunk for
/// the corners, or the shadow along the arc is never drawn.
#[test]
fn a_small_blur_under_a_large_radius_keeps_the_arcs_shadow() {
    let p = painted_at(
        &format!(
            "<div style=\"{B} left:150px; top:50px; width:200px; height:200px; border-radius: 60px; box-shadow: inset 0 0 4px rgb(0,0,0)\"></div>"
        ),
        500,
        450,
    );
    // Over white: 1 - red/255 is the shadow's alpha.
    let a = |x: u32, y: u32| 1.0 - p.pixels()[((y * 500 + x) * 4) as usize] as f64 / 255.0;
    let on_arc = a(150 + 18, 50 + 18);
    assert!(
        on_arc > 0.2,
        "the shadow along the rounded corner's arc is missing: alpha {on_arc:.3}"
    );
}

/// The standard normal CDF (Abramowitz & Stegun 7.1.26), for the oracle.
fn phi(x: f64) -> f64 {
    let z = x / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z.abs());
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-z * z).exp();
    0.5 * (1.0 + if z >= 0.0 { y } else { -y })
}

/// The blurred profile against the analytic Gaussian — §7.1's definition,
/// exact for a square hole — to about one alpha level in a hundred, at blur
/// 6, 20 and 60 (a sampled kernel for the first, three box blurs for the
/// others). The Chrome samples above allow 0.04, which an off-by-one in the
/// box blur's running sum gets through; this does not.
#[test]
fn the_blurred_profile_is_the_gaussian_to_one_percent() {
    for blur in [6.0f64, 20.0, 60.0] {
        let sigma = blur / 2.0;
        let p = painted_at(
            &format!(
                "<div style=\"{B} left:150px; top:50px; width:300px; height:300px; box-shadow: inset 0 0 {blur}px rgb(0,0,0)\"></div>"
            ),
            600,
            450,
        );
        let covered = |t: f64| phi(t / sigma) - phi((t - 300.0) / sigma);
        let mut worst: f64 = 0.0;
        for d in 0..150u32 {
            let (x, y) = (150 + d, 200);
            let got = 1.0 - p.pixels()[((y * 600 + x) * 4) as usize] as f64 / 255.0;
            let want = 1.0 - covered(d as f64 + 0.5) * covered(150.0);
            worst = worst.max((got - want).abs());
        }
        assert!(
            worst <= 0.012,
            "blur {blur}: the profile is {worst:.4} from the Gaussian at worst"
        );
    }
}
