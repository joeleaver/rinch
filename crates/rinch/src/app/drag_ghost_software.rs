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
//! It is painted only for a window that presents with the software renderer
//! ([`SoftwareSnapshot`]): a desktop build carries both renderers, and a
//! window the GPU presents draws the recorded scene, so a pixmap painted for
//! it was up to four surfaces of memory and paint at every drag start, for
//! nothing (#3065).
//!
//! [`paint_subtree_bounds`]: rinch_dom::paint::paint_subtree_bounds

use std::cell::RefCell;

use peniko::kurbo::{Rect, Vec2};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// The software ghost of a drag, painted or still owed.
///
/// A drag paints it when it starts if the window's last frame was a software
/// one, or there has been none (`RinchApp::presents_gpu_frames`). Where the
/// GPU presented that frame it is [`Self::Owed`]: nothing is allocated, and
/// the GPU frames draw the recorded scene. Should the window present with
/// software before the drag ends (its GPU renderer lost, and the software
/// one taking over), that frame paints it first (`RinchApp::build_pixels`),
/// from the document as it is then rather than as it was when the drag
/// started: the picture of the press was never taken, and the dragged box as
/// it now stands is the closest there is.
pub(crate) enum SoftwareSnapshot {
    /// Not painted yet: what to paint when a software frame needs it.
    Owed(GhostRequest),
    /// Painted; empty where no pixmap could be had.
    Painted(SoftwareGhost),
}

impl SoftwareSnapshot {
    /// Paint the ghost if it is owed, for a software frame to blit: whether
    /// it was.
    pub(crate) fn paint_owed(&mut self, doc: &RefCell<RinchDocument>) -> bool {
        let Self::Owed(request) = self else {
            return false;
        };
        *self = Self::Painted(request.paint(doc));
        true
    }

    /// The painted ghost; `None` while it is owed.
    pub(crate) fn painted(&self) -> Option<&SoftwareGhost> {
        match self {
            Self::Painted(ghost) => Some(ghost),
            Self::Owed(_) => None,
        }
    }
}

/// What a software ghost is painted from: the dragged node, and what the drag
/// measured when it started, in that drag's device pixels.
pub(crate) struct GhostRequest {
    pub(crate) node_id: u64,
    pub(crate) scale: f64,
    /// [`GhostPaint::reach`].
    pub(crate) reach: Option<Rect>,
    /// [`GhostPaint::clip`].
    pub(crate) clip: Rect,
    /// [`GhostPaint::anchor`].
    pub(crate) anchor: (f64, f64),
}

impl GhostRequest {
    /// Paint it ([`paint_software_ghost`]), within the document's viewport as
    /// it is now.
    pub(crate) fn paint(&self, doc: &RefCell<RinchDocument>) -> SoftwareGhost {
        self.paint_with(doc, TinySkiaPainter::try_new)
    }

    /// [`Self::paint`], each pixmap had from `surface`
    /// ([`paint_software_ghost`]).
    pub(crate) fn paint_with(
        &self,
        doc: &RefCell<RinchDocument>,
        surface: impl Fn(u32, u32) -> Option<TinySkiaPainter>,
    ) -> SoftwareGhost {
        let mut d = doc.borrow_mut();
        let d = &mut *d;
        let ghost = GhostPaint {
            tree: &d.tree,
            node_id: self.node_id,
            scale: self.scale,
            reach: self.reach,
            clip: self.clip,
            anchor: self.anchor,
            surface: (
                d.tree.viewport.width as f64 * self.scale,
                d.tree.viewport.height as f64 * self.scale,
            ),
        };
        paint_software_ghost(&ghost, surface, &mut d.font_cx, &mut d.layout_cx)
    }
}

/// The software ghost: premultiplied RGBA pixels, `width` x `height`, whose
/// top-left pixel sits at `origin` around the snapshot's origin (the dragged
/// box's top-left), in device pixels: negative where an outline, a shadow or
/// a scaled-up transform reaches left of or above the box. Empty (0 x 0)
/// when no pixmap could be had; such a ghost draws nothing and damages
/// nothing on a software frame (`RinchApp::ghost_overlay_rect`).
pub(crate) struct SoftwareGhost {
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) origin: (i32, i32),
}

impl SoftwareGhost {
    /// The pixmap's extent around the snapshot's origin, device pixels.
    pub(crate) fn rect(&self) -> Rect {
        let (x, y) = (self.origin.0 as f64, self.origin.1 as f64);
        Rect::new(x, y, x + self.width as f64, y + self.height as f64)
    }
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
///
/// Each pixmap is had from `surface` (a width and a height in device pixels;
/// `None` where that surface cannot be had): `TinySkiaPainter::try_new`, or
/// a test's, refusing the sizes it chooses.
pub(crate) fn paint_software_ghost(
    g: &GhostPaint<'_>,
    surface: impl Fn(u32, u32) -> Option<TinySkiaPainter>,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> SoftwareGhost {
    let whole = g.reach.unwrap_or(g.clip);
    let Some((pix, mut painter)) = [whole, g.clip].into_iter().find_map(|r| {
        let pix = ghost_pixmap_rect(r, g.anchor, g.surface);
        let (w, h) = (pix.width().max(1.0) as u32, pix.height().max(1.0) as u32);
        surface(w, h).map(|p| (pix, p))
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
