//! Paint and layout must agree about where a box is.
//!
//! Three ACOS reports — #222, #224, #225 — all described the same symptom from
//! embed mode: a box is *painted* in one place and *hit-tested* in another, so
//! a click on what the user sees fires the wrong handler or nothing at all.
//! Each report proposed a different cause (a `right:` inset resolved against a
//! box one padding too wide, a row measured taller by layout than by paint, a
//! flex `gap` re-applied at paint time), and none of those causes exists: paint
//! hands a block child its parent's **border-box** origin and adds the child's
//! own `layout.{x,y}`, full stop. It reads no inset, no `gap`, no flex property.
//!
//! Nothing in the tree asserted that, which is why three reports could describe
//! it as broken. This file pins the invariant for the shapes they used:
//!
//! > For a child paint reaches through `paint_children_with_stacking` —
//! > out-of-flow or in-flow, flex item or block — the box it is **drawn** in,
//! > minus the box its parent is drawn in, is exactly the child's own
//! > parent-relative `layout.{x,y}`.
//!
//! The oracle is deliberately the **rasterised frame**, not another geometry
//! helper: each test paints through `TinySkiaPainter` and reads the bounding
//! box of a uniquely coloured fill out of the pixmap. A helper-versus-helper
//! assertion would agree with itself even if both walks were wrong together,
//! which is the failure mode these reports allege. `painted_border_box` is
//! checked against those pixels as well, since it is what hit testing,
//! `ClickContext` and the MCP `absolute` field all read (#203).
//!
//! Every test also states the *wrong* answer explicitly — the box the reported
//! defect would have produced — because the right and wrong boxes overlap, so a
//! positive assertion alone survives most of these regressions.
//!
//! The four were checked against two mutations, because paint reaches a box by
//! two different routes and one test cannot cover both. Handing a block child
//! the parent's *content*-box origin in `paint_children_with_stacking` breaks
//! only the flex test: an out-of-flow box is not painted from that recursion at
//! all. Adding the same padding+border in `stacking::descend` — the ancestor
//! walk that positions a box painted from its stacking root — breaks the other
//! three and leaves the flex one green, reproducing #224's reported box to the
//! pixel. Keep a test on each side of that split.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painted_border_box;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

/// Rasterise the document with the software painter — the same
/// `paint_document` entry point the desktop shell's `build_pixels` uses.
fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
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

/// The bounding box of every pixel painted in exactly `rgb`, as
/// `(x0, y0, x1, y1)` with `x1`/`y1` exclusive. Opaque fills of distinct solid
/// colours are used throughout, so an exact match needs no tolerance and an
/// anti-aliased edge simply is not counted — which makes the box the painter
/// covered *at least* this, never more.
fn color_bbox(px: &[u8], rgb: (u8, u8, u8)) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (VW as u32, VH as u32);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (px[i], px[i + 1], px[i + 2]) == rgb {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

fn layout_xy(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let l = doc.tree.get(node.0).unwrap().layout;
    (l.x, l.y)
}

/// #224 — a right-anchored absolute inside a padded, bordered positioned panel.
///
/// Reported as: "the paint resolves `right:14px` against a box ~one-padding
/// wider than the one Taffy used", painting the button ≈(+24, −9) off its hit
/// rect. Neither half of that is what happens.
///
/// CSS resolves an absolute box against its containing block's **padding box**,
/// and Taffy 0.12 already does: `compute/block.rs` subtracts `resolved_border`
/// from the containing block and nothing else. Paint then descends into a block
/// child at the parent's *border-box* origin and adds the child's own
/// `layout.{x,y}`, into which Taffy has already baked border + inset — so the
/// padding cannot be counted twice, or at all.
#[test]
fn a_right_anchored_absolute_in_a_padded_panel_paints_where_it_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = child_of(
        &mut doc,
        body,
        "position: absolute; right: 16px; top: 58px; bottom: 16px; width: 724px; \
         padding: 16px 18px; border: 1px solid rgb(0, 0, 255); \
         background-color: rgb(0, 255, 0)",
    );
    let btn = child_of(
        &mut doc,
        panel,
        "position: absolute; top: 12px; right: 14px; width: 61px; height: 25px; \
         background-color: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);

    // rinch sets a global `box-sizing: border-box`, so `width: 724px` IS the
    // panel's border box; `right: 16px` puts its left edge at 800-16-724.
    let panel_layout = doc.tree.get(panel.0).unwrap().layout;
    assert_eq!(
        (panel_layout.x, panel_layout.y, panel_layout.width),
        (60.0, 58.0, 724.0),
        "sanity: the panel is the box the rest of the arithmetic is built on"
    );

    // Taffy's answer for the button, parent-border-box-relative. The containing
    // block is the panel's PADDING box: 724 wide minus the 1px border each
    // side = 722, offset 1 from the border-box origin. So
    // x = 1 + (722 - 14 - 61) = 648, y = 1 + 12 = 13.
    assert_eq!(
        layout_xy(&doc, btn),
        (648.0, 13.0),
        "the containing block is the panel's padding box"
    );
    assert_ne!(
        layout_xy(&doc, btn),
        (630.0, 29.0),
        "…not its CONTENT box, which would inset the button by the padding too"
    );

    // The invariant, read off the rasterised frame. Panel border box starts at
    // (60, 58); the button must be drawn at (60+648, 58+13) = (708, 71).
    let px = rasterize(&mut doc);
    assert_eq!(
        color_bbox(&px, (255, 0, 0)),
        Some((708, 71, 769, 96)),
        "the button is painted exactly one `layout.{{x,y}}` off the panel's origin"
    );
    assert_ne!(
        color_bbox(&px, (255, 0, 0)),
        Some((727, 88, 788, 113)),
        "#224 as reported: paint must not add the panel's padding+border on top \
         of the inset Taffy already resolved (that is the #319 signature)"
    );
    // The panel really does have the padding and border the button is supposed
    // to ignore: its background stops one border pixel inside its border box.
    assert_eq!(
        color_bbox(&px, (0, 255, 0)),
        Some((61, 59, 783, 583)),
        "sanity: the 1px border and the 724x526 border box are real"
    );

    // And the box every geometry consumer reads — hit testing, ClickContext,
    // the MCP `absolute` field (#203) — is that same painted box.
    let pr = painted_border_box(&doc.tree, panel.0, 1.0);
    let br = painted_border_box(&doc.tree, btn.0, 1.0);
    assert_eq!((br.x0, br.y0, br.x1, br.y1), (708.0, 71.0, 769.0, 96.0));
    assert_eq!(
        ((br.x0 - pr.x0) as f32, (br.y0 - pr.y0) as f32),
        layout_xy(&doc, btn),
        "painted delta == the child's own parent-relative layout"
    );
}

/// #225 (a) — a row whose only child is out of flow.
///
/// Reported as: the row is laid out ~36px tall (its declared 20 plus one child
/// row) but painted at the declared 20, so every following row's hit rect lands
/// one 16px pitch below its paint.
///
/// An absolute child maps to `taffy::Position::Absolute` and contributes
/// nothing to its parent's in-flow size, so the declared height stands; and
/// paint reads each following sibling's own `layout.y`, so even a mis-measured
/// row would move paint and layout *together*. A split of exactly one row pitch
/// is not producible from this shape at all.
#[test]
fn an_absolute_only_row_does_not_inflate_its_parents_flow() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = child_of(
        &mut doc,
        body,
        "position: absolute; right: 16px; top: 58px; width: 724px; padding: 16px 18px",
    );
    // The offending row: a declared height, no inline content, one absolute child.
    let row0 = child_of(&mut doc, panel, "position: relative; height: 20px");
    let btn = child_of(
        &mut doc,
        row0,
        "position: absolute; top: 0; left: 620px; width: 66px; height: 16px; \
         background-color: rgb(255, 0, 0)",
    );
    // Four 16px content rows behind it, each a distinguishable colour.
    let rows: Vec<NodeId> = (0..4u8)
        .map(|i| {
            let style = format!(
                "position: relative; height: 16px; background-color: rgb(0, {}, 0)",
                40 + i * 40
            );
            child_of(&mut doc, panel, &style)
        })
        .collect();
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree.get(row0.0).unwrap().layout.height,
        20.0,
        "the declared height stands; the out-of-flow child adds nothing to it"
    );
    assert_eq!(
        doc.tree.get(panel.0).unwrap().layout.height,
        116.0,
        "16 padding + 20 + 4x16 + 16 padding — the row contributes 20, not 36"
    );

    // Layout: padding-top 16 + row0's 20 = 36, then a 16px pitch.
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(
            layout_xy(&doc, *r).1,
            36.0 + 16.0 * i as f32,
            "row {i} sits at the declared pitch"
        );
    }

    // Paint: the panel's border box starts at (60, 58), so the rows are drawn
    // at y = 94, 110, 126, 142 — one `layout.y` below it, each.
    let px = rasterize(&mut doc);
    for (i, _) in rows.iter().enumerate() {
        let y = 94 + 16 * i as u32;
        assert_eq!(
            color_bbox(&px, (0, 40 + 40 * i as u8, 0)),
            Some((78, y, 766, y + 16)),
            "painted row {i} sits exactly one layout.y below the panel origin"
        );
        assert_ne!(
            color_bbox(&px, (0, 40 + 40 * i as u8, 0)),
            Some((78, y + 16, 766, y + 32)),
            "#225 as reported: paint and layout must not differ by one row pitch"
        );
    }

    // The absolute child of the zero-content row is drawn at its own inset too:
    // (60 + 18 + 620, 58 + 16 + 0).
    assert_eq!(
        color_bbox(&px, (255, 0, 0)),
        Some((698, 74, 764, 90)),
        "the absolute-only row's button paints at its declared inset"
    );
    let rr = painted_border_box(&doc.tree, row0.0, 1.0);
    let br = painted_border_box(&doc.tree, btn.0, 1.0);
    assert_eq!(
        ((br.x0 - rr.x0) as f32, (br.y0 - rr.y0) as f32),
        layout_xy(&doc, btn)
    );
}

/// #225 (b) — the shape the report's workaround moved to: a row that mixes
/// inline text with an out-of-flow child, inside a padded panel.
///
/// This is the one that could plausibly have drifted. An out-of-flow child
/// counts as block content in `create_anonymous_block_boxes`, so such a row is
/// classified as mixed inline+block and mints an anonymous block box CSS would
/// never create — and that box clones the row's padding and border (#319). The
/// absolute child escapes it only because Stylo blockifies an out-of-flow box,
/// which takes it out of the IFC entirely. Assert that, so a Stylo change
/// removing the blockification fails here rather than silently displacing every
/// absolutely positioned control written beside a row's own label.
///
/// The row's height comes from text metrics, so the expected boxes are derived
/// from `layout` rather than hard-coded — which is the whole point: the test
/// fails exactly when paint stops agreeing with layout.
#[test]
fn a_row_mixing_inline_text_with_an_absolute_child_paints_where_it_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = child_of(
        &mut doc,
        body,
        "position: absolute; right: 16px; top: 58px; width: 724px; padding: 16px 18px",
    );
    let mut rows = Vec::new();
    for i in 0..3u8 {
        let row = child_of(
            &mut doc,
            panel,
            "position: relative; padding: 5px 7px; border: 3px solid rgb(0, 0, 255); \
             font-size: 16px",
        );
        let label = doc.create_text(&format!("row {i}"));
        doc.append_child(row, label);
        let style = format!(
            "position: absolute; left: 272px; top: 0; width: 30px; height: 14px; \
             background-color: rgb({}, 0, 0)",
            80 + i * 60
        );
        let btn = child_of(&mut doc, row, &style);
        rows.push((row, btn));
    }
    doc.resolve_layout(VW, VH);

    let px = rasterize(&mut doc);
    // The panel's border box, hand-computed: right 16, width 724, top 58.
    let (px0, py0) = (60.0_f32, 58.0_f32);
    for (i, (row, btn)) in rows.iter().enumerate() {
        // Taffy's answer for the button: the row's padding box, i.e. its
        // 3px border plus the declared `left`/`top`.
        assert_eq!(
            layout_xy(&doc, *btn),
            (275.0, 3.0),
            "row {i}: the button resolves against the row's padding box"
        );
        let (rx, ry) = layout_xy(&doc, *row);
        let (bx, by) = layout_xy(&doc, *btn);
        let (ex, ey) = (px0 + rx + bx, py0 + ry + by);
        assert_eq!(
            color_bbox(&px, (80 + i as u8 * 60, 0, 0)),
            Some((ex as u32, ey as u32, ex as u32 + 30, ey as u32 + 14)),
            "row {i}: painted where layout put it"
        );
        assert_ne!(
            color_bbox(&px, (80 + i as u8 * 60, 0, 0)),
            Some((
                ex as u32 + 10,
                ey as u32 + 8,
                ex as u32 + 40,
                ey as u32 + 22
            )),
            "row {i}: the anonymous block box's cloned padding+border (#319) must \
             not reach a child Stylo took out of the IFC"
        );
    }
}

/// #222-adjacent — an absolute flex row with `gap`.
///
/// Reported (on wasm only) as a paint-time gap/justify recompute widening the
/// row ≈9px per gap. Paint reads no flex property at all: `gap` reaches
/// `taffy::Style` and stops there, and a flex item's entire geometric
/// contribution to paint is its own `layout` rect. This pins that — cheaply,
/// on the desktop half of the code both platforms share.
#[test]
fn a_flex_row_with_gap_paints_its_items_where_they_are_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = child_of(
        &mut doc,
        body,
        "position: absolute; right: 16px; top: 40px; display: flex; gap: 10px; \
         padding: 4px 6px; border: 2px solid rgb(0, 0, 255)",
    );
    let items: Vec<NodeId> = (0..3u8)
        .map(|i| {
            let style = format!(
                "width: 40px; height: 20px; background-color: rgb({}, 0, 0)",
                80 + i * 60
            );
            child_of(&mut doc, row, &style)
        })
        .collect();
    doc.resolve_layout(VW, VH);

    // 3x40 + 2x10 gap + 2x6 padding + 2x2 border = 156, right-anchored at
    // 800 - 16 - 156 = 628.
    let rl = doc.tree.get(row.0).unwrap().layout;
    assert_eq!(
        (rl.x, rl.y, rl.width, rl.height),
        (628.0, 40.0, 156.0, 32.0)
    );

    let px = rasterize(&mut doc);
    for (i, it) in items.iter().enumerate() {
        // Content box starts at 6+2 = 8 across, 4+2 = 6 down; pitch 40+10.
        assert_eq!(
            layout_xy(&doc, *it),
            (8.0 + 50.0 * i as f32, 6.0),
            "item {i}: one gap between items, applied by Taffy"
        );
        let x = 636 + 50 * i as u32;
        assert_eq!(
            color_bbox(&px, (80 + i as u8 * 60, 0, 0)),
            Some((x, 46, x + 40, 66)),
            "item {i}: painted at the row's border-box origin plus its own layout"
        );
        if i > 0 {
            // A second application of the gap would push item `i` a further
            // 10px per preceding gap; item 0 has none, so it has no such box.
            let drift = 10 * i as u32;
            assert_ne!(
                color_bbox(&px, (80 + i as u8 * 60, 0, 0)),
                Some((x + drift, 46, x + drift + 40, 66)),
                "#222 as reported: paint must not apply the gap a second time"
            );
        }
    }
}
