//! Extract animation specs from Stylo's ComputedValues.

use style::properties::ComputedValues;

use crate::transition::types::TimingFunction;

use super::types::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, AnimationSpec, IterationCount,
};

impl AnimationSpec {
    /// Extract animation specs from Stylo's ComputedValues.
    /// Mirrors TransitionSpec::extract_from_stylo() pattern.
    pub fn extract_from_stylo(cv: &ComputedValues) -> Vec<AnimationSpec> {
        let ui = cv.get_ui();

        if !ui.specifies_animations() {
            return Vec::new();
        }

        let name_count = ui.animation_name_count();
        let dur_count = ui.animation_duration_count();
        let delay_count = ui.animation_delay_count();
        let tf_count = ui.animation_timing_function_count();
        let iter_count = ui.animation_iteration_count_count();
        let dir_count = ui.animation_direction_count();
        let fill_count = ui.animation_fill_mode_count();
        let play_count = ui.animation_play_state_count();

        let mut specs = Vec::new();

        for i in 0..name_count {
            let name = ui.animation_name_at(i);

            if name.is_none() {
                continue;
            }

            let name_str = match name.as_atom() {
                Some(atom) => atom.to_string(),
                None => continue,
            };

            // Duration (modular indexing per CSS spec)
            let duration_ms = {
                let dur = ui.animation_duration_at(i % dur_count);
                match dur {
                    style::values::computed::AnimationDuration::Time(t) => {
                        t.seconds() as f64 * 1000.0
                    }
                    style::values::computed::AnimationDuration::Auto => 0.0,
                }
            };

            // Delay
            let delay_ms = ui.animation_delay_at(i % delay_count).seconds() as f64 * 1000.0;

            // Timing function
            let stylo_tf = ui.animation_timing_function_at(i % tf_count);
            let timing = convert_timing_function(&stylo_tf);

            // Iteration count
            let stylo_iter = ui.animation_iteration_count_at(i % iter_count);
            let iteration_count = convert_iteration_count(&stylo_iter);

            // Direction
            let stylo_dir = ui.animation_direction_at(i % dir_count);
            let direction = convert_direction(&stylo_dir);

            // Fill mode
            let stylo_fill = ui.animation_fill_mode_at(i % fill_count);
            let fill_mode = convert_fill_mode(&stylo_fill);

            // Play state
            let stylo_play = ui.animation_play_state_at(i % play_count);
            let play_state = convert_play_state(&stylo_play);

            specs.push(AnimationSpec {
                name: name_str,
                duration_ms,
                delay_ms,
                timing,
                direction,
                iteration_count,
                fill_mode,
                play_state,
            });
        }

        specs
    }
}

/// Convert Stylo timing function to our TimingFunction.
/// Reuses the same logic as transition/types.rs.
pub(crate) fn convert_timing_function(
    tf: &style::values::computed::easing::TimingFunction,
) -> TimingFunction {
    use style::values::generics::easing::TimingFunction as StyloTF;
    use style::values::generics::easing::TimingKeyword;

    match tf {
        StyloTF::Keyword(kw) => match kw {
            TimingKeyword::Linear => TimingFunction::Linear,
            TimingKeyword::Ease => TimingFunction::Ease,
            TimingKeyword::EaseIn => TimingFunction::EaseIn,
            TimingKeyword::EaseOut => TimingFunction::EaseOut,
            TimingKeyword::EaseInOut => TimingFunction::EaseInOut,
        },
        StyloTF::CubicBezier { x1, y1, x2, y2 } => TimingFunction::CubicBezier(*x1, *y1, *x2, *y2),
        StyloTF::Steps(..) | StyloTF::LinearFunction(..) => TimingFunction::Linear,
    }
}

fn convert_iteration_count(
    ic: &style::values::computed::animation::AnimationIterationCount,
) -> IterationCount {
    // Computed AnimationIterationCount is a newtype struct wrapping f32.
    // f32::INFINITY means infinite iterations.
    if ic.0.is_infinite() {
        IterationCount::Infinite
    } else {
        IterationCount::Finite(ic.0 as f64)
    }
}

fn convert_direction(
    dir: &style::values::computed::animation::AnimationDirection,
) -> AnimationDirection {
    use style::values::computed::animation::AnimationDirection as StyloDir;
    match dir {
        StyloDir::Normal => AnimationDirection::Normal,
        StyloDir::Reverse => AnimationDirection::Reverse,
        StyloDir::Alternate => AnimationDirection::Alternate,
        StyloDir::AlternateReverse => AnimationDirection::AlternateReverse,
    }
}

fn convert_fill_mode(
    fm: &style::values::computed::animation::AnimationFillMode,
) -> AnimationFillMode {
    use style::values::computed::animation::AnimationFillMode as StyloFM;
    match fm {
        StyloFM::None => AnimationFillMode::None,
        StyloFM::Forwards => AnimationFillMode::Forwards,
        StyloFM::Backwards => AnimationFillMode::Backwards,
        StyloFM::Both => AnimationFillMode::Both,
    }
}

fn convert_play_state(
    ps: &style::values::computed::animation::AnimationPlayState,
) -> AnimationPlayState {
    use style::values::computed::animation::AnimationPlayState as StyloPS;
    match ps {
        StyloPS::Running => AnimationPlayState::Running,
        StyloPS::Paused => AnimationPlayState::Paused,
    }
}
