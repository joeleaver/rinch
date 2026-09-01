//! Extract keyframe stop values from Stylo's KeyframesAnimation.

use style::properties::{LonghandId, PropertyDeclaration, PropertyDeclarationId};
use style::shared_lock::SharedRwLockReadGuard;
use style::stylesheets::keyframes_rule::{KeyframesAnimation, KeyframesStepValue};
use style::values::specified::Color as SpecifiedColor;

use crate::computed_style::{
    ComputedStyle, DimensionValue, LengthPercentageAutoValue, LengthPercentageValue,
    color_from_specified,
};
use crate::transition::types::{AnimatableValue, TransformOp, TransitionProperty};

use super::types::KeyframeStop;
use crate::transition::types::TimingFunction;

/// Extract KeyframeStops from a Stylo KeyframesAnimation.
///
/// For `ComputedValues` steps (auto-generated 0%/100%), we use `base_style` values.
/// For `Declarations` steps, colours are read as typed stylo values; the other
/// properties are serialized to CSS text and parsed back. `parent_color` is
/// what a `color: currentcolor` stop inherits.
pub fn extract_keyframe_stops(
    animation: &KeyframesAnimation,
    base_style: &ComputedStyle,
    parent_color: Option<peniko::Color>,
    guard: &SharedRwLockReadGuard,
) -> Vec<KeyframeStop> {
    let mut stops = Vec::new();

    for step in &animation.steps {
        let percentage = step.start_percentage.0;

        let (values, timing_function) = match &step.value {
            KeyframesStepValue::ComputedValues => {
                // Auto-generated step: use current element style as values
                let values = extract_base_style_values(base_style);
                (values, None)
            }
            KeyframesStepValue::Declarations { block } => {
                let block = block.read_with(guard);
                let values = extract_declaration_values(block, base_style, parent_color);

                // Extract per-keyframe timing function if declared
                let tf = if step.declared_timing_function {
                    step.get_animation_timing_function(guard)
                        .and_then(|spec_tf| convert_specified_timing_function(&spec_tf))
                } else {
                    None
                };

                (values, tf)
            }
        };

        stops.push(KeyframeStop {
            percentage,
            values,
            timing_function,
        });
    }

    stops
}

/// Extract animatable values from a PropertyDeclarationBlock.
fn extract_declaration_values(
    block: &style::properties::PropertyDeclarationBlock,
    base_style: &ComputedStyle,
    parent_color: Option<peniko::Color>,
) -> Vec<(TransitionProperty, AnimatableValue)> {
    let mut values = Vec::new();

    for declaration in block.normal_declaration_iter() {
        if let Some((prop, val)) = convert_declaration(declaration, base_style, parent_color) {
            values.push((prop, val));
        }
    }

    values
}

/// Convert a single PropertyDeclaration to our (TransitionProperty, AnimatableValue).
///
/// Colour longhands are read as typed stylo values. Everything else goes
/// through ToCss serialization and is parsed back from the text.
fn convert_declaration(
    declaration: &PropertyDeclaration,
    base_style: &ComputedStyle,
    parent_color: Option<peniko::Color>,
) -> Option<(TransitionProperty, AnimatableValue)> {
    if let Some((property, color)) = color_declaration(declaration) {
        let color = match color {
            // On `color` itself, `currentcolor` means `inherit` (CSS Color 4
            // §7.1): the parent's colour, not the element's own.
            SpecifiedColor::CurrentColor if property == TransitionProperty::Color => {
                parent_color.or(base_style.color)?
            }
            // The element's own colour, as for every other `currentcolor`.
            SpecifiedColor::CurrentColor => base_style.color?,
            other => color_from_specified(other)?,
        };
        return Some((property, AnimatableValue::Color(color)));
    }

    let id = declaration.id();
    let css_text = {
        let mut s = String::new();
        declaration.to_css(&mut s).ok()?;
        s
    };

    match id {
        PropertyDeclarationId::Longhand(LonghandId::Opacity) => {
            let val: f32 = css_text.trim().parse().ok()?;
            Some((
                TransitionProperty::Opacity,
                AnimatableValue::Float(val.clamp(0.0, 1.0)),
            ))
        }
        PropertyDeclarationId::Longhand(LonghandId::Width) => parse_css_dimension(&css_text)
            .map(|d| (TransitionProperty::Width, AnimatableValue::Dimension(d))),
        PropertyDeclarationId::Longhand(LonghandId::Height) => parse_css_dimension(&css_text)
            .map(|d| (TransitionProperty::Height, AnimatableValue::Dimension(d))),
        PropertyDeclarationId::Longhand(LonghandId::PaddingTop) => {
            parse_css_length(&css_text).map(|lp| {
                (
                    TransitionProperty::PaddingTop,
                    AnimatableValue::LengthPercentage(lp),
                )
            })
        }
        PropertyDeclarationId::Longhand(LonghandId::PaddingRight) => parse_css_length(&css_text)
            .map(|lp| {
                (
                    TransitionProperty::PaddingRight,
                    AnimatableValue::LengthPercentage(lp),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::PaddingBottom) => parse_css_length(&css_text)
            .map(|lp| {
                (
                    TransitionProperty::PaddingBottom,
                    AnimatableValue::LengthPercentage(lp),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::PaddingLeft) => parse_css_length(&css_text)
            .map(|lp| {
                (
                    TransitionProperty::PaddingLeft,
                    AnimatableValue::LengthPercentage(lp),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::MarginTop) => {
            parse_css_margin(&css_text).map(|lpa| {
                (
                    TransitionProperty::MarginTop,
                    AnimatableValue::LengthPercentageAuto(lpa),
                )
            })
        }
        PropertyDeclarationId::Longhand(LonghandId::MarginRight) => parse_css_margin(&css_text)
            .map(|lpa| {
                (
                    TransitionProperty::MarginRight,
                    AnimatableValue::LengthPercentageAuto(lpa),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::MarginBottom) => parse_css_margin(&css_text)
            .map(|lpa| {
                (
                    TransitionProperty::MarginBottom,
                    AnimatableValue::LengthPercentageAuto(lpa),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::MarginLeft) => {
            parse_css_margin(&css_text).map(|lpa| {
                (
                    TransitionProperty::MarginLeft,
                    AnimatableValue::LengthPercentageAuto(lpa),
                )
            })
        }
        PropertyDeclarationId::Longhand(LonghandId::BorderTopWidth) => parse_css_length(&css_text)
            .map(|lp| {
                (
                    TransitionProperty::BorderTopWidth,
                    AnimatableValue::LengthPercentage(lp),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::BorderRightWidth) => {
            parse_css_length(&css_text).map(|lp| {
                (
                    TransitionProperty::BorderRightWidth,
                    AnimatableValue::LengthPercentage(lp),
                )
            })
        }
        PropertyDeclarationId::Longhand(LonghandId::BorderBottomWidth) => {
            parse_css_length(&css_text).map(|lp| {
                (
                    TransitionProperty::BorderBottomWidth,
                    AnimatableValue::LengthPercentage(lp),
                )
            })
        }
        PropertyDeclarationId::Longhand(LonghandId::BorderLeftWidth) => parse_css_length(&css_text)
            .map(|lp| {
                (
                    TransitionProperty::BorderLeftWidth,
                    AnimatableValue::LengthPercentage(lp),
                )
            }),
        PropertyDeclarationId::Longhand(LonghandId::FontSize) => parse_css_px_value(&css_text)
            .map(|px| (TransitionProperty::FontSize, AnimatableValue::Float(px))),
        PropertyDeclarationId::Longhand(LonghandId::Transform) => {
            parse_css_transform_components(&css_text).map(|ops| {
                (
                    TransitionProperty::Transform,
                    // An authored `@keyframes` stop is parsed by this file's own
                    // mini-parser, which drops a percentage translate on the
                    // floor before it ever reaches a `TransformOp` — a separate
                    // gap from #403, tracked as its own issue. The channel is
                    // zero here until that is fixed.
                    AnimatableValue::TransformComponents {
                        ops,
                        pct_translate_w: [0.0, 0.0],
                        pct_translate_h: [0.0, 0.0],
                    },
                )
            })
        }
        _ => None, // Unsupported property — silently skip
    }
}

/// Extract all animatable values from a ComputedStyle (for auto-generated keyframes).
fn extract_base_style_values(style: &ComputedStyle) -> Vec<(TransitionProperty, AnimatableValue)> {
    use crate::computed_style::BackgroundValue;

    let mut values = Vec::new();

    values.push((
        TransitionProperty::Opacity,
        AnimatableValue::Float(style.opacity),
    ));

    if let BackgroundValue::Color(c) = &style.background {
        values.push((
            TransitionProperty::BackgroundColor,
            AnimatableValue::Color(*c),
        ));
    }

    if let Some(c) = style.color {
        values.push((TransitionProperty::Color, AnimatableValue::Color(c)));
    }

    if !style.transform.is_identity {
        values.push((
            TransitionProperty::Transform,
            AnimatableValue::TransformComponents {
                ops: vec![TransformOp::Matrix(style.transform.matrix)],
                // The percentage part of a translate lives outside the matrix
                // and would otherwise be lost for the whole animation (#403).
                pct_translate_w: style.transform.pct_translate_w,
                pct_translate_h: style.transform.pct_translate_h,
            },
        ));
    } else {
        values.push((
            TransitionProperty::Transform,
            AnimatableValue::TransformComponents {
                ops: vec![TransformOp::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])],
                pct_translate_w: [0.0, 0.0],
                pct_translate_h: [0.0, 0.0],
            },
        ));
    }

    values
}

/// The animatable colour longhands, with their typed specified value.
///
/// Typed on purpose: stylo serialises an authored keyword verbatim, so
/// re-parsing the CSS text needs a colour parser of its own — and the private
/// one that used to live here knew eleven names, which is how a
/// `rebeccapurple` stop silently dropped out of its animation (#250).
fn color_declaration(
    declaration: &PropertyDeclaration,
) -> Option<(TransitionProperty, &SpecifiedColor)> {
    Some(match declaration {
        PropertyDeclaration::BackgroundColor(c) => (TransitionProperty::BackgroundColor, c),
        PropertyDeclaration::Color(c) => (TransitionProperty::Color, &c.0),
        PropertyDeclaration::BorderTopColor(c) => (TransitionProperty::BorderTopColor, c),
        PropertyDeclaration::BorderRightColor(c) => (TransitionProperty::BorderRightColor, c),
        PropertyDeclaration::BorderBottomColor(c) => (TransitionProperty::BorderBottomColor, c),
        PropertyDeclaration::BorderLeftColor(c) => (TransitionProperty::BorderLeftColor, c),
        _ => return None,
    })
}

// =============================================================================
// CSS text parsing helpers
// =============================================================================

/// Parse a CSS dimension value (width/height): "100px", "auto", etc.
fn parse_css_dimension(css: &str) -> Option<DimensionValue> {
    let css = css.trim();
    if css == "auto" {
        return Some(DimensionValue::Auto);
    }
    parse_css_px_value(css).map(DimensionValue::Length)
}

/// Parse a CSS length/percentage value: "10px", "0", etc.
fn parse_css_length(css: &str) -> Option<LengthPercentageValue> {
    let css = css.trim();
    if css == "0" || css == "0px" {
        return Some(LengthPercentageValue::Zero);
    }
    parse_css_px_value(css).map(LengthPercentageValue::Length)
}

/// Parse a CSS margin value: "10px", "auto", "0", etc.
fn parse_css_margin(css: &str) -> Option<LengthPercentageAutoValue> {
    let css = css.trim();
    if css == "auto" {
        return Some(LengthPercentageAutoValue::Auto);
    }
    parse_css_px_value(css).map(LengthPercentageAutoValue::Length)
}

/// Parse a pixel value from CSS text like "100px", "0".
fn parse_css_px_value(css: &str) -> Option<f32> {
    let css = css.trim();
    if css == "0" {
        return Some(0.0);
    }
    if let Some(num) = css.strip_suffix("px") {
        return num.parse().ok();
    }
    // Try parsing as bare number (some properties serialize without units when 0)
    if let Ok(v) = css.parse::<f32>() {
        return Some(v);
    }
    None
}

/// Parse a CSS transform value into individual TransformOp components.
/// This preserves the individual operations (rotate, scale, etc.) so they can
/// be interpolated component-wise rather than as matrices.
fn parse_css_transform_components(css: &str) -> Option<Vec<TransformOp>> {
    let css = css.trim();
    if css == "none" {
        return Some(vec![TransformOp::Scale(1.0, 1.0)]); // identity as scale
    }

    let mut ops = Vec::new();

    let mut remaining = css;
    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        if let Some((func, args, rest)) = parse_transform_function(remaining) {
            if let Some(op) = eval_transform_to_op(&func, &args) {
                ops.push(op);
            }
            remaining = rest;
        } else {
            break;
        }
    }

    if ops.is_empty() { None } else { Some(ops) }
}

/// Convert a transform function to a TransformOp (preserving the operation type).
fn eval_transform_to_op(func: &str, args: &str) -> Option<TransformOp> {
    match func {
        "rotate" => {
            let rad = parse_angle_value(args)?;
            Some(TransformOp::Rotate(rad))
        }
        "scale" => {
            let parts: Vec<&str> = args.split(',').map(str::trim).collect();
            let sx: f64 = parts.first()?.parse().ok()?;
            let sy: f64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(sx);
            Some(TransformOp::Scale(sx, sy))
        }
        "scaleX" => {
            let sx: f64 = args.trim().parse().ok()?;
            Some(TransformOp::Scale(sx, 1.0))
        }
        "scaleY" => {
            let sy: f64 = args.trim().parse().ok()?;
            Some(TransformOp::Scale(1.0, sy))
        }
        "translateX" => {
            let tx = parse_css_px_value(args)? as f64;
            Some(TransformOp::Translate(tx, 0.0))
        }
        "translateY" => {
            let ty = parse_css_px_value(args)? as f64;
            Some(TransformOp::Translate(0.0, ty))
        }
        "translate" => {
            let parts: Vec<&str> = args.split(',').map(str::trim).collect();
            let tx = parse_css_px_value(parts.first()?)? as f64;
            let ty = parts
                .get(1)
                .and_then(|s| parse_css_px_value(s))
                .unwrap_or(0.0) as f64;
            Some(TransformOp::Translate(tx, ty))
        }
        "skewX" => {
            let rad = parse_angle_value(args)?;
            Some(TransformOp::SkewX(rad))
        }
        "skewY" => {
            let rad = parse_angle_value(args)?;
            Some(TransformOp::SkewY(rad))
        }
        "matrix" => {
            let parts: Vec<f64> = args
                .split(',')
                .map(|s| s.trim().parse().unwrap_or(0.0))
                .collect();
            if parts.len() >= 6 {
                Some(TransformOp::Matrix([
                    parts[0], parts[1], parts[2], parts[3], parts[4], parts[5],
                ]))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_transform_function(css: &str) -> Option<(String, String, &str)> {
    let open = css.find('(')?;
    let func = css[..open].trim().to_string();
    let close = css[open..].find(')')? + open;
    let args = css[open + 1..close].trim().to_string();
    Some((func, args, &css[close + 1..]))
}

fn parse_angle_value(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(deg) = s.strip_suffix("deg") {
        let d: f64 = deg.trim().parse().ok()?;
        Some(d * std::f64::consts::PI / 180.0)
    } else if let Some(rad) = s.strip_suffix("rad") {
        rad.trim().parse().ok()
    } else if let Some(turn) = s.strip_suffix("turn") {
        let t: f64 = turn.trim().parse().ok()?;
        Some(t * 2.0 * std::f64::consts::PI)
    } else if let Some(grad) = s.strip_suffix("grad") {
        let g: f64 = grad.trim().parse().ok()?;
        Some(g * std::f64::consts::PI / 200.0)
    } else {
        // Try as bare number (radians)
        s.parse().ok()
    }
}

/// Convert a specified timing function to our TimingFunction.
fn convert_specified_timing_function(
    tf: &style::values::specified::easing::TimingFunction,
) -> Option<TimingFunction> {
    use style::values::generics::easing::TimingFunction as StyloTF;
    use style::values::generics::easing::TimingKeyword;

    Some(match tf {
        StyloTF::Keyword(kw) => match kw {
            TimingKeyword::Linear => TimingFunction::Linear,
            TimingKeyword::Ease => TimingFunction::Ease,
            TimingKeyword::EaseIn => TimingFunction::EaseIn,
            TimingKeyword::EaseOut => TimingFunction::EaseOut,
            TimingKeyword::EaseInOut => TimingFunction::EaseInOut,
        },
        StyloTF::CubicBezier { x1, y1, x2, y2 } => {
            // Specified CubicBezier uses Number type — extract f32 value
            TimingFunction::CubicBezier(x1.get(), y1.get(), x2.get(), y2.get())
        }
        StyloTF::Steps(..) | StyloTF::LinearFunction(..) => TimingFunction::Linear,
    })
}
