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
//! Only the **first** layer is read: a comma-separated `background-image`
//! list paints its first image. The `background-color` is painted under it
//! ([`ComputedStyle::background_underlay`](crate::computed_style::ComputedStyle)).

use peniko::Fill;
use peniko::kurbo::{Affine, Rect};

use super::painter::{PaintShape, Painter};
use super::svg::{build_linear_gradient_brush, build_radial_gradient_brush};
use crate::computed_style::{
    BackgroundOriginValue, BackgroundRepeatValue, BackgroundSizeValue, BackgroundValue,
    BorderStyleValue, ComputedStyle, LengthPercentageAutoValue, LengthPercentageValue,
    ObjectFitValue,
};
use crate::node::NodeTree;

/// More tiles than this on the tile-by-tile path and the layer is not drawn
/// (the colour under it still is). It bounds the cost of a tiny tile on a
/// huge box (a `1px` stripe across a 4K scroller is ~8 million fills). Only a
/// layer the pattern path declines reaches it — a transformed box, a tile
/// that is not a whole number of pixels, or `repeat-x`/`repeat-y`; Chrome
/// has no such limit.
pub(super) const MAX_BACKGROUND_TILES: usize = 4096;

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
            let tiles = layer.tiles(rect, MAX_BACKGROUND_TILES);
            // One tile covering the whole painting area needs no clip: the
            // shape is the clip. That is every `auto`-sized layer at `0 0` in
            // a box without a border, which is what every gradient was before.
            //
            // A gradient is filled *into the shape*, so a tile that covers it
            // is enough. An image is drawn into its tile rect, so skipping the
            // clip is only safe when the tile *is* the shape: a plain rect.
            if let Some(tiles) = &tiles
                && tiles.len() == 1
                && match image {
                    BackgroundValue::Image { .. } => {
                        matches!(shape, PaintShape::Rect(_)) && same_rect(tiles[0], rect)
                    }
                    _ => contains(tiles[0], rect),
                }
            {
                draw_tile(
                    painter, tree, image, scale, fill, transform, tiles[0], shape,
                );
                return;
            }
            // More than one tile: one repeating pattern where the layer can be
            // expressed as one (#468's review, F2) — its cost does not grow
            // with the tile count, and there is no cap to fall off.
            if tiles.as_ref().is_none_or(|t| t.len() > 1)
                && fill_as_pattern(painter, tree, image, scale, fill, transform, &layer, shape)
            {
                return;
            }
            // Otherwise tile by tile. Past the cap the layer is not drawn at
            // all (the colour under it still is): drawing it as one stretched
            // tile, as this once did, is a different picture.
            let Some(tiles) = tiles else {
                return;
            };
            painter.push_clip(fill, transform, shape);
            for tile in &tiles {
                let target = PaintShape::Rect(tile.intersect(rect));
                draw_tile(painter, tree, image, scale, fill, transform, *tile, &target);
            }
            painter.pop_layer();
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

/// The largest tile, in device pixels, the pattern path rasterises.
#[cfg(feature = "software-renderer")]
const MAX_PATTERN_TILE_PX: f64 = 4.0 * 1024.0 * 1024.0;

/// Paint the layer as one repeating pattern: rasterise one tile with the
/// software painter, then [`Painter::fill_repeating`] it over the shape. Both
/// painters sample the same pixels, so they agree.
///
/// Only where that is exact: both axes repeat, the fill's transform is a
/// plain translation (a tile rasterised in shape space is then rasterised at
/// device resolution), and the tile is a whole number of pixels in each axis
/// (a fractional tile snaps to alternating widths, which one pattern cannot
/// express). `false` when it declined, and nothing was drawn.
#[allow(clippy::too_many_arguments)]
fn fill_as_pattern(
    painter: &mut dyn Painter,
    tree: &NodeTree,
    image: &BackgroundValue,
    scale: f64,
    fill: Fill,
    transform: Affine,
    layer: &LayerGeometry,
    shape: &PaintShape,
) -> bool {
    #[cfg(feature = "software-renderer")]
    {
        use super::skia_painter::TinySkiaPainter;
        let m = transform.as_coeffs();
        let translation_only = (m[0] - 1.0).abs() < 1e-9
            && m[1].abs() < 1e-9
            && m[2].abs() < 1e-9
            && (m[3] - 1.0).abs() < 1e-9;
        let (tw, th) = layer.size;
        let whole = |v: f64| (v - v.round()).abs() < 1e-6 && v.round() >= 1.0;
        if !(layer.repeats_x
            && layer.repeats_y
            && translation_only
            && whole(tw)
            && whole(th)
            && tw * th <= MAX_PATTERN_TILE_PX)
        {
            return false;
        }
        let (w, h) = (tw.round() as u32, th.round() as u32);
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
        let tile = super::painter::RepeatTile {
            data: raster.pixels(),
            width: w,
            height: h,
            origin: (layer.origin.0.round(), layer.origin.1.round()),
        };
        painter.fill_repeating(fill, transform, &tile, shape);
        true
    }
    #[cfg(not(feature = "software-renderer"))]
    {
        let _ = (painter, tree, image, scale, fill, transform, layer, shape);
        false
    }
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 1e-6
        && (a.y0 - b.y0).abs() < 1e-6
        && (a.x1 - b.x1).abs() < 1e-6
        && (a.y1 - b.y1).abs() < 1e-6
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
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
    /// Every tile that touches the painting area `rect`, **snapped to device
    /// pixels** (#468's review, F1): each tile edge is rounded, so two
    /// neighbours share one whole-pixel edge. Filled as two antialiased
    /// rects meeting mid-pixel, each covers that pixel by half and they
    /// composite to 75% — a page-coloured seam at every tile boundary, which
    /// Chrome does not draw (it snaps the tile phase too). `None` past
    /// `limit` tiles, or when none touches `rect`.
    pub(super) fn tiles(&self, rect: Rect, limit: usize) -> Option<Vec<Rect>> {
        let (ox, oy) = self.origin;
        let (tw, th) = self.size;
        let starts = |origin: f64, size: f64, lo: f64, hi: f64, repeat: bool| {
            if !repeat {
                return Some(vec![origin]);
            }
            let first = origin - ((origin - lo) / size).ceil() * size;
            let count = ((hi - first) / size).ceil().max(0.0) as usize;
            if count > limit {
                return None;
            }
            Some((0..count).map(|i| first + i as f64 * size).collect())
        };
        let xs = starts(ox, tw, rect.x0, rect.x1, self.repeats_x)?;
        let ys = starts(oy, th, rect.y0, rect.y1, self.repeats_y)?;
        if xs.len() * ys.len() > limit {
            return None;
        }
        let mut tiles = Vec::with_capacity(xs.len() * ys.len());
        for &y in &ys {
            for &x in &xs {
                let tile = Rect::new(x.round(), y.round(), (x + tw).round(), (y + th).round());
                if tile.width() > 0.0
                    && tile.height() > 0.0
                    && tile.x1 > rect.x0
                    && tile.x0 < rect.x1
                    && tile.y1 > rect.y0
                    && tile.y0 < rect.y1
                {
                    tiles.push(tile);
                }
            }
        }
        (!tiles.is_empty()).then_some(tiles)
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
