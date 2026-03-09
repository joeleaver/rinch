//! Image painting for `<img>` elements and `background-image` CSS.

use peniko::Fill;
use peniko::kurbo::{Affine, Rect};

use super::painter::{PaintImage, Painter};
use crate::image_cache::DecodedImage;

/// Paint a decoded image into the given rectangle.
///
/// `object_fit` controls how the image scales within the rect:
/// - `"fill"` (default): stretch to fill exactly
/// - `"contain"`: uniform scale to fit inside rect
/// - `"cover"`: uniform scale to cover rect (clips overflow)
pub fn paint_image(
    painter: &mut dyn Painter,
    decoded: &DecodedImage,
    rect: Rect,
    scale: f64,
    object_fit: &str,
    node_transform: Affine,
) {
    if decoded.width == 0 || decoded.height == 0 {
        return;
    }

    let iw = decoded.width as f64;
    let ih = decoded.height as f64;
    let rw = rect.width();
    let rh = rect.height();

    if rw <= 0.0 || rh <= 0.0 {
        return;
    }

    // Compute the transform to map image pixels into the layout rect.
    // Image pixels are in their own coordinate space (0,0)-(iw,ih).
    // We need to scale and position them into (rect.x0, rect.y0)-(rect.x1, rect.y1).
    // The rect is already in scaled (physical) coordinates.
    let (sx, sy, tx, ty) = match object_fit {
        "contain" => {
            let scale_x = rw / iw;
            let scale_y = rh / ih;
            let s = scale_x.min(scale_y);
            let offset_x = rect.x0 + (rw - iw * s) * 0.5;
            let offset_y = rect.y0 + (rh - ih * s) * 0.5;
            (s, s, offset_x, offset_y)
        }
        "cover" => {
            let scale_x = rw / iw;
            let scale_y = rh / ih;
            let s = scale_x.max(scale_y);
            let offset_x = rect.x0 + (rw - iw * s) * 0.5;
            let offset_y = rect.y0 + (rh - ih * s) * 0.5;
            (s, s, offset_x, offset_y)
        }
        _ => {
            // "fill" — stretch to fill
            let sx = rw / iw;
            let sy = rh / ih;
            (sx, sy, rect.x0, rect.y0)
        }
    };

    let _ = scale; // rect coords are already in physical pixels

    let img_transform =
        node_transform * Affine::translate((tx, ty)) * Affine::scale_non_uniform(sx, sy);

    // For "cover", clip to the element rect to hide overflow
    if object_fit == "cover" {
        painter.push_clip(Fill::NonZero, node_transform, &rect.into());
    }

    let paint_image = PaintImage {
        data: &decoded.data,
        width: decoded.width,
        height: decoded.height,
    };
    painter.draw_image(&paint_image, img_transform);

    if object_fit == "cover" {
        painter.pop_layer();
    }
}
