//! #2973: the outline of a flowed inline element (`<a>`, `<span>`), the
//! keyboard focus ring among them, is drawn round its line fragments.
//! Before, nothing drew it: the box paint path never reaches an inline
//! element (it owns no box), and the inline path drew only backgrounds and
//! borders.
//!
//! The expected pixels are **Chrome 158's**, from a headless screenshot at
//! 1x of the same page (`MARKUP`, the text in this crate's Inter, loaded by
//! Chrome through `@font-face` from the same file, and transparent so the
//! outline is the only ink). What Chrome draws:
//!
//! - a fragment whose outline meets no other's is outlined on its own:
//!   two lines 40px apart give two rings (`A`, and `B` with
//!   `outline-offset: 3px`; `D`, one line, `offset: -1px`, 3px wide);
//! - fragments whose outlines meet are outlined as **one** shape: lines
//!   overlapping (`C`, `line-height: 18px`), lines 2px apart whose 2px
//!   bands close the gap (`E`, no band between them), and a fragment
//!   starting mid-line over a full one (`F`, a stepped ring).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 300;
const H: u32 = 300;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const CSS: &str = "html, body { margin: 0; background: #fff; }
     div { position: absolute; left: 10px; width: 100px;
           font: 16px/40px ProbeFace; color: transparent; }";

const MARKUP: &str = r#"
<div style="top:0"><span style="outline:2px solid #000">MMMMM MMMMM</span></div>
<div style="top:100px"><span style="outline:2px solid #000;outline-offset:3px">MMMMM MMMMM</span></div>
<div style="top:200px;line-height:18px"><span style="outline:2px solid #000">MMMMM MMMMM</span></div>
<div style="top:260px;left:140px;width:200px"><span style="outline:3px solid #000;outline-offset:-1px">MM</span></div>
<div style="top:0;left:150px;line-height:22px"><span style="outline:2px solid #000">MMMMM MMMMM</span></div>
<div style="top:100px;left:150px;line-height:18px">MM <span style="outline:2px solid #000">MM MMMMM</span></div>
"#;

fn document(markup: &str) -> RinchDocument {
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
    doc.load_css(CSS);
    let b = doc.body();
    doc.set_inner_html(b, markup);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn paint(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(W, H);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

fn lum(p: [u8; 4]) -> u32 {
    (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000
}

/// The dark runs of row `y`, as `(first x, last x)`.
fn runs(px: &[[u8; 4]], y: u32) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for x in 0..W {
        if lum(px[(y * W + x) as usize]) < 100 {
            match out.last_mut() {
                Some(r) if r.1 + 1 == x => r.1 = x,
                _ => out.push((x, x)),
            }
        }
    }
    out
}

/// The page as Chrome's table lists it: each row whose runs differ from the
/// row above, with its runs.
fn changes(px: &[[u8; 4]]) -> Vec<(u32, Vec<(u32, u32)>)> {
    let mut out: Vec<(u32, Vec<(u32, u32)>)> = Vec::new();
    for y in 0..H {
        let r = runs(px, y);
        if out.last().is_none_or(|(_, last)| *last != r) {
            out.push((y, r));
        }
    }
    out
}

/// Chrome 158's dark runs (luminance under 100), listed where they change.
#[rustfmt::skip]
const CHROME: &[(u32, &[(u32, u32)])] = &[
    (0, &[(148, 223)]),
    (1, &[(148, 149), (222, 223)]),
    (8, &[(8, 83), (148, 149), (222, 223)]),
    (10, &[(8, 9), (82, 83), (148, 149), (222, 223)]),
    (30, &[(8, 83), (148, 149), (222, 223)]),
    (32, &[(148, 149), (222, 223)]),
    (43, &[(148, 223)]),
    (45, &[]),
    (48, &[(8, 83)]),
    (50, &[(8, 9), (82, 83)]),
    (70, &[(8, 83)]),
    (72, &[]),
    (97, &[(181, 213)]),
    (99, &[(181, 182), (212, 213)]),
    (105, &[(5, 86), (181, 182), (212, 213)]),
    (107, &[(5, 6), (85, 86), (181, 182), (212, 213)]),
    (115, &[(5, 6), (85, 86), (148, 182), (212, 223)]),
    (117, &[(5, 6), (85, 86), (148, 149), (222, 223)]),
    (133, &[(5, 86), (148, 149), (222, 223)]),
    (135, &[(148, 149), (222, 223)]),
    (137, &[(148, 223)]),
    (139, &[]),
    (145, &[(5, 86)]),
    (147, &[(5, 6), (85, 86)]),
    (173, &[(5, 86)]),
    (175, &[]),
    (197, &[(8, 83)]),
    (199, &[(8, 9), (82, 83)]),
    (237, &[(8, 83)]),
    (239, &[]),
    (268, &[(138, 170)]),
    (271, &[(138, 140), (168, 170)]),
    (289, &[(138, 170)]),
    (292, &[]),
];

/// **Mutant: every fragment outlined on its own** (no grouping) — `C`, `E`
/// and `F` draw a band between their lines. **Mutant: a group's ring
/// without the shrink** (the outer union less the fragments grown by the
/// offset only) — `E`'s 2px gap is filled.
#[test]
fn inline_outlines_are_drawn_round_their_fragments_as_chrome_draws_them() {
    let px = paint(&mut document(MARKUP));
    let rinch = changes(&px);
    let chrome: Vec<(u32, Vec<(u32, u32)>)> =
        CHROME.iter().map(|&(y, r)| (y, r.to_vec())).collect();
    if rinch != chrome {
        let show = |v: &[(u32, Vec<(u32, u32)>)]| {
            v.iter()
                .map(|(y, r)| format!("{y} {r:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        panic!("rinch:\n{}\n\nChrome 158:\n{}", show(&rinch), show(&chrome));
    }
}

// ---------------------------------------------------------------------------
// Partial frames: focus moving onto and off a link.
//
// A link's focus ring is a restyle (`:focus-visible`), which re-shapes
// nothing; the link has no box, so its damage is its IFC root's rect grown by
// the root's ink, now and as it was painted. The ring reaches past the root by
// its offset and width (here 8px, twice the damage margin), so the root's
// ink must count it (`text::inline_ink_past_lines`) or the frame that draws
// the ring, and the one that takes it away, leave pixels of it behind.
//
// Each case paints a frame, moves focus, repaints only the damage named,
// and compares that with a full paint of the new state on a fresh document.

mod partial {
    use super::DomDocument;
    use peniko::Brush;
    use peniko::kurbo::Rect;
    use rinch_dom::RinchDocument;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    const W: f32 = 300.0;
    const H: f32 = 200.0;

    const RING: &str =
        "a:focus-visible { outline: 2px solid rgb(0, 0, 255); outline-offset: 6px; }";

    /// A shrink-wrapped label with a link, well inside the page.
    const ONE_LINE: &str = r#"<div style="position: absolute; left: 60px; top: 50px; width: max-content; font: 20px/30px sans-serif">go <a id="l" href="/x">link</a></div>"#;
    /// The same with a link that wraps onto three lines.
    const WRAPPED: &str = r#"<div style="position: absolute; left: 60px; top: 50px; width: 90px; font: 20px/30px sans-serif">go <a id="l" href="/x">a link that wraps</a></div>"#;

    fn document(page: &str, focused: bool) -> RinchDocument {
        let mut doc = RinchDocument::new();
        doc.load_css(RING);
        let body = doc.body();
        doc.set_inner_html(body, page);
        doc.resolve_layout(W, H);
        if focused {
            focus(&mut doc, true);
            doc.resolve_layout(W, H);
        }
        doc
    }

    fn focus(doc: &mut RinchDocument, on: bool) {
        let l = doc.query_selector_all("#l")[0].0;
        doc.update_focus(on.then_some(l));
        doc.set_focus_visible(l, on);
    }

    fn paint(doc: &mut RinchDocument, p: &mut TinySkiaPainter, scale: f64) {
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(&doc.tree, p, scale, (W, H), &mut doc.font_cx, &mut lcx);
    }

    #[track_caller]
    fn partial_matches_full(page: &str, from: bool, scale: f64) {
        let mut doc = document(page, from);
        let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
        let mut p = TinySkiaPainter::new(pw, ph);
        p.fill_white();
        paint(&mut doc, &mut p, scale);
        let before = p.pixels().to_vec();
        doc.tree.consume_paint_dirty();
        focus(&mut doc, !from);
        doc.resolve_layout(W, H);
        let damage = rinch_dom::paint::compute_damage(&doc.tree, scale, pw as f64, ph as f64);
        assert!(!damage.is_full(), "the change damaged the whole frame");
        let rects: Vec<Rect> = damage.rects().to_vec();
        assert!(!rects.is_empty(), "the change named no damage");
        for r in &rects {
            let r = r
                .intersect(Rect::new(0.0, 0.0, pw as f64, ph as f64))
                .expand();
            p.clear_rect_white(
                r.x0 as u32,
                r.y0 as u32,
                r.width() as u32,
                r.height() as u32,
            );
        }
        p.push_damage_clip(&damage);
        rinch_dom::paint::set_dirty_rects(Some(&rects));
        paint(&mut doc, &mut p, scale);
        rinch_dom::paint::set_dirty_region(None);
        let partial = p.pixels().to_vec();
        let mut fresh = document(page, !from);
        let mut full = TinySkiaPainter::new(pw, ph);
        full.fill_white();
        paint(&mut fresh, &mut full, scale);
        assert_ne!(full.pixels(), &before[..], "the change changed nothing");
        let bad: Vec<(usize, usize)> = partial
            .chunks(4)
            .zip(full.pixels().chunks(4))
            .enumerate()
            .filter(|(_, (a, b))| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
            .map(|(i, _)| (i % pw as usize, i / pw as usize))
            .collect();
        assert!(
            bad.is_empty(),
            "{} pixels of the partial repaint are not the new state, first {:?} (damage {rects:?})",
            bad.len(),
            &bad[..bad.len().min(5)]
        );
    }

    /// **Mutant: the root's ink without the outlines** — the ring's corners,
    /// 8px past the label, are outside the damage.
    #[test]
    fn focusing_a_link_damages_its_ring() {
        for page in [ONE_LINE, WRAPPED] {
            for scale in [1.0, 2.0] {
                partial_matches_full(page, false, scale);
            }
        }
    }

    #[test]
    fn unfocusing_a_link_damages_where_its_ring_was() {
        for page in [ONE_LINE, WRAPPED] {
            for scale in [1.0, 2.0] {
                partial_matches_full(page, true, scale);
            }
        }
    }
}
