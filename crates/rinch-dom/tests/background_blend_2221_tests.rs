//! #2221 — `background-blend-mode` on the software painter.
//!
//! Every expected pixel is Chrome 156's (headless, `--force-device-scale-factor=1`,
//! the same markup with the image as a `data:` PNG). The GPU twin is
//! `crates/rinch/tests/gpu_background_blend_2221.rs`, with the same boxes.
//!
//! The backdrop is `Cb = rgb(204, 102, 51)` (`background-color`) and the
//! source `Cs = rgb(51, 153, 230)` (a one-colour gradient layer). Chrome's
//! pixels agree with the compositing-1 formulas to within one step (its
//! `color-dodge` gives 254 where the formula gives 255), for example
//! `multiply` = `Cb × Cs` = (40.8, 61.2, 46.0), `screen` = `Cb + Cs − Cb × Cs`
//! = (214.2, 193.8, 235.0), `difference` = `|Cb − Cs|` = (153, 51, 179), and
//! `luminosity` = `SetLum(Cb, Lum(Cs))` = (207.9, 105.9, 54.9). The tolerance is
//! [`TOL`], one step per channel: tiny-skia and Chrome's Skia round the
//! 8-bit premultiplied intermediates differently, either way (`multiply`'s
//! 40.8 is 41 here and 40 in Chrome; `color-dodge`'s exact 255 is Chrome's
//! 254), and no case here differs by more.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::BlendModeValue as M;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// Per channel, against Chrome (see the module docs).
const TOL: u8 = 1;

const W: u32 = 400;
const H: u32 = 220;

/// `background-blend-mode`'s keywords, in the order of the first row of boxes.
const MODES: [&str; 16] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

/// Chrome 156's pixel at the centre of each box of the first row.
const CHROME_MODES: [[u8; 3]; 16] = [
    [51, 153, 230],
    [40, 61, 46],
    [214, 194, 235],
    [173, 122, 92],
    [51, 102, 51],
    [204, 153, 230],
    [254, 254, 255],
    [0, 0, 28],
    [81, 132, 214],
    [179, 114, 102],
    [153, 51, 179],
    [174, 133, 189],
    [59, 146, 212],
    [217, 98, 38],
    [47, 149, 226],
    [207, 105, 54],
];

const CS: &str = "linear-gradient(rgb(51, 153, 230), rgb(51, 153, 230))";
const CS_HALF: &str = "linear-gradient(rgba(51, 153, 230, 0.5), rgba(51, 153, 230, 0.5))";
const C2: &str = "linear-gradient(rgb(120, 200, 40), rgb(120, 200, 40))";
const C2_HALF: &str = "linear-gradient(rgba(120, 200, 40, 0.5), rgba(120, 200, 40, 0.5))";
const CB: &str = "rgb(204, 102, 51)";

/// 2x2: white, rgb(200, 150, 100) / rgb(80, 160, 240), transparent.
fn tint_image() -> DecodedImage {
    DecodedImage::new(
        [
            [255, 255, 255, 255],
            [200, 150, 100, 255],
            [80, 160, 240, 255],
            [0, 0, 0, 0],
        ]
        .concat(),
        2,
        2,
    )
}

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } div { position: absolute; display: block; }",
    );
    doc.tree
        .image_cache
        .insert_decoded("t2221.png".into(), tint_image());
    doc
}

fn add_to(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

fn add(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    add_to(doc, body, style)
}

fn mode_box(i: usize, mode: &str) -> String {
    format!(
        "left: {}px; top: 0; width: 20px; height: 20px; background-image: {CS}; \
         background-color: {CB}; background-blend-mode: {mode}",
        i * 20
    )
}

fn paint(doc: &mut RinchDocument) -> TinySkiaPainter {
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Each `(name, point, chrome)` within `tol` per channel.
fn expect(p: &TinySkiaPainter, want: &[(&str, (u32, u32), [u8; 3])], tol: u8) {
    let bad: Vec<_> = want
        .iter()
        .filter_map(|&(name, (x, y), w)| {
            let got = px(p, x, y);
            (got.iter().zip(w).any(|(a, b)| a.abs_diff(b) > tol)).then_some((name, got, w))
        })
        .collect();
    assert!(bad.is_empty(), "(case, rinch, chrome): {bad:?}");
}

/// The property reaches the computed style, one entry per layer: the first
/// layer's in `background_blend_mode`, each other layer's on its
/// `BackgroundLayer`. A layer rinch cannot paint (a conic gradient) is left
/// out of `background_layers`, and the layers after it still take the entry
/// at their own index in the image list, not at their place in the kept
/// list; a shorter list repeats.
#[test]
fn background_blend_mode_is_computed_per_layer() {
    let mut doc = new_doc();
    let plain = add(&mut doc, &format!("background-image: {CS}"));
    let modes: Vec<NodeId> = MODES
        .iter()
        .map(|m| {
            add(
                &mut doc,
                &format!("background-image: {CS}; background-blend-mode: {m}"),
            )
        })
        .collect();
    let skipped = add(
        &mut doc,
        &format!(
            "background-image: {CS}, conic-gradient(red, blue), {C2}; \
             background-blend-mode: multiply, screen, difference"
        ),
    );
    let repeated = add(
        &mut doc,
        &format!("background-image: {CS}, {C2}, {CS}; background-blend-mode: hue, color-burn"),
    );
    doc.resolve_layout(W as f32, H as f32);
    let cs = |n: NodeId| &doc.tree.nodes[n.0].computed_style;
    assert_eq!(cs(plain).background_blend_mode, M::Normal);
    let want = [
        M::Normal,
        M::Multiply,
        M::Screen,
        M::Overlay,
        M::Darken,
        M::Lighten,
        M::ColorDodge,
        M::ColorBurn,
        M::HardLight,
        M::SoftLight,
        M::Difference,
        M::Exclusion,
        M::Hue,
        M::Saturation,
        M::Color,
        M::Luminosity,
    ];
    for (n, w) in modes.iter().zip(want) {
        assert_eq!(cs(*n).background_blend_mode, w);
    }
    assert_eq!(cs(skipped).background_blend_mode, M::Multiply);
    let kept: Vec<M> = cs(skipped)
        .background_layers
        .iter()
        .map(|l| l.blend)
        .collect();
    assert_eq!(kept, [M::Difference], "the conic layer's index is kept");
    assert_eq!(cs(repeated).background_blend_mode, M::Hue);
    let kept: Vec<M> = cs(repeated)
        .background_layers
        .iter()
        .map(|l| l.blend)
        .collect();
    assert_eq!(kept, [M::ColorBurn, M::Hue], "the list repeats");
}

/// Each of the sixteen modes, `Cs` over `Cb`.
#[test]
fn every_mode_matches_chrome() {
    let mut doc = new_doc();
    for (i, m) in MODES.iter().enumerate() {
        add(&mut doc, &mode_box(i, m));
    }
    let p = paint(&mut doc);
    let want: Vec<_> = MODES
        .iter()
        .zip(CHROME_MODES)
        .enumerate()
        .map(|(i, (m, c))| (*m, (i as u32 * 20 + 10, 10), c))
        .collect();
    expect(&p, &want, TOL);
}

/// A half-transparent layer: `B(Cb, Cs)` composited over `Cb` at the layer's
/// alpha, `0.5 × Cb × Cs + 0.5 × Cb` = (122.4, 81.6, 48.5).
#[test]
fn a_translucent_layer_blends_by_its_alpha() {
    let mut doc = new_doc();
    add(
        &mut doc,
        &format!(
            "left: 0; top: 30px; width: 20px; height: 20px; background-image: {CS_HALF}; \
             background-color: {CB}; background-blend-mode: multiply"
        ),
    );
    let p = paint(&mut doc);
    expect(&p, &[("half-alpha multiply", (10, 40), [122, 81, 48])], TOL);
}

/// The element's background is an isolated group (compositing-1 §8.1): a
/// layer blends with the layers and the colour under it, never with what is
/// behind the element. Over a red parent and with no `background-color`, a
/// `multiply` layer has a transparent backdrop and is drawn as itself; were
/// it blended with the parent, it would be `red × Cs` = (51, 0, 0).
#[test]
fn the_background_is_an_isolated_group() {
    let mut doc = new_doc();
    let parent = add(
        &mut doc,
        "left: 30px; top: 30px; width: 40px; height: 20px; background: red",
    );
    add_to(
        &mut doc,
        parent,
        &format!(
            "left: 10px; top: 0; width: 20px; height: 20px; background-image: {CS}; \
             background-blend-mode: multiply"
        ),
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ("isolated", (50, 40), [51, 153, 230]),
            ("parent", (35, 40), [255, 0, 0]),
        ],
        TOL,
    );
}

/// Each layer blends with what is under it by its own entry, and a shorter
/// list repeats.
///
/// `multiply, screen`: the bottom layer, `C2` at alpha 0.5, is screened onto
/// `Cb`, then `Cs` multiplies that: Chrome (43, 97, 60). A single
/// `difference` serves both layers: `|Cb − C2|` = (84, 98, 11), then `Cs` at
/// alpha 0.5 differenced onto it: Chrome (59, 76, 115).
#[test]
fn each_layer_takes_its_own_entry() {
    let mut doc = new_doc();
    add(
        &mut doc,
        &format!(
            "left: 80px; top: 30px; width: 20px; height: 20px; background-image: {CS}, {C2_HALF}; \
             background-color: {CB}; background-blend-mode: multiply, screen"
        ),
    );
    add(
        &mut doc,
        &format!(
            "left: 110px; top: 30px; width: 20px; height: 20px; background-image: {CS_HALF}, {C2}; \
             background-color: {CB}; background-blend-mode: difference"
        ),
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ("multiply, screen", (90, 40), [43, 97, 60]),
            ("difference repeated", (120, 40), [59, 76, 115]),
        ],
        TOL,
    );
}

/// The tint: an image multiplied by its `background-color` and masked by
/// itself, so each opaque texel is `texel × tint` and the transparent one
/// shows the page. Tint rgb(255, 128, 64): white → the tint, (200, 150, 100)
/// → (200, 75, 25), (80, 160, 240) → (80, 80, 60).
#[test]
fn an_image_is_tinted_by_its_colour() {
    let mut doc = new_doc();
    add(
        &mut doc,
        "left: 0; top: 60px; width: 40px; height: 40px; background-image: url(t2221.png); \
         background-size: 100% 100%; background-color: rgb(255, 128, 64); \
         background-blend-mode: multiply; mask-image: url(t2221.png); mask-size: 100% 100%; \
         image-rendering: pixelated",
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ("white texel", (10, 70), [255, 128, 64]),
            ("orange texel", (30, 70), [200, 75, 25]),
            ("blue texel", (10, 90), [80, 80, 60]),
            ("clear texel", (30, 90), [255, 255, 255]),
        ],
        TOL,
    );
}

/// A blended background under the element's own opacity, a transform, a
/// rounded border box and an ancestor's clip.
#[test]
fn blending_under_opacity_transform_radius_and_clip() {
    let mut doc = new_doc();
    add(
        &mut doc,
        &format!(
            "left: 50px; top: 60px; width: 20px; height: 20px; opacity: 0.5; \
             background-image: {CS}; background-color: {CB}; background-blend-mode: screen"
        ),
    );
    add(
        &mut doc,
        &format!(
            "left: 80px; top: 60px; width: 20px; height: 20px; transform: scale(2); \
             transform-origin: 0 0; background-image: {CS}; background-color: {CB}; \
             background-blend-mode: color-burn"
        ),
    );
    add(
        &mut doc,
        &format!(
            "left: 130px; top: 60px; width: 40px; height: 40px; border-radius: 20px; \
             background-image: {CS}; background-color: {CB}; background-blend-mode: luminosity"
        ),
    );
    let clip = add(
        &mut doc,
        "left: 180px; top: 60px; width: 20px; height: 20px; overflow: hidden",
    );
    add_to(
        &mut doc,
        clip,
        &format!(
            "left: 0; top: 0; width: 40px; height: 40px; background-image: {CS}; \
             background-color: {CB}; background-blend-mode: hue"
        ),
    );
    let p = paint(&mut doc);
    expect(
        &p,
        &[
            ("opacity 0.5 over white", (60, 70), [234, 224, 244]),
            ("scale(2)", (100, 80), [0, 0, 28]),
            ("rounded", (150, 80), [207, 105, 54]),
            ("rounded corner", (132, 62), [255, 255, 255]),
            ("clipped", (190, 70), [59, 146, 212]),
            ("outside the clip", (210, 70), [255, 255, 255]),
        ],
        TOL,
    );
}
