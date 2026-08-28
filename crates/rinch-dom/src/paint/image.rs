//! Image painting for `<img>` elements and `background-image` CSS.

use peniko::Fill;
use peniko::kurbo::{Affine, Rect};

use super::painter::{PaintImage, Painter};
use crate::computed_style::ObjectFitValue;
use crate::image_cache::DecodedImage;

/// Paint a decoded image into the given rectangle.
///
/// `object_fit` controls how the image scales within the rect:
/// - `Fill` (default): stretch to fill exactly
/// - `Contain`: uniform scale to fit inside rect (letterboxed)
/// - `Cover`: uniform scale to cover rect (clips overflow)
/// - `None`: no scaling, image centered at natural size
/// - `ScaleDown`: like `Contain`, but never scales up
pub fn paint_image(
    painter: &mut dyn Painter,
    decoded: &DecodedImage,
    rect: Rect,
    scale: f64,
    object_fit: ObjectFitValue,
    node_transform: Affine,
) {
    paint_image_data(
        painter,
        &decoded.data,
        decoded.width,
        decoded.height,
        rect,
        scale,
        object_fit,
        node_transform,
    );
}

/// The same, over borrowed RGBA8 pixels rather than a [`DecodedImage`].
///
/// A live frame source — a `RenderSurface`, a decoded video frame — already
/// owns its pixels somewhere else, and at 60fps for a 1080p frame the copy into
/// a `DecodedImage` just to hand `paint_image` a `&Vec<u8>` costs ~8MB of
/// memcpy per frame for nothing.
#[allow(clippy::too_many_arguments)]
pub fn paint_image_data(
    painter: &mut dyn Painter,
    data: &[u8],
    width: u32,
    height: u32,
    rect: Rect,
    _scale: f64,
    object_fit: ObjectFitValue,
    node_transform: Affine,
) {
    if width == 0 || height == 0 {
        return;
    }

    let iw = width as f64;
    let ih = height as f64;
    let rw = rect.width();
    let rh = rect.height();

    if rw <= 0.0 || rh <= 0.0 {
        return;
    }

    // Compute the transform to map image pixels into the layout rect.
    // Image pixels are in their own coordinate space (0,0)-(iw,ih).
    // We need to scale and position them into (rect.x0, rect.y0)-(rect.x1, rect.y1).
    // The rect is already in scaled (physical) coordinates.
    let (sx, sy, tx, ty, needs_clip) = match object_fit {
        ObjectFitValue::Contain => {
            let scale_x = rw / iw;
            let scale_y = rh / ih;
            let s = scale_x.min(scale_y);
            let offset_x = rect.x0 + (rw - iw * s) * 0.5;
            let offset_y = rect.y0 + (rh - ih * s) * 0.5;
            (s, s, offset_x, offset_y, false)
        }
        ObjectFitValue::Cover => {
            let scale_x = rw / iw;
            let scale_y = rh / ih;
            let s = scale_x.max(scale_y);
            let offset_x = rect.x0 + (rw - iw * s) * 0.5;
            let offset_y = rect.y0 + (rh - ih * s) * 0.5;
            (s, s, offset_x, offset_y, true)
        }
        ObjectFitValue::None => {
            // No scaling — draw at natural size, centered in the rect
            let offset_x = rect.x0 + (rw - iw) * 0.5;
            let offset_y = rect.y0 + (rh - ih) * 0.5;
            // Clip if the image is larger than the rect
            let clip = iw > rw || ih > rh;
            (1.0, 1.0, offset_x, offset_y, clip)
        }
        ObjectFitValue::ScaleDown => {
            // Like contain, but never scales up (max scale = 1.0)
            let scale_x = rw / iw;
            let scale_y = rh / ih;
            let s = scale_x.min(scale_y).min(1.0);
            let offset_x = rect.x0 + (rw - iw * s) * 0.5;
            let offset_y = rect.y0 + (rh - ih * s) * 0.5;
            (s, s, offset_x, offset_y, false)
        }
        ObjectFitValue::Fill => {
            // Stretch to fill
            let sx = rw / iw;
            let sy = rh / ih;
            (sx, sy, rect.x0, rect.y0, false)
        }
    };

    let img_transform =
        node_transform * Affine::translate((tx, ty)) * Affine::scale_non_uniform(sx, sy);

    if needs_clip {
        painter.push_clip(Fill::NonZero, node_transform, &rect.into());
    }

    let paint_image = PaintImage {
        data,
        width,
        height,
    };
    painter.draw_image(&paint_image, img_transform);

    if needs_clip {
        painter.pop_layer();
    }
}
