//! The software drag ghost's pixmap (#3065): how much of what the dragged
//! subtree paints it holds, and painting it.
//!
//! The GPU ghost is a recorded scene and costs nothing past what it draws.
//! The software ghost is a pixmap, painted once when the drag starts and
//! blitted at the pointer on every frame, so its size is memory: a
//! `box-shadow: 0 0 0 9999px` reaches about 20,000 device pixels each way at
//! 1x. It holds what the subtree paints ([`paint_subtree_bounds`]) only as
//! far as the window can show it ([`ghost_pixmap_rect`]), and a pixmap that
//! cannot be had leaves a smaller ghost, or none, rather than a panic or an
//! abort ([`paint_software_ghost`]).
//!
//! [`paint_subtree_bounds`]: rinch_dom::paint::paint_subtree_bounds

use peniko::kurbo::{Rect, Vec2};
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// The software ghost: premultiplied RGBA pixels, `width` x `height`, whose
/// top-left pixel sits at `origin` around the snapshot's origin (the dragged
/// box's top-left), in device pixels. Empty (0 x 0) when no pixmap could be
/// had; such a ghost draws nothing and damages nothing
/// (`RinchApp::ghost_overlay_rect`).
pub(crate) struct SoftwareGhost {
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) origin: (i32, i32),
}

/// The part of `reach` (what the ghost paints, around the snapshot's origin)
/// that the pixmap holds, out to whole device pixels: no further from the
/// grab point `anchor` than the surface (`surface`, device pixels) is wide
/// and tall.
///
/// The ghost is drawn with `anchor` under the pointer
/// (`ActiveDrag::ghost_translate`). With the pointer anywhere on the surface,
/// the ghost pixels the surface shows are within one surface width and height
/// of `anchor`, so the rest is never seen while the pointer is over the
/// window, and the pixmap is at most twice the surface each way, whatever the
/// subtree paints. (A pointer captured past the window's edge can show the
/// cut edge.) It was bounded around the whole box instead (`ghost_clip`
/// inflated by the surface), which a large box made larger again.
pub(crate) fn ghost_pixmap_rect(reach: Rect, anchor: (f64, f64), surface: (f64, f64)) -> Rect {
    let (ax, ay) = anchor;
    let (vw, vh) = surface;
    reach
        .intersect(Rect::new(ax - vw, ay - vh, ax + vw, ay + vh))
        .expand()
}

/// What [`paint_software_ghost`] paints, and where.
pub(crate) struct GhostPaint<'a> {
    pub(crate) tree: &'a rinch_dom::node::NodeTree,
    pub(crate) node_id: u64,
    pub(crate) scale: f64,
    /// What the subtree paints around its origin; `None` where that cannot
    /// be bounded, and the ghost is clipped to `clip`.
    pub(crate) reach: Option<Rect>,
    /// The box as `paint_subtree` draws it, through its own transform.
    pub(crate) clip: Rect,
    /// The grab point around the snapshot's origin, in device pixels.
    pub(crate) anchor: (f64, f64),
    /// The surface's size, in device pixels.
    pub(crate) surface: (f64, f64),
}

/// Paint the dragged subtree into a pixmap holding
/// [`ghost_pixmap_rect`] of what it paints. Where that pixmap cannot be had
/// (memory the allocator will not reserve), the box alone as drawn, and
/// where not even that, an empty ghost: the drag goes on without a picture.
pub(crate) fn paint_software_ghost(
    g: &GhostPaint<'_>,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> SoftwareGhost {
    let whole = g.reach.unwrap_or(g.clip);
    let Some((pix, mut painter)) = [whole, g.clip].into_iter().find_map(|r| {
        let pix = ghost_pixmap_rect(r, g.anchor, g.surface);
        let (w, h) = (pix.width().max(1.0) as u32, pix.height().max(1.0) as u32);
        TinySkiaPainter::try_new(w, h).map(|p| (pix, p))
    }) else {
        return SoftwareGhost {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            origin: (0, 0),
        };
    };
    // Where the subtree's origin lands in the pixmap.
    let at = (-pix.x0, -pix.y0);
    // Grey text, as the GPU ghost (#2286 stage 3): drawn at the pointer,
    // off the subpixel grid.
    painter.set_subpixel_text(false);

    if g.reach.is_none() {
        painter.push_clip(
            peniko::Fill::NonZero,
            peniko::kurbo::Affine::IDENTITY,
            &PaintShape::Rect(g.clip + Vec2::new(at.0, at.1)),
        );
    }
    rinch_dom::paint::paint_subtree_at(
        g.tree,
        &mut painter,
        g.node_id,
        g.scale,
        at,
        font_cx,
        layout_cx,
    );
    if g.reach.is_none() {
        painter.pop_layer();
    }
    SoftwareGhost {
        width: painter.width(),
        height: painter.height(),
        // Taken, not copied: the pixmap can be large.
        pixels: painter.take_pixels(),
        origin: (pix.x0 as i32, pix.y0 as i32),
    }
}
