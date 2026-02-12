//! CSS Transitions engine.
//!
//! Handles parsing transition specs from Stylo, detecting property changes,
//! interpolating values over time, and writing intermediate values into ComputedStyle.

use std::collections::HashMap;

use peniko::Color;
use style::values::computed::TransitionProperty as StyloTransitionProperty;
use style::values::generics::easing::TimingKeyword;

use crate::computed_style::{
    BackgroundValue, ComputedStyle, DimensionValue, LengthPercentageAutoValue,
    LengthPercentageValue, TransformValue,
};
use crate::node::{DirtyFlags, NodeTree, RawNodeId};

// =============================================================================
// TransitionProperty — which CSS properties can be transitioned
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionProperty {
    All,
    Opacity,
    BackgroundColor,
    Color,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    Width,
    Height,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderRadiusTopLeft,
    BorderRadiusTopRight,
    BorderRadiusBottomRight,
    BorderRadiusBottomLeft,
    FontSize,
    Transform,
}

/// All individual animatable properties (excluding All).
const _ALL_ANIMATABLE: &[TransitionProperty] = &[
    TransitionProperty::Opacity,
    TransitionProperty::BackgroundColor,
    TransitionProperty::Color,
    TransitionProperty::BorderTopColor,
    TransitionProperty::BorderRightColor,
    TransitionProperty::BorderBottomColor,
    TransitionProperty::BorderLeftColor,
    TransitionProperty::Width,
    TransitionProperty::Height,
    TransitionProperty::PaddingTop,
    TransitionProperty::PaddingRight,
    TransitionProperty::PaddingBottom,
    TransitionProperty::PaddingLeft,
    TransitionProperty::MarginTop,
    TransitionProperty::MarginRight,
    TransitionProperty::MarginBottom,
    TransitionProperty::MarginLeft,
    TransitionProperty::BorderTopWidth,
    TransitionProperty::BorderRightWidth,
    TransitionProperty::BorderBottomWidth,
    TransitionProperty::BorderLeftWidth,
    TransitionProperty::BorderRadiusTopLeft,
    TransitionProperty::BorderRadiusTopRight,
    TransitionProperty::BorderRadiusBottomRight,
    TransitionProperty::BorderRadiusBottomLeft,
    TransitionProperty::FontSize,
    TransitionProperty::Transform,
];

impl TransitionProperty {
    /// Map a CSS property name to our enum.
    fn from_css_name(name: &str) -> Option<Self> {
        Some(match name {
            "all" => Self::All,
            "opacity" => Self::Opacity,
            "background-color" => Self::BackgroundColor,
            "color" => Self::Color,
            "border-top-color" => Self::BorderTopColor,
            "border-right-color" => Self::BorderRightColor,
            "border-bottom-color" => Self::BorderBottomColor,
            "border-left-color" => Self::BorderLeftColor,
            "width" => Self::Width,
            "height" => Self::Height,
            "padding-top" => Self::PaddingTop,
            "padding-right" => Self::PaddingRight,
            "padding-bottom" => Self::PaddingBottom,
            "padding-left" => Self::PaddingLeft,
            "margin-top" => Self::MarginTop,
            "margin-right" => Self::MarginRight,
            "margin-bottom" => Self::MarginBottom,
            "margin-left" => Self::MarginLeft,
            "border-top-width" => Self::BorderTopWidth,
            "border-right-width" => Self::BorderRightWidth,
            "border-bottom-width" => Self::BorderBottomWidth,
            "border-left-width" => Self::BorderLeftWidth,
            "border-top-left-radius" => Self::BorderRadiusTopLeft,
            "border-top-right-radius" => Self::BorderRadiusTopRight,
            "border-bottom-right-radius" => Self::BorderRadiusBottomRight,
            "border-bottom-left-radius" => Self::BorderRadiusBottomLeft,
            "font-size" => Self::FontSize,
            "transform" => Self::Transform,
            _ => return None,
        })
    }

    /// Whether this property affects layout (needs Taffy re-sync).
    pub fn affects_layout(&self) -> bool {
        !matches!(
            self,
            Self::Opacity
                | Self::BackgroundColor
                | Self::Color
                | Self::BorderTopColor
                | Self::BorderRightColor
                | Self::BorderBottomColor
                | Self::BorderLeftColor
                | Self::Transform
        )
    }
}

// =============================================================================
// TimingFunction — easing curves
// =============================================================================

#[derive(Debug, Clone, Copy, Default)]
pub enum TimingFunction {
    Linear,
    #[default]
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
}

impl TimingFunction {
    /// Evaluate the timing function at progress `t` (0.0 to 1.0).
    pub fn apply(&self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Ease => cubic_bezier_solve(0.25, 0.1, 0.25, 1.0, t),
            Self::EaseIn => cubic_bezier_solve(0.42, 0.0, 1.0, 1.0, t),
            Self::EaseOut => cubic_bezier_solve(0.0, 0.0, 0.58, 1.0, t),
            Self::EaseInOut => cubic_bezier_solve(0.42, 0.0, 0.58, 1.0, t),
            Self::CubicBezier(x1, y1, x2, y2) => cubic_bezier_solve(*x1, *y1, *x2, *y2, t),
        }
    }
}

/// Solve a cubic bezier curve for the Y value at a given X (time) position.
/// Uses Newton-Raphson iteration with bisection fallback.
fn cubic_bezier_solve(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // Find the parameter value for the given x using Newton-Raphson
    let mut guess = t;
    for _ in 0..8 {
        let x = sample_bezier(x1, x2, guess) - t;
        if x.abs() < 1e-6 {
            return sample_bezier(y1, y2, guess);
        }
        let dx = sample_bezier_derivative(x1, x2, guess);
        if dx.abs() < 1e-6 {
            break;
        }
        guess -= x / dx;
    }

    // Bisection fallback
    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    guess = t;
    for _ in 0..20 {
        let x = sample_bezier(x1, x2, guess);
        if (x - t).abs() < 1e-6 {
            return sample_bezier(y1, y2, guess);
        }
        if x < t {
            lo = guess;
        } else {
            hi = guess;
        }
        guess = (lo + hi) / 2.0;
    }
    sample_bezier(y1, y2, guess)
}

/// Sample a 1D cubic bezier at parameter `t`.
/// B(t) = 3(1-t)^2*t*p1 + 3(1-t)*t^2*p2 + t^3
fn sample_bezier(p1: f32, p2: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3
}

/// Derivative of a 1D cubic bezier at parameter `t`.
fn sample_bezier_derivative(p1: f32, p2: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

// =============================================================================
// TransitionSpec — parsed from CSS
// =============================================================================

#[derive(Debug, Clone)]
pub struct TransitionSpec {
    pub property: TransitionProperty,
    pub duration_ms: f64,
    pub delay_ms: f64,
    pub timing: TimingFunction,
}

impl TransitionSpec {
    /// Extract transition specs from Stylo's ComputedValues.
    pub fn extract_from_stylo(
        cv: &style::properties::ComputedValues,
    ) -> Vec<TransitionSpec> {
        let ui = cv.get_ui();

        if !ui.specifies_transitions() {
            return Vec::new();
        }

        let prop_count = ui.transition_property_count();
        let dur_count = ui.transition_duration_count();
        let delay_count = ui.transition_delay_count();
        let tf_count = ui.transition_timing_function_count();

        let mut specs = Vec::new();

        for i in 0..prop_count {
            let stylo_prop = ui.transition_property_at(i);

            // Map Stylo's TransitionProperty to our enum
            let property = match &stylo_prop {
                StyloTransitionProperty::NonCustom(id) => {
                    match id.longhand_or_shorthand() {
                        Ok(longhand_id) => {
                            let name = longhand_id.name();
                            TransitionProperty::from_css_name(name)
                        }
                        Err(shorthand_id) => {
                            // Check for "all" or other shorthands
                            let name = shorthand_id.name();
                            if name == "all" {
                                Some(TransitionProperty::All)
                            } else {
                                // For shorthands like "border-color", "padding", etc.
                                // we skip them — individual longhands will be listed
                                // or "all" handles them
                                None
                            }
                        }
                    }
                }
                StyloTransitionProperty::Custom(_) | StyloTransitionProperty::Unsupported(_) => {
                    None
                }
            };

            let property = match property {
                Some(p) => p,
                None => continue,
            };

            // Duration (modular indexing per CSS spec)
            let duration_s = ui.transition_duration_at(i % dur_count).seconds() as f64;
            let duration_ms = duration_s * 1000.0;

            // Delay
            let delay_s = ui.transition_delay_at(i % delay_count).seconds() as f64;
            let delay_ms = delay_s * 1000.0;

            // Timing function
            let stylo_tf = ui.transition_timing_function_at(i % tf_count);
            let timing = convert_timing_function(&stylo_tf);

            if duration_ms > 0.0 || delay_ms > 0.0 {
                specs.push(TransitionSpec {
                    property,
                    duration_ms,
                    delay_ms,
                    timing,
                });
            }
        }

        specs
    }
}

/// Convert a Stylo timing function to our TimingFunction.
fn convert_timing_function(
    tf: &style::values::computed::easing::TimingFunction,
) -> TimingFunction {
    use style::values::generics::easing::TimingFunction as StyloTF;

    match tf {
        StyloTF::Keyword(kw) => match kw {
            TimingKeyword::Linear => TimingFunction::Linear,
            TimingKeyword::Ease => TimingFunction::Ease,
            TimingKeyword::EaseIn => TimingFunction::EaseIn,
            TimingKeyword::EaseOut => TimingFunction::EaseOut,
            TimingKeyword::EaseInOut => TimingFunction::EaseInOut,
        },
        StyloTF::CubicBezier { x1, y1, x2, y2 } => {
            TimingFunction::CubicBezier(*x1, *y1, *x2, *y2)
        }
        // Steps and linear functions fall back to linear for V1
        StyloTF::Steps(..) | StyloTF::LinearFunction(..) => TimingFunction::Linear,
    }
}

// =============================================================================
// AnimatableValue — values that can be interpolated
// =============================================================================

#[derive(Debug, Clone)]
pub enum AnimatableValue {
    Float(f32),
    Color(Color),
    Dimension(DimensionValue),
    LengthPercentage(LengthPercentageValue),
    LengthPercentageAuto(LengthPercentageAutoValue),
    Transform([f64; 6]),
}

impl AnimatableValue {
    /// Interpolate between two values at progress `t` (0.0 to 1.0).
    pub fn interpolate(&self, to: &AnimatableValue, t: f32) -> Option<AnimatableValue> {
        match (self, to) {
            (AnimatableValue::Float(a), AnimatableValue::Float(b)) => {
                Some(AnimatableValue::Float(a + (b - a) * t))
            }
            (AnimatableValue::Color(a), AnimatableValue::Color(b)) => {
                Some(AnimatableValue::Color(lerp_color(*a, *b, t)))
            }
            (
                AnimatableValue::Dimension(DimensionValue::Length(a)),
                AnimatableValue::Dimension(DimensionValue::Length(b)),
            ) => Some(AnimatableValue::Dimension(DimensionValue::Length(
                a + (b - a) * t,
            ))),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Length(a)),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Length(b)),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Length(a + (b - a) * t),
            )),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Length(b)),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Length(b * t),
            )),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Length(a)),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Length(a * (1.0 - t)),
            )),
            (
                AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Length(a)),
                AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Length(b)),
            ) => Some(AnimatableValue::LengthPercentageAuto(
                LengthPercentageAutoValue::Length(a + (b - a) * t),
            )),
            (AnimatableValue::Transform(a), AnimatableValue::Transform(b)) => {
                let mut result = [0.0_f64; 6];
                for i in 0..6 {
                    result[i] = a[i] + (b[i] - a[i]) * t as f64;
                }
                Some(AnimatableValue::Transform(result))
            }
            // Incompatible types — snap immediately
            _ => None,
        }
    }
}

/// Linearly interpolate between two colors in sRGB space.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let a = a.to_rgba8();
    let b = b.to_rgba8();
    let r = (a.r as f32 + (b.r as f32 - a.r as f32) * t).round() as u8;
    let g = (a.g as f32 + (b.g as f32 - a.g as f32) * t).round() as u8;
    let bl = (a.b as f32 + (b.b as f32 - a.b as f32) * t).round() as u8;
    let alpha = (a.a as f32 + (b.a as f32 - a.a as f32) * t).round() as u8;
    Color::from_rgba8(r, g, bl, alpha)
}

// =============================================================================
// ActiveTransition — runtime state per property
// =============================================================================

#[derive(Debug, Clone)]
pub struct ActiveTransition {
    pub property: TransitionProperty,
    pub from: AnimatableValue,
    pub to: AnimatableValue,
    pub timing: TimingFunction,
    pub start_time_ms: f64,
    pub duration_ms: f64,
    pub delay_ms: f64,
}

impl ActiveTransition {
    /// Compute the current interpolated value.
    pub fn value_at(&self, current_time_ms: f64) -> Option<AnimatableValue> {
        let elapsed = current_time_ms - self.start_time_ms;

        // Still in delay period
        if elapsed < self.delay_ms {
            return Some(self.from.clone());
        }

        let active_elapsed = elapsed - self.delay_ms;
        if self.duration_ms <= 0.0 {
            return Some(self.to.clone());
        }

        let raw_t = (active_elapsed / self.duration_ms).min(1.0) as f32;
        let eased_t = self.timing.apply(raw_t);
        self.from.interpolate(&self.to, eased_t)
    }

    /// Whether this transition has completed.
    pub fn is_complete(&self, current_time_ms: f64) -> bool {
        let elapsed = current_time_ms - self.start_time_ms;
        elapsed >= self.delay_ms + self.duration_ms
    }
}

// =============================================================================
// Style diffing — detect which animatable properties changed
// =============================================================================

/// A detected change in an animatable property.
pub struct PropertyChange {
    pub property: TransitionProperty,
    pub old_value: AnimatableValue,
    pub new_value: AnimatableValue,
}

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
    true
}

// =============================================================================
// Apply interpolated value back to ComputedStyle
// =============================================================================

/// Write an interpolated AnimatableValue into the correct ComputedStyle field.
pub fn apply_value_to_style(style: &mut ComputedStyle, prop: TransitionProperty, value: &AnimatableValue) {
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

// =============================================================================
// Transition start logic
// =============================================================================

/// Find a matching TransitionSpec for a property change.
pub fn find_matching_spec(
    specs: &[TransitionSpec],
    property: TransitionProperty,
) -> Option<&TransitionSpec> {
    // First try exact match
    specs.iter().find(|spec| spec.property == property)
        // Then try "all"
        .or_else(|| specs.iter().find(|spec| spec.property == TransitionProperty::All))
}

/// Start transitions for property changes, storing them in the node tree.
/// Returns the properties that are transitioning (so the caller can preserve old values).
pub fn start_transitions(
    active_transitions: &mut HashMap<TransitionProperty, ActiveTransition>,
    specs: &[TransitionSpec],
    changes: &[PropertyChange],
    current_time_ms: f64,
) -> Vec<TransitionProperty> {
    let mut transitioning = Vec::new();

    for change in changes {
        let spec = match find_matching_spec(specs, change.property) {
            Some(s) => s,
            None => continue,
        };

        // If already transitioning this property, start reversal from current value
        let from = if let Some(existing) = active_transitions.get(&change.property) {
            // Get current interpolated value for smooth reversal
            match existing.value_at(current_time_ms) {
                Some(v) => v,
                None => change.old_value.clone(),
            }
        } else {
            change.old_value.clone()
        };

        active_transitions.insert(
            change.property,
            ActiveTransition {
                property: change.property,
                from,
                to: change.new_value.clone(),
                timing: spec.timing,
                start_time_ms: current_time_ms,
                duration_ms: spec.duration_ms,
                delay_ms: spec.delay_ms,
            },
        );

        transitioning.push(change.property);
    }

    transitioning
}

// =============================================================================
// Tick engine — advance all transitions by one frame
// =============================================================================

/// Advance all active transitions. Returns true if any transitions are still active.
///
/// For each active transition:
/// 1. Compute interpolated value
/// 2. Write to node's computed_style
/// 3. Mark node dirty (PAINT, and LAYOUT if layout-affecting)
/// 4. Remove completed transitions
pub fn tick_transitions(tree: &mut NodeTree, current_time_ms: f64) -> bool {
    let node_ids: Vec<RawNodeId> = tree.active_transitions.keys().copied().collect();
    let mut any_active = false;

    for node_id in node_ids {
        if !tree.nodes.contains(node_id) {
            tree.active_transitions.remove(&node_id);
            continue;
        }

        let transitions = match tree.active_transitions.get(&node_id) {
            Some(t) => t.clone(),
            None => continue,
        };

        let mut completed = Vec::new();
        let mut needs_layout = false;
        let mut needs_paint = false;

        for (prop, transition) in &transitions {
            if transition.is_complete(current_time_ms) {
                // Apply final value
                apply_value_to_style(
                    &mut tree.nodes[node_id].computed_style,
                    *prop,
                    &transition.to,
                );
                completed.push(*prop);
                needs_paint = true;
                if prop.affects_layout() {
                    needs_layout = true;
                }
            } else {
                // Apply interpolated value
                if let Some(value) = transition.value_at(current_time_ms) {
                    apply_value_to_style(
                        &mut tree.nodes[node_id].computed_style,
                        *prop,
                        &value,
                    );
                    needs_paint = true;
                    if prop.affects_layout() {
                        needs_layout = true;
                    }
                }
                any_active = true;
            }
        }

        // Mark dirty
        if needs_paint {
            tree.nodes[node_id].dirty.insert(DirtyFlags::PAINT);
        }
        if needs_layout {
            tree.nodes[node_id]
                .dirty
                .insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            tree.dirty_nodes.insert(node_id);
        }

        // Remove completed
        if let Some(map) = tree.active_transitions.get_mut(&node_id) {
            for prop in completed {
                map.remove(&prop);
            }
            if map.is_empty() {
                tree.active_transitions.remove(&node_id);
            }
        }
    }

    any_active
}
