//! #2719 (item 3) — `conic-gradient()` and `repeating-conic-gradient()` on
//! the software painter, against Chrome 157.
//!
//! The scene and Chrome's pixels are shared with the GPU twin
//! (`crates/rinch/tests/gpu_conic_gradient_2719.rs`) through
//! `tests/support/conic_gradient_2719.rs`: `from <angle>`, `at <position>`,
//! stops at angles and percentages, hard stops, repeating, transparency,
//! layers, `background-size`/`-position`/`-repeat`, a border, `border-radius`,
//! a pie and a gauge. Before #2719 a conic layer was dropped at computed
//! style, so every box was blank (or its colour).
//!
//! tiny-skia evaluates the sweep as Chrome's Skia does (the same atan
//! polynomial) and is within one level of Chrome at every sample (measured:
//! 98 of 505 samples are one level off, none more); [`TOL`] is 2.
//!
//! Regenerating Chrome's samples: run `write_chrome_page` with
//! `CONIC_2719_PAGE=<dir>/page.html`, screenshot it with
//! `google-chrome --headless --force-device-scale-factor=1 --hide-scrollbars
//! --window-size=520,430 --screenshot=<dir>/chrome.png file://<dir>/page.html`,
//! and sample it as the support module's docs say.
#![cfg(feature = "software-renderer")]

#[path = "support/conic_gradient_2719.rs"]
mod scene;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::computed_style::{BackgroundValue, LengthPercentageValue};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use scene::{H, W};

/// Per channel, against Chrome (see the module docs).
const TOL: u8 = 2;

fn render() -> Vec<[u8; 4]> {
    let doc = &mut scene::document().0;
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
    painter.pixels().as_chunks::<4>().0.to_vec()
}

#[test]
fn conic_gradients_match_chrome() {
    let per_box = scene::samples_per_box();
    for (b, n) in per_box.iter().enumerate() {
        assert!(*n >= 6, "box {b} has only {n} of Chrome's samples");
    }
    let bad = scene::misses(&render(), TOL);
    assert!(
        bad.is_empty(),
        "{} of {} samples differ from Chrome:\n{}",
        bad.len(),
        scene::CHROME.len(),
        bad.join("\n")
    );
}

/// The gradient reaches the computed style: `from`, `at`, each stop's angle
/// or percentage as a share of a turn, and `repeating`; a conic layer below
/// the first is kept.
#[test]
fn conic_gradient_is_computed() {
    let (doc, ids) = scene::document();
    let style_of = |i: usize| &doc.tree.nodes[ids[i].0].computed_style;
    match &style_of(3).background {
        BackgroundValue::ConicGradient {
            from_degrees,
            stops,
            position_x,
            position_y,
            repeating,
            ..
        } => {
            assert!(
                (from_degrees - 54.0).abs() < 1e-3,
                "0.15turn: {from_degrees}"
            );
            assert!(matches!(position_x, LengthPercentageValue::Length(px) if *px == 23.0));
            assert!(matches!(position_y, LengthPercentageValue::Length(px) if *px == 41.0));
            assert!(!repeating);
            let turns: Vec<Option<f32>> = stops
                .iter()
                .map(|s| match s.position {
                    Some(LengthPercentageValue::Percent(p)) => Some(p),
                    None => None,
                    Some(other) => panic!("a conic stop is a share of a turn: {other:?}"),
                })
                .collect();
            let want = [
                0.0,
                47.0 / 360.0,
                47.0 / 360.0,
                133.0 / 360.0,
                133.0 / 360.0,
            ];
            assert_eq!(turns.len(), want.len());
            for (g, w) in turns.iter().zip(want) {
                assert!((g.unwrap() - w).abs() < 1e-6, "{turns:?}");
            }
        }
        other => panic!("box 3 is a conic gradient: {other:?}"),
    }
    match &style_of(4).background {
        BackgroundValue::ConicGradient { stops, .. } => {
            assert!(
                matches!(stops[1].position, Some(LengthPercentageValue::Percent(p)) if (p - 0.125).abs() < 1e-6)
            );
        }
        other => panic!("box 4 is a conic gradient: {other:?}"),
    }
    assert!(matches!(
        style_of(5).background,
        BackgroundValue::ConicGradient {
            repeating: true,
            ..
        }
    ));
    let layers = &style_of(8).background_layers;
    assert_eq!(layers.len(), 1, "the second conic layer is kept");
    assert!(matches!(
        layers[0].image,
        BackgroundValue::ConicGradient { .. }
    ));
}

/// The page Chrome is captured from: the boxes on white, each with its
/// border box in `data-box`.
fn page_html() -> String {
    let mut s = String::from(
        "<!doctype html><html><head><style>html, body { margin: 0; background: #fff; } \
         div { position: absolute; box-sizing: border-box; }</style></head><body>\n",
    );
    for &(l, t, w, h, css) in scene::BOXES {
        s.push_str(&format!(
            "<div data-box=\"{l},{t},{w},{h}\" style=\"left: {l}px; top: {t}px; width: {w}px; \
             height: {h}px; {css}\"></div>\n"
        ));
    }
    s.push_str("</body></html>\n");
    s
}

/// Writes the Chrome oracle page to `$CONIC_2719_PAGE`, and the seam scene's to
/// `$CONIC_2719_SEAM_PAGE` when set (see the module docs).
#[test]
#[ignore = "writes the Chrome oracle page"]
fn write_chrome_page() {
    let path = std::env::var("CONIC_2719_PAGE").expect("CONIC_2719_PAGE");
    std::fs::write(path, page_html()).unwrap();
    if let Ok(path) = std::env::var("CONIC_2719_SEAM_PAGE") {
        std::fs::write(path, seam_page_html()).unwrap();
    }
}

/// The seam scene (review of #2719): a `repeating-conic-gradient` with a 1°
/// period whose first stop is at 0°, in a 1000px box centred on its
/// bottom-right corner, so the box shows the quarter turn up to the seam
/// (270° to 360°) out to a radius of 1000px.
const SEAM_CSS: &str = "background: repeating-conic-gradient(at 100% 100%, #000 0 0.5deg, \
                        #fff 0.5deg 1deg)";
const SEAM: u32 = 1000;

/// The seam scene's samples: the middle of each half-degree band of the last
/// ten degrees before the seam (350.25°, 350.75°, … 359.75°), at a radius of
/// 900px, floored to a device pixel.
fn seam_points() -> Vec<(u32, u32)> {
    (0..20)
        .map(|j| {
            let a = (350.25 + 0.5 * j as f64).to_radians();
            let c = SEAM as f64;
            ((c + 900.0 * a.sin()) as u32, (c - 900.0 * a.cos()) as u32)
        })
        .collect()
}

/// Chrome 157's pixel at each of [`seam_points`]: black and white bands,
/// each period starting black (captured as the main scene, from the page
/// `seam_page_html` writes, 1000x1000).
const SEAM_CHROME: [[u8; 3]; 20] = [
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 255],
];

/// A repeating conic gradient keeps its period exactly to the seam. The
/// software painter's sweep has to be moved off tiny-skia's identity check
/// (`skia_painter::sweep_span_degrees`); moving only its start made every
/// period 0.001° long, which after 350 periods put each band 0.35° (5.5px at
/// this radius) late, and most of these samples on the wrong band.
#[test]
fn a_repeating_conic_gradient_keeps_its_period_to_the_seam() {
    let mut doc = rinch_dom::RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    let n = doc.create_element("div");
    doc.set_attribute(
        n,
        "style",
        &format!(
            "position: absolute; left: 0; top: 0; width: {SEAM}px; height: {SEAM}px; {SEAM_CSS}"
        ),
    );
    doc.append_child(body, n);
    doc.resolve_layout(SEAM as f32, SEAM as f32);
    let mut painter = TinySkiaPainter::new(SEAM, SEAM);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (SEAM as f32, SEAM as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = painter.pixels().as_chunks::<4>().0;
    let bad: Vec<_> = seam_points()
        .into_iter()
        .zip(SEAM_CHROME)
        .filter(|&((x, y), want)| {
            let got = px[(y * SEAM + x) as usize];
            got[..3].iter().zip(want).any(|(g, w)| g.abs_diff(w) > TOL)
        })
        .collect();
    assert!(bad.is_empty(), "samples off Chrome's band: {bad:?}");
}

/// The seam scene's Chrome page (see [`page_html`]).
fn seam_page_html() -> String {
    format!(
        "<!doctype html><html><head><style>html, body {{ margin: 0; background: #fff; }}\
         </style></head><body><div style=\"position: absolute; left: 0; top: 0; \
         width: {SEAM}px; height: {SEAM}px; {SEAM_CSS}\"></div></body></html>\n"
    )
}
