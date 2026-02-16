//! Visual property Stylo conversion functions: visibility, cursor, pointer-events,
//! z-index, transforms, text-shadow, background/gradients, filters.

use crate::computed_style::values::*;

use super::color::{color_from_absolute, color_from_stylo};

pub(super) fn visibility_from_stylo(
    vis: &style::properties::longhands::visibility::computed_value::T,
) -> VisibilityValue {
    use style::properties::longhands::visibility::computed_value::T as Vis;
    match *vis {
        Vis::Visible => VisibilityValue::Visible,
        Vis::Hidden => VisibilityValue::Hidden,
        Vis::Collapse => VisibilityValue::Collapse,
    }
}

pub(super) fn cursor_from_stylo(
    cursor: &style::values::specified::ui::CursorKind,
) -> CursorValue {
    use style::values::specified::ui::CursorKind;
    match *cursor {
        CursorKind::Auto => CursorValue::Auto,
        CursorKind::Default => CursorValue::Default,
        CursorKind::Pointer => CursorValue::Pointer,
        CursorKind::Text => CursorValue::Text,
        CursorKind::Move => CursorValue::Move,
        CursorKind::NotAllowed => CursorValue::NotAllowed,
        CursorKind::Crosshair => CursorValue::Crosshair,
        CursorKind::Grab => CursorValue::Grab,
        CursorKind::Grabbing => CursorValue::Grabbing,
        CursorKind::ColResize => CursorValue::ColResize,
        CursorKind::RowResize => CursorValue::RowResize,
        CursorKind::NResize => CursorValue::NResize,
        CursorKind::SResize => CursorValue::SResize,
        CursorKind::EResize => CursorValue::EResize,
        CursorKind::WResize => CursorValue::WResize,
        CursorKind::NeResize => CursorValue::NeResize,
        CursorKind::NwResize => CursorValue::NwResize,
        CursorKind::SeResize => CursorValue::SeResize,
        CursorKind::SwResize => CursorValue::SwResize,
        CursorKind::EwResize => CursorValue::EwResize,
        CursorKind::NsResize => CursorValue::NsResize,
        CursorKind::Wait => CursorValue::Wait,
        CursorKind::Progress => CursorValue::Progress,
        CursorKind::Help => CursorValue::Help,
        CursorKind::ZoomIn => CursorValue::ZoomIn,
        CursorKind::ZoomOut => CursorValue::ZoomOut,
        CursorKind::None => CursorValue::None,
        _ => CursorValue::Auto,
    }
}

pub(super) fn pointer_events_from_stylo(
    pe: &style::values::specified::ui::PointerEvents,
) -> PointerEventsValue {
    use style::values::specified::ui::PointerEvents;
    match *pe {
        PointerEvents::None => PointerEventsValue::None,
        _ => PointerEventsValue::Auto,
    }
}

pub(super) fn z_index_from_stylo(z: &style::values::computed::ZIndex) -> Option<i32> {
    use style::values::generics::position::ZIndex;
    match z {
        ZIndex::Integer(val) => Some(*val),
        ZIndex::Auto => None,
    }
}

pub(super) fn transform_from_stylo(
    transform: &style::values::computed::Transform,
) -> TransformValue {
    use style::values::generics::transform::GenericTransformOperation;

    if transform.0.is_empty() {
        return TransformValue::default();
    }

    // Compose all operations into a single 2D affine matrix [a, b, c, d, e, f]
    let mut m = [1.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0]; // identity

    for op in &*transform.0 {
        let op_matrix = match op {
            GenericTransformOperation::Matrix(mat) => [
                mat.a as f64,
                mat.b as f64,
                mat.c as f64,
                mat.d as f64,
                mat.e as f64,
                mat.f as f64,
            ],
            GenericTransformOperation::Rotate(angle) => {
                let rad = angle.radians64();
                let cos = rad.cos();
                let sin = rad.sin();
                [cos, sin, -sin, cos, 0.0, 0.0]
            }
            GenericTransformOperation::Scale(sx, sy) => {
                [*sx as f64, 0.0, 0.0, *sy as f64, 0.0, 0.0]
            }
            GenericTransformOperation::ScaleX(sx) => [*sx as f64, 0.0, 0.0, 1.0, 0.0, 0.0],
            GenericTransformOperation::ScaleY(sy) => [1.0, 0.0, 0.0, *sy as f64, 0.0, 0.0],
            GenericTransformOperation::TranslateX(tx) => {
                let tx_px = length_or_pct_to_px(tx);
                [1.0, 0.0, 0.0, 1.0, tx_px, 0.0]
            }
            GenericTransformOperation::TranslateY(ty) => {
                let ty_px = length_or_pct_to_px(ty);
                [1.0, 0.0, 0.0, 1.0, 0.0, ty_px]
            }
            GenericTransformOperation::Translate(tx, ty) => {
                let tx_px = length_or_pct_to_px(tx);
                let ty_px = length_or_pct_to_px(ty);
                [1.0, 0.0, 0.0, 1.0, tx_px, ty_px]
            }
            GenericTransformOperation::SkewX(angle) => {
                let tan = angle.radians64().tan();
                [1.0, 0.0, tan, 1.0, 0.0, 0.0]
            }
            GenericTransformOperation::SkewY(angle) => {
                let tan = angle.radians64().tan();
                [1.0, tan, 0.0, 1.0, 0.0, 0.0]
            }
            GenericTransformOperation::Skew(ax, ay) => {
                let tan_x = ax.radians64().tan();
                let tan_y = ay.radians64().tan();
                [1.0, tan_y, tan_x, 1.0, 0.0, 0.0]
            }
            // 3D transforms -- flatten to 2D identity (skip)
            _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        };

        // Matrix multiply: m = m * op_matrix
        let a = m[0] * op_matrix[0] + m[2] * op_matrix[1];
        let b = m[1] * op_matrix[0] + m[3] * op_matrix[1];
        let c = m[0] * op_matrix[2] + m[2] * op_matrix[3];
        let d = m[1] * op_matrix[2] + m[3] * op_matrix[3];
        let e = m[0] * op_matrix[4] + m[2] * op_matrix[5] + m[4];
        let f = m[1] * op_matrix[4] + m[3] * op_matrix[5] + m[5];
        m = [a, b, c, d, e, f];
    }

    let is_identity = (m[0] - 1.0).abs() < 1e-6
        && m[1].abs() < 1e-6
        && m[2].abs() < 1e-6
        && (m[3] - 1.0).abs() < 1e-6
        && m[4].abs() < 1e-6
        && m[5].abs() < 1e-6;

    TransformValue {
        matrix: m,
        is_identity,
    }
}

fn length_or_pct_to_px(lp: &style::values::computed::LengthPercentage) -> f64 {
    if let Some(len) = lp.to_length() {
        len.px() as f64
    } else {
        // Percentage transforms need element size -- store 0 for now
        // (will be resolved at paint time for percentage-based translates)
        0.0
    }
}

pub(super) fn transform_origin_component_from_stylo(
    origin: &style::values::computed::LengthPercentage,
) -> LengthPercentageValue {
    if let Some(len) = origin.to_length() {
        LengthPercentageValue::Length(len.px())
    } else if let Some(pct) = origin.to_percentage() {
        LengthPercentageValue::Percent(pct.0)
    } else {
        LengthPercentageValue::Percent(0.5) // default 50%
    }
}

pub(super) fn text_shadow_from_stylo(
    shadows: &style::properties::longhands::text_shadow::computed_value::T,
    text_color: &style::color::AbsoluteColor,
) -> Vec<TextShadowValue> {
    shadows
        .0
        .iter()
        .map(|s| {
            let color = if s.color.is_currentcolor() {
                color_from_absolute(text_color)
            } else {
                color_from_stylo(&s.color)
            };
            TextShadowValue {
                offset_x: s.horizontal.px(),
                offset_y: s.vertical.px(),
                blur_radius: s.blur.0.px(),
                color,
            }
        })
        .collect()
}

pub(super) fn background_from_stylo(
    bg: &style::properties::style_structs::Background,
    text_color: &style::color::AbsoluteColor,
) -> BackgroundValue {
    use style::values::computed::image::Image;
    use style::values::generics::image::GenericGradient;

    // Check for gradient in background-image first
    if !bg.background_image.0.is_empty()
        && let Image::Gradient(boxed_gradient) = &bg.background_image.0[0]
    {
        let gradient = &**boxed_gradient;
        match gradient {
            GenericGradient::Linear {
                direction, items, ..
            } => {
                let angle = gradient_direction_to_angle(direction);
                let stops = gradient_stops_from_stylo(items, text_color);
                if !stops.is_empty() {
                    return BackgroundValue::LinearGradient {
                        angle_degrees: angle,
                        stops,
                    };
                }
            }
            GenericGradient::Radial { items, .. } => {
                let stops = gradient_stops_from_stylo(items, text_color);
                if !stops.is_empty() {
                    return BackgroundValue::RadialGradient { stops };
                }
            }
            _ => {} // conic gradients not supported yet
        }
    }

    // Fall back to background-color
    let color = if bg.background_color.is_currentcolor() {
        color_from_absolute(text_color)
    } else {
        color_from_stylo(&bg.background_color)
    };
    match color {
        Some(c) => BackgroundValue::Color(c),
        None => BackgroundValue::None,
    }
}

fn gradient_direction_to_angle(
    direction: &style::values::computed::image::LineDirection,
) -> f32 {
    use style::values::computed::image::LineDirection;
    use style::values::specified::position::{
        HorizontalPositionKeyword, VerticalPositionKeyword,
    };
    match direction {
        LineDirection::Angle(angle) => angle.degrees(),
        LineDirection::Horizontal(h) => match *h {
            HorizontalPositionKeyword::Left => 270.0,
            HorizontalPositionKeyword::Right => 90.0,
        },
        LineDirection::Vertical(v) => match *v {
            VerticalPositionKeyword::Top => 0.0,
            VerticalPositionKeyword::Bottom => 180.0,
        },
        LineDirection::Corner(h, v) => match (h, v) {
            (HorizontalPositionKeyword::Right, VerticalPositionKeyword::Top) => 45.0,
            (HorizontalPositionKeyword::Right, VerticalPositionKeyword::Bottom) => 135.0,
            (HorizontalPositionKeyword::Left, VerticalPositionKeyword::Bottom) => 225.0,
            (HorizontalPositionKeyword::Left, VerticalPositionKeyword::Top) => 315.0,
        },
    }
}

fn gradient_stops_from_stylo(
    items: &[style::values::generics::image::GenericGradientItem<
        style::values::computed::color::Color,
        style::values::computed::LengthPercentage,
    >],
    text_color: &style::color::AbsoluteColor,
) -> Vec<GradientStop> {
    use style::values::generics::image::GenericGradientItem;

    let mut stops = Vec::new();
    let total = items.len();

    for (i, item) in items.iter().enumerate() {
        match item {
            GenericGradientItem::SimpleColorStop(color) => {
                let c = if color.is_currentcolor() {
                    color_from_absolute(text_color)
                } else {
                    color_from_stylo(color)
                };
                // Auto-distribute position
                let offset = if total <= 1 {
                    0.0
                } else {
                    i as f32 / (total - 1) as f32
                };
                stops.push(GradientStop { offset, color: c });
            }
            GenericGradientItem::ComplexColorStop { color, position } => {
                let c = if color.is_currentcolor() {
                    color_from_absolute(text_color)
                } else {
                    color_from_stylo(color)
                };
                let offset = if let Some(pct) = position.to_percentage() {
                    pct.0
                } else if let Some(len) = position.to_length() {
                    // Length stops need container size to resolve -- approximate
                    len.px() / 100.0
                } else {
                    i as f32 / (total - 1).max(1) as f32
                };
                stops.push(GradientStop { offset, color: c });
            }
            _ => {}
        }
    }
    stops
}

pub(super) fn extract_filter_brightness(
    filter: &style::properties::longhands::filter::computed_value::T,
) -> f32 {
    use style::values::generics::effects::GenericFilter;
    for f in &*filter.0 {
        if let GenericFilter::Brightness(val) = f {
            return val.0;
        }
    }
    1.0
}

pub(super) fn extract_filter_grayscale(
    filter: &style::properties::longhands::filter::computed_value::T,
) -> f32 {
    use style::values::generics::effects::GenericFilter;
    for f in &*filter.0 {
        if let GenericFilter::Grayscale(val) = f {
            return val.0;
        }
    }
    0.0
}

pub(super) fn extract_filter_saturate(
    filter: &style::properties::longhands::filter::computed_value::T,
) -> f32 {
    use style::values::generics::effects::GenericFilter;
    for f in &*filter.0 {
        if let GenericFilter::Saturate(val) = f {
            return val.0;
        }
    }
    1.0
}

pub(super) fn extract_filter_hue_rotate(
    filter: &style::properties::longhands::filter::computed_value::T,
) -> f32 {
    use style::values::generics::effects::GenericFilter;
    for f in &*filter.0 {
        if let GenericFilter::HueRotate(angle) = f {
            return angle.degrees();
        }
    }
    0.0
}
