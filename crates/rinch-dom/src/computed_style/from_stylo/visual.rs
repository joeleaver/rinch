//! Visual property Stylo conversion functions: visibility, cursor, pointer-events,
//! z-index, transforms, text-shadow, background/gradients, filters.

use crate::computed_style::values::*;

use super::color::color_from_computed;

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

pub(super) fn object_fit_from_stylo(
    fit: &style::properties::longhands::object_fit::computed_value::T,
) -> ObjectFitValue {
    use style::properties::longhands::object_fit::computed_value::T as ObjFit;
    match *fit {
        ObjFit::Fill => ObjectFitValue::Fill,
        ObjFit::Contain => ObjectFitValue::Contain,
        ObjFit::Cover => ObjectFitValue::Cover,
        ObjFit::None => ObjectFitValue::None,
        ObjFit::ScaleDown => ObjectFitValue::ScaleDown,
    }
}

pub(super) fn cursor_from_stylo(cursor: &style::values::specified::ui::CursorKind) -> CursorValue {
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

/// A computed `transform`, composed about a `transform-origin` whose z is
/// `origin_z` CSS px (#997; the x and y are applied at paint).
pub(super) fn transform_from_stylo(
    transform: &style::values::computed::Transform,
    origin_z: f64,
) -> TransformValue {
    use crate::transition::{Affine, TransformOp, compose_about_origin_z};
    use style::values::generics::transform::GenericTransformOperation;

    if transform.0.is_empty() {
        return TransformValue::default();
    }

    // The percentage part of a translate cannot be resolved here — the
    // element's border box is not known until Taffy has run — so a
    // `Translate` function carries it beside its pixel part, and composing the
    // list accumulates its *linear form* in (width, height), each translate in
    // the frame the functions before it establish (#212; see `Affine::then`).
    let split = |lp: Option<&style::values::computed::LengthPercentage>| {
        lp.map_or((0.0, 0.0), length_or_pct_split)
    };
    let translate = |x: Option<&style::values::computed::LengthPercentage>,
                     y: Option<&style::values::computed::LengthPercentage>,
                     z: f64| {
        let (px, pct_x) = split(x);
        let (py, pct_y) = split(y);
        TransformOp::Translate {
            px: [px, py],
            pct: [pct_x, pct_y],
            z,
        }
    };
    let functions: Vec<TransformOp> = transform
        .0
        .iter()
        .map(|op| match op {
            GenericTransformOperation::Matrix(mat) => TransformOp::Matrix(Affine::from_matrix([
                mat.a as f64,
                mat.b as f64,
                mat.c as f64,
                mat.d as f64,
                mat.e as f64,
                mat.f as f64,
            ])),
            GenericTransformOperation::Matrix3D(m) => TransformOp::matrix3d(
                [
                    m.m11, m.m12, m.m13, m.m14, m.m21, m.m22, m.m23, m.m24, m.m31, m.m32, m.m33,
                    m.m34, m.m41, m.m42, m.m43, m.m44,
                ]
                .map(f64::from),
            ),
            GenericTransformOperation::Rotate(angle)
            | GenericTransformOperation::RotateZ(angle) => TransformOp::Rotate(angle.radians64()),
            GenericTransformOperation::RotateX(angle) => {
                TransformOp::rotate3d(1.0, 0.0, 0.0, angle.radians64())
            }
            GenericTransformOperation::RotateY(angle) => {
                TransformOp::rotate3d(0.0, 1.0, 0.0, angle.radians64())
            }
            GenericTransformOperation::Rotate3D(x, y, z, angle) => {
                TransformOp::rotate3d(*x as f64, *y as f64, *z as f64, angle.radians64())
            }
            GenericTransformOperation::Scale(sx, sy) => {
                TransformOp::Scale(*sx as f64, *sy as f64, 1.0)
            }
            GenericTransformOperation::ScaleX(sx) => TransformOp::Scale(*sx as f64, 1.0, 1.0),
            GenericTransformOperation::ScaleY(sy) => TransformOp::Scale(1.0, *sy as f64, 1.0),
            GenericTransformOperation::ScaleZ(sz) => TransformOp::Scale(1.0, 1.0, *sz as f64),
            GenericTransformOperation::Scale3D(sx, sy, sz) => {
                TransformOp::Scale(*sx as f64, *sy as f64, *sz as f64)
            }
            GenericTransformOperation::TranslateX(tx) => translate(Some(tx), None, 0.0),
            GenericTransformOperation::TranslateY(ty) => translate(None, Some(ty), 0.0),
            GenericTransformOperation::Translate(tx, ty) => translate(Some(tx), Some(ty), 0.0),
            // `translate3d()` carries two `LengthPercentage`s, and goes
            // through the same percentage split as `translate()` (#212).
            GenericTransformOperation::Translate3D(tx, ty, tz) => {
                translate(Some(tx), Some(ty), tz.px() as f64)
            }
            GenericTransformOperation::TranslateZ(tz) => translate(None, None, tz.px() as f64),
            GenericTransformOperation::SkewX(angle) => TransformOp::SkewX(angle.radians64()),
            GenericTransformOperation::SkewY(angle) => TransformOp::SkewY(angle.radians64()),
            GenericTransformOperation::Skew(ax, ay) => {
                TransformOp::Skew(ax.radians64(), ay.radians64())
            }
            GenericTransformOperation::Perspective(p) => {
                TransformOp::perspective(p.infinity_or(|l| l.px()) as f64)
            }
            // `InterpolateMatrix` / `AccumulateMatrix` are stylo's own
            // animation intermediates; rinch interpolates function lists
            // itself (#414) and never computes one.
            _ => TransformOp::Matrix(Affine::IDENTITY),
        })
        .collect();

    let (composed, back_facing) = compose_about_origin_z(&functions, origin_z);
    let (m, pct_w, pct_h) = (composed.matrix, composed.pct_w, composed.pct_h);
    let has_pct = pct_w.iter().chain(&pct_h).any(|c| c.abs() > 1e-9);

    let is_identity = !has_pct
        && (m[0] - 1.0).abs() < 1e-6
        && m[1].abs() < 1e-6
        && m[2].abs() < 1e-6
        && (m[3] - 1.0).abs() < 1e-6
        && m[4].abs() < 1e-6
        && m[5].abs() < 1e-6;

    TransformValue {
        matrix: m,
        is_identity,
        pct_translate_w: pct_w,
        pct_translate_h: pct_h,
        functions,
        back_facing,
    }
}

/// Split a LengthPercentage into (px_value, percentage_fraction).
///
/// Returns `(px, 0.0)` for a plain length, `(0.0, fraction)` for a plain
/// percentage, and the recovered affine pair for a genuinely mixed `calc()` —
/// `translateX(calc(50% - 10px))` yields `(-10.0, 0.5)` (#404; it used to
/// yield `(0.0, 0.0)`, no translation at all). See `from_stylo/calc.rs`.
fn length_or_pct_split(lp: &style::values::computed::LengthPercentage) -> (f64, f64) {
    let (px, pct) = super::calc::split_length_percentage(lp);
    (px as f64, pct as f64)
}

pub(super) fn transform_origin_component_from_stylo(
    origin: &style::values::computed::LengthPercentage,
) -> LengthPercentageValue {
    if let Some(len) = origin.to_length() {
        LengthPercentageValue::Length(len.px())
    } else if let Some(pct) = origin.to_percentage() {
        LengthPercentageValue::Percent(pct.0)
    } else {
        // A mixed calc used to degrade to the 50% default; carry the pair
        // and let paint resolve it against the box (#278/#404 family).
        let (px, pct) = super::calc::split_length_percentage(origin);
        LengthPercentageValue::Calc { px, pct }
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
            let color = color_from_computed(&s.color, text_color);
            TextShadowValue {
                offset_x: s.horizontal.px(),
                offset_y: s.vertical.px(),
                blur_radius: s.blur.0.px(),
                color,
            }
        })
        .collect()
}

pub(super) fn box_shadow_from_stylo(
    shadows: &style::properties::longhands::box_shadow::computed_value::T,
    text_color: &style::color::AbsoluteColor,
) -> Vec<BoxShadowValue> {
    shadows
        .0
        .iter()
        .map(|s| {
            let color = color_from_computed(&s.base.color, text_color);
            BoxShadowValue {
                offset_x: s.base.horizontal.px(),
                offset_y: s.base.vertical.px(),
                blur_radius: s.base.blur.0.px(),
                spread_radius: s.spread.px(),
                color,
                inset: s.inset,
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

    // Check for gradient or image URL in background-image first
    if !bg.background_image.0.is_empty() {
        match &bg.background_image.0[0] {
            Image::Gradient(boxed_gradient) => {
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
            Image::Url(url_value) => {
                let url_str = match url_value {
                    style::values::computed::url::ComputedUrl::Valid(url) => {
                        url.as_str().to_string()
                    }
                    style::values::computed::url::ComputedUrl::Invalid(s) => s.to_string(),
                };
                if !url_str.is_empty() {
                    return BackgroundValue::Image { url: url_str };
                }
            }
            _ => {}
        }
    }

    // Fall back to background-color
    let color = color_from_computed(&bg.background_color, text_color);
    match color {
        Some(c) => BackgroundValue::Color(c),
        None => BackgroundValue::None,
    }
}

fn gradient_direction_to_angle(direction: &style::values::computed::image::LineDirection) -> f32 {
    use style::values::computed::image::LineDirection;
    use style::values::specified::position::{HorizontalPositionKeyword, VerticalPositionKeyword};
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
                let c = color_from_computed(color, text_color);
                // Auto-distribute position
                let offset = if total <= 1 {
                    0.0
                } else {
                    i as f32 / (total - 1) as f32
                };
                stops.push(GradientStop { offset, color: c });
            }
            GenericGradientItem::ComplexColorStop { color, position } => {
                let c = color_from_computed(color, text_color);
                let offset = if let Some(pct) = position.to_percentage() {
                    pct.0
                } else if let Some(len) = position.to_length() {
                    // Length stops need container size to resolve -- approximate
                    len.px() / 100.0
                } else {
                    // Mixed calc: same px/100 approximation as the plain-length
                    // arm for the length part, plus the exact percentage part.
                    // Used to fall through to the auto-distributed index.
                    let (px, pct) = super::calc::split_length_percentage(position);
                    pct + px / 100.0
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
