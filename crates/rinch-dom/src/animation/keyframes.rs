//! Extract keyframe stop values from Stylo's KeyframesAnimation.

use style::properties::PropertyDeclaration;
use style::shared_lock::SharedRwLockReadGuard;
use style::stylesheets::keyframes_rule::{KeyframesAnimation, KeyframesStepValue};
use style::values::specified::Color as SpecifiedColor;

use crate::computed_style::{
    ComputedStyle, DimensionValue, LengthPercentageAutoValue, LengthPercentageValue,
    accumulate_pct, color_from_specified,
};
use crate::transition::types::{AnimatableValue, TransformOp, TransitionProperty};

use super::types::KeyframeStop;
use crate::transition::types::TimingFunction;

/// Extract KeyframeStops from a Stylo KeyframesAnimation.
///
/// For `ComputedValues` steps (auto-generated 0%/100%), we use `base_style`
/// values. For `Declarations` steps the stop's *typed* specified value is
/// matched directly — the same discipline `plain_inset` uses on the inset fast
/// path — rather than serialised to CSS text and re-parsed.
///
/// `parent_color` is what a `color: currentcolor` stop inherits;
/// `root_font_size` is what a `rem` stop resolves against.
pub fn extract_keyframe_stops(
    animation: &KeyframesAnimation,
    base_style: &ComputedStyle,
    parent_color: Option<peniko::Color>,
    root_font_size: f32,
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
                let values =
                    extract_declaration_values(block, base_style, parent_color, root_font_size);

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
    root_font_size: f32,
) -> Vec<(TransitionProperty, AnimatableValue)> {
    let mut values = Vec::new();

    for declaration in block.normal_declaration_iter() {
        if let Some((prop, val)) =
            convert_declaration(declaration, base_style, parent_color, root_font_size)
        {
            values.push((prop, val));
        }
    }

    values
}

/// Convert a single PropertyDeclaration to our (TransitionProperty, AnimatableValue).
///
/// Every arm reads the **typed** specified value stylo already parsed. This
/// used to serialise each declaration back to CSS text with `to_css()` and
/// re-parse it with a private mini-parser that understood `px` and nothing
/// else — so `width: 10em`, `padding: 1.25rem`, `height: 5%`,
/// `transform: rotate(calc(45deg))` and `transform: translate(50%, 0)` all
/// returned `None` and the whole stop was dropped on the floor (#255).
///
/// `border-*-width` is the one property still going through its serialisation:
/// stylo's `BorderSideWidth` is a newtype over a **private** `LineWidth` field
/// with no accessor, so there is no typed value to match from outside the
/// crate. See `border_width_px`.
fn convert_declaration(
    declaration: &PropertyDeclaration,
    base_style: &ComputedStyle,
    parent_color: Option<peniko::Color>,
    root_font_size: f32,
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

    let fs = base_style.font_size;
    let len = |lp: &SpecLengthPercentage| StopLength::resolve(lp, fs, root_font_size);

    match declaration {
        // `Opacity` is the second stylo newtype with a private field (see
        // `border_width`), but it always serialises as a bare number — its
        // `Parse` converts a `<percentage>` to a `<number>` up front — so
        // reading the serialisation loses nothing here.
        PropertyDeclaration::Opacity(_) => {
            let mut css = String::new();
            declaration.to_css(&mut css).ok()?;
            let v: f32 = css.trim().parse().ok()?;
            Some((
                TransitionProperty::Opacity,
                AnimatableValue::Float(v.clamp(0.0, 1.0)),
            ))
        }

        PropertyDeclaration::Width(w) => Some((
            TransitionProperty::Width,
            AnimatableValue::Dimension(size(w, fs, root_font_size)?),
        )),
        PropertyDeclaration::Height(h) => Some((
            TransitionProperty::Height,
            AnimatableValue::Dimension(size(h, fs, root_font_size)?),
        )),

        PropertyDeclaration::PaddingTop(p) => Some((
            TransitionProperty::PaddingTop,
            AnimatableValue::LengthPercentage(len(&p.0)?.length_percentage()),
        )),
        PropertyDeclaration::PaddingRight(p) => Some((
            TransitionProperty::PaddingRight,
            AnimatableValue::LengthPercentage(len(&p.0)?.length_percentage()),
        )),
        PropertyDeclaration::PaddingBottom(p) => Some((
            TransitionProperty::PaddingBottom,
            AnimatableValue::LengthPercentage(len(&p.0)?.length_percentage()),
        )),
        PropertyDeclaration::PaddingLeft(p) => Some((
            TransitionProperty::PaddingLeft,
            AnimatableValue::LengthPercentage(len(&p.0)?.length_percentage()),
        )),

        PropertyDeclaration::MarginTop(m) => Some((
            TransitionProperty::MarginTop,
            AnimatableValue::LengthPercentageAuto(margin(m, fs, root_font_size)?),
        )),
        PropertyDeclaration::MarginRight(m) => Some((
            TransitionProperty::MarginRight,
            AnimatableValue::LengthPercentageAuto(margin(m, fs, root_font_size)?),
        )),
        PropertyDeclaration::MarginBottom(m) => Some((
            TransitionProperty::MarginBottom,
            AnimatableValue::LengthPercentageAuto(margin(m, fs, root_font_size)?),
        )),
        PropertyDeclaration::MarginLeft(m) => Some((
            TransitionProperty::MarginLeft,
            AnimatableValue::LengthPercentageAuto(margin(m, fs, root_font_size)?),
        )),

        PropertyDeclaration::BorderTopWidth(_) => Some((
            TransitionProperty::BorderTopWidth,
            AnimatableValue::LengthPercentage(border_width(declaration, fs, root_font_size)?),
        )),
        PropertyDeclaration::BorderRightWidth(_) => Some((
            TransitionProperty::BorderRightWidth,
            AnimatableValue::LengthPercentage(border_width(declaration, fs, root_font_size)?),
        )),
        PropertyDeclaration::BorderBottomWidth(_) => Some((
            TransitionProperty::BorderBottomWidth,
            AnimatableValue::LengthPercentage(border_width(declaration, fs, root_font_size)?),
        )),
        PropertyDeclaration::BorderLeftWidth(_) => Some((
            TransitionProperty::BorderLeftWidth,
            AnimatableValue::LengthPercentage(border_width(declaration, fs, root_font_size)?),
        )),

        // `font-size` resolves against the *parent's* font size, which the
        // extractor does not have — the element's own is already the resolved
        // one. So `em` and `%` are declined here rather than resolved against
        // the wrong base; `rem` is exact, because its base is the root.
        PropertyDeclaration::FontSize(f) => {
            use style::values::specified::FontSize;
            let px = match f {
                FontSize::Length(lp) => match StopLength::resolve_no_em(lp, root_font_size)? {
                    StopLength::Px(px) => px,
                    StopLength::Percent(_) => return None,
                },
                _ => return None,
            };
            Some((TransitionProperty::FontSize, AnimatableValue::Float(px)))
        }

        PropertyDeclaration::Transform(t) => {
            let (ops, pct_translate_w, pct_translate_h) = transform_ops(t, fs, root_font_size)?;
            Some((
                TransitionProperty::Transform,
                AnimatableValue::TransformComponents {
                    ops,
                    pct_translate_w,
                    pct_translate_h,
                },
            ))
        }

        _ => None, // Unsupported property — silently skip
    }
}

/// `width`/`height`: `auto`, a length or a percentage. `min-content` and its
/// siblings are `#[animation(error)]` in stylo and are declined here too.
fn size(
    s: &style::values::specified::Size,
    font_size: f32,
    root_font_size: f32,
) -> Option<DimensionValue> {
    use style::values::generics::length::GenericSize;
    match s {
        GenericSize::Auto => Some(DimensionValue::Auto),
        GenericSize::LengthPercentage(lp) => {
            Some(StopLength::resolve(&lp.0, font_size, root_font_size)?.dimension())
        }
        _ => None,
    }
}

/// `margin-*`: `auto`, a length or a percentage. The two anchor-positioning
/// variants need an anchor element and are declined.
fn margin(
    m: &style::values::specified::length::Margin,
    font_size: f32,
    root_font_size: f32,
) -> Option<LengthPercentageAutoValue> {
    use style::values::generics::length::GenericMargin;
    match m {
        GenericMargin::Auto => Some(LengthPercentageAutoValue::Auto),
        GenericMargin::LengthPercentage(lp) => {
            Some(StopLength::resolve(lp, font_size, root_font_size)?.length_percentage_auto())
        }
        _ => None,
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
// Specified-value resolution
// =============================================================================

type SpecLengthPercentage = style::values::specified::LengthPercentage;

/// A `<length-percentage>` from an authored keyframe stop, resolved as far as
/// the extractor can take it.
///
/// It is deliberately *not* total. A keyframe stop is read before layout, with
/// no stylo `Context`, so anything needing more than a font size is declined —
/// by name, so the gaps are a list rather than a shrug:
///
/// - `calc()` — may mix lengths and percentages, and flattening needs the
///   containing block.
/// - `vw`/`vh`/`vmin`/`vmax`/`svh`/… — needs the viewport (stylo's `Device` is
///   not threaded into the extractor).
/// - `cqw`/`cqh`/… — needs a query container.
/// - every font-relative unit but `em` and `rem`: `ex`, `ch`, `cap`, `ic`,
///   `lh` and their `r` forms need font metrics we do not carry here.
///
/// A declined value means the whole stop is dropped for that property, exactly
/// as before — the point of #255 is that the *list above* used to also contain
/// `em`, `rem` and every percentage.
#[derive(Clone, Copy)]
enum StopLength {
    Px(f32),
    Percent(f32),
}

impl StopLength {
    fn resolve(lp: &SpecLengthPercentage, font_size: f32, root_font_size: f32) -> Option<Self> {
        use style::values::specified::length::{FontRelativeLength, NoCalcLength};
        match lp {
            SpecLengthPercentage::Length(NoCalcLength::Absolute(abs)) => {
                let px = abs.to_px();
                px.is_finite().then_some(Self::Px(px))
            }
            SpecLengthPercentage::Length(NoCalcLength::FontRelative(FontRelativeLength::Em(v))) => {
                Some(Self::Px(v * font_size))
            }
            SpecLengthPercentage::Length(NoCalcLength::FontRelative(FontRelativeLength::Rem(
                v,
            ))) => Some(Self::Px(v * root_font_size)),
            SpecLengthPercentage::Percentage(pct) => Some(Self::Percent(pct.0)),
            _ => None,
        }
    }

    /// `resolve` without the `em` arm, for a property whose `em` is relative to
    /// something the extractor does not have.
    fn resolve_no_em(lp: &SpecLengthPercentage, root_font_size: f32) -> Option<Self> {
        use style::values::specified::length::{FontRelativeLength, NoCalcLength};
        if matches!(
            lp,
            SpecLengthPercentage::Length(NoCalcLength::FontRelative(FontRelativeLength::Em(_)))
        ) {
            return None;
        }
        Self::resolve(lp, 0.0, root_font_size)
    }

    fn dimension(self) -> DimensionValue {
        match self {
            Self::Px(px) => DimensionValue::Length(px),
            Self::Percent(p) => DimensionValue::Percent(p),
        }
    }

    fn length_percentage(self) -> LengthPercentageValue {
        match self {
            // A zero length is unitless, and `LengthPercentageValue` has a
            // variant that says so. It matters: `Zero` interpolates against a
            // percentage as well as against a length, so `padding: 0` to
            // `padding: 20%` animates instead of snapping.
            Self::Px(0.0) => LengthPercentageValue::Zero,
            Self::Px(px) => LengthPercentageValue::Length(px),
            Self::Percent(p) => LengthPercentageValue::Percent(p),
        }
    }

    fn length_percentage_auto(self) -> LengthPercentageAutoValue {
        match self {
            Self::Px(px) => LengthPercentageAutoValue::Length(px),
            Self::Percent(p) => LengthPercentageAutoValue::Percent(p),
        }
    }
}

/// `border-*-width`, the one property that has to go through its own CSS
/// serialisation.
///
/// `style::values::specified::BorderSideWidth` is `struct BorderSideWidth(LineWidth)`
/// with a **private** field and no accessor, so there is no way to match its
/// typed value from outside stylo — `to_css()` or a full `computed::Context`
/// are the only doors, and building a `Context` here is the larger change this
/// extractor still wants. `border-width` takes no percentage in CSS, so the
/// grammar this has to cover is just `thin | medium | thick | <length>`.
fn border_width(
    declaration: &PropertyDeclaration,
    font_size: f32,
    root_font_size: f32,
) -> Option<LengthPercentageValue> {
    let mut css = String::new();
    declaration.to_css(&mut css).ok()?;
    let css = css.trim();
    // The CSS 2.1 keyword widths, as stylo computes them.
    let px = match css {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        _ => serialised_length_px(css, font_size, root_font_size)?,
    };
    Some(LengthPercentageValue::Length(px))
}

/// A length from stylo's own serialisation: `10px`, `1.5em`, `0.5rem`, `0`.
/// Anything else — a `calc()`, a viewport unit, a container unit — is declined,
/// same as [`StopLength::resolve`].
fn serialised_length_px(css: &str, font_size: f32, root_font_size: f32) -> Option<f32> {
    let css = css.trim();
    if css == "0" {
        return Some(0.0);
    }
    for (suffix, scale) in [("px", 1.0), ("rem", root_font_size), ("em", font_size)] {
        if let Some(num) = css.strip_suffix(suffix) {
            let v: f32 = num.trim().parse().ok()?;
            return (v * scale).is_finite().then_some(v * scale);
        }
    }
    None
}

/// A specified `transform` list as component ops, plus the percentage part of
/// its translates as the linear form in (width, height) — the #212 channel,
/// accumulated by the same `accumulate_pct` the cascade uses.
///
/// Percentage translates are why this is typed now: `translate(50%, 0)` used to
/// be parsed by a `strip_suffix("px")` and dropped, taking the whole transform
/// with it (#412). `calc(45deg)` came back the same way — stylo folds a
/// `calc()` angle into `AngleDimension::Deg`, so `Angle::radians()` handles it
/// where `strip_suffix("deg")` could not.
///
/// The 3D operations stay unimplemented and flatten to identity, matching
/// `transform_from_stylo`'s own `_` arm (#405). `translate3d` is spelled out
/// rather than left to that arm for the same reason it is there: it carries two
/// `LengthPercentage`s, and dropping it silently would lose a whole
/// translation.
fn transform_ops(
    transform: &style::values::specified::Transform,
    font_size: f32,
    root_font_size: f32,
) -> Option<(Vec<TransformOp>, [f64; 2], [f64; 2])> {
    use style::values::generics::transform::GenericTransformOperation as Op;

    // `transform: none`. Identity as a scale, so it interpolates
    // component-wise against a `scale()` stop rather than falling back to
    // matrix interpolation.
    if transform.0.is_empty() {
        return Some((vec![TransformOp::Scale(1.0, 1.0)], [0.0; 2], [0.0; 2]));
    }

    let mut ops: Vec<TransformOp> = Vec::new();
    let mut m = [1.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut pct_w = [0.0_f64; 2];
    let mut pct_h = [0.0_f64; 2];

    // A translate's px part goes into the op; its percentage part goes into the
    // linear form, accumulated against the matrix *as composed so far*.
    let split = |lp: &SpecLengthPercentage| match StopLength::resolve(lp, font_size, root_font_size)
    {
        Some(StopLength::Px(px)) => Some((px as f64, 0.0)),
        Some(StopLength::Percent(p)) => Some((0.0, p as f64)),
        None => None,
    };

    for op in transform.0.iter() {
        let top = match op {
            Op::Matrix(mat) => TransformOp::Matrix([
                mat.a.get() as f64,
                mat.b.get() as f64,
                mat.c.get() as f64,
                mat.d.get() as f64,
                mat.e.get() as f64,
                mat.f.get() as f64,
            ]),
            Op::Rotate(angle) => TransformOp::Rotate(angle.radians() as f64),
            Op::Scale(sx, sy) => TransformOp::Scale(sx.get() as f64, sy.get() as f64),
            Op::ScaleX(sx) => TransformOp::Scale(sx.get() as f64, 1.0),
            Op::ScaleY(sy) => TransformOp::Scale(1.0, sy.get() as f64),
            Op::SkewX(angle) => TransformOp::SkewX(angle.radians() as f64),
            Op::SkewY(angle) => TransformOp::SkewY(angle.radians() as f64),
            Op::Skew(ax, ay) => TransformOp::Matrix([
                1.0,
                (ay.radians() as f64).tan(),
                (ax.radians() as f64).tan(),
                1.0,
                0.0,
                0.0,
            ]),
            Op::TranslateX(tx) => {
                let (px, pct) = split(tx)?;
                accumulate_pct(&m, pct, 0.0, &mut pct_w, &mut pct_h);
                TransformOp::Translate(px, 0.0)
            }
            Op::TranslateY(ty) => {
                let (py, pct) = split(ty)?;
                accumulate_pct(&m, 0.0, pct, &mut pct_w, &mut pct_h);
                TransformOp::Translate(0.0, py)
            }
            Op::Translate(tx, ty) => {
                let (px, x_pct) = split(tx)?;
                let (py, y_pct) = split(ty)?;
                accumulate_pct(&m, x_pct, y_pct, &mut pct_w, &mut pct_h);
                TransformOp::Translate(px, py)
            }
            Op::Translate3D(tx, ty, _tz) => {
                let (px, x_pct) = split(tx)?;
                let (py, y_pct) = split(ty)?;
                accumulate_pct(&m, x_pct, y_pct, &mut pct_w, &mut pct_h);
                TransformOp::Translate(px, py)
            }
            // 3D operations flatten to identity, as in the cascade (#405).
            _ => TransformOp::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        };

        let o = top.to_matrix();
        m = [
            m[0] * o[0] + m[2] * o[1],
            m[1] * o[0] + m[3] * o[1],
            m[0] * o[2] + m[2] * o[3],
            m[1] * o[2] + m[3] * o[3],
            m[0] * o[4] + m[2] * o[5] + m[4],
            m[1] * o[4] + m[3] * o[5] + m[5],
        ];
        ops.push(top);
    }

    Some((ops, pct_w, pct_h))
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
