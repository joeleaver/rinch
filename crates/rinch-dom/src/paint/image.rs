//! Image painting for `<img>` elements and `background-image` CSS.

use peniko::kurbo::{Affine, Rect};
use peniko::{Blob, Fill, ImageAlphaType, ImageData, ImageFormat};
use vello::Scene;

use crate::image_cache::DecodedImage;

/// Paint a decoded image into the given rectangle.
///
/// `object_fit` controls how the image scales within the rect:
/// - `"fill"` (default): stretch to fill exactly
/// - `"contain"`: uniform scale to fit inside rect
/// - `"cover"`: uniform scale to cover rect (clips overflow)
pub fn paint_image(
    scene: &mut Scene,
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

    // Build peniko::ImageData from decoded RGBA8 data
    let blob: Blob<u8> = Blob::from(decoded.data.to_vec());
    let image_data = ImageData {
        data: blob,
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: decoded.width,
        height: decoded.height,
    };

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
        scene.push_clip_layer(Fill::NonZero, node_transform, &rect);
    }

    scene.draw_image(&image_data, img_transform);

    if object_fit == "cover" {
        scene.pop_layer();
    }
}
