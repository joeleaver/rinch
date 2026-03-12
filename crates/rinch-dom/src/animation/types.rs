//! Animation types: AnimationSpec, ActiveAnimation, KeyframeStop.

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
                AnimationFillMode::Backwards | AnimationFillMode::Both => {
                    self.values_at_progress(self.start_progress())
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
                            self.values_at_progress(self.end_progress(n))
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

        self.values_at_progress(directed_progress)
    }

    /// Whether this animation has completed all iterations.
    pub fn is_complete(&self, current_time_ms: f64) -> bool {
        if self.paused_elapsed_ms.is_some() {
            return false; // Paused animations are never "complete"
        }
        if let IterationCount::Finite(n) = self.iteration_count {
            let elapsed = current_time_ms - self.start_time_ms;
            let total = self.delay_ms + self.duration_ms * n;
            elapsed >= total
        } else {
            false // Infinite animations never complete
        }
    }

    /// Whether this animation is filling (complete but fill-mode keeps it active).
    pub fn is_filling(&self, current_time_ms: f64) -> bool {
        if !self.is_complete(current_time_ms) {
            return false;
        }
        matches!(
            self.fill_mode,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        )
    }

    fn start_progress(&self) -> f32 {
        match self.direction {
            AnimationDirection::Normal | AnimationDirection::Alternate => 0.0,
            AnimationDirection::Reverse | AnimationDirection::AlternateReverse => 1.0,
        }
    }

    fn end_progress(&self, iterations: f64) -> f32 {
        let last_iter = (iterations.ceil() as u64).saturating_sub(1);
        self.apply_direction(1.0, last_iter)
    }

    fn apply_direction(&self, progress: f64, iteration: u64) -> f32 {
        let reversed = match self.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => iteration % 2 == 1,
            AnimationDirection::AlternateReverse => iteration.is_multiple_of(2),
        };
        if reversed {
            (1.0 - progress) as f32
        } else {
            progress as f32
        }
    }

    fn final_values(&self) -> AnimationResult {
        match self.fill_mode {
            AnimationFillMode::Forwards | AnimationFillMode::Both => {
                let n = match self.iteration_count {
                    IterationCount::Finite(n) => n,
                    IterationCount::Infinite => 1.0,
                };
                self.values_at_progress(self.end_progress(n))
            }
            _ => AnimationResult::Complete,
        }
    }

    fn values_at_progress(&self, progress: f32) -> AnimationResult {
        if self.keyframe_stops.is_empty() {
            return AnimationResult::NoValues;
        }

        // Find surrounding keyframe stops
        let (prev_idx, next_idx) = self.find_surrounding_stops(progress);

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
        let timing = prev.timing_function.unwrap_or(self.default_timing);
        let eased = timing.apply(local);

        // Interpolate each property
        let mut result = Vec::new();
        for (prop, from_val) in &prev.values {
            if let Some((_, to_val)) = next.values.iter().find(|(p, _)| p == prop) {
                if let Some(interpolated) = from_val.interpolate(to_val, eased) {
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

    /// Binary search for surrounding keyframe stops.
    fn find_surrounding_stops(&self, progress: f32) -> (usize, usize) {
        let stops = &self.keyframe_stops;
        if stops.is_empty() {
            return (0, 0);
        }
        if progress <= stops[0].percentage {
            return (0, 0);
        }
        if progress >= stops[stops.len() - 1].percentage {
            let last = stops.len() - 1;
            return (last, last);
        }

        // Find the last stop with percentage <= progress
        let mut prev_idx = 0;
        for (i, stop) in stops.iter().enumerate() {
            if stop.percentage <= progress {
                prev_idx = i;
            } else {
                break;
            }
        }
        let next_idx = (prev_idx + 1).min(stops.len() - 1);
        (prev_idx, next_idx)
    }
}
