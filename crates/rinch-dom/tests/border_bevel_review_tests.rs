//! #731's review fixtures: the odd-width groove split, the 1px groove collapse,
//! and the unmitred corner (#1348), each measured against Chrome 153.
use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 400.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let id = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(id, "style", style);
    }
    doc.append_child(parent, id);
    id
}

fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

fn pixel_at(px: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let i = ((y * VW as u32 + x) * 4) as usize;
    (px[i], px[i + 1], px[i + 2], px[i + 3])
}

/// Finding M3: no fixture in border_bevel_tests.rs uses an odd width for
/// groove/ridge (the one existing groove/ridge test uses 40px, even), so the
/// `outer_w = (width * 0.5).ceil()` rounding rule (Chromium's `(y1+y2+1)/2`)
/// is unpinned. At 3px this gives outer=2 (dark), inner=1 (light) — verified
/// against real Chrome 153 (`chromium.googlesource.com` / headless capture)
/// giving the identical 2-dark/1-light split. A `.ceil() -> .floor()` mutant
/// survives the whole shipped suite; this fixture kills it.
#[test]
fn odd_width_groove_gives_the_outer_half_the_extra_pixel() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 3px groove gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);
    // Top band is 3px: rows 0,1 dark (outer), row 2 light (inner).
    assert_eq!(pixel_at(&px, 100, 0), (44, 44, 44, 255));
    assert_eq!(pixel_at(&px, 100, 1), (44, 44, 44, 255));
    assert_eq!(pixel_at(&px, 100, 2), (212, 212, 212, 255));
}

/// Finding M7: no fixture exercises the `width <= 1.0` groove/ridge
/// "EffectiveStyle" collapse to a plain undarkened solid. Verified against
/// real Chrome 153: a 1px `groove` border paints flat `rgb(128,128,128)`,
/// not a (necessarily-degenerate) two-tone split. Removing the whole
/// collapse branch in `paint_border_side` passes every test the PR ships;
/// this fixture kills that mutant.
#[test]
fn one_pixel_groove_paints_solid_undarkened() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 1px groove gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);
    assert_eq!(pixel_at(&px, 100, 0), (128, 128, 128, 255));
}

/// Corner-mitre gap, issue #1348 (not a mutant — a geometry limitation the PR doesn't
/// mention or test). Real Chrome 153 mitres the top-right corner of an
/// `inset` border diagonally: the top band's dark wedge and the right
/// band's light wedge meet along the corner square's own diagonal (measured
/// with headless Chrome — see the review report). rinch's per-side
/// straight-line painter draws each side as a full-length open line and
/// paints them in top/right/bottom/left order, so at every corner the
/// later-painted side's colour simply overwrites the earlier one across the
/// WHOLE corner square — no diagonal split at all. This fixture pins
/// rinch's *current* (non-mitred) behaviour so a future attempt at mitring
/// is a deliberate, visible change rather than an accidental one.
#[test]
fn current_behaviour_top_right_corner_is_not_mitred_like_chrome() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 20px inset gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);
    // TR corner square x:[180,200) y:[0,20). Chrome: upper-left triangle
    // dark (top's wedge), lower-right triangle light (right's wedge).
    // rinch: the WHOLE square is light (right, painted last, wins outright).
    assert_eq!(
        pixel_at(&px, 181, 1),
        (212, 212, 212, 255),
        "rinch's TR corner near the top edge is already 'right' colour, not \
         a mitred 'top' wedge as Chrome draws there"
    );
    assert_eq!(pixel_at(&px, 199, 19), (212, 212, 212, 255));
}
