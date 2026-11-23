//! `filter` lists that move pixels — `blur()` and `drop-shadow()` — drawn
//! through an offscreen raster (#2719).
//!
//! A box whose `filter` list holds a `blur()` or a `drop-shadow()`
//! ([`FilterOp::moves_pixels`]) is painted, with everything in its stacking
//! context, into a `TinySkiaPainter` (`paint::skia_painter`) the size of the part that can show
//! (grown by how far the list spreads ink), on whole device pixels at the
//! device scale and under the box's own transforms. The list is then applied
//! to that raster function by function, and the result is composited back
//! with [`Painter::draw_raster`] at the box's `opacity`. The same code runs
//! under both painters: the GPU painter receives the raster as an image.
//!
//! A list of `drop-shadow()`s alone, on a box at full opacity, draws only
//! the shadows from the raster and then paints the box itself again, in
//! place, with the painter it was given — so the content (text above all) is
//! the painter's own and only the shadows come from the raster.
//!
//! **The geometry.** Filter Effects 1 applies the list in the element's local
//! space and transforms the result. A Gaussian is isotropic, so blurring in
//! device space with each axis's deviation stretched by the transform's scale
//! on that axis is the same picture for a translation, a scale or a rotation;
//! a skew is approximated by the lengths of the transform's columns. A
//! `drop-shadow()` offset is carried through the transform's linear part and
//! rounded to whole device pixels.
//!
//! **The blur** is Skia's raster blur, which Chrome's software path draws
//! with: three box blurs whose window is `floor(σ · 3√(2π)/4 + ½)` — three of
//! that width when it is odd; when it is even, two of it (the second shifted
//! a pixel right) and one a pixel wider, centred. Measured against Chrome
//! 157's software raster (`blur(1.3px)`, `blur(5px)`, `blur(13px)`,
//! `drop-shadow` deviations 6 and 10): within 1 level everywhere. Each pass
//! rounds to 8 bits, as Skia's does. Pixels past the raster's edge are
//! transparent (Chrome's `decal`).
//!
//! **The colour functions** are the Filter Effects 1 matrices in gamma-encoded
//! sRGB, on straight alpha, each clamped to 0..=1: Chrome's results for
//! `brightness`, `contrast`, `saturate`, `hue-rotate`, `sepia`, `invert`,
//! `grayscale` and `opacity` on a solid colour, to the level.
//!
//! **The cull.** Everything drawn inside the raster is culled against the
//! damage, the window and the open clips **grown by the list's reach**
//! ([`CullScope`]): a pixel outside them can still be blurred into them.
//! Damage is grown the same way (`own_ink_outsets` for the box itself,
//! [`ancestor_reach`] for a change inside it).
// Without the raster (no `filter-raster`), only the reach is used: a box is
// painted unfiltered, and its damage still covers the reach.
#![cfg_attr(not(feature = "filter-raster"), allow(dead_code))]

use std::cell::Cell;

use peniko::kurbo::{Affine, Rect};

use crate::computed_style::FilterOp;
use crate::node::{NodeTree, RawNodeId};

use super::painter::{Painter, RasterImage};
use super::raster_cache::Kind;

thread_local! {
    /// The box whose filter is being drawn right now, and whether its
    /// `opacity` is left out: [`super::paint_node`] paints it the ordinary
    /// way instead of starting its filter again, without its `opacity` when
    /// the raster is composited at it.
    static FILTERING: Cell<Option<(RawNodeId, bool)>> = const { Cell::new(None) };
}

/// Whether `node_id`'s filter is being drawn: paint it without its filter.
#[inline]
pub(super) fn is_filtering(node_id: RawNodeId) -> bool {
    FILTERING.with(|f| f.get().is_some_and(|(id, _)| id == node_id))
}

/// The `opacity` paint gives `node_id`: none while its filter's raster,
/// composited at it, is being drawn.
#[inline]
pub(super) fn own_opacity(node_id: RawNodeId, opacity: f32) -> f32 {
    if opacity < 1.0
        && (FILTERING.with(|f| f.get() == Some((node_id, true)))
            || super::backdrop::is_bare(node_id))
    {
        1.0
    } else {
        opacity
    }
}

// ── Reach ───────────────────────────────────────────────────────────────────

/// Skia's box window for a blur of deviation `sigma` device px.
fn box_window(sigma: f64) -> usize {
    if sigma <= 0.0 {
        return 1;
    }
    (sigma * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5)
        .floor()
        .max(1.0) as usize
}

/// The three box passes of a blur of window `d`, as `(width, shift)`: pass
/// `i` averages `width` pixels starting at `i - width / 2 + shift`.
fn box_passes(d: usize) -> [(usize, usize); 3] {
    if d % 2 == 1 {
        [(d, 0), (d, 0), (d, 0)]
    } else {
        [(d, 0), (d, 1), (d + 1, 0)]
    }
}

/// How far one pixel's value spreads under a blur of deviation `sigma`
/// device px, in whole pixels, either way.
pub(super) fn blur_reach(sigma: f64) -> usize {
    let d = box_window(sigma);
    if d <= 1 {
        return 0;
    }
    let (mut left, mut right) = (0usize, 0usize);
    for (w, shift) in box_passes(d) {
        // A pass reads `[i - w/2 + shift, i - w/2 + shift + w - 1]`.
        left += (w / 2).saturating_sub(shift);
        right += (w - 1 + shift).saturating_sub(w / 2);
    }
    left.max(right)
}

/// How far a blur of CSS deviation `sigma` reaches, in CSS px: three
/// deviations, the most Skia's boxes reach (2.82σ + 1), with a pixel of
/// slack.
fn blur_reach_css(sigma: f32) -> f32 {
    if sigma <= 0.0 { 0.0 } else { 3.0 * sigma + 1.0 }
}

/// Outsets `[left, top, right, bottom]` a list spreads ink by, given how each
/// function maps: `sigma(s)` its blur deviations and `offset(dx, dy)` its
/// shadow offsets into the units the answer is in.
fn outsets_with(
    list: &[FilterOp],
    reach: impl Fn(f32) -> f32,
    offset: impl Fn(f32, f32) -> (f32, f32),
) -> [f32; 4] {
    let mut o = [0.0_f32; 4];
    for op in list {
        match *op {
            FilterOp::Blur(s) if s > 0.0 => {
                let r = reach(s);
                o.iter_mut().for_each(|side| *side += r);
            }
            FilterOp::DropShadow {
                offset_x,
                offset_y,
                std_dev,
                ..
            } => {
                // The shadow of everything the list has drawn so far, moved
                // and blurred, under it.
                let r = reach(std_dev);
                let (dx, dy) = offset(offset_x, offset_y);
                let shadow = [o[0] + r - dx, o[1] + r - dy, o[2] + r + dx, o[3] + r + dy];
                for (side, s) in o.iter_mut().zip(shadow) {
                    *side = side.max(s);
                }
            }
            _ => {}
        }
    }
    o
}

/// How far `list` carries a box's ink past the box, in CSS px
/// `[left, top, right, bottom]`, before any transform: its damage and paint
/// cull reach (`own_ink_outsets`). Zero for a list with nothing that moves
/// pixels.
pub(crate) fn list_outsets(list: &[FilterOp]) -> [f32; 4] {
    outsets_with(list, blur_reach_css, |dx, dy| (dx, dy))
}

/// The lengths of `t`'s columns: how far it stretches each axis.
pub(super) fn axis_scales(t: Affine) -> (f64, f64) {
    let c = t.as_coeffs();
    (c[0].hypot(c[1]), c[2].hypot(c[3]))
}

/// One function of a list as the raster applies it, in device px.
#[derive(Clone, Copy, Debug)]
pub(super) enum DeviceOp {
    Blur {
        sx: f64,
        sy: f64,
    },
    Shadow {
        dx: i32,
        dy: i32,
        sx: f64,
        sy: f64,
        /// Premultiplied `[r, g, b, a]`, 0..=1.
        color: [f32; 4],
    },
    Colour(FilterOp),
}

/// `list` in device px under `transform` at `scale`.
pub(super) fn device_ops(list: &[FilterOp], scale: f64, transform: Affine) -> Vec<DeviceOp> {
    let (ax, ay) = axis_scales(transform);
    let linear = Affine::new({
        let c = transform.as_coeffs();
        [c[0], c[1], c[2], c[3], 0.0, 0.0]
    });
    list.iter()
        .filter_map(|op| match *op {
            FilterOp::Blur(s) if s > 0.0 => Some(DeviceOp::Blur {
                sx: s as f64 * scale * ax,
                sy: s as f64 * scale * ay,
            }),
            FilterOp::Blur(_) => None,
            FilterOp::DropShadow {
                offset_x,
                offset_y,
                std_dev,
                color,
            } => {
                let v = linear
                    * peniko::kurbo::Point::new(offset_x as f64 * scale, offset_y as f64 * scale);
                let c = color
                    .unwrap_or(peniko::Color::BLACK)
                    .premultiply()
                    .components;
                Some(DeviceOp::Shadow {
                    dx: v.x.round() as i32,
                    dy: v.y.round() as i32,
                    sx: std_dev.max(0.0) as f64 * scale * ax,
                    sy: std_dev.max(0.0) as f64 * scale * ay,
                    color: c,
                })
            }
            other => Some(DeviceOp::Colour(other)),
        })
        .collect()
}

/// How far the device-px list spreads ink, `[left, top, right, bottom]`, in
/// whole pixels.
pub(super) fn device_outsets(ops: &[DeviceOp]) -> [i32; 4] {
    let mut o = [0_i32; 4];
    for op in ops {
        match *op {
            DeviceOp::Blur { sx, sy } => {
                let (rx, ry) = (blur_reach(sx) as i32, blur_reach(sy) as i32);
                o[0] += rx;
                o[2] += rx;
                o[1] += ry;
                o[3] += ry;
            }
            DeviceOp::Shadow { dx, dy, sx, sy, .. } => {
                let (rx, ry) = (blur_reach(sx) as i32, blur_reach(sy) as i32);
                let shadow = [
                    o[0] + rx - dx,
                    o[1] + ry - dy,
                    o[2] + rx + dx,
                    o[3] + ry + dy,
                ];
                for (side, s) in o.iter_mut().zip(shadow) {
                    *side = (*side).max(s);
                }
            }
            DeviceOp::Colour(_) => {}
        }
    }
    o
}

/// How far the raster filters of `node_id`'s ancestors spread a change made
/// inside them, in device px (the largest side, summed over every ancestor
/// with one): what a damage rect for a change in their subtree is grown by.
/// `0.0` while no element has ever had such a filter
/// ([`NodeTree::raster_filters`]), without a walk.
pub(crate) fn ancestor_reach(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> f64 {
    if !tree.raster_filters {
        return 0.0;
    }
    let mut total = 0.0;
    let mut cur = tree.get(node_id).and_then(|n| n.parent);
    while let Some(id) = cur {
        let Some(n) = tree.get_live(id) else { break };
        if n.computed_style.has_raster_filter() {
            let (_, _, t) = super::compute_absolute_position_and_transform(tree, id, scale);
            let (ax, ay) = axis_scales(t);
            let o = list_outsets(&n.computed_style.filter);
            let side = o.iter().copied().fold(0.0_f32, f32::max) as f64;
            total += side * scale * ax.max(ay);
        }
        if id == tree.body_id {
            break;
        }
        cur = n.parent;
    }
    total
}

// ── The cull, grown ─────────────────────────────────────────────────────────

/// While alive, the window and the open clips paint culls against are grown
/// by `m` device px — what may be drawn outside them and still be blurred into
/// them — the clip-cull stack is a fresh one seeded with the grown top, and
/// the damage is the raster's own rect (which holds the damage grown by `m`).
/// Restored on drop.
struct CullScope {
    dirty: Option<Vec<Rect>>,
    viewport: Option<super::Viewport>,
    clips: Vec<Option<Rect>>,
}

impl CullScope {
    /// Grown by `m`, and the damage replaced by `canvas`: everything the
    /// raster covers is painted into it.
    fn enter(m: f64, canvas: Rect) -> Self {
        let dirty = super::DIRTY_REGION.with(|d| d.replace(Some(vec![canvas])));
        let viewport = super::VIEWPORT.with(|v| {
            let saved = v.get();
            v.set(saved.map(|vp| super::Viewport {
                target: vp.target.inflate(m, m),
                cull: vp.cull.inflate(m, m),
            }));
            saved
        });
        let clips = super::CLIP_CULL.with(|c| {
            let mut c = c.borrow_mut();
            let saved = std::mem::take(&mut *c);
            if let Some(top) = saved.last().copied().flatten() {
                c.push(Some(top.inflate(m, m)));
            }
            saved
        });
        Self {
            dirty,
            viewport,
            clips,
        }
    }
}

impl Drop for CullScope {
    fn drop(&mut self) {
        super::DIRTY_REGION.with(|d| *d.borrow_mut() = self.dirty.take());
        super::VIEWPORT.with(|v| v.set(self.viewport));
        super::CLIP_CULL.with(|c| *c.borrow_mut() = std::mem::take(&mut self.clips));
    }
}

/// The part of the device anything drawn now can show on: the open clips
/// and the window's cull rect, and the damage's bounds when `with_damage`,
/// intersected. `None` when none of them bounds it.
pub(super) fn visible_now(with_damage: bool) -> Option<Rect> {
    let mut v: Option<Rect> = None;
    let mut meet = |r: Rect| v = Some(v.map_or(r, |a| a.intersect(r)));
    if with_damage
        && let Some(u) = super::DIRTY_REGION.with(|d| {
            d.borrow().as_ref().map(|rects| {
                rects
                    .iter()
                    .copied()
                    .reduce(|a, b| a.union(b))
                    .unwrap_or(Rect::ZERO)
            })
        })
    {
        meet(u);
    }
    if let Some(c) = super::open_clip_cull() {
        meet(c);
    }
    if let Some(vp) = super::VIEWPORT.with(|vp| vp.get()) {
        meet(vp.cull);
    }
    v
}

/// The most pixels a filter raster may have; past it the box is painted
/// unfiltered (64 MB of RGBA).
pub(super) const MAX_RASTER_PIXELS: u64 = 16 * 1024 * 1024;

// ── Paint ───────────────────────────────────────────────────────────────────

/// Paint `node_id`, whose `filter` moves pixels, through its offscreen raster
/// (the module doc). Arguments as [`super::paint_node`]'s.
#[cfg(feature = "filter-raster")]
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_filtered(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn Painter,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<super::Brush>,
    parent_transform: Affine,
) {
    use super::skia_painter::TinySkiaPainter;

    let Some(node) = tree.get(node_id) else {
        return;
    };
    let cs = &node.computed_style;
    let opacity = cs.opacity;
    if opacity <= 0.0 {
        tree.perf.bump(crate::perf::Counter::PaintNodesVisited);
        return;
    }
    // The box painted the ordinary way, without its filter; without its
    // `opacity` too when `drop_opacity`.
    let paint_plain = |painter: &mut dyn Painter,
                       font_cx: &mut parley::FontContext,
                       layout_cx: &mut parley::LayoutContext<super::Brush>,
                       drop_opacity: bool| {
        let prev = FILTERING.with(|f| f.replace(Some((node_id, drop_opacity))));
        super::paint_node(
            tree,
            node_id,
            painter,
            scale,
            offset_x,
            offset_y,
            font_cx,
            layout_cx,
            parent_transform,
        );
        FILTERING.with(|f| f.set(prev));
    };

    let layout = &node.layout;
    let x = offset_x + layout.x as f64 * scale;
    let y = super::sticky_y(
        tree,
        node,
        offset_y + layout.y as f64 * scale,
        offset_y,
        scale,
    );
    let node_transform = super::compose_node_transform(node, x, y, scale, parent_transform);
    let ops = device_ops(&cs.filter, scale, node_transform);
    let [ol, ot, or, ob] = device_outsets(&ops);
    let m = ol.max(ot).max(or).max(ob).max(0) as f64;

    // What the box and its stacking context draw, filter reach included
    // (`layer_bounds` grows a filtered box by it), on the device.
    let local = super::opacity_layer_bounds(tree, node_id, scale, x, y);
    let content = (local != super::UNBOUNDED).then(|| node_transform.transform_rect_bbox(local));
    // What can show: the content where the clips and the window let it, and
    // within that what a partial repaint's damage asks for.
    let fallback = || {
        let (w, h) = (layout.width as f64 * scale, layout.height as f64 * scale);
        node_transform
            .transform_rect_bbox(Rect::new(x, y, x + w, y + h))
            .inflate(m, m)
    };
    let meet = |a: Option<Rect>, b: Option<Rect>| match (a, b) {
        (Some(a), Some(b)) => Some(a.intersect(b)),
        (a, b) => a.or(b),
    };
    let shows = meet(content, visible_now(false)).unwrap_or_else(fallback);
    let damage = super::DIRTY_REGION.with(|d| {
        d.borrow().as_ref().map(|rects| {
            rects
                .iter()
                .copied()
                .reduce(|a, b| a.union(b))
                .unwrap_or(Rect::ZERO)
        })
    });
    let shows_now = meet(Some(shows), damage).unwrap_or(shows);
    let empty = |r: Rect| !(r.width() > 0.0 && r.height() > 0.0);
    if empty(shows) || empty(shows_now) {
        tree.perf.bump(crate::perf::Counter::PaintNodesVisited);
        return;
    }
    // A raster: what shows, grown by what can be spread into it, on whole
    // pixels, and no larger than the content.
    let canvas_of = |r: Rect| {
        let mut c = r.inflate(m, m);
        if let Some(content) = content {
            c = c.intersect(content.inflate(1.0, 1.0));
        }
        c.expand()
    };
    let whole = canvas_of(shows);
    let shadows_only = opacity >= 1.0 && ops.iter().all(|op| matches!(op, DeviceOp::Shadow { .. }));
    let key = super::raster_cache::CacheKey {
        canvas: [whole.x0, whole.y0, whole.x1, whole.y1],
        origin: (x, y),
        transform: node_transform.as_coeffs(),
        scale,
        size: (layout.width, layout.height),
        list: cs.filter.clone(),
        shadows_only,
        tag: 0,
    };

    // Kept from an earlier paint, and nothing it was drawn from changed.
    let hit = super::raster_cache::lookup(tree, node, Kind::Filter, &key);
    if let Some((image, ix, iy)) = hit {
        tree.perf.bump(crate::perf::Counter::FilterCacheHits);
        if shadows_only {
            painter.draw_raster(&image, ix, iy, 1.0);
            paint_plain(painter, font_cx, layout_cx, false);
        } else {
            painter.draw_raster(&image, ix, iy, opacity.min(1.0));
        }
        return;
    }

    // The whole visible box is rasterised, and kept, unless a partial
    // repaint shows only a small part of it: then only that part, not kept
    // (a change inside a large blurred panel, a caret blinking in it).
    let part = canvas_of(shows_now);
    let keep = whole.area() <= super::raster_cache::KEEP_RATIO * part.area();
    let canvas = if keep { whole } else { part };
    let (cx0, cy0) = (canvas.x0 as i32, canvas.y0 as i32);
    let (cw, ch) = (
        canvas.width().max(0.0) as u32,
        canvas.height().max(0.0) as u32,
    );
    if cw == 0 || ch == 0 {
        tree.perf.bump(crate::perf::Counter::PaintNodesVisited);
        return;
    }
    if cw as u64 * ch as u64 > MAX_RASTER_PIXELS {
        paint_plain(painter, font_cx, layout_cx, false);
        return;
    }
    tree.perf.bump(crate::perf::Counter::FilterRasters);
    tree.perf.add(
        crate::perf::Counter::FilterRasterPixels,
        cw as u64 * ch as u64,
    );

    let mut raster = TinySkiaPainter::new(cw, ch);
    // Composited, as a layer is: grey text, as on the GPU (#2286 stage 3).
    raster.set_subpixel_text(false);
    {
        let _cull = CullScope::enter(m, canvas);
        let mut shifted = Shifted {
            inner: &mut raster,
            shift: Affine::translate((-(cx0 as f64), -(cy0 as f64))),
        };
        // The raster's layers go on the stack above the outer painter's: a
        // fixed box painted into it lifts none of those (#549).
        let _floor = super::FixedFloor::here();
        let mut tracking =
            super::ClipTrackingPainter::new(&mut shifted, super::INK_MARGIN_CSS_PX * scale);
        paint_plain(&mut tracking, font_cx, layout_cx, true);
    }
    let mut pixels = raster.take_pixels();

    let image = if shadows_only {
        RasterImage {
            pixels: shadow_layer(&pixels, cw as usize, ch as usize, &ops).into(),
            width: cw,
            height: ch,
        }
    } else {
        apply(&mut pixels, cw as usize, ch as usize, &ops, Edge::Decal);
        RasterImage {
            pixels: pixels.into(),
            width: cw,
            height: ch,
        }
    };
    if shadows_only {
        painter.draw_raster(&image, cx0, cy0, 1.0);
        paint_plain(painter, font_cx, layout_cx, false);
    } else {
        painter.draw_raster(&image, cx0, cy0, opacity.min(1.0));
    }
    if keep {
        super::raster_cache::store(tree, node_id, node, Kind::Filter, key, image, cx0, cy0);
    }
}

/// Paint layered inline span `span_id` (#2869, `paints_in_own_entry`), whose
/// `filter` is not `none`, through an offscreen raster, as [`paint_filtered`]
/// paints a box: everything its entry draws, every line's fragment, is drawn
/// once into the raster by `plain` (its entry, painted again with
/// [`is_filtering`] answering `true`), the list applied to it, and the
/// result composited at the span's `opacity`. A list of `drop-shadow()`s
/// alone at full opacity draws only the shadows from the raster and the span
/// again over them with `painter`.
///
/// Unlike a box's, every function of the list is applied in the raster,
/// the colour ones too: a span has no box for the colour overlays to cover.
/// `extent` is what the span draws ([`super::text::layered_span_extent`]),
/// in the device space before `transform`. Nothing is cached: a span is in
/// no raster cache.
#[cfg(feature = "filter-raster")]
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_filtered_span(
    tree: &NodeTree,
    span_id: RawNodeId,
    list: &[FilterOp],
    opacity: f32,
    painter: &mut dyn Painter,
    scale: f64,
    transform: Affine,
    extent: Rect,
    plain: &mut dyn FnMut(&mut dyn Painter),
) {
    use super::skia_painter::TinySkiaPainter;

    // `plain` with the span's filter off, and its opacity too when `drop`.
    let mut plain_as = |painter: &mut dyn Painter, drop: bool| {
        let prev = FILTERING.with(|f| f.replace(Some((span_id, drop))));
        plain(painter);
        FILTERING.with(|f| f.set(prev));
    };
    let ops = device_ops(list, scale, transform);
    let [ol, ot, or, ob] = device_outsets(&ops);
    let m = ol.max(ot).max(or).max(ob).max(0) as f64;
    // What the span and its filter can draw, and of that what can show now.
    let content = transform.transform_rect_bbox(extent).inflate(m, m);
    let shows = visible_now(true).map_or(content, |v| v.intersect(content));
    if !(shows.width() > 0.0 && shows.height() > 0.0) {
        return;
    }
    let canvas = shows
        .inflate(m, m)
        .intersect(content.inflate(1.0, 1.0))
        .expand();
    let (cx0, cy0) = (canvas.x0 as i32, canvas.y0 as i32);
    let (cw, ch) = (
        canvas.width().max(0.0) as u32,
        canvas.height().max(0.0) as u32,
    );
    if cw == 0 || ch == 0 {
        return;
    }
    if cw as u64 * ch as u64 > MAX_RASTER_PIXELS {
        plain_as(painter, false);
        return;
    }
    tree.perf.bump(crate::perf::Counter::FilterRasters);
    tree.perf.add(
        crate::perf::Counter::FilterRasterPixels,
        cw as u64 * ch as u64,
    );
    let mut raster = TinySkiaPainter::new(cw, ch);
    // Composited, as a layer is: grey text, as on the GPU (#2286 stage 3).
    raster.set_subpixel_text(false);
    {
        let _cull = CullScope::enter(m, canvas);
        let mut shifted = Shifted {
            inner: &mut raster,
            shift: Affine::translate((-(cx0 as f64), -(cy0 as f64))),
        };
        let _floor = super::FixedFloor::here();
        let mut tracking =
            super::ClipTrackingPainter::new(&mut shifted, super::INK_MARGIN_CSS_PX * scale);
        plain_as(&mut tracking, true);
    }
    let mut pixels = raster.take_pixels();
    let shadows_only = opacity >= 1.0 && ops.iter().all(|op| matches!(op, DeviceOp::Shadow { .. }));
    if shadows_only {
        let image = RasterImage {
            pixels: shadow_layer(&pixels, cw as usize, ch as usize, &ops).into(),
            width: cw,
            height: ch,
        };
        painter.draw_raster(&image, cx0, cy0, 1.0);
        plain_as(painter, false);
    } else {
        apply(&mut pixels, cw as usize, ch as usize, &ops, Edge::Decal);
        let image = RasterImage {
            pixels: pixels.into(),
            width: cw,
            height: ch,
        };
        painter.draw_raster(&image, cx0, cy0, opacity.min(1.0));
    }
}

// ── The raster's painter ────────────────────────────────────────────────────

/// A painter that draws into `inner` moved by `shift` (a whole-pixel
/// translation): the raster's origin is the canvas's top-left, not the
/// device's.
pub(super) struct Shifted<'a> {
    pub(super) inner: &'a mut dyn Painter,
    pub(super) shift: Affine,
}

impl Painter for Shifted<'_> {
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn fill(
        &mut self,
        fill: peniko::Fill,
        transform: Affine,
        brush: &peniko::Brush,
        shape: &super::PaintShape,
    ) {
        self.inner.fill(fill, self.shift * transform, brush, shape);
    }
    fn set_coarse_curves(&mut self, coarse: bool) {
        self.inner.set_coarse_curves(coarse);
    }
    fn fill_color(
        &mut self,
        fill: peniko::Fill,
        transform: Affine,
        color: peniko::color::AlphaColor<peniko::color::Srgb>,
        shape: &super::PaintShape,
    ) {
        self.inner
            .fill_color(fill, self.shift * transform, color, shape);
    }
    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        brush: &peniko::Brush,
        shape: &super::PaintShape,
    ) {
        self.inner
            .stroke(stroke, self.shift * transform, brush, shape);
    }
    fn stroke_color(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        color: peniko::color::AlphaColor<peniko::color::Srgb>,
        shape: &super::PaintShape,
    ) {
        self.inner
            .stroke_color(stroke, self.shift * transform, color, shape);
    }
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        font_size: f32,
        transform: Affine,
        synthesis: super::painter::GlyphSynthesis,
        brush: &peniko::Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[super::painter::PaintGlyph],
    ) {
        self.inner.draw_glyphs(
            font,
            font_size,
            self.shift * transform,
            synthesis,
            brush,
            hint,
            normalized_coords,
            glyphs,
        );
    }
    fn draw_image(&mut self, image: &super::painter::PaintImage<'_>, transform: Affine) {
        self.inner.draw_image(image, self.shift * transform);
    }
    fn draw_gpu_image(&mut self, image: &peniko::ImageData, transform: Affine) {
        self.inner.draw_gpu_image(image, self.shift * transform);
    }
    fn draw_raster(&mut self, image: &RasterImage, x: i32, y: i32, opacity: f32) {
        let t = self.shift.translation();
        self.inner
            .draw_raster(image, x + t.x as i32, y + t.y as i32, opacity);
    }
    fn draw_alpha_mask(
        &mut self,
        mask: &[u8],
        width: u32,
        height: u32,
        color: peniko::color::AlphaColor<peniko::color::Srgb>,
        transform: Affine,
    ) {
        self.inner
            .draw_alpha_mask(mask, width, height, color, self.shift * transform);
    }
    fn fill_repeating(
        &mut self,
        fill: peniko::Fill,
        transform: Affine,
        tile: &super::painter::RepeatTile<'_>,
        shape: &super::PaintShape,
    ) {
        self.inner
            .fill_repeating(fill, self.shift * transform, tile, shape);
    }
    fn erase(&mut self, fill: peniko::Fill, transform: Affine, shape: &super::PaintShape) {
        self.inner.erase(fill, self.shift * transform, shape);
    }
    fn push_clip(&mut self, fill: peniko::Fill, transform: Affine, shape: &super::PaintShape) {
        self.inner.push_clip(fill, self.shift * transform, shape);
    }
    fn push_layer(
        &mut self,
        blend: super::BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &super::PaintShape,
    ) {
        self.inner
            .push_layer(blend, opacity, self.shift * transform, bounds);
    }
    fn push_isolated_layer(&mut self, opacity: f32, transform: Affine, bounds: &super::PaintShape) {
        self.inner
            .push_isolated_layer(opacity, self.shift * transform, bounds);
    }
    fn pop_layer(&mut self) {
        self.inner.pop_layer();
    }
    fn lifts_clips(&self) -> bool {
        self.inner.lifts_clips()
    }
    fn lift_clips(&mut self, n: usize) -> bool {
        self.inner.lift_clips(n)
    }
    fn restore_lifted_clips(&mut self) {
        self.inner.restore_lifted_clips();
    }
}

// ── Applying the list ───────────────────────────────────────────────────────

/// Apply `ops` to the `w` x `h` premultiplied RGBA8 raster, in order.
pub(super) fn apply(pixels: &mut [u8], w: usize, h: usize, ops: &[DeviceOp], edge: Edge) {
    let mut i = 0;
    while i < ops.len() {
        match ops[i] {
            DeviceOp::Blur { sx, sy } => {
                blur_rgba(pixels, w, h, sx, sy, edge);
                i += 1;
            }
            DeviceOp::Shadow { .. } => {
                let shadow = shadow_layer(pixels, w, h, &ops[i..=i]);
                // The source over its shadow.
                for (s, d) in pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(shadow.as_chunks::<4>().0)
                {
                    let inv = 255 - s[3] as u32;
                    if inv == 0 {
                        continue;
                    }
                    for c in 0..4 {
                        s[c] = (s[c] as u32 + (d[c] as u32 * inv + 127) / 255).min(255) as u8;
                    }
                }
                i += 1;
            }
            DeviceOp::Colour(_) => {
                // Consecutive colour functions in one pass over the pixels.
                let start = i;
                while i < ops.len() && matches!(ops[i], DeviceOp::Colour(_)) {
                    i += 1;
                }
                colour_pass(pixels, &ops[start..i]);
            }
        }
    }
}

/// What a list of `drop-shadow()`s alone draws **under** the source: each
/// shadow is of the source with the shadows before it beneath (Filter Effects
/// 1: a function's input is the previous one's output), and the source is
/// left out, so it can be painted over this in place.
fn shadow_layer(source: &[u8], w: usize, h: usize, ops: &[DeviceOp]) -> Vec<u8> {
    let mut under = vec![0_u8; source.len()];
    let shadows = ops
        .iter()
        .filter(|op| matches!(op, DeviceOp::Shadow { .. }))
        .count();
    // The alpha of the source over what is under it so far.
    let mut alpha: Vec<u8> = source.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
    let mut a = Vec::with_capacity(alpha.len());
    for (k, op) in ops.iter().enumerate() {
        let DeviceOp::Shadow {
            dx,
            dy,
            sx,
            sy,
            color,
        } = *op
        else {
            continue;
        };
        a.clear();
        a.extend_from_slice(&alpha);
        blur_channel(&mut a, w, h, sx, sy);
        // The colour, premultiplied, in levels.
        let cc = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u32);
        // The new shadow goes under everything so far: the rows and columns
        // its offset keeps on the raster.
        let (y0, y1) = (dy.max(0) as usize, (h as i32 + dy.min(0)).max(0) as usize);
        let (x0, x1) = (dx.max(0) as usize, (w as i32 + dx.min(0)).max(0) as usize);
        for y in y0..y1.min(h) {
            let src_row = (y as i32 - dy) as usize * w;
            for x in x0..x1.min(w) {
                let m = a[src_row + (x as i32 - dx) as usize] as u32;
                if m == 0 {
                    continue;
                }
                let px = &mut under[(y * w + x) * 4..(y * w + x) * 4 + 4];
                // `under` over the new shadow: it fills only what `under`
                // leaves uncovered.
                let inv = 255 - px[3] as u32;
                if inv == 0 {
                    continue;
                }
                for (p, &c) in px.iter_mut().zip(&cc) {
                    let v = *p as u32 + (c * m * inv + 32_512) / 65_025;
                    *p = v.min(255) as u8;
                }
            }
        }
        // The next shadow is cast by the source over everything so far.
        if k + 1 < ops.len() && shadows > 1 {
            for (i, a) in alpha.iter_mut().enumerate() {
                let s = source[i * 4 + 3] as u32;
                let u = under[i * 4 + 3] as u32;
                *a = (s + (u * (255 - s) + 127) / 255).min(255) as u8;
            }
        }
    }
    under
}

/// What a blur reads past the raster's edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    /// Transparent (`filter`, Chrome's `decal`).
    Decal,
    /// The raster mirrored, its edge pixel repeated: what Chrome's
    /// `backdrop-filter` reads past the element's box (#2719).
    Mirror,
}

/// Where sample `j` of a line of `n` is read under `edge`: `None` for a
/// transparent one.
#[inline]
fn edge_index<const MIRROR: bool>(j: isize, n: usize) -> Option<usize> {
    if j >= 0 && (j as usize) < n {
        return Some(j as usize);
    }
    match MIRROR {
        false => None,
        true => {
            let period = 2 * n as isize;
            let m = j.rem_euclid(period);
            Some(if m < n as isize { m } else { period - 1 - m } as usize)
        }
    }
}

/// A box of `w` samples' reciprocal in 8.24 fixed point, rounded (within
/// Chrome's levels; truncated, the chained case was a level further off).
#[inline]
fn box_scale(w: usize) -> u64 {
    ((1u64 << 24) + w as u64 / 2) / w as u64
}

/// A box's sum as its mean, rounded: Skia's `(sum * scale + ½) >> 24`, a
/// multiply where a division per channel per pixel cost most of a blur.
#[inline]
fn box_mean(sum: u32, scale: u64) -> u8 {
    ((sum as u64 * scale + (1 << 23)) >> 24).min(255) as u8
}

/// One box pass along a line of `n` samples of `C` channels: each output is
/// the rounded mean of the `w` samples starting `w / 2 - shift` before it,
/// read past either end as `edge` says.
fn box_pass<const C: usize, const MIRROR: bool>(
    src: &[[u8; C]],
    dst: &mut [[u8; C]],
    w: usize,
    shift: usize,
) {
    let n = src.len();
    let half = (w / 2) as isize - shift as isize;
    let mut sum = [0_u32; C];
    for j in -half..-half + w as isize {
        if let Some(j) = edge_index::<MIRROR>(j, n) {
            for (s, v) in sum.iter_mut().zip(src[j]) {
                *s += v as u32;
            }
        }
    }
    let scale = box_scale(w);
    for (i, out) in dst.iter_mut().enumerate().take(n) {
        for (o, s) in out.iter_mut().zip(sum) {
            *o = box_mean(s, scale);
        }
        let leave = i as isize - half;
        if let Some(j) = edge_index::<MIRROR>(leave, n) {
            for (s, v) in sum.iter_mut().zip(src[j]) {
                *s -= v as u32;
            }
        }
        if let Some(j) = edge_index::<MIRROR>(leave + w as isize, n) {
            for (s, v) in sum.iter_mut().zip(src[j]) {
                *s += v as u32;
            }
        }
    }
}

/// The same pass down every column of a `w`-wide raster at once, row by
/// row, so memory is read in order: `sums` holds a running sum per column.
fn box_pass_down<const C: usize, const MIRROR: bool>(
    src: &[[u8; C]],
    dst: &mut [[u8; C]],
    w: usize,
    win: usize,
    shift: usize,
    sums: &mut Vec<[u32; C]>,
) {
    let h = src.len() / w;
    let half = (win / 2) as isize - shift as isize;
    sums.clear();
    sums.resize(w, [0; C]);
    let row = |y: usize| &src[y * w..(y + 1) * w];
    for j in -half..-half + win as isize {
        if let Some(j) = edge_index::<MIRROR>(j, h) {
            for (s, p) in sums.iter_mut().zip(row(j)) {
                for (a, &v) in s.iter_mut().zip(p) {
                    *a += v as u32;
                }
            }
        }
    }
    let scale = box_scale(win);
    for y in 0..h {
        for (o, s) in dst[y * w..(y + 1) * w].iter_mut().zip(sums.iter()) {
            for (oc, &sc) in o.iter_mut().zip(s) {
                *oc = box_mean(sc, scale);
            }
        }
        let leave = y as isize - half;
        if let Some(j) = edge_index::<MIRROR>(leave, h) {
            for (s, p) in sums.iter_mut().zip(row(j)) {
                for (a, &v) in s.iter_mut().zip(p) {
                    *a -= v as u32;
                }
            }
        }
        if let Some(j) = edge_index::<MIRROR>(leave + win as isize, h) {
            for (s, p) in sums.iter_mut().zip(row(j)) {
                for (a, &v) in s.iter_mut().zip(p) {
                    *a += v as u32;
                }
            }
        }
    }
}

/// Skia's three-box blur along one line, in place (`tmp` is scratch).
fn blur_line<const C: usize, const MIRROR: bool>(
    line: &mut [[u8; C]],
    tmp: &mut Vec<[u8; C]>,
    d: usize,
) {
    if d <= 1 || line.is_empty() {
        return;
    }
    tmp.clear();
    tmp.resize(line.len(), [0; C]);
    let passes = box_passes(d);
    // line -> tmp -> line -> tmp, then back into line.
    box_pass::<C, MIRROR>(line, tmp, passes[0].0, passes[0].1);
    box_pass::<C, MIRROR>(tmp, line, passes[1].0, passes[1].1);
    box_pass::<C, MIRROR>(line, tmp, passes[2].0, passes[2].1);
    line.copy_from_slice(tmp);
}

/// Blur a `w` x `h` raster of `C`-channel pixels by deviations `sx`, `sy`:
/// three box passes along each row, then three down the columns.
fn blur_planar<const C: usize, const MIRROR: bool>(
    px: &mut [[u8; C]],
    w: usize,
    h: usize,
    sx: f64,
    sy: f64,
) {
    let (dx, dy) = (box_window(sx), box_window(sy));
    if dx > 1 {
        let mut tmp = Vec::new();
        for row in px.chunks_mut(w) {
            blur_line::<C, MIRROR>(row, &mut tmp, dx);
        }
    }
    if dy > 1 && h > 0 {
        let mut tmp = vec![[0_u8; C]; w * h];
        let mut sums = Vec::new();
        let passes = box_passes(dy);
        box_pass_down::<C, MIRROR>(px, &mut tmp, w, passes[0].0, passes[0].1, &mut sums);
        box_pass_down::<C, MIRROR>(&tmp, px, w, passes[1].0, passes[1].1, &mut sums);
        box_pass_down::<C, MIRROR>(px, &mut tmp, w, passes[2].0, passes[2].1, &mut sums);
        px.copy_from_slice(&tmp);
    }
}

/// The blur over premultiplied RGBA, one copy of the passes per edge mode
/// (the mode a constant inside each, so the decal one pays nothing for the
/// mirror's indexing).
fn blur_rgba(pixels: &mut [u8], w: usize, h: usize, sx: f64, sy: f64, edge: Edge) {
    let px = pixels.as_chunks_mut::<4>().0;
    match edge {
        Edge::Decal => blur_planar::<4, false>(px, w, h, sx, sy),
        Edge::Mirror => blur_planar::<4, true>(px, w, h, sx, sy),
    }
}

fn blur_channel(a: &mut [u8], w: usize, h: usize, sx: f64, sy: f64) {
    blur_planar::<1, false>(a.as_chunks_mut::<1>().0, w, h, sx, sy);
}

/// The colour functions `ops` (all [`DeviceOp::Colour`]) over every pixel:
/// straightened, each applied in order and clamped, premultiplied again.
fn colour_pass(pixels: &mut [u8], ops: &[DeviceOp]) {
    for px in pixels.as_chunks_mut::<4>().0 {
        // No CSS colour function makes a transparent pixel visible: each
        // leaves alpha alone or scales it.
        if px[3] == 0 {
            continue;
        }
        let a = px[3] as f32 / 255.0;
        let mut c = if a > 0.0 {
            [
                px[0] as f32 / 255.0 / a,
                px[1] as f32 / 255.0 / a,
                px[2] as f32 / 255.0 / a,
                a,
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };
        for op in ops {
            if let DeviceOp::Colour(f) = op {
                c = colour_op(*f, c);
            }
        }
        let a = c[3].clamp(0.0, 1.0);
        for i in 0..3 {
            px[i] = (c[i].clamp(0.0, 1.0) * a * 255.0).round() as u8;
        }
        px[3] = (a * 255.0).round() as u8;
    }
}

/// One colour function on a straight-alpha colour (Filter Effects 1 §13, in
/// gamma-encoded sRGB, as Chrome draws them).
pub(super) fn colour_op(f: FilterOp, [r, g, b, a]: [f32; 4]) -> [f32; 4] {
    let matrix = |m: [[f32; 3]; 3]| {
        [
            (m[0][0] * r + m[0][1] * g + m[0][2] * b).clamp(0.0, 1.0),
            (m[1][0] * r + m[1][1] * g + m[1][2] * b).clamp(0.0, 1.0),
            (m[2][0] * r + m[2][1] * g + m[2][2] * b).clamp(0.0, 1.0),
            a,
        ]
    };
    match f {
        FilterOp::Brightness(k) => [
            (r * k).clamp(0.0, 1.0),
            (g * k).clamp(0.0, 1.0),
            (b * k).clamp(0.0, 1.0),
            a,
        ],
        FilterOp::Contrast(k) => {
            let i = 0.5 - 0.5 * k;
            [
                (r * k + i).clamp(0.0, 1.0),
                (g * k + i).clamp(0.0, 1.0),
                (b * k + i).clamp(0.0, 1.0),
                a,
            ]
        }
        FilterOp::Invert(k) => {
            let k = k.clamp(0.0, 1.0);
            [
                k + r * (1.0 - 2.0 * k),
                k + g * (1.0 - 2.0 * k),
                k + b * (1.0 - 2.0 * k),
                a,
            ]
        }
        FilterOp::Opacity(k) => [r, g, b, a * k.clamp(0.0, 1.0)],
        FilterOp::Grayscale(k) => {
            let s = 1.0 - k.clamp(0.0, 1.0);
            matrix([
                [
                    0.2126 + 0.7874 * s,
                    0.7152 - 0.7152 * s,
                    0.0722 - 0.0722 * s,
                ],
                [
                    0.2126 - 0.2126 * s,
                    0.7152 + 0.2848 * s,
                    0.0722 - 0.0722 * s,
                ],
                [
                    0.2126 - 0.2126 * s,
                    0.7152 - 0.7152 * s,
                    0.0722 + 0.9278 * s,
                ],
            ])
        }
        FilterOp::Sepia(k) => {
            let s = 1.0 - k.clamp(0.0, 1.0);
            matrix([
                [0.393 + 0.607 * s, 0.769 - 0.769 * s, 0.189 - 0.189 * s],
                [0.349 - 0.349 * s, 0.686 + 0.314 * s, 0.168 - 0.168 * s],
                [0.272 - 0.272 * s, 0.534 - 0.534 * s, 0.131 + 0.869 * s],
            ])
        }
        FilterOp::Saturate(s) => matrix([
            [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
        ]),
        FilterOp::HueRotate(deg) => {
            let (sin, cos) = deg.to_radians().sin_cos();
            matrix([
                [
                    0.213 + cos * 0.787 - sin * 0.213,
                    0.715 - cos * 0.715 - sin * 0.715,
                    0.072 - cos * 0.072 + sin * 0.928,
                ],
                [
                    0.213 - cos * 0.213 + sin * 0.143,
                    0.715 + cos * 0.285 + sin * 0.140,
                    0.072 - cos * 0.072 - sin * 0.283,
                ],
                [
                    0.213 - cos * 0.213 - sin * 0.787,
                    0.715 - cos * 0.715 + sin * 0.715,
                    0.072 + cos * 0.928 + sin * 0.072,
                ],
            ])
        }
        FilterOp::Blur(_) | FilterOp::DropShadow { .. } => [r, g, b, a],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrome 157's software raster of `rgb(200, 100, 50)` under each colour
    /// function (`.scratch` probe, `#c1`–`#c9`), to the level.
    #[test]
    fn colour_functions_match_chrome() {
        let base = [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 1.0];
        let cases: &[(FilterOp, [u8; 3])] = &[
            (FilterOp::Brightness(0.5), [100, 50, 25]),
            (FilterOp::Contrast(1.7), [251, 81, 0]),
            (FilterOp::Saturate(0.3), [142, 112, 97]),
            (FilterOp::HueRotate(77.0), [70, 141, 22]),
            (FilterOp::Sepia(0.6), [179, 128, 89]),
            (FilterOp::Invert(0.3), [156, 116, 96]),
            (FilterOp::Grayscale(0.7), [142, 112, 97]),
            (FilterOp::Brightness(1.6), [255, 160, 80]),
        ];
        for &(f, want) in cases {
            let c = colour_op(f, base);
            let got = [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8);
            for i in 0..3 {
                assert!(
                    (got[i] as i32 - want[i] as i32).abs() <= 1,
                    "{f:?}: {got:?}, Chrome {want:?}"
                );
            }
        }
    }

    /// Skia's windows: odd three times, even twice plus one wider.
    #[test]
    fn box_windows_follow_skia() {
        assert_eq!(box_window(5.0), 9);
        assert_eq!(box_window(1.3), 2);
        assert_eq!(box_window(13.0), 24);
        assert_eq!(box_passes(9), [(9, 0), (9, 0), (9, 0)]);
        assert_eq!(box_passes(24), [(24, 0), (24, 1), (25, 0)]);
        // A pixel's value reaches no further than the reach says.
        for sigma in [0.6, 1.3, 2.0, 5.0, 6.5, 13.0] {
            let n = 200;
            let mut line = vec![[0_u8; 1]; n];
            line[n / 2] = [255];
            let mut tmp = Vec::new();
            blur_line::<1, false>(&mut line, &mut tmp, box_window(sigma));
            let r = blur_reach(sigma);
            for (i, v) in line.iter().enumerate() {
                if (i as isize - (n / 2) as isize).unsigned_abs() > r {
                    assert_eq!(v[0], 0, "sigma {sigma}: {i} is past the reach {r}");
                }
            }
        }
    }
}
