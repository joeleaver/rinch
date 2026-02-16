//! Apply interpolated transition values back to ComputedStyle.

use crate::computed_style::{BackgroundValue, ComputedStyle, TransformValue};

use super::types::{AnimatableValue, TransitionProperty};

/// Write an interpolated AnimatableValue into the correct ComputedStyle field.
pub fn apply_value_to_style(
    style: &mut ComputedStyle,
    prop: TransitionProperty,
    value: &AnimatableValue,
) {
    match (prop, value) {
        (TransitionProperty::Opacity, AnimatableValue::Float(v)) => {
            style.opacity = *v;
        }
        (TransitionProperty::BackgroundColor, AnimatableValue::Color(c)) => {
            style.background = BackgroundValue::Color(*c);
        }
        (TransitionProperty::Color, AnimatableValue::Color(c)) => {
            style.color = Some(*c);
        }
        (TransitionProperty::BorderTopColor, AnimatableValue::Color(c)) => {
            style.border_top_color = Some(*c);
        }
        (TransitionProperty::BorderRightColor, AnimatableValue::Color(c)) => {
            style.border_right_color = Some(*c);
        }
        (TransitionProperty::BorderBottomColor, AnimatableValue::Color(c)) => {
            style.border_bottom_color = Some(*c);
        }
        (TransitionProperty::BorderLeftColor, AnimatableValue::Color(c)) => {
            style.border_left_color = Some(*c);
        }
        (TransitionProperty::Width, AnimatableValue::Dimension(d)) => {
            style.width = *d;
        }
        (TransitionProperty::Height, AnimatableValue::Dimension(d)) => {
            style.height = *d;
        }
        (TransitionProperty::PaddingTop, AnimatableValue::LengthPercentage(lp)) => {
            style.padding_top = *lp;
        }
        (TransitionProperty::PaddingRight, AnimatableValue::LengthPercentage(lp)) => {
            style.padding_right = *lp;
        }
        (TransitionProperty::PaddingBottom, AnimatableValue::LengthPercentage(lp)) => {
            style.padding_bottom = *lp;
        }
        (TransitionProperty::PaddingLeft, AnimatableValue::LengthPercentage(lp)) => {
            style.padding_left = *lp;
        }
        (TransitionProperty::MarginTop, AnimatableValue::LengthPercentageAuto(lpa)) => {
            style.margin_top = *lpa;
        }
        (TransitionProperty::MarginRight, AnimatableValue::LengthPercentageAuto(lpa)) => {
            style.margin_right = *lpa;
        }
        (TransitionProperty::MarginBottom, AnimatableValue::LengthPercentageAuto(lpa)) => {
            style.margin_bottom = *lpa;
        }
        (TransitionProperty::MarginLeft, AnimatableValue::LengthPercentageAuto(lpa)) => {
            style.margin_left = *lpa;
        }
        (TransitionProperty::BorderTopWidth, AnimatableValue::LengthPercentage(lp)) => {
            style.border_top_width = *lp;
        }
        (TransitionProperty::BorderRightWidth, AnimatableValue::LengthPercentage(lp)) => {
            style.border_right_width = *lp;
        }
        (TransitionProperty::BorderBottomWidth, AnimatableValue::LengthPercentage(lp)) => {
            style.border_bottom_width = *lp;
        }
        (TransitionProperty::BorderLeftWidth, AnimatableValue::LengthPercentage(lp)) => {
            style.border_left_width = *lp;
        }
        (TransitionProperty::BorderRadiusTopLeft, AnimatableValue::LengthPercentage(lp)) => {
            style.border_radius_top_left = *lp;
        }
        (TransitionProperty::BorderRadiusTopRight, AnimatableValue::LengthPercentage(lp)) => {
            style.border_radius_top_right = *lp;
        }
        (TransitionProperty::BorderRadiusBottomRight, AnimatableValue::LengthPercentage(lp)) => {
            style.border_radius_bottom_right = *lp;
        }
        (TransitionProperty::BorderRadiusBottomLeft, AnimatableValue::LengthPercentage(lp)) => {
            style.border_radius_bottom_left = *lp;
        }
        (TransitionProperty::FontSize, AnimatableValue::Float(v)) => {
            style.font_size = *v;
        }
        (TransitionProperty::Transform, AnimatableValue::Transform(m)) => {
            style.transform = TransformValue {
                matrix: *m,
                is_identity: false,
            };
        }
        _ => {}
    }
}
