//! #2371 and #2372 on the software painter (tiny-skia), against Chrome 156.
//!
//! #2371: a `1px dotted` border drew an almost solid dark line (levels 24 to
//! 31), where Chrome alternates whole dots and gaps. Dotted was a `[w, w]`
//! dash with round caps, and a round cap reaches half a width past each end
//! of a dash, so the dots touched; at 1px the stroke was also a tiny-skia
//! hairline.
//!
//! #2372: both painters dashed from a fixed phase at the path's start, so a
//! corner could fall in a gap and the pattern did not match Chrome's.
//! Chrome fits each side's dashes so that a whole dash or dot starts and
//! ends it.
//!
//! The scene and Chrome's pixels are in `support/border_dashes_2371.rs`,
//! shared with `gpu_border_dashes_2371` (in `rinch`).

#![cfg(feature = "software-renderer")]

#[path = "support/border_dashes_2371.rs"]
mod scene;

use rinch_dom::paint::skia_painter::TinySkiaPainter;
use scene::{H, W};

fn render(scale: u32) -> (u32, Vec<[u8; 4]>) {
    let doc = &mut scene::document();
    let (pw, ph) = (W * scale, H * scale);
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    (pw, painter.pixels().as_chunks::<4>().0.to_vec())
}

fn check(scale: u32) {
    let (pw, px) = render(scale);
    let bad = scene::misses(&px, pw, scale);
    assert!(
        bad.is_empty(),
        "{} boxes differ from Chrome:\n{}",
        bad.len(),
        bad.join("\n")
    );
    let bad = scene::corner_misses(&px, pw, scale);
    assert!(bad.is_empty(), "per-side corners:\n{}", bad.join("\n"));
}

#[test]
fn dashed_and_dotted_borders_match_chrome_at_scale_1() {
    check(1);
}

#[test]
fn dashed_and_dotted_borders_match_chrome_at_scale_2() {
    check(2);
}

/// The issue's own box: a 1px dotted border on a 40x20 box at scale 1.
/// Chrome: the top row is `##.#.#…#.#`, a doubled first dot (the length is
/// even) and then one-pixel dots and gaps, every one 0 or 255.
#[test]
fn the_issue_box_alternates_whole_dots_and_gaps() {
    let (pw, px) = render(1);
    let i = scene::BOXES
        .iter()
        .position(|b| b.2 == 40 && b.4 == "border:1px dotted #000")
        .unwrap();
    let (top, _) = scene::sample(&px, pw, 1, i);
    let mut want = vec![0u8, 0, 255];
    while want.len() < 40 {
        want.extend([0, 255]);
    }
    want.truncate(40);
    let off: Vec<_> = (0..40).filter(|&k| top[k].abs_diff(want[k]) > 8).collect();
    assert!(off.is_empty(), "levels {top:?}, off at {off:?}");
}
