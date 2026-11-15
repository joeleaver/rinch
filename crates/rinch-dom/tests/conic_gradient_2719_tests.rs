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

/// Writes the Chrome oracle page to `$CONIC_2719_PAGE` (see the module docs).
#[test]
#[ignore = "writes the Chrome oracle page"]
fn write_chrome_page() {
    let path = std::env::var("CONIC_2719_PAGE").expect("CONIC_2719_PAGE");
    std::fs::write(path, page_html()).unwrap();
}
