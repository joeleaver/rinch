//! `backdrop-filter` (#2719): what is painted behind an element, filtered,
//! drawn under the element's own painting and clipped to its border box.
//!
//! **What "behind" is.** Filter Effects 2 reads the *backdrop root image*:
//! everything the nearest **backdrop root** has painted before the element.
//! A backdrop root is the document, or an ancestor with a `filter`, an
//! `opacity` below 1, a `mask-image` or a `backdrop-filter`
//! ([`is_root`]). The root's own effects are not in its image — its opacity,
//! filter and mask apply to the group the element is drawn into afterwards,
//! and a root's own filtered backdrop is not part of what it painted.
//! Measured in Chrome 157: a backdrop-blurred box inside an `opacity: 0.8`
//! box blurs only that box's content and is then faded with it; a box
//! inside another backdrop-filtered box blurs the outer box's background,
//! not the outer box's blurred backdrop.
//!
//! **How it is read.** No painter can read back what it has drawn (the GPU
//! painter only encodes a scene), so the backdrop is painted **again**:
//! paint keeps a stack of the backdrop roots it is inside ([`RootGuard`]),
//! and [`draw`] paints the innermost one, bare ([`is_bare`]: without its own
//! opacity, filter, mask and backdrop), into a `TinySkiaPainter` the size of
//! the element's box on the device, culled to it, and **stops** when it
//! reaches the element ([`stop_here`]): everything paint enters after that
//! returns at once. The same code runs under both painters.
//!
//! **The filter** is the `filter` list's (`paint::filter`), with one
//! difference measured in Chrome 157: the blur reads no pixel past the
//! element's box — the backdrop is cut to the box and its edges mirrored
//! ([`super::filter::Edge::Mirror`]). The result is composited, inside the
//! element's `opacity` (Chrome draws it faded with the element), through a
//! clip of the element's border box (rounded) and before its background.
//!
//! **Damage.** A change behind a backdrop element changes the element's
//! pixels as far as the blur spreads it, inside the element's box:
//! [`grow_damage`] adds, for each backdrop element a damage rect meets, that
//! rect grown by the filter's reach and cut to the box.
//!
//! **Costs and limits.** Each paint of a backdrop element paints what is
//! behind it, inside its box, a second time, and filters it. A GPU frame
//! (`RenderSurface`, `GameViewport`, host texture) behind it is not in that
//! paint, and neither is anything the host draws under the embed overlay:
//! the backdrop is what rinch paints. Backdrop elements nest at most
//! [`MAX_DEPTH`] deep in one another's backdrops; past that, one draws no
//! backdrop.
#![cfg_attr(not(feature = "filter-raster"), allow(dead_code))]

use std::cell::{Cell, RefCell};

use peniko::kurbo::{Affine, Rect};

use crate::node::{Node, NodeTree, RawNodeId};

/// A backdrop root paint is inside: where [`super::paint_node`] was asked to
/// paint it.
#[derive(Clone, Copy)]
struct RootFrame {
    node: RawNodeId,
    offset_x: f64,
    offset_y: f64,
    parent_transform: Affine,
}

thread_local! {
    static ROOTS: RefCell<Vec<RootFrame>> = const { RefCell::new(Vec::new()) };
    /// The element whose backdrop is being painted: paint stops there.
    static STOP: Cell<Option<RawNodeId>> = const { Cell::new(None) };
    /// Paint reached [`STOP`]: everything after it draws nothing.
    static STOPPED: Cell<bool> = const { Cell::new(false) };
    /// The backdrop root being painted for a backdrop, without its own
    /// effects.
    static BARE: Cell<Option<RawNodeId>> = const { Cell::new(None) };
    /// How many backdrops are being painted inside one another.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// How many backdrops may be painted inside one another (a backdrop element
/// painted as part of another's backdrop paints its own).
pub(super) const MAX_DEPTH: u32 = 4;

/// Whether `node` is a backdrop root: it has a `filter` or `backdrop-filter`
/// (both cached in [`Node::has_filter`]), an `opacity` below 1 or a
/// `mask-image`.
#[inline]
pub(super) fn is_root(node: &Node) -> bool {
    node.has_filter()
        || node.computed_style.opacity < 1.0
        || !node.computed_style.mask_image.is_empty()
}

/// While alive, paint is inside the backdrop root it was pushed for.
pub(super) struct RootGuard;

impl RootGuard {
    pub(super) fn push(
        node: RawNodeId,
        offset_x: f64,
        offset_y: f64,
        parent_transform: Affine,
    ) -> Self {
        ROOTS.with(|r| {
            r.borrow_mut().push(RootFrame {
                node,
                offset_x,
                offset_y,
                parent_transform,
            })
        });
        RootGuard
    }
}

impl Drop for RootGuard {
    fn drop(&mut self) {
        ROOTS.with(|r| {
            r.borrow_mut().pop();
        });
    }
}

/// Whether paint must not draw `node_id`: a backdrop is being painted and
/// this is its element (from here on nothing is drawn), or the element was
/// already reached.
#[inline]
pub(super) fn stop_here(node_id: RawNodeId) -> bool {
    if STOPPED.with(Cell::get) {
        return true;
    }
    if STOP.with(Cell::get) == Some(node_id) {
        STOPPED.with(|s| s.set(true));
        return true;
    }
    false
}

/// Whether a backdrop's paint has reached its element: what a box draws after
/// its children (its scrollbars, its filter overlays) is past it.
#[inline]
pub(super) fn stopped() -> bool {
    STOPPED.with(Cell::get)
}

/// Whether `node_id` is the backdrop root being painted for a backdrop:
/// without its own opacity, filter, mask and backdrop.
#[inline]
pub(super) fn is_bare(node_id: RawNodeId) -> bool {
    BARE.with(|b| b.get() == Some(node_id))
}

/// How far a `backdrop-filter` list spreads a pixel, in CSS px (the largest
/// side of [`super::filter::list_outsets`]).
pub(crate) fn reach_css(node: &Node) -> f32 {
    super::filter::list_outsets(&node.computed_style.backdrop_filter)
        .into_iter()
        .fold(0.0, f32::max)
}

/// Grow `region` for the backdrop elements of `tree` (the module doc): each
/// rect that meets one's box (grown by its reach) adds itself grown by the
/// reach, cut to the box. One pass, in tree order.
pub(super) fn grow_damage(tree: &NodeTree, scale: f64, region: &mut super::DamageRegion) {
    if !tree.backdrop_filters || region.is_full() || region.is_empty() {
        return;
    }
    for &id in &tree.backdrop_nodes {
        let Some(node) = tree.get_live(id) else {
            continue;
        };
        if !node.computed_style.has_backdrop_effect()
            || node.layout.width <= 0.0
            || node.layout.height <= 0.0
        {
            continue;
        }
        let (_, _, t) = super::compute_absolute_position_and_transform(tree, id, scale);
        let c = t.as_coeffs();
        let m = reach_css(node) as f64 * scale * c[0].hypot(c[1]).max(c[2].hypot(c[3]));
        if m <= 0.0 {
            continue;
        }
        let bbox = super::painted_border_box(tree, id, scale);
        let meets = |r: &Rect| {
            let g = r.inflate(m, m);
            g.x0 < bbox.x1 && g.x1 > bbox.x0 && g.y0 < bbox.y1 && g.y1 > bbox.y0
        };
        let grown: Vec<Rect> = region
            .rects()
            .iter()
            .filter(|r| meets(r))
            .map(|r| r.inflate(m, m).intersect(bbox))
            .collect();
        for r in grown {
            if r.width() > 0.0 && r.height() > 0.0 && region.add(r) {
                return;
            }
        }
    }
}

/// Draw `node_id`'s filtered backdrop: its border box `rect` (device px,
/// before `node_transform`), clipped to `shape` (its rounded border box).
#[cfg(feature = "filter-raster")]
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn super::Painter,
    scale: f64,
    rect: Rect,
    shape: &super::PaintShape,
    node_transform: Affine,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<super::Brush>,
) {
    use super::filter::{Edge, apply, device_ops};
    use super::skia_painter::TinySkiaPainter;

    let Some(node) = tree.get(node_id) else {
        return;
    };
    if DEPTH.with(Cell::get) >= MAX_DEPTH {
        return;
    }
    // The innermost root around the element: the element is a backdrop root
    // itself (for its descendants), pushed before it got here.
    let Some(root) = ROOTS.with(|r| r.borrow().iter().rev().find(|f| f.node != node_id).copied())
    else {
        return;
    };
    // The element's box on the device, on whole pixels: the backdrop is cut
    // to it.
    let bbox = node_transform.transform_rect_bbox(rect).expand();
    let ops = device_ops(&node.computed_style.backdrop_filter, scale, node_transform);
    let [l, t, r, b] = super::filter::device_outsets(&ops);
    let m = l.max(t).max(r).max(b).max(0) as f64;
    // What of it shows (clips and window), and of that what a partial
    // repaint's damage asks for.
    let empty = |r: Rect| !(r.width() > 0.0 && r.height() > 0.0);
    let shows = match super::filter::visible_now(false) {
        Some(v) => v.intersect(bbox),
        None => bbox,
    };
    let shows_now = match super::filter::visible_now(true) {
        Some(v) => v.intersect(bbox),
        None => bbox,
    };
    if empty(shows) || empty(shows_now) {
        return;
    }
    let key = super::filter::CacheKey {
        canvas: [shows.x0, shows.y0, shows.x1, shows.y1],
        origin: (root.offset_x, root.offset_y),
        transform: node_transform.as_coeffs(),
        scale,
        size: (node.layout.width, node.layout.height),
        list: node.computed_style.backdrop_filter.clone(),
        shadows_only: false,
        tag: root.node,
    };
    // Kept from an earlier paint, and nothing behind it changed.
    let hit = node
        .raster_cache
        .borrow()
        .as_ref()
        .and_then(|c| c.backdrop.as_ref())
        .filter(|e| e.key == key)
        .map(|e| (e.image.clone(), e.x, e.y));
    if let Some((image, x, y)) = hit {
        tree.perf.bump(crate::perf::Counter::BackdropCacheHits);
        painter.push_clip(peniko::Fill::NonZero, node_transform, shape);
        painter.draw_raster(&image, x, y, 1.0);
        painter.pop_layer();
        return;
    }
    // All that shows is painted again, and kept, unless a partial repaint
    // shows only a small part of it.
    let region_of = |r: Rect| r.inflate(m, m).intersect(bbox).expand();
    let whole = region_of(shows);
    let part = region_of(shows_now);
    let keep_it = whole.area() <= super::filter::KEEP_RATIO * part.area();
    let (region, wanted) = if keep_it {
        (whole, shows)
    } else {
        (part, shows_now)
    };
    let (rx0, ry0) = (region.x0 as i32, region.y0 as i32);
    let (rw, rh) = (region.width() as u32, region.height() as u32);
    if rw == 0 || rh == 0 || rw as u64 * rh as u64 > super::filter::MAX_RASTER_PIXELS {
        return;
    }
    tree.perf.bump(crate::perf::Counter::BackdropRasters);
    tree.perf.add(
        crate::perf::Counter::BackdropRasterPixels,
        rw as u64 * rh as u64,
    );

    let mut raster = TinySkiaPainter::new(rw, rh);
    raster.set_subpixel_text(false);
    {
        let _state = Repaint::enter(node_id, root.node, region);
        let mut shifted = super::filter::Shifted {
            inner: &mut raster,
            shift: Affine::translate((-(rx0 as f64), -(ry0 as f64))),
        };
        let mut tracking = super::ClipTrackingPainter {
            inner: &mut shifted,
            ink_margin: super::INK_MARGIN_CSS_PX * scale,
        };
        super::paint_node(
            tree,
            root.node,
            &mut tracking,
            scale,
            root.offset_x,
            root.offset_y,
            font_cx,
            layout_cx,
            root.parent_transform,
        );
    }
    let mut pixels = raster.take_pixels();
    // A side the region was cut at (not the box's own edge) is read as
    // mirrored too; the pixels it changes are within the reach of that side,
    // outside what shows, and are cut away below.
    apply(&mut pixels, rw as usize, rh as usize, &ops, Edge::Mirror);

    // Only what shows is drawn.
    let crop = wanted.expand().intersect(region);
    let (kx0, ky0) = (crop.x0 as i32, crop.y0 as i32);
    let (kw, kh) = (crop.width() as u32, crop.height() as u32);
    if kw == 0 || kh == 0 {
        return;
    }
    let pixels = if (kx0, ky0, kw, kh) == (rx0, ry0, rw, rh) {
        pixels
    } else {
        let mut out = Vec::with_capacity((kw * kh * 4) as usize);
        for y in 0..kh {
            let row = ((ky0 - ry0) as u32 + y) * rw + (kx0 - rx0) as u32;
            let start = (row * 4) as usize;
            out.extend_from_slice(&pixels[start..start + (kw * 4) as usize]);
        }
        out
    };
    let image = super::painter::RasterImage {
        pixels: pixels.into(),
        width: kw,
        height: kh,
    };
    painter.push_clip(peniko::Fill::NonZero, node_transform, shape);
    painter.draw_raster(&image, kx0, ky0, 1.0);
    painter.pop_layer();
    if keep_it {
        super::filter::store(
            tree,
            node_id,
            node,
            true,
            super::filter::CacheEntry {
                key,
                image,
                x: kx0,
                y: ky0,
            },
        );
    }
}

/// Whether `d` is painted after `e`, so that nothing it draws is in `e`'s
/// backdrop. Answers only what is certain, and `false` otherwise: the two
/// meet under one parent as siblings `a` (holding `e`) and `s` (holding `d`),
/// `s` comes later, `a` is a stacking context, and `s` is positioned or one,
/// at a `z-index` no lower than `a`'s — so `s` is painted after the whole of
/// `a`. A modal's panel after its blurred overlay is the case this is for
/// (#2719 review).
fn paints_after(tree: &NodeTree, d: RawNodeId, e: RawNodeId) -> bool {
    let chain = |mut id: RawNodeId| {
        let mut v = vec![id];
        while let Some(p) = tree.get_live(id).and_then(|n| n.parent) {
            v.push(p);
            id = p;
        }
        v.reverse();
        v
    };
    let (ce, cd) = (chain(e), chain(d));
    let common = ce.iter().zip(&cd).take_while(|(a, b)| a == b).count();
    if common == 0 || common >= ce.len() || common >= cd.len() {
        // `d` is `e`, an ancestor (painted under it) or a descendant (handled
        // by the caller), or they are not in one tree.
        return false;
    }
    let (Some(p), Some(a), Some(s)) = (
        tree.get_live(ce[common - 1]),
        tree.get_live(ce[common]),
        tree.get_live(cd[common]),
    ) else {
        return false;
    };
    let pos = |id: RawNodeId| p.children.iter().position(|&c| c == id);
    let (Some(ia), Some(is)) = (pos(a.id), pos(s.id)) else {
        return false;
    };
    use crate::computed_style::PositionValue;
    let layered = |n: &Node| {
        n.creates_stacking_context() || n.computed_style.position != PositionValue::Static
    };
    let z = |n: &Node| n.computed_style.z_index.unwrap_or(0);
    is > ia && a.creates_stacking_context() && layered(s) && z(s) >= z(a)
}

/// Forget the kept backdrops a change since the last paint may have made
/// stale (run beside `filter::invalidate`): each one with a paint-dirty node
/// that is not inside the element and not painted after it
/// ([`paints_after`]), whose box (now or as painted) grown by the element's
/// reach meets the element's box; one a removed node's rect meets; all of
/// them after a whole-document restyle.
pub(super) fn invalidate(tree: &NodeTree, scale: f64) {
    if !tree.backdrop_filters {
        return;
    }
    let cached: Vec<RawNodeId> = tree
        .raster_cached
        .borrow()
        .iter()
        .copied()
        .filter(|&id| {
            tree.get_live(id).is_some_and(|n| {
                n.raster_cache
                    .borrow()
                    .as_ref()
                    .is_some_and(|c| c.backdrop.is_some())
            })
        })
        .collect();
    if cached.is_empty() {
        return;
    }
    let clear = |id: RawNodeId| {
        if let Some(c) = tree
            .get_live(id)
            .and_then(|n| n.raster_cache.borrow_mut().take())
        {
            let filter = c.filter;
            if filter.is_some() {
                *tree.get_live(id).unwrap().raster_cache.borrow_mut() =
                    Some(Box::new(super::filter::RasterCache {
                        filter,
                        backdrop: None,
                    }));
            }
        }
    };
    if tree.whole_document_damaged {
        cached.iter().for_each(|&id| clear(id));
        return;
    }
    // The rect a node paints in, now and as painted: its own box, or the box
    // that paints it (a text node, an inline).
    let rects = |d: RawNodeId| -> Vec<Rect> {
        let mut id = d;
        loop {
            let Some(n) = tree.get_live(id) else {
                return Vec::new();
            };
            if n.layout.width > 0.0 && n.layout.height > 0.0 || n.parent.is_none() {
                break;
            }
            id = n.parent.unwrap();
        }
        let mut v = vec![super::painted_border_box(tree, id, scale)];
        if let Some(r) = super::previous_painted_rect(tree, id, scale) {
            v.push(r);
        }
        v
    };
    let removed: Vec<Rect> = tree
        .paint_dirty_removed_rects
        .iter()
        .map(|&(x, y, w, h)| Rect::new(x * scale, y * scale, (x + w) * scale, (y + h) * scale))
        .collect();
    for e in cached {
        let Some(en) = tree.get_live(e) else { continue };
        let (_, _, t) = super::compute_absolute_position_and_transform(tree, e, scale);
        let c = t.as_coeffs();
        let m = reach_css(en) as f64 * scale * c[0].hypot(c[1]).max(c[2].hypot(c[3])) + 1.0;
        let bbox = super::painted_border_box(tree, e, scale);
        let meets = |r: &Rect| {
            let g = r.inflate(m, m);
            g.x0 < bbox.x1 && g.x1 > bbox.x0 && g.y0 < bbox.y1 && g.y1 > bbox.y0
        };
        let inside = |d: RawNodeId| {
            let mut cur = Some(d);
            while let Some(id) = cur {
                if id == e {
                    return true;
                }
                cur = tree.get_live(id).and_then(|n| n.parent);
            }
            false
        };
        let stale = removed.iter().any(meets)
            || tree
                .paint_dirty_nodes
                .iter()
                .any(|&d| !inside(d) && !paints_after(tree, d, e) && rects(d).iter().any(meets));
        if stale {
            clear(e);
        }
    }
}

/// The paint state of one backdrop's second paint, restored on drop: paint
/// stops at the element, the root is bare, the damage is the region, and the
/// clip-cull stack is a fresh one.
struct Repaint {
    stop: Option<RawNodeId>,
    stopped: bool,
    bare: Option<RawNodeId>,
    dirty: Option<Vec<Rect>>,
    clips: Vec<Option<Rect>>,
    roots: usize,
}

impl Repaint {
    fn enter(element: RawNodeId, root: RawNodeId, region: Rect) -> Self {
        DEPTH.with(|d| d.set(d.get() + 1));
        Self {
            stop: STOP.with(|s| s.replace(Some(element))),
            stopped: STOPPED.with(|s| s.replace(false)),
            bare: BARE.with(|b| b.replace(Some(root))),
            dirty: super::DIRTY_REGION.with(|d| d.replace(Some(vec![region]))),
            clips: super::CLIP_CULL.with(|c| std::mem::take(&mut *c.borrow_mut())),
            roots: ROOTS.with(|r| r.borrow().len()),
        }
    }
}

impl Drop for Repaint {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
        STOP.with(|s| s.set(self.stop));
        STOPPED.with(|s| s.set(self.stopped));
        BARE.with(|b| b.set(self.bare));
        super::DIRTY_REGION.with(|d| *d.borrow_mut() = self.dirty.take());
        super::CLIP_CULL.with(|c| *c.borrow_mut() = std::mem::take(&mut self.clips));
        ROOTS.with(|r| r.borrow_mut().truncate(self.roots));
    }
}
