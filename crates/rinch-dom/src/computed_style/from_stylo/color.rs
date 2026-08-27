//! Color-related Stylo conversion functions.
//!
//! Every stylo colour that becomes a `peniko::Color` comes through
//! `color_from_stylo` — the cascade, the bare-value parser
//! (`layout::parse_color`) and the keyframe extractor (`color_from_specified`)
//! — so all three paths quantise identically.

use peniko::color::{AlphaColor, Srgb};
use style::color::{AbsoluteColor, ColorSpace};

/// A computed stylo colour as a peniko Color, when it is absolute:
/// `currentcolor`, or a `color-mix()`/relative colour over it, needs an
/// element to resolve against and yields `None`.
pub(crate) fn color_from_stylo(color: &style::values::computed::Color) -> Option<peniko::Color> {
    color.as_absolute().and_then(color_from_absolute)
}

/// A specified (parsed, not yet cascaded) stylo colour as a peniko Color, when
/// it is absolute on its own: `currentcolor`, `light-dark()`, or a
/// `color-mix()` over `currentcolor` needs an element to resolve against and
/// yields `None`.
pub(crate) fn color_from_specified(
    color: &style::values::specified::Color,
) -> Option<peniko::Color> {
    color_from_stylo(&color.to_computed_color(None)?)
}

/// An absolute stylo colour as an 8-bit sRGB peniko Color.
///
/// Channels are rounded, not truncated: `hsl(270 50% 40%)` lands on
/// 0.19999… for green and must be 51 (rebeccapurple), not 50 (#250).
/// Compare the result through `to_rgba8()` — the edge every consumer
/// quantises at — rather than `==` on the f32 components.
pub(super) fn color_from_absolute(color: &AbsoluteColor) -> Option<peniko::Color> {
    let rgba = color.to_color_space(ColorSpace::Srgb);
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(AlphaColor::<Srgb>::from_rgba8(
        channel(rgba.components.0),
        channel(rgba.components.1),
        channel(rgba.components.2),
        channel(rgba.alpha),
    ))
}
