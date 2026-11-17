//! Thin SVG strokes on the software painter against Chrome 156 (review of
//! #2344). A stroke at most one device pixel wide is a *hairline* to tiny-skia
//! and to Chrome's Skia alike, whose coverage is not the stroke's area, and
//! the two agree to a level or two. #2344's first fix filled every stroke's
//! outline instead: a 1px circle's edge went from Chrome's 152/103 to 128/127,
//! a 0.1px line vanished (255 where Chrome draws 251/236, below tiny-skia's
//! quarter-pixel vertical supersampling), and a 0.5px line at 1.5x moved by
//! up to 36 levels. These pin the hairline. Every expectation is a pixel
//! Chrome 156 drew (`--headless=new --force-device-scale-factor=N`).
//!
//! The SVGs' boxes sit on whole device pixels at both scales (Chrome snaps a
//! box at a fractional one and rinch does not; #2321), so only the stroke is
//! compared.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// A 1px circle, and a 0.1px line at a fractional y, at 1x.
const PAGE_1X: &str = r##"<svg style="left:11px; top:7px" width="40" height="40" viewBox="0 0 40 40"><circle cx="20" cy="20" r="15" fill="none" stroke="#000" stroke-width="1"/></svg><svg style="left:61px; top:7px" width="40" height="20" viewBox="0 0 40 20"><line x1="3" y1="9.3" x2="37" y2="9.3" stroke="#000" stroke-width="0.1"/></svg>"##;

/// Chrome 156's pixels for [`PAGE_1X`] at 1x: `((x, y), red)`, on black over
/// white.
const CHROME_1X: &[((u32, u32), u8)] = &[
    // The circle's top, right and upper-right edges.
    ((31, 10), 255),
    ((31, 11), 152),
    ((31, 12), 103),
    ((31, 13), 255),
    ((45, 27), 103),
    ((46, 27), 152),
    ((47, 27), 255),
    ((41, 16), 64),
    ((42, 16), 229),
    ((42, 17), 38),
    // The 0.1px line, y = 16.3: faint in rows 15 and 16.
    ((80, 14), 255),
    ((80, 15), 251),
    ((80, 16), 236),
    ((80, 17), 255),
];

/// 0.5px lines (0.75 device px at 1.5x) at y = 3 + 3k + k/10, k = 0..10, so
/// each sits at a different fraction of a device pixel.
fn page_lines() -> String {
    let mut s = String::from(
        r#"<svg style="left:6px; top:6px" width="40" height="40" viewBox="0 0 40 40">"#,
    );
    for k in 0..10 {
        let y = 3.0 + 3.0 * k as f64 + k as f64 / 10.0;
        s += &format!(
            r##"<line x1="3" y1="{y}" x2="37" y2="{y}" stroke="#000" stroke-width="0.5"/>"##
        );
    }
    s + "</svg>"
}

/// Chrome 156's pixels for [`page_lines`] at 1.5x, down device column 40:
/// `(y, red)`, the rows either side of three of the lines.
const CHROME_LINES_15X: &[(u32, u8)] = &[
    (25, 255),
    (26, 244),
    (27, 76),
    (28, 255),
    (40, 235),
    (41, 85),
    (49, 255),
    (50, 100),
    (51, 219),
    (52, 255),
];

fn render(html: &str, w: u32, h: u32, scale: f64) -> (Vec<[u8; 4]>, u32) {
    let mut doc = rinch_dom::RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } svg { position: absolute; display: block; }",
    );
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(w as f32, h as f32);
    let (pw, ph) = (
        (w as f64 * scale).round() as u32,
        (h as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    (painter.pixels().as_chunks::<4>().0.to_vec(), pw)
}

#[test]
fn a_1px_circle_and_a_tenth_pixel_line_are_hairlines_as_chrome_draws_them() {
    let (px, pw) = render(PAGE_1X, 120, 60, 1.0);
    let bad: Vec<_> = CHROME_1X
        .iter()
        .map(|&((x, y), want)| ((x, y), px[(y * pw + x) as usize][0], want))
        .filter(|&(_, got, want)| got.abs_diff(want) > 3)
        .collect();
    assert!(bad.is_empty(), "(point, tiny-skia, chrome): {bad:?}");
}

#[test]
fn half_pixel_lines_at_1_5x_are_hairlines_as_chrome_draws_them() {
    let (px, pw) = render(&page_lines(), 60, 60, 1.5);
    let bad: Vec<_> = CHROME_LINES_15X
        .iter()
        .map(|&(y, want)| (y, px[(y * pw + 40) as usize][0], want))
        .filter(|&(_, got, want)| got.abs_diff(want) > 3)
        .collect();
    assert!(bad.is_empty(), "(row, tiny-skia, chrome): {bad:?}");
}
