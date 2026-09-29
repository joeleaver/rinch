//! Painting an element's background: the colour, and the first image layer
//! sized, positioned and tiled by `background-size`, `background-position`,
//! `background-repeat` and `background-origin` (#468).
//!
//! Before #468 none of those four reached `ComputedStyle`, and an image layer
//! was stretched over the border box. That is also what this module still
//! does for the case where it is right — an `auto`-sized image at `0 0` in a
//! box with no border — so the common gradient pays nothing extra: one fill,
//! no clip.
//!
//! What it models (css-backgrounds-3):
//!
//! - **The positioning area** is the `background-origin` box (§3.7; initial
//!   `padding-box`). The **painting area** is the border box: rinch reads no
//!   `background-clip`, which is therefore always its initial `border-box`.
//! - **The tile size** (§3.9). A gradient has no intrinsic size or ratio, so an
//!   `auto` axis, `cover` and `contain` are all the positioning area's; an
//!   image's `auto` is its intrinsic size (or keeps its ratio against the
//!   other axis), and `cover`/`contain` scale that ratio.
//! - **The position** (§3.6): a percentage is a share of the positioning area
//!   **less** the tile, so `100%` puts the tile's far edge on the area's.
//! - **Repetition** (§3.4): `repeat` tiles in both directions from the
//!   positioned tile across the painting area; `no-repeat` draws the one.
//!   `space` and `round` are painted as `repeat` — their spacing and rescaling
//!   are not modelled.
//!
//! # How a layer is drawn
//!
//! 1. **One tile covering the box** — every `auto` gradient at `0 0` without a
//!    border — is one fill into the box's shape, no clip.
//! 2. **Anything that repeats** is one [`Painter::fill_repeating`]: the tile is
//!    rasterised once, at the resolution it lands on the device (a `scale()`d
//!    box rasterises bigger, a 12.5-device-pixel tile rasterises at 13 and is
//!    sampled back down bilinearly), and filled as a repeating pattern over the
//!    shape. An axis that does not repeat clips the pattern to the one-tile
//!    strip. The raster is cached by what determines its pixels, so a frame
//!    that does not change the layer does not re-rasterise it (counters
//!    `background_tile_rasters` / `background_tile_cache_hits`).
//! 3. **Otherwise tile by tile**, over only the tiles that can be seen (the
//!    window, the open clips and a partial repaint's damage): a `no-repeat`
//!    tile that does not cover the box, a tile too large to rasterise, and
//!    every repeating layer in a build without the software painter (`embed`),
//!    which has nothing to rasterise a tile with. Never skipped, however many
//!    tiles that is.
//!
//! **Positions are whole device pixels** (tracked separately as #1151): the
//! tile phase is rounded in device space, so tiles meet on pixel edges and a
//! `background-position` animation moves in one-device-pixel steps. Chrome
//! keeps the fractional phase and blends the edge pixel, so its stripes move
//! smoothly; rinch's are within one device pixel of it at every frame.
//!
//! Only the **first** layer is read: a comma-separated `background-image`
//! list paints its first image. The `background-color` is painted under it
//! ([`ComputedStyle::background_underlay`](crate::computed_style::ComputedStyle)).

#[cfg(feature = "software-renderer")]
use std::cell::RefCell;
#[cfg(feature = "software-renderer")]
use std::collections::HashMap;
#[cfg(feature = "software-renderer")]
use std::hash::{Hash, Hasher};

#[cfg(feature = "software-renderer")]
use peniko::Blob;
use peniko::Fill;
use peniko::kurbo::{Affine, Rect};

#[cfg(feature = "software-renderer")]
use super::painter::RepeatTile;
use super::painter::{PaintShape, Painter};
use super::svg::{build_linear_gradient_brush, build_radial_gradient_brush};
use crate::computed_style::{
    BackgroundOriginValue, BackgroundRepeatValue, BackgroundSizeValue, BackgroundValue,
    BorderStyleValue, ComputedStyle, LengthPercentageAutoValue, LengthPercentageValue,
    ObjectFitValue,
};
use crate::node::NodeTree;
#[cfg(feature = "software-renderer")]
use crate::perf::Counter;

/// Paint the element's background into `shape` (its border box, rounded or
/// with viewport holes cut out, already built by the caller), whose bounding
/// rect is `rect`.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_background(
    painter: &mut dyn Painter,
    tree: &NodeTree,
    cs: &ComputedStyle,
    scale: f64,
    rect: Rect,
    fill: Fill,
    shape: &PaintShape,
    transform: Affine,
) {
    match &cs.background {
        BackgroundValue::None => {}
        BackgroundValue::Color(c) => painter.fill_color(fill, transform, *c, shape),
        image => {
            if let Some(under) = cs.background_underlay {
                painter.fill_color(fill, transform, under, shape);
            }
            let intrinsic = match image {
                BackgroundValue::Image { url } => match tree.image_cache.get(url) {
                    Some(decoded) => {
                        Some((decoded.width as f64 * scale, decoded.height as f64 * scale))
                    }
                    // Not loaded yet: nothing to draw.
                    None => return,
                },
                _ => None,
            };
            let Some(layer) = layer_geometry(cs, scale, rect, intrinsic) else {
                return;
            };
            let snap = DeviceSnap::of(transform);

            // 1. One tile covering the whole painting area needs no clip:
            // the shape is the clip. A gradient is filled *into the shape*,
            // so a tile that covers it is enough; an image is drawn into its
            // tile rect, so only a tile that *is* the shape (a plain rect).
            if let Some(tile) = layer.covering_tile(rect, snap)
                && match image {
                    BackgroundValue::Image { .. } => {
                        matches!(shape, PaintShape::Rect(_)) && same_rect(tile, rect)
                    }
                    _ => true,
                }
            {
                draw_tile(painter, tree, image, scale, fill, transform, tile, shape);
                return;
            }

            // 2. A repeating layer is one pattern (#468's review, F2/N1).
            if (layer.repeats_x || layer.repeats_y)
                && fill_as_pattern(
                    painter, tree, image, scale, fill, transform, &layer, rect, shape, snap,
                )
            {
                return;
            }

            // 3. Tile by tile, over the tiles that can be seen.
            let visible = visible_in_shape_space(rect, transform);
            let tiles = layer.tiles(visible, snap);
            if tiles.is_empty() {
                return;
            }
            painter.push_clip(fill, transform, shape);
            for tile in &tiles {
                let target = PaintShape::Rect(tile.intersect(rect));
                draw_tile(painter, tree, image, scale, fill, transform, *tile, &target);
            }
            painter.pop_layer();
        }
    }
}

/// The part of `rect` (shape space) that can be seen: the window, the open
/// clips and a partial repaint's damage ([`super::visible_paint_rect`], device
/// space), mapped back through `transform`. `rect` itself when none is known
/// or the transform cannot be inverted.
fn visible_in_shape_space(rect: Rect, transform: Affine) -> Rect {
    let Some(device) = super::visible_paint_rect() else {
        return rect;
    };
    if transform.determinant().abs() < 1e-12 {
        return rect;
    }
    let back = transform.inverse().transform_rect_bbox(device);
    let v = rect.intersect(back);
    if v.width() > 0.0 && v.height() > 0.0 {
        v
    } else {
        Rect::ZERO
    }
}

/// How a shape-space coordinate is snapped to a device pixel edge (#468's
/// review, F1/N2). Under a plain translation `(tx, ty)` a shape-space `x` is
/// device `x + tx`, so it snaps as `round(x + tx) - tx` — snapping `x` alone
/// left a fractional translate's edges mid-pixel, the 75% seam again. Under
/// any other transform there is no pixel grid in shape space to snap to.
#[derive(Clone, Copy)]
pub(super) enum DeviceSnap {
    Translate(f64, f64),
    None,
}

impl DeviceSnap {
    fn of(transform: Affine) -> Self {
        let m = transform.as_coeffs();
        if (m[0] - 1.0).abs() < 1e-9
            && m[1].abs() < 1e-9
            && m[2].abs() < 1e-9
            && (m[3] - 1.0).abs() < 1e-9
        {
            Self::Translate(m[4], m[5])
        } else {
            Self::None
        }
    }
    fn x(self, v: f64) -> f64 {
        match self {
            Self::Translate(tx, _) => (v + tx).round() - tx,
            Self::None => v,
        }
    }
    fn y(self, v: f64) -> f64 {
        match self {
            Self::Translate(_, ty) => (v + ty).round() - ty,
            Self::None => v,
        }
    }
}

/// Draw one tile of the layer: a gradient filled into `target` with the
/// gradient's geometry laid on `tile`, or the image drawn into `tile`.
#[allow(clippy::too_many_arguments)]
fn draw_tile(
    painter: &mut dyn Painter,
    tree: &NodeTree,
    image: &BackgroundValue,
    scale: f64,
    fill: Fill,
    transform: Affine,
    tile: Rect,
    target: &PaintShape,
) {
    match image {
        BackgroundValue::LinearGradient {
            angle_degrees,
            stops,
        } => {
            let brush = build_linear_gradient_brush(*angle_degrees, stops, &tile);
            painter.fill(fill, transform, &brush, target);
        }
        BackgroundValue::RadialGradient { stops } => {
            let brush = build_radial_gradient_brush(stops, &tile);
            painter.fill(fill, transform, &brush, target);
        }
        BackgroundValue::Image { url } => {
            if let Some(decoded) = tree.image_cache.get(url) {
                super::image::paint_image(
                    painter,
                    decoded,
                    tile,
                    scale,
                    ObjectFitValue::Fill,
                    transform,
                );
            }
        }
        BackgroundValue::None | BackgroundValue::Color(_) => {}
    }
}

/// The largest tile, in device pixels, the pattern path rasterises. A larger
/// one (a 2000px image tiled at its natural size on a 3x display) is drawn
/// tile by tile instead: there are few of them.
#[cfg(feature = "software-renderer")]
const MAX_PATTERN_TILE_PX: f64 = 4.0 * 1024.0 * 1024.0;

#[cfg(feature = "software-renderer")]
/// Rasterised tiles, keyed by what decides their pixels: the image (the
/// gradient's angle and stops, or the url and the decoded image's identity)
/// and the raster size. Bounded by [`TILE_CACHE_BUDGET`] bytes; cleared when
/// a new entry would pass it.
struct TileCache {
    entries: HashMap<(u64, u32, u32), Blob<u8>>,
    bytes: usize,
}

#[cfg(feature = "software-renderer")]
const TILE_CACHE_BUDGET: usize = 32 * 1024 * 1024;

#[cfg(feature = "software-renderer")]
thread_local! {
    static TILE_CACHE: RefCell<TileCache> = RefCell::new(TileCache {
        entries: HashMap::new(),
        bytes: 0,
    });
}

#[cfg(feature = "software-renderer")]
/// What decides a rasterised tile's pixels, besides its size.
fn image_key(tree: &NodeTree, image: &BackgroundValue) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let stops_hash = |h: &mut std::collections::hash_map::DefaultHasher,
                      stops: &[crate::computed_style::GradientStop]| {
        for s in stops {
            s.offset.to_bits().hash(h);
            match s.color {
                Some(c) => {
                    for v in c.components {
                        v.to_bits().hash(h);
                    }
                }
                None => 0u8.hash(h),
            }
        }
    };
    match image {
        BackgroundValue::LinearGradient {
            angle_degrees,
            stops,
        } => {
            1u8.hash(&mut h);
            angle_degrees.to_bits().hash(&mut h);
            stops_hash(&mut h, stops);
        }
        BackgroundValue::RadialGradient { stops } => {
            2u8.hash(&mut h);
            stops_hash(&mut h, stops);
        }
        BackgroundValue::Image { url } => {
            3u8.hash(&mut h);
            url.hash(&mut h);
            if let Some(d) = tree.image_cache.get(url) {
                (d.data.as_ptr() as usize, d.width, d.height).hash(&mut h);
            }
        }
        BackgroundValue::None | BackgroundValue::Color(_) => 0u8.hash(&mut h),
    }
    h.finish()
}

/// Paint the layer as one repeating pattern (steps 2 of the module doc).
/// `false` when it declined — no software painter to rasterise with, or a
/// tile too large — and nothing was drawn.
#[allow(clippy::too_many_arguments)]
fn fill_as_pattern(
    painter: &mut dyn Painter,
    tree: &NodeTree,
    image: &BackgroundValue,
    scale: f64,
    fill: Fill,
    transform: Affine,
    layer: &LayerGeometry,
    rect: Rect,
    shape: &PaintShape,
    snap: DeviceSnap,
) -> bool {
    #[cfg(feature = "software-renderer")]
    {
        use super::skia_painter::TinySkiaPainter;
        let m = transform.as_coeffs();
        let (tw, th) = layer.size;
        // The device size of one tile, and the raster that covers it.
        let dw = tw * m[0].hypot(m[1]);
        let dh = th * m[2].hypot(m[3]);
        if !(dw > 0.0 && dh > 0.0) {
            return false;
        }
        let w = (dw - 1e-6).ceil().max(1.0);
        let h = (dh - 1e-6).ceil().max(1.0);
        if w * h > MAX_PATTERN_TILE_PX {
            return false;
        }
        let (w, h) = (w as u32, h as u32);
        let key = (image_key(tree, image), w, h);
        let cached = TILE_CACHE.with(|c| c.borrow().entries.get(&key).cloned());
        let pixels = match cached {
            Some(blob) => {
                tree.perf.bump(Counter::BackgroundTileCacheHits);
                blob
            }
            None => {
                tree.perf.bump(Counter::BackgroundTileRasters);
                let mut raster = TinySkiaPainter::new(w, h);
                let unit = Rect::new(0.0, 0.0, w as f64, h as f64);
                draw_tile(
                    &mut raster,
                    tree,
                    image,
                    scale,
                    Fill::NonZero,
                    Affine::IDENTITY,
                    unit,
                    &PaintShape::Rect(unit),
                );
                let blob = Blob::from(raster.pixels().to_vec());
                TILE_CACHE.with(|c| {
                    let mut c = c.borrow_mut();
                    let n = blob.len();
                    if c.bytes + n > TILE_CACHE_BUDGET {
                        c.entries.clear();
                        c.bytes = 0;
                    }
                    c.bytes += n;
                    c.entries.insert(key, blob.clone());
                });
                blob
            }
        };
        // Tile (0, 0) at the positioned tile's device-snapped origin, the
        // raster scaled back to the tile's shape-space size.
        let origin = (snap.x(layer.origin.0), snap.y(layer.origin.1));
        let exact = (dw - w as f64).abs() < 1e-6 && (dh - h as f64).abs() < 1e-6;
        let tile = RepeatTile {
            pixels: &pixels,
            width: w,
            height: h,
            transform: Affine::translate(origin)
                * Affine::scale_non_uniform(tw / w as f64, th / h as f64),
            smooth: !(exact && matches!(snap, DeviceSnap::Translate(..))),
        };
        // An axis that does not repeat: the pattern is clipped to the strip
        // the one tile occupies on it.
        let strip = if layer.repeats_x && layer.repeats_y {
            None
        } else {
            let (x0, x1) = if layer.repeats_x {
                (rect.x0, rect.x1)
            } else {
                (origin.0, snap.x(layer.origin.0 + tw))
            };
            let (y0, y1) = if layer.repeats_y {
                (rect.y0, rect.y1)
            } else {
                (origin.1, snap.y(layer.origin.1 + th))
            };
            Some(Rect::new(x0, y0, x1, y1))
        };
        if let Some(strip) = strip {
            painter.push_clip(Fill::NonZero, transform, &PaintShape::Rect(strip));
        }
        painter.fill_repeating(fill, transform, &tile, shape);
        if strip.is_some() {
            painter.pop_layer();
        }
        true
    }
    #[cfg(not(feature = "software-renderer"))]
    {
        let _ = (
            painter, tree, image, scale, fill, transform, layer, rect, shape, snap,
        );
        false
    }
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 1e-6
        && (a.y0 - b.y0).abs() < 1e-6
        && (a.x1 - b.x1).abs() < 1e-6
        && (a.y1 - b.y1).abs() < 1e-6
}

/// Where the first image layer's tiles go: the positioned tile's origin and
/// size, in the same (scaled) space as the border box, and which axes repeat.
pub(super) struct LayerGeometry {
    pub origin: (f64, f64),
    pub size: (f64, f64),
    pub repeats_x: bool,
    pub repeats_y: bool,
}

/// `None` when there is nothing to draw (a zero-sized tile). `intrinsic` is
/// the image's size in the border box's space; `None` for a gradient.
pub(super) fn layer_geometry(
    cs: &ComputedStyle,
    scale: f64,
    rect: Rect,
    intrinsic: Option<(f64, f64)>,
) -> Option<LayerGeometry> {
    let area = positioning_area(cs, scale, rect);
    let (aw, ah) = (area.width(), area.height());
    let (tw, th) = tile_size(cs.background_size, scale, aw, ah, intrinsic);
    if !(tw > 0.0 && th > 0.0 && tw.is_finite() && th.is_finite()) {
        return None;
    }
    Some(LayerGeometry {
        origin: (
            area.x0 + resolve_lp(cs.background_position_x, scale, aw - tw),
            area.y0 + resolve_lp(cs.background_position_y, scale, ah - th),
        ),
        size: (tw, th),
        repeats_x: !matches!(cs.background_repeat_x, BackgroundRepeatValue::NoRepeat),
        repeats_y: !matches!(cs.background_repeat_y, BackgroundRepeatValue::NoRepeat),
    })
}

impl LayerGeometry {
    /// The start of the first tile, along one axis, whose far edge is past
    /// `lo` — the tile covering `lo` when the axis repeats.
    fn first_start(origin: f64, size: f64, lo: f64, repeat: bool) -> f64 {
        if repeat {
            origin - ((origin - lo) / size).ceil() * size
        } else {
            origin
        }
    }

    /// The one tile, snapped, when it alone covers all of `rect`.
    fn covering_tile(&self, rect: Rect, snap: DeviceSnap) -> Option<Rect> {
        let (tw, th) = self.size;
        let x = Self::first_start(self.origin.0, tw, rect.x0, self.repeats_x);
        let y = Self::first_start(self.origin.1, th, rect.y0, self.repeats_y);
        let tile = Rect::new(snap.x(x), snap.y(y), snap.x(x + tw), snap.y(y + th));
        (tile.x0 <= rect.x0 && tile.y0 <= rect.y0 && tile.x1 >= rect.x1 && tile.y1 >= rect.y1)
            .then_some(tile)
    }

    /// Every tile that touches `visible` (shape space), each edge **snapped to
    /// device pixels** (#468's review, F1/N2): two neighbours share one
    /// whole-pixel edge. Filled as two antialiased rects meeting mid-pixel,
    /// each covered that pixel by half and they composited to 75% — a
    /// page-coloured seam at every tile boundary.
    pub(super) fn tiles(&self, visible: Rect, snap: DeviceSnap) -> Vec<Rect> {
        let (tw, th) = self.size;
        if visible.width() <= 0.0 || visible.height() <= 0.0 {
            return Vec::new();
        }
        let starts = |origin: f64, size: f64, lo: f64, hi: f64, repeat: bool| {
            let first = Self::first_start(origin, size, lo, repeat);
            if !repeat {
                return vec![first];
            }
            let count = ((hi - first) / size).ceil().max(0.0) as usize;
            (0..count)
                .map(|i| first + i as f64 * size)
                .collect::<Vec<_>>()
        };
        let xs = starts(self.origin.0, tw, visible.x0, visible.x1, self.repeats_x);
        let ys = starts(self.origin.1, th, visible.y0, visible.y1, self.repeats_y);
        let mut tiles = Vec::with_capacity(xs.len() * ys.len());
        for &y in &ys {
            for &x in &xs {
                let tile = Rect::new(snap.x(x), snap.y(y), snap.x(x + tw), snap.y(y + th));
                if tile.width() > 0.0
                    && tile.height() > 0.0
                    && tile.x1 > visible.x0
                    && tile.x0 < visible.x1
                    && tile.y1 > visible.y0
                    && tile.y0 < visible.y1
                {
                    tiles.push(tile);
                }
            }
        }
        tiles
    }
}

/// The `background-origin` box. Border widths count only where the side has a
/// style (a `none` border is 0 wide); padding percentages resolve to 0, as in
/// the rest of paint (they need the containing block).
fn positioning_area(cs: &ComputedStyle, scale: f64, rect: Rect) -> Rect {
    if matches!(cs.background_origin, BackgroundOriginValue::BorderBox) {
        return rect;
    }
    let bw = |w: &LengthPercentageValue, s: BorderStyleValue| {
        if matches!(s, BorderStyleValue::None | BorderStyleValue::Hidden) {
            0.0
        } else {
            w.to_px().max(0.0) as f64 * scale
        }
    };
    let mut l = bw(&cs.border_left_width, cs.border_left_style);
    let mut t = bw(&cs.border_top_width, cs.border_top_style);
    let mut r = bw(&cs.border_right_width, cs.border_right_style);
    let mut b = bw(&cs.border_bottom_width, cs.border_bottom_style);
    if matches!(cs.background_origin, BackgroundOriginValue::ContentBox) {
        l += cs.padding_left.to_px().max(0.0) as f64 * scale;
        t += cs.padding_top.to_px().max(0.0) as f64 * scale;
        r += cs.padding_right.to_px().max(0.0) as f64 * scale;
        b += cs.padding_bottom.to_px().max(0.0) as f64 * scale;
    }
    let x0 = rect.x0 + l;
    let y0 = rect.y0 + t;
    Rect::new(x0, y0, (rect.x1 - r).max(x0), (rect.y1 - b).max(y0))
}

/// css-backgrounds-3 §3.9 for one layer.
fn tile_size(
    size: BackgroundSizeValue,
    scale: f64,
    aw: f64,
    ah: f64,
    intrinsic: Option<(f64, f64)>,
) -> (f64, f64) {
    let ratio = intrinsic.and_then(|(w, h)| (w > 0.0 && h > 0.0).then_some(w / h));
    match size {
        BackgroundSizeValue::Cover | BackgroundSizeValue::Contain => match ratio {
            None => (aw, ah),
            Some(r) => {
                let by_width = (aw, aw / r);
                let by_height = (ah * r, ah);
                let cover = matches!(size, BackgroundSizeValue::Cover);
                // `cover` takes the larger of the two fits, `contain` the smaller.
                if (by_width.1 >= ah) == cover {
                    by_width
                } else {
                    by_height
                }
            }
        },
        BackgroundSizeValue::Explicit { width, height } => {
            let w = resolve_lpa(width, scale, aw);
            let h = resolve_lpa(height, scale, ah);
            match (w, h, intrinsic, ratio) {
                (Some(w), Some(h), _, _) => (w, h),
                (Some(w), None, _, Some(r)) => (w, w / r),
                (None, Some(h), _, Some(r)) => (h * r, h),
                (None, None, Some((iw, ih)), _) => (iw, ih),
                // No ratio to keep (a gradient): the auto axis is the area's.
                (w, h, _, _) => (w.unwrap_or(aw), h.unwrap_or(ah)),
            }
        }
    }
}

fn resolve_lp(v: LengthPercentageValue, scale: f64, basis: f64) -> f64 {
    match v {
        LengthPercentageValue::Zero => 0.0,
        LengthPercentageValue::Length(px) => px as f64 * scale,
        LengthPercentageValue::Percent(p) => p as f64 * basis,
        LengthPercentageValue::Calc { px, pct } => px as f64 * scale + pct as f64 * basis,
    }
}

fn resolve_lpa(v: LengthPercentageAutoValue, scale: f64, basis: f64) -> Option<f64> {
    match v {
        LengthPercentageAutoValue::Auto => None,
        LengthPercentageAutoValue::Length(px) => Some(px as f64 * scale),
        LengthPercentageAutoValue::Percent(p) => Some(p as f64 * basis),
        LengthPercentageAutoValue::Calc { px, pct } => Some(px as f64 * scale + pct as f64 * basis),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #468's review, N2: under a translation with a fractional part the
    /// tile-by-tile path's edges land on whole **device** pixels. Snapping in
    /// shape space left them mid-pixel (a seam). This path is reached by a
    /// build without the software painter and by a tile too large to
    /// rasterise, neither of which a pixel fixture reaches, so the geometry
    /// is pinned here.
    #[test]
    fn tile_edges_snap_in_device_space_under_a_fractional_translate() {
        let layer = LayerGeometry {
            origin: (0.0, 0.0),
            size: (10.0, 20.0),
            repeats_x: true,
            repeats_y: false,
        };
        let snap = DeviceSnap::of(Affine::translate((10.5, 0.25)));
        let tiles = layer.tiles(Rect::new(0.0, 0.0, 100.0, 20.0), snap);
        assert!(tiles.len() >= 10);
        for t in tiles {
            for (v, tx) in [(t.x0, 10.5), (t.x1, 10.5), (t.y0, 0.25), (t.y1, 0.25)] {
                let device = v + tx;
                assert!(
                    (device - device.round()).abs() < 1e-9,
                    "{t:?}: edge at device {device}"
                );
            }
        }
    }
}
