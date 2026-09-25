//! Transition types: TransitionProperty, TimingFunction, TransitionSpec, AnimatableValue, ActiveTransition.

use peniko::Color;

use style::values::computed::TransitionProperty as StyloTransitionProperty;
use style::values::generics::easing::TimingKeyword;

use super::transform::{Affine, TransformOp, compose, interpolate_lists, lists_equivalent};
use crate::computed_style::{
    DimensionValue, LengthPercentageAutoValue, LengthPercentageValue, TransformValue,
    VisibilityValue,
};

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
    /// `visibility`, interpolated by css-values-4's rule for it: a discrete
    /// step in which every progress strictly between 0 and 1 is `visible` when
    /// either end is (#759). See [`interpolate_visibility`].
    Visibility,
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
    TransitionProperty::Visibility,
];

impl TransitionProperty {
    /// Map a CSS property name to our enum.
    pub(super) fn from_css_name(name: &str) -> Option<Self> {
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
            "visibility" => Self::Visibility,
            _ => return None,
        })
    }

    /// Whether interpolating this property changes how text **measures** — the
    /// shaped width of a run, and the number of line boxes it breaks into.
    ///
    /// A different question from [`Self::affects_layout`], which asks whether
    /// the *Taffy* style has to be rebuilt. `font-size` is the only property in
    /// this enum whose effect on a box runs through text
    /// measurement rather than through a Taffy field — it reaches a Taffy field
    /// only via a value that *uses* it, an `em` length or a `line-height`
    /// multiplier — so a frame of a `transition: font-size` re-wraps the text
    /// and leaves the box around it at the size it was measured at one frame
    /// earlier, unless the derived layout and the cached measure are dropped.
    /// That is what this predicate gates (issue #678).
    ///
    /// **The `All` arm is unreachable today**, and that is the good outcome
    /// rather than a hedge. `start_transitions` inserts under
    /// `change.property`, and the changes come from `diff_animatable` over
    /// `_ALL_ANIMATABLE`, which excludes `All`; animation keyframes carry
    /// concrete properties too. So `transition: all 150ms ease` — which
    /// `checkbox.rs` and `radio.rs` both declare — pays a text-measure
    /// invalidation only on the frames where `font-size` is what changed. The
    /// arm stays as the safe answer if a future path ever does key a map by the
    /// wildcard: a spare invalidation rather than a stale box.
    pub fn changes_text_measure(&self) -> bool {
        matches!(self, Self::FontSize | Self::All)
    }

    /// Whether a transition on this property animates an inset (`left`,
    /// `top`, `right`, `bottom`).
    ///
    /// Asked by the `set_style` inset fast path, which writes an inset
    /// straight to `ComputedStyle` and Taffy without running the cascade's
    /// transition hooks (#280): it declines for a node whose `transition`
    /// covers an inset, so such a move takes the cascade and its transition
    /// hooks.
    ///
    /// No variant answers `true` today — no inset is animatable — and the
    /// match is exhaustive on purpose, so adding `Left`/`Top`/… here forces
    /// the question. `All` answers for whatever `_ALL_ANIMATABLE` holds, so
    /// it follows along without an edit.
    pub fn covers_inset(&self) -> bool {
        match self {
            Self::All => _ALL_ANIMATABLE.iter().any(|p| p.covers_inset()),
            Self::Opacity
            | Self::BackgroundColor
            | Self::Color
            | Self::BorderTopColor
            | Self::BorderRightColor
            | Self::BorderBottomColor
            | Self::BorderLeftColor
            | Self::Width
            | Self::Height
            | Self::PaddingTop
            | Self::PaddingRight
            | Self::PaddingBottom
            | Self::PaddingLeft
            | Self::MarginTop
            | Self::MarginRight
            | Self::MarginBottom
            | Self::MarginLeft
            | Self::BorderTopWidth
            | Self::BorderRightWidth
            | Self::BorderBottomWidth
            | Self::BorderLeftWidth
            | Self::BorderRadiusTopLeft
            | Self::BorderRadiusTopRight
            | Self::BorderRadiusBottomRight
            | Self::BorderRadiusBottomLeft
            | Self::FontSize
            | Self::Transform
            | Self::Visibility => false,
        }
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
                | Self::Visibility
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
    pub fn extract_from_stylo(cv: &style::properties::ComputedValues) -> Vec<TransitionSpec> {
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
pub fn convert_timing_function(
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
        StyloTF::CubicBezier { x1, y1, x2, y2 } => TimingFunction::CubicBezier(*x1, *y1, *x2, *y2),
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
    Transform(AnimatableTransform),
    /// A `visibility` value — discrete, see [`interpolate_visibility`].
    Visibility(VisibilityValue),
}

/// A transform captured for interpolation: the computed **function list**
/// (#414).
///
/// It used to be the composed matrix plus the percentage-translate
/// coefficients (#403), lerped entry by entry. That kept a percentage
/// translate alive but made every rotation shrink toward its midpoint; the
/// function list is what CSS interpolates, and a percentage translate rides
/// inside its `Translate` function (see [`super::transform`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatableTransform {
    /// The functions, in list order. Empty is `none`.
    pub functions: Vec<TransformOp>,
}

impl AnimatableTransform {
    /// The interpolable projection of a computed transform.
    pub fn from_style(tf: &TransformValue) -> Self {
        Self {
            functions: tf.functions.clone(),
        }
    }

    /// The composed transform, percentage coefficients included.
    pub fn composed(&self) -> Affine {
        compose(&self.functions)
    }

    /// Write an interpolated transform back into a computed style.
    ///
    /// `is_identity` is `false` unconditionally: an element mid-transition has
    /// a transform even on the frame where it happens to compose to the
    /// identity, and the flag also decides whether the element establishes a
    /// stacking context — which must not flicker across the animation.
    pub fn to_style(&self) -> TransformValue {
        let c = self.composed();
        TransformValue {
            matrix: c.matrix,
            is_identity: false,
            pct_translate_w: c.pct_w,
            pct_translate_h: c.pct_h,
            functions: self.functions.clone(),
        }
    }

    fn interpolate(&self, to: &AnimatableTransform, t: f64) -> Self {
        Self {
            functions: interpolate_lists(&self.functions, &to.functions, t),
        }
    }
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

            // Percentage-to-percentage, for all three value types (#255).
            //
            // Until an authored keyframe stop could carry a percentage at all
            // there was nothing to interpolate, so these arms did not exist —
            // and adding `Percent` to the extractor without adding them would
            // have made a percentage stop *step* at 50% instead of animating,
            // via the `_ => None` snap below. Better than being dropped, but
            // not the fix.
            (
                AnimatableValue::Dimension(DimensionValue::Percent(a)),
                AnimatableValue::Dimension(DimensionValue::Percent(b)),
            ) => Some(AnimatableValue::Dimension(DimensionValue::Percent(
                a + (b - a) * t,
            ))),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Percent(a)),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Percent(b)),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Percent(a + (b - a) * t),
            )),
            (
                AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Percent(a)),
                AnimatableValue::LengthPercentageAuto(LengthPercentageAutoValue::Percent(b)),
            ) => Some(AnimatableValue::LengthPercentageAuto(
                LengthPercentageAutoValue::Percent(a + (b - a) * t),
            )),

            // `Zero` is unitless, so it pairs with a percentage as readily as
            // with a length — `padding: 0` to `padding: 10%` is a legal
            // animation and must not snap.
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Percent(b)),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Percent(b * t),
            )),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Percent(a)),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Percent(a * (1.0 - t)),
            )),
            (
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
                AnimatableValue::LengthPercentage(LengthPercentageValue::Zero),
            ) => Some(AnimatableValue::LengthPercentage(
                LengthPercentageValue::Zero,
            )),
            (AnimatableValue::Transform(a), AnimatableValue::Transform(b)) => {
                Some(AnimatableValue::Transform(a.interpolate(b, t as f64)))
            }
            (AnimatableValue::Visibility(a), AnimatableValue::Visibility(b)) => Some(
                AnimatableValue::Visibility(interpolate_visibility(*a, *b, t)),
            ),
            // Incompatible types — snap immediately.
            //
            // A length against a percentage lands here on purpose: CSS
            // interpolates that pair as a `calc()`, and there is no value in
            // `ComputedStyle` that can hold one. Snapping is wrong, but it is
            // the same wrong the transition path has always been, and
            // inventing a resolution here would need the containing block.
            _ => None,
        }
    }

    /// Whether two animatable values denote the **same computed value**.
    ///
    /// css-transitions-1 §3 "Starting of transitions" compares a running
    /// transition's end value against the value in the after-change style, and
    /// both are computed values off the same resolution path — so this is an
    /// equality test, not a distance test. It uses the tolerances the style
    /// differ uses (a thousandth of a pixel for lengths, exact 8-bit channels
    /// for colours) so that "the differ saw a change" and
    /// "the running transition is already serving that change" are decided on
    /// one notion of sameness.
    ///
    /// Two values of different kinds are never the same value, with one
    /// deliberate exception: `LengthPercentage`'s unitless `Zero` and a zero
    /// `Length` are the same computed value and compare equal. A percentage
    /// compares only against a percentage — resolving one needs a containing
    /// block, which style resolution does not have at diff time.
    ///
    /// Two transforms are the same value when interpolating between them would
    /// hold still — see [`lists_equivalent`](super::transform::lists_equivalent).
    pub fn same_computed_value(&self, other: &AnimatableValue) -> bool {
        use super::diff::{approx_eq, colors_equal};
        match (self, other) {
            (AnimatableValue::Float(a), AnimatableValue::Float(b)) => approx_eq(*a, *b),
            (AnimatableValue::Color(a), AnimatableValue::Color(b)) => colors_equal(*a, *b),
            (AnimatableValue::Dimension(a), AnimatableValue::Dimension(b)) => dimension_eq(a, b),
            (AnimatableValue::LengthPercentage(a), AnimatableValue::LengthPercentage(b)) => {
                length_percentage_eq(a, b)
            }
            (
                AnimatableValue::LengthPercentageAuto(a),
                AnimatableValue::LengthPercentageAuto(b),
            ) => length_percentage_auto_eq(a, b),
            (AnimatableValue::Transform(a), AnimatableValue::Transform(b)) => {
                lists_equivalent(&a.functions, &b.functions)
            }
            (AnimatableValue::Visibility(a), AnimatableValue::Visibility(b)) => a == b,
            _ => false,
        }
    }
}

/// `visibility` at eased progress `p` (css-values-4 §3, "Combining Values":
/// *visibility* is "discrete, except that if one of the values is `visible`,
/// interpolated as a discrete step where values of p between 0 and 1 map to
/// `visible` and other values of p map to the closer endpoint").
///
/// So a closing overlay (`visible → hidden`) stays visible for the whole
/// transition and vanishes at its end, and an opening one (`hidden → visible`)
/// is visible from the first step. Without a `visible` end (`hidden ↔
/// collapse`) it is the ordinary discrete rule, which flips at 50%. A `p`
/// outside `0..=1` (an overshooting `cubic-bezier`) is the closer endpoint.
pub fn interpolate_visibility(
    from: VisibilityValue,
    to: VisibilityValue,
    p: f32,
) -> VisibilityValue {
    let either_visible = from == VisibilityValue::Visible || to == VisibilityValue::Visible;
    if either_visible {
        if p <= 0.0 {
            from
        } else if p >= 1.0 {
            to
        } else {
            VisibilityValue::Visible
        }
    } else if p < 0.5 {
        from
    } else {
        to
    }
}

fn dimension_eq(a: &DimensionValue, b: &DimensionValue) -> bool {
    use super::diff::approx_eq;
    match (a, b) {
        (DimensionValue::Auto, DimensionValue::Auto) => true,
        (DimensionValue::Length(x), DimensionValue::Length(y)) => approx_eq(*x, *y),
        (DimensionValue::Percent(x), DimensionValue::Percent(y)) => approx_eq(*x, *y),
        (
            DimensionValue::Calc { px: px1, pct: pct1 },
            DimensionValue::Calc { px: px2, pct: pct2 },
        ) => approx_eq(*px1, *px2) && approx_eq(*pct1, *pct2),
        _ => false,
    }
}

fn length_percentage_eq(a: &LengthPercentageValue, b: &LengthPercentageValue) -> bool {
    use super::diff::approx_eq;
    match (a, b) {
        (LengthPercentageValue::Zero, LengthPercentageValue::Zero) => true,
        (LengthPercentageValue::Zero, LengthPercentageValue::Length(v))
        | (LengthPercentageValue::Length(v), LengthPercentageValue::Zero) => approx_eq(*v, 0.0),
        (LengthPercentageValue::Length(x), LengthPercentageValue::Length(y)) => approx_eq(*x, *y),
        (LengthPercentageValue::Percent(x), LengthPercentageValue::Percent(y)) => approx_eq(*x, *y),
        (
            LengthPercentageValue::Calc { px: px1, pct: pct1 },
            LengthPercentageValue::Calc { px: px2, pct: pct2 },
        ) => approx_eq(*px1, *px2) && approx_eq(*pct1, *pct2),
        _ => false,
    }
}

fn length_percentage_auto_eq(a: &LengthPercentageAutoValue, b: &LengthPercentageAutoValue) -> bool {
    use super::diff::approx_eq;
    match (a, b) {
        (LengthPercentageAutoValue::Auto, LengthPercentageAutoValue::Auto) => true,
        (LengthPercentageAutoValue::Length(x), LengthPercentageAutoValue::Length(y)) => {
            approx_eq(*x, *y)
        }
        (LengthPercentageAutoValue::Percent(x), LengthPercentageAutoValue::Percent(y)) => {
            approx_eq(*x, *y)
        }
        (
            LengthPercentageAutoValue::Calc { px: px1, pct: pct1 },
            LengthPercentageAutoValue::Calc { px: px2, pct: pct2 },
        ) => approx_eq(*px1, *px2) && approx_eq(*pct1, *pct2),
        _ => false,
    }
}

/// Linearly interpolate between two colors in sRGB space.
///
/// Per CSS spec, `transparent` interpolates as the other color with alpha 0,
/// not as rgba(0,0,0,0). This avoids dark flashes when transitioning between
/// transparent and a light color.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let mut a = a.to_rgba8();
    let mut b = b.to_rgba8();

    // When one side is fully transparent, inherit RGB from the opaque side
    // so the transition only fades alpha, matching browser behavior.
    if a.a == 0 && b.a != 0 {
        a.r = b.r;
        a.g = b.g;
        a.b = b.b;
    } else if b.a == 0 && a.a != 0 {
        b.r = a.r;
        b.g = a.g;
        b.b = a.b;
    }

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
    /// The duration this transition will actually take. It is the declared
    /// `transition-duration` for every transition but a reversal, which
    /// css-transitions-1 §3 shortens by [`Self::reversing_shortening_factor`].
    pub duration_ms: f64,
    pub delay_ms: f64,
    /// The value a *reversal* of this transition would head back to
    /// (css-transitions-1 §3's "reversing-adjusted start value"). It is the
    /// start value for an ordinary transition, and the **end value of the
    /// transition it reversed** for a reversal — which is what makes reversing
    /// a reversal come back out at the right length.
    pub reversing_adjusted_start_value: AnimatableValue,
    /// The fraction of the declared **duration** this transition was given, in
    /// `0.0..=1.0`. `1.0` for everything but a reversal: a reversal taken when
    /// the transition was half way *there* is half as much travel back, so it
    /// gets half the duration.
    ///
    /// It keys on *progress*, not elapsed time, so "half way" means half way
    /// along the curve. Under `linear` the two coincide and reversing a 150ms
    /// transition after 75ms gives 75ms; under `ease` — which is what every
    /// transition in `rinch-components` declares — the output at input 0.5 is
    /// 0.8024, so the same reversal gets 120.4ms.
    ///
    /// The declared **delay** is not scaled by it unless the delay is negative;
    /// see [`ActiveTransition::reversing`].
    pub reversing_shortening_factor: f64,
}

impl ActiveTransition {
    /// Start a transition from `from` to `to` over `spec`'s declared timing.
    ///
    /// This is css-transitions-1 §3 item 1 (a property that was not
    /// transitioning) and item 4.4 (a running transition whose target changed
    /// to something that is not a reversal) alike: both take the full declared
    /// duration and reset the reversing bookkeeping to its identity.
    pub fn starting(
        property: TransitionProperty,
        from: AnimatableValue,
        to: AnimatableValue,
        spec: &TransitionSpec,
        current_time_ms: f64,
    ) -> Self {
        Self {
            property,
            reversing_adjusted_start_value: from.clone(),
            from,
            to,
            timing: spec.timing,
            start_time_ms: current_time_ms,
            duration_ms: spec.duration_ms,
            delay_ms: spec.delay_ms,
            reversing_shortening_factor: 1.0,
        }
    }

    /// Reverse `self`: head back to `to` from wherever it currently is, over a
    /// *shortened* slice of `spec`'s declared timing (css-transitions-1 §3
    /// item 4.3).
    ///
    /// The caller has already established that `to` is `self`'s
    /// reversing-adjusted start value, which is what makes this a reversal
    /// rather than a plain retarget. The new shortening factor folds `self`'s
    /// own factor in, so reversing a reversal is measured against the declared
    /// duration rather than compounding.
    pub fn reversing(
        &self,
        from: AnimatableValue,
        to: AnimatableValue,
        spec: &TransitionSpec,
        current_time_ms: f64,
    ) -> Self {
        let progress = self.output_progress_at(current_time_ms) as f64;
        let factor = ((self.reversing_shortening_factor * progress)
            + (1.0 - self.reversing_shortening_factor))
            .abs()
            .clamp(0.0, 1.0);
        Self {
            property: self.property,
            from,
            // The value this new transition would itself be reversed back to is
            // the one the transition it cancelled was heading for.
            reversing_adjusted_start_value: self.to.clone(),
            to,
            timing: spec.timing,
            start_time_ms: current_time_ms,
            duration_ms: spec.duration_ms * factor,
            // Only a **negative** delay is shortened. A negative delay is an
            // offset into the curve, so a shortened curve has to be entered
            // proportionally further along; a nonnegative delay is a *wait*
            // before the curve begins and the spec uses it as declared. Getting
            // this backwards halves a grace period: `HoverCard`'s close
            // direction carries `transition-delay: 150ms` for exactly that, and
            // a hover-out part way through the fade-in is this code path.
            delay_ms: if spec.delay_ms < 0.0 {
                spec.delay_ms * factor
            } else {
                spec.delay_ms
            },
            reversing_shortening_factor: factor,
        }
    }

    /// The timing function's **output progress** at `current_time_ms` — the
    /// eased fraction of the way from `from` to `to`, which is the quantity
    /// css-transitions-1 §3 measures a reversal's shortening against.
    ///
    /// `0` throughout a positive delay (nothing has moved yet) and `1` once the
    /// duration is spent.
    pub fn output_progress_at(&self, current_time_ms: f64) -> f32 {
        let elapsed = current_time_ms - self.start_time_ms;
        if elapsed < self.delay_ms {
            return self.timing.apply(0.0);
        }
        if self.duration_ms <= 0.0 {
            return self.timing.apply(1.0);
        }
        let raw_t = ((elapsed - self.delay_ms) / self.duration_ms).clamp(0.0, 1.0) as f32;
        self.timing.apply(raw_t)
    }

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

/// A detected change in an animatable property.
pub struct PropertyChange {
    pub property: TransitionProperty,
    pub old_value: AnimatableValue,
    pub new_value: AnimatableValue,
}
