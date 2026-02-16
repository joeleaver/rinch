//! Color-related Stylo conversion functions.

pub(super) fn color_from_stylo(color: &style::values::computed::Color) -> Option<peniko::Color> {
    // Stylo's Color has as_absolute() to get resolved RGBA
    color.as_absolute().map(|abs| {
        let rgba = abs.to_color_space(style::color::ColorSpace::Srgb);
        let r = (rgba.components.0.clamp(0.0, 1.0) * 255.0) as u8;
        let g = (rgba.components.1.clamp(0.0, 1.0) * 255.0) as u8;
        let b = (rgba.components.2.clamp(0.0, 1.0) * 255.0) as u8;
        let a = (rgba.alpha.clamp(0.0, 1.0) * 255.0) as u8;
        peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(r, g, b, a)
    })
}

pub(super) fn color_from_absolute(color: &style::color::AbsoluteColor) -> Option<peniko::Color> {
    let rgba = color.to_color_space(style::color::ColorSpace::Srgb);
    let r = (rgba.components.0.clamp(0.0, 1.0) * 255.0) as u8;
    let g = (rgba.components.1.clamp(0.0, 1.0) * 255.0) as u8;
    let b = (rgba.components.2.clamp(0.0, 1.0) * 255.0) as u8;
    let a = (rgba.alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Some(peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(r, g, b, a))
}
