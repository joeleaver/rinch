//! Animation types: AnimationSpec, ActiveAnimation, KeyframeStop.

use style::stylesheets::keyframes_rule::{KeyframesAnimation, KeyframesStepValue};

use super::keyframes::StopBasis;
use crate::transition::types::{AnimatableValue, TimingFunction, TransitionProperty};

// =============================================================================
// Animation enums (mirror Stylo's, kept in our domain)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IterationCount {
    Finite(f64),
    Infinite,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

// =============================================================================
// AnimationSpec — parsed from Stylo's computed animation-* properties
// =============================================================================

#[derive(Debug, Clone)]
pub struct AnimationSpec {
    pub name: String,
    pub duration_ms: f64,
    pub delay_ms: f64,
    pub timing: TimingFunction,
    pub direction: AnimationDirection,
    pub iteration_count: IterationCount,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

// =============================================================================
// KeyframeStop — pre-extracted keyframe with our value types
// =============================================================================

#[derive(Debug, Clone)]
pub struct KeyframeStop {
    /// Percentage 0.0 to 1.0.
    pub percentage: f32,
    /// Property values at this stop.
    pub values: Vec<(TransitionProperty, AnimatableValue)>,
    /// Per-keyframe timing function override.
    pub timing_function: Option<TimingFunction>,
    /// The properties whose value in this stop is not authored but the
    /// element's **base value** — Web Animations' neutral keyframe (#781).
    ///
    /// Only a 0% or 100% stop has any: a property some keyframe of the rule
    /// animates and that stop does not set. That covers the whole stop Stylo
    /// generates for a missing `from`/`to` (`KeyframesStepValue::ComputedValues`)
    /// and a property an authored `from`/`to` leaves out. A property no
    /// keyframe names is in no stop, so the animation never writes it.
    /// [`super::keyframes::refresh_implicit_values`] re-reads these values from
    /// the current base style on every cascade of the node, so they follow it.
    pub implicit: Vec<TransitionProperty>,
}

// =============================================================================
// ActiveAnimation — runtime state for a running animation
// =============================================================================

#[derive(Debug, Clone)]
pub struct ActiveAnimation {
    pub name: String,
    pub keyframe_stops: Vec<KeyframeStop>,
    pub default_timing: TimingFunction,
    pub duration_ms: f64,
    pub delay_ms: f64,
    pub direction: AnimationDirection,
    pub iteration_count: IterationCount,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
    pub start_time_ms: f64,
    /// When paused, stores the elapsed time at the moment of pause.
    pub paused_elapsed_ms: Option<f64>,
    /// Whether a tick has written the end of this **finished** animation
    /// (#782, #783).
    ///
    /// For a `forwards`/`both` animation that end is its fill: the tick that
    /// finishes it writes the fill and marks the node dirty, and from then on
    /// its sample is constant, like a paused one's. For a `none`/`backwards`
    /// one it is *no* effect: the tick that finishes it asks for a restyle of
    /// the node, so the cascaded values replace the last sample
    /// ([`crate::node::NodeTree::ended_animations`]), and the entry is kept as
    /// a finished marker in [`crate::node::NodeTree::finished_animations`],
    /// which no tick walks — a later cascade finds it by name and does not mint
    /// the still-declared animation again from t=0, which is what a browser
    /// does (a finished CSS animation stays finished until its
    /// `animation-name` changes or its element is rendered again).
    ///
    /// Either way later ticks do not dirty anything and do not count it, and
    /// [`crate::node::NodeTree::has_running_animations`] does not count it.
    /// `start_animations` clears it again when a restyle leaves the animation
    /// no longer finished (more iterations, say), so it describes the timing
    /// the animation has now rather than a latch set once. Pausing it does not
    /// clear it ([`Self::has_ended`]): a paused finished animation is still
    /// finished, and resuming it costs no frame.
    pub settled: bool,
    /// What [`Self::keyframe_stops`] were extracted from (#766): the
    /// `@keyframes` rule's declaration blocks and the base-style inputs an
    /// authored stop resolves against. A cascade that finds them unchanged
    /// keeps the stops and refreshes only their implicit values; anything else
    /// re-extracts them.
    pub(crate) stop_inputs: StopInputs,
    /// This animation's index in the node's `animation-name` list at its last
    /// cascade, so a later cascade matches repeated names by position even
    /// when the entries are split between the running and the finished maps
    /// (#780).
    pub(crate) position: usize,
}

/// The inputs a kept animation's keyframe stops were extracted from (#766).
///
/// The blocks are held, not just their addresses, so an address cannot be
/// reused by a different rule while this compares against it. Stylo shares a
/// keyframe's declaration block between the `@keyframes` rule and every
/// cascade data rebuilt from it, so a sheet flush that leaves the rule alone
/// leaves these equal, and a redefined, removed or newly winning rule does not.
///
/// Of the base-style inputs, only the ones the authored stops **read** at
/// extraction count ([`StopDeps`]): a `to { transform: rotate(…) }` spin reads
/// none, so a hover that moves the element's `color` or `font-size` keeps its
/// stops, and a `from { background-color: currentcolor }` reads the colour, so
/// the same hover re-extracts them. The implicit (base) values of a 0%/100%
/// stop are not inputs: they are re-read on every cascade anyway
/// ([`super::keyframes::refresh_implicit_values`]).
#[derive(Clone)]
pub(crate) struct StopInputs {
    /// One per keyframe step; `None` for a step Stylo generated.
    blocks: Vec<
        Option<
            servo_arc::Arc<style::shared_lock::Locked<style::properties::PropertyDeclarationBlock>>,
        >,
    >,
    /// The values the stops resolved against, and which of them they read.
    basis: StopBasis,
}

impl StopInputs {
    /// The inputs of stops just extracted from `animation` against `basis`.
    pub(crate) fn new(animation: &KeyframesAnimation, basis: StopBasis) -> Self {
        Self {
            blocks: animation
                .steps
                .iter()
                .map(|step| match &step.value {
                    KeyframesStepValue::Declarations { block } => Some(block.clone()),
                    KeyframesStepValue::ComputedValues => None,
                })
                .collect(),
            basis,
        }
    }

    /// Whether stops extracted from `animation` against `basis` would come out
    /// as the ones these inputs produced: the same declaration blocks, and the
    /// same value for every base-style input those blocks read.
    pub(crate) fn hold_for(&self, animation: &KeyframesAnimation, basis: &StopBasis) -> bool {
        self.blocks.len() == animation.steps.len()
            && self
                .blocks
                .iter()
                .zip(&animation.steps)
                .all(|(held, step)| match (held, &step.value) {
                    (Some(a), KeyframesStepValue::Declarations { block }) => {
                        servo_arc::Arc::ptr_eq(a, block)
                    }
                    (None, KeyframesStepValue::ComputedValues) => true,
                    _ => false,
                })
            && self.basis.agrees_on(basis, self.basis.read())
    }
}

impl std::fmt::Debug for StopInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopInputs")
            .field("steps", &self.blocks.len())
            .field("read", &self.basis.read())
            .finish()
    }
}

/// Which base-style inputs a rule's authored keyframe stops read when they
/// were extracted ([`StopInputs`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StopDeps {
    /// An `em` length: the element's `font-size`.
    pub(crate) em: bool,
    /// A `rem` length: the root font size.
    pub(crate) rem: bool,
    /// A `currentcolor`: the element's `color`.
    pub(crate) color: bool,
    /// `color: currentcolor`: the parent's `color`.
    pub(crate) parent_color: bool,
}

/// Result of computing animation values at a given time.
pub enum AnimationResult {
    /// Interpolated property values to apply.
    Values(Vec<(TransitionProperty, AnimatableValue)>),
    /// Animation is in delay period with no fill.
    NoValues,
    /// Animation has completed (past all iterations) with no fill.
    Complete,
}

impl ActiveAnimation {
    /// Compute interpolated values at the given time.
    pub fn values_at(&self, current_time_ms: f64) -> AnimationResult {
        if self.duration_ms <= 0.0 && self.delay_ms <= 0.0 {
            // Zero-duration: jump to end
            return self.final_values();
        }

        let elapsed = if let Some(paused) = self.paused_elapsed_ms {
            paused
        } else {
            current_time_ms - self.start_time_ms
        };

        // Handle delay period
        if elapsed < self.delay_ms {
            return match self.fill_mode {
                // The before phase: CSS Easing 2's before flag is set when
                // the first iteration runs forwards.
                AnimationFillMode::Backwards | AnimationFillMode::Both => {
                    self.values_at_progress(self.start_progress(), !self.reversed(0))
                }
                _ => AnimationResult::NoValues,
            };
        }

        let active_time = elapsed - self.delay_ms;

        // Check completion
        if let IterationCount::Finite(n) = self.iteration_count {
            if self.duration_ms > 0.0 {
                let current_iteration = active_time / self.duration_ms;
                if current_iteration >= n {
                    return match self.fill_mode {
                        AnimationFillMode::Forwards | AnimationFillMode::Both => {
                            let (progress, before_flag) = self.end_progress();
                            self.values_at_progress(progress, before_flag)
                        }
                        _ => AnimationResult::Complete,
                    };
                }
            }
        }

        if self.duration_ms <= 0.0 {
            return self.final_values();
        }

        // Compute current iteration and progress within it
        let current_iteration = (active_time / self.duration_ms).floor() as u64;
        let iteration_progress = (active_time % self.duration_ms) / self.duration_ms;

        // Apply direction
        let directed_progress = self.apply_direction(iteration_progress, current_iteration);

        self.values_at_progress(directed_progress, false)
    }

    /// Whether this animation's clock is frozen (`animation-play-state: paused`).
    ///
    /// Keyed on `paused_elapsed_ms` rather than `play_state` because that is the
    /// field [`Self::values_at`] and [`Self::is_complete`] read: a paused
    /// animation is exactly one whose elapsed time does not depend on the current
    /// time, so a tick cannot move it. `start_animations` sets the two together.
    pub fn is_paused(&self) -> bool {
        self.paused_elapsed_ms.is_some()
    }

    /// Whether any keyframe of this animation sets a property that changes how
    /// text is measured (`TransitionProperty::changes_text_measure`, #678).
    pub fn changes_text_measure(&self) -> bool {
        self.keyframe_stops
            .iter()
            .any(|k| k.values.iter().any(|(p, _)| p.changes_text_measure()))
    }

    /// Whether any keyframe sets part of an inline element's edge boxes
    /// (`TransitionProperty::changes_inline_edge`, #1426).
    pub fn changes_inline_edge(&self) -> bool {
        self.keyframe_stops
            .iter()
            .any(|k| k.values.iter().any(|(p, _)| p.changes_inline_edge()))
    }

    /// Whether any keyframe of this animation sets `property`.
    pub fn animates(&self, property: TransitionProperty) -> bool {
        self.keyframe_stops
            .iter()
            .any(|k| k.values.iter().any(|(p, _)| *p == property))
    }

    /// Whether this animation has completed all iterations.
    ///
    /// Its active duration is `duration × iterations`, and a zero duration makes
    /// that zero whatever the count — `0s infinite` included (Web Animations
    /// §4.6; Chrome 155 reports such an animation `finished` at its delay's
    /// end, #786). Only an infinite animation with a positive duration never
    /// completes.
    pub fn is_complete(&self, current_time_ms: f64) -> bool {
        if self.paused_elapsed_ms.is_some() {
            return false; // Paused animations are never "complete"
        }
        let active_duration = match self.iteration_count {
            IterationCount::Finite(n) => self.duration_ms * n,
            IterationCount::Infinite if self.duration_ms <= 0.0 => 0.0,
            IterationCount::Infinite => return false,
        };
        current_time_ms - self.start_time_ms >= self.delay_ms + active_duration
    }

    /// Whether the active duration has passed on this animation's own clock —
    /// [`Self::is_complete`], except that a **paused** animation is asked at
    /// its frozen elapsed time instead of answering `false`. Pausing a
    /// finished animation does not un-finish it.
    pub fn has_ended(&self, current_time_ms: f64) -> bool {
        let elapsed = self
            .paused_elapsed_ms
            .unwrap_or(current_time_ms - self.start_time_ms);
        let active_duration = match self.iteration_count {
            IterationCount::Finite(n) => self.duration_ms * n,
            IterationCount::Infinite if self.duration_ms <= 0.0 => 0.0,
            IterationCount::Infinite => return false,
        };
        elapsed >= self.delay_ms + active_duration
    }

    /// Whether `animation-fill-mode` holds the end value once the animation
    /// has finished (`forwards` / `both`).
    pub fn fills_forwards(&self) -> bool {
        matches!(
            self.fill_mode,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        )
    }

    /// Whether this animation is filling (complete but fill-mode keeps it active).
    pub fn is_filling(&self, current_time_ms: f64) -> bool {
        self.is_complete(current_time_ms) && self.fills_forwards()
    }

    fn start_progress(&self) -> f32 {
        match self.direction {
            AnimationDirection::Normal | AnimationDirection::Alternate => 0.0,
            AnimationDirection::Reverse | AnimationDirection::AlternateReverse => 1.0,
        }
    }

    /// The directed progress at the end of the active duration — what a
    /// `forwards` fill holds (Web Animations §4.8.4, #785).
    ///
    /// The active duration is `duration × count`, so it ends in iteration
    /// `floor(count)` at progress `fract(count)`: 2.5 iterations stop half way
    /// through the third, and the direction is that iteration's. A whole count
    /// above zero ends at progress 1 of its last iteration (`count - 1`), and a
    /// count of 0 at progress 0 of iteration 0. This holds for a zero duration
    /// too, whose overall progress is its count. An infinite count (reachable
    /// only with a zero duration, which is finished at once, #786) ends at
    /// progress 1 of an infinite iteration, which Chrome 155 does not count as
    /// odd: `alternate` fills forwards, `alternate-reverse` backwards, as
    /// iteration 0 does.
    ///
    /// The second value is CSS Easing 2's before flag, which Web Animations
    /// sets in the after phase when that last iteration runs backwards (#1564:
    /// `steps(4, start) reverse forwards` ends at its `from` value, not a
    /// step past it).
    fn end_progress(&self) -> (f32, bool) {
        let (progress, iteration) = match self.iteration_count {
            IterationCount::Finite(n) if n <= 0.0 => (0.0, 0),
            IterationCount::Finite(n) => {
                let whole = n.floor();
                if n > whole {
                    (n - whole, whole as u64)
                } else {
                    (1.0, whole as u64 - 1)
                }
            }
            IterationCount::Infinite => (1.0, 0),
        };
        (
            self.apply_direction(progress, iteration),
            self.reversed(iteration),
        )
    }

    /// Whether iteration `iteration` runs backwards.
    fn reversed(&self, iteration: u64) -> bool {
        match self.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => iteration % 2 == 1,
            AnimationDirection::AlternateReverse => iteration.is_multiple_of(2),
        }
    }

    fn apply_direction(&self, progress: f64, iteration: u64) -> f32 {
        if self.reversed(iteration) {
            (1.0 - progress) as f32
        } else {
            progress as f32
        }
    }

    fn final_values(&self) -> AnimationResult {
        match self.fill_mode {
            AnimationFillMode::Forwards | AnimationFillMode::Both => {
                let (progress, before_flag) = self.end_progress();
                self.values_at_progress(progress, before_flag)
            }
            _ => AnimationResult::Complete,
        }
    }

    /// The values at directed progress `progress`, the interval's easing
    /// evaluated with `before_flag` (CSS Easing 2's before flag).
    fn values_at_progress(&self, progress: f32, before_flag: bool) -> AnimationResult {
        if self.keyframe_stops.is_empty() {
            return AnimationResult::NoValues;
        }

        // Find the keyframe interval progress falls in
        let (prev_idx, next_idx) = self.interval(progress);

        let prev = &self.keyframe_stops[prev_idx];
        let next = &self.keyframe_stops[next_idx];

        if prev_idx == next_idx || (prev.percentage - next.percentage).abs() < f32::EPSILON {
            // Exactly on a keyframe
            return AnimationResult::Values(prev.values.clone());
        }

        // Compute local progress between the two stops
        let span = next.percentage - prev.percentage;
        let local = ((progress - prev.percentage) / span).clamp(0.0, 1.0);

        // Apply timing function (from prev keyframe, or default)
        let timing = prev
            .timing_function
            .as_ref()
            .unwrap_or(&self.default_timing);
        let eased = timing.apply_with_before_flag(local, before_flag);

        // Interpolate each property
        let mut result = Vec::new();
        for (prop, from_val) in &prev.values {
            if let Some((_, to_val)) = next.values.iter().find(|(p, _)| p == prop) {
                // An eased 0 or 1 is the keyframe's own value, as it was when
                // an iteration's ends snapped to it: an interpolation there
                // need not reproduce it (a `matrix3d` against `none` is
                // decomposed and put back together), and Chrome 153 shows the
                // keyframe.
                if eased == 0.0 {
                    result.push((*prop, from_val.clone()));
                } else if eased == 1.0 {
                    result.push((*prop, to_val.clone()));
                } else if let Some(interpolated) = from_val.interpolate(to_val, eased) {
                    result.push((*prop, interpolated));
                } else {
                    // Snap to target at 50%
                    if eased >= 0.5 {
                        result.push((*prop, to_val.clone()));
                    } else {
                        result.push((*prop, from_val.clone()));
                    }
                }
            } else {
                // Property only in prev stop — keep it
                result.push((*prop, from_val.clone()));
            }
        }
        // Properties only in next stop
        for (prop, to_val) in &next.values {
            if !prev.values.iter().any(|(p, _)| p == prop) {
                result.push((*prop, to_val.clone()));
            }
        }

        AnimationResult::Values(result)
    }

    /// The keyframe interval `progress` falls in (Web Animations, "the
    /// effect value of a keyframe effect"): it starts at the last stop at or
    /// before `progress` that is below 100%, so an iteration's start (0) and end
    /// (1) are the first and last interval at distances 0 and 1 and are
    /// **eased**, not snapped to the keyframe there — `steps(4, start)` at 0 and
    /// `linear(0, 0.5)` at 1 sit a step or half way off it (#1564). Only a
    /// progress of 1 with more than one stop at 100% takes the last stop alone.
    fn interval(&self, progress: f32) -> (usize, usize) {
        let stops = &self.keyframe_stops;
        let last = stops.len().saturating_sub(1);
        if last == 0 {
            return (0, 0);
        }
        if progress >= 1.0 && stops.iter().filter(|s| s.percentage >= 1.0).count() > 1 {
            return (last, last);
        }
        let start = stops
            .iter()
            .rposition(|s| s.percentage <= progress && s.percentage < 1.0)
            .unwrap_or(0);
        (start, (start + 1).min(last))
    }
}
