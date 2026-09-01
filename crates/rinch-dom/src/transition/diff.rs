//! Style diffing — detect which animatable properties changed.

use peniko::Color;

use crate::computed_style::{
    BackgroundValue, ComputedStyle, DimensionValue, LengthPercentageAutoValue,
    LengthPercentageValue, TransformValue,
};

use super::types::{AnimatableValue, PropertyChange, TransitionProperty};

/// Compare two ComputedStyles and return changes in animatable properties.
pub fn diff_animatable(old: &ComputedStyle, new: &ComputedStyle) -> Vec<PropertyChange> {
    let mut changes = Vec::new();

    // Opacity
    if !approx_eq(old.opacity, new.opacity) {
        changes.push(PropertyChange {
            property: TransitionProperty::Opacity,
            old_value: AnimatableValue::Float(old.opacity),
            new_value: AnimatableValue::Float(new.opacity),
        });
    }

    // Background color
    if let (BackgroundValue::Color(old_c), BackgroundValue::Color(new_c)) =
        (&old.background, &new.background)
        && !colors_equal(*old_c, *new_c)
    {
        changes.push(PropertyChange {
            property: TransitionProperty::BackgroundColor,
            old_value: AnimatableValue::Color(*old_c),
            new_value: AnimatableValue::Color(*new_c),
        });
    }

    // Text color
    if let (Some(old_c), Some(new_c)) = (old.color, new.color)
        && !colors_equal(old_c, new_c)
    {
        changes.push(PropertyChange {
            property: TransitionProperty::Color,
            old_value: AnimatableValue::Color(old_c),
            new_value: AnimatableValue::Color(new_c),
        });
    }

    // Border colors
    diff_option_color(
        &mut changes,
        TransitionProperty::BorderTopColor,
        old.border_top_color,
        new.border_top_color,
    );
    diff_option_color(
        &mut changes,
        TransitionProperty::BorderRightColor,
        old.border_right_color,
        new.border_right_color,
    );
    diff_option_color(
        &mut changes,
        TransitionProperty::BorderBottomColor,
        old.border_bottom_color,
        new.border_bottom_color,
    );
    diff_option_color(
        &mut changes,
        TransitionProperty::BorderLeftColor,
        old.border_left_color,
        new.border_left_color,
    );

    // Width, Height
    diff_dimension(
        &mut changes,
        TransitionProperty::Width,
        &old.width,
        &new.width,
    );
    diff_dimension(
        &mut changes,
        TransitionProperty::Height,
        &old.height,
        &new.height,
    );

    // Padding
    diff_lp(
        &mut changes,
        TransitionProperty::PaddingTop,
        &old.padding_top,
        &new.padding_top,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::PaddingRight,
        &old.padding_right,
        &new.padding_right,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::PaddingBottom,
        &old.padding_bottom,
        &new.padding_bottom,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::PaddingLeft,
        &old.padding_left,
        &new.padding_left,
    );

    // Margin
    diff_lpa(
        &mut changes,
        TransitionProperty::MarginTop,
        &old.margin_top,
        &new.margin_top,
    );
    diff_lpa(
        &mut changes,
        TransitionProperty::MarginRight,
        &old.margin_right,
        &new.margin_right,
    );
    diff_lpa(
        &mut changes,
        TransitionProperty::MarginBottom,
        &old.margin_bottom,
        &new.margin_bottom,
    );
    diff_lpa(
        &mut changes,
        TransitionProperty::MarginLeft,
        &old.margin_left,
        &new.margin_left,
    );

    // Border widths
    diff_lp(
        &mut changes,
        TransitionProperty::BorderTopWidth,
        &old.border_top_width,
        &new.border_top_width,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderRightWidth,
        &old.border_right_width,
        &new.border_right_width,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderBottomWidth,
        &old.border_bottom_width,
        &new.border_bottom_width,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderLeftWidth,
        &old.border_left_width,
        &new.border_left_width,
    );

    // Border radius
    diff_lp(
        &mut changes,
        TransitionProperty::BorderRadiusTopLeft,
        &old.border_radius_top_left,
        &new.border_radius_top_left,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderRadiusTopRight,
        &old.border_radius_top_right,
        &new.border_radius_top_right,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderRadiusBottomRight,
        &old.border_radius_bottom_right,
        &new.border_radius_bottom_right,
    );
    diff_lp(
        &mut changes,
        TransitionProperty::BorderRadiusBottomLeft,
        &old.border_radius_bottom_left,
        &new.border_radius_bottom_left,
    );

    // Font size
    if !approx_eq(old.font_size, new.font_size) {
        changes.push(PropertyChange {
            property: TransitionProperty::FontSize,
            old_value: AnimatableValue::Float(old.font_size),
            new_value: AnimatableValue::Float(new.font_size),
        });
    }

    // Transform
    if !transforms_equal(&old.transform, &new.transform) {
        changes.push(PropertyChange {
            property: TransitionProperty::Transform,
            old_value: AnimatableValue::Transform(old.transform.matrix),
            new_value: AnimatableValue::Transform(new.transform.matrix),
        });
    }

    changes
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

fn colors_equal(a: Color, b: Color) -> bool {
    let a = a.to_rgba8();
    let b = b.to_rgba8();
    a.r == b.r && a.g == b.g && a.b == b.b && a.a == b.a
}

fn diff_option_color(
    changes: &mut Vec<PropertyChange>,
    prop: TransitionProperty,
    old: Option<Color>,
    new: Option<Color>,
) {
    if let (Some(a), Some(b)) = (old, new)
        && !colors_equal(a, b)
    {
        changes.push(PropertyChange {
            property: prop,
            old_value: AnimatableValue::Color(a),
            new_value: AnimatableValue::Color(b),
        });
    }
}

fn diff_dimension(
    changes: &mut Vec<PropertyChange>,
    prop: TransitionProperty,
    old: &DimensionValue,
    new: &DimensionValue,
) {
    match (old, new) {
        (DimensionValue::Length(a), DimensionValue::Length(b)) if !approx_eq(*a, *b) => {
            changes.push(PropertyChange {
                property: prop,
                old_value: AnimatableValue::Dimension(*old),
                new_value: AnimatableValue::Dimension(*new),
            });
        }
        _ => {}
    }
}

fn diff_lp(
    changes: &mut Vec<PropertyChange>,
    prop: TransitionProperty,
    old: &LengthPercentageValue,
    new: &LengthPercentageValue,
) {
    let old_px = old.to_px();
    let new_px = new.to_px();
    if !approx_eq(old_px, new_px) {
        changes.push(PropertyChange {
            property: prop,
            old_value: AnimatableValue::LengthPercentage(*old),
            new_value: AnimatableValue::LengthPercentage(*new),
        });
    }
}

fn diff_lpa(
    changes: &mut Vec<PropertyChange>,
    prop: TransitionProperty,
    old: &LengthPercentageAutoValue,
    new: &LengthPercentageAutoValue,
) {
    match (old, new) {
        (LengthPercentageAutoValue::Length(a), LengthPercentageAutoValue::Length(b))
            if !approx_eq(*a, *b) =>
        {
            changes.push(PropertyChange {
                property: prop,
                old_value: AnimatableValue::LengthPercentageAuto(*old),
                new_value: AnimatableValue::LengthPercentageAuto(*new),
            });
        }
        _ => {}
    }
}

fn transforms_equal(a: &TransformValue, b: &TransformValue) -> bool {
    if a.is_identity && b.is_identity {
        return true;
    }
    for i in 0..6 {
        if (a.matrix[i] - b.matrix[i]).abs() > 0.001 {
            return false;
        }
    }
    for i in 0..2 {
        if (a.pct_translate_w[i] - b.pct_translate_w[i]).abs() > 0.001 {
            return false;
        }
        if (a.pct_translate_h[i] - b.pct_translate_h[i]).abs() > 0.001 {
            return false;
        }
    }
    true
}
