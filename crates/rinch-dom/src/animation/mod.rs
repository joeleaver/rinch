//! CSS Animations engine.
//!
//! Handles parsing animation specs from Stylo, extracting keyframe values,
//! interpolating over time, and writing animated values into ComputedStyle.

pub(crate) mod extract;
pub mod keyframes;
pub mod types;

pub use types::*;

use std::collections::HashMap;

use style::Atom;
use style::shared_lock::SharedRwLockReadGuard;
use style::stylist::Stylist;

use crate::computed_style::ComputedStyle;
use crate::node::{DirtyFlags, NodeTree, RawNodeId};
use crate::stylo_impl::RinchNode;
use crate::transition::apply_value_to_style;

/// Start or update animations for a node based on its current animation specs.
///
/// Compares new specs against existing animations **by name**, repeated names
/// by position counted from the end of the list (css-animations-2; #780):
/// - New animation name: create and start
/// - Same name: keep running (don't restart)
/// - Removed name: cancel
/// - Play state change: pause/resume
///
/// A kept animation keeps its **clock** — `start_time_ms`,
/// `paused_elapsed_ms` and `play_state` — and nothing else, on every cascade of
/// its node, whatever caused it (a class change, a `<style>` added, a theme, a
/// viewport change; #766, #780, #781). CSS Animations 1 §3: a changed
/// `animation-*` longhand applies to the running animation, and a changed
/// `@keyframes` rule takes effect at once. So:
/// - the timing (duration, delay, direction, fill, iteration count, timing
///   function) comes from the spec. A re-timed paused animation keeps
///   `paused_elapsed_ms` as elapsed *time*, so its progress moves and its
///   `currentTime` does not, as in Chrome;
/// - the `@keyframes` rule is looked up again, and the animation dropped if it
///   is gone (Chrome cancels it);
/// - the stops are re-extracted when the rule's blocks or the base-style
///   inputs an authored stop resolves against ([`StopInputs`]) changed, and
///   otherwise kept with only their implicit (base) values re-read
///   ([`keyframes::refresh_implicit_values`]).
///
/// That costs a node that is cascaded while it animates one keyframes lookup
/// and a comparison of a few values per animation; the extraction itself runs
/// only when something it reads moved. A node with no animation never gets
/// here.
#[allow(clippy::too_many_arguments)]
pub fn start_animations(
    active_animations: &mut HashMap<RawNodeId, Vec<ActiveAnimation>>,
    finished_animations: &mut HashMap<RawNodeId, Vec<ActiveAnimation>>,
    node_id: RawNodeId,
    specs: &[AnimationSpec],
    base_style: &ComputedStyle,
    stylist: &Stylist,
    guard: &SharedRwLockReadGuard,
    tree: &NodeTree,
    current_time_ms: f64,
) {
    // Both maps: a finished animation without a fill is kept in
    // `finished_animations` (#783), and is found by name the same way.
    let mut existing = active_animations.remove(&node_id).unwrap_or_default();
    if let Some(finished) = finished_animations.remove(&node_id) {
        existing.extend(finished);
    }

    if specs.is_empty() {
        // No animations — remove all
        return;
    }

    // Back in declaration order, so repeated names match by position.
    existing.sort_by_key(|a| a.position);

    let element = RinchNode::new(node_id, tree);
    // A `color: currentcolor` stop inherits the parent's colour.
    let parent_color = tree
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|parent| tree.get(parent))
        .and_then(|parent| parent.computed_style.color);
    // `rem` in a keyframe stop resolves against the root's font size, the same
    // base the cascade uses.
    let root_font_size = tree
        .get(tree.root_id)
        .map_or(16.0, |root| root.computed_style.font_size);

    // What the stops of a rule extract from against this node's base style.
    let inputs_of = |kf_anim: &style::stylesheets::keyframes_rule::KeyframesAnimation| StopInputs {
        blocks: kf_anim
            .steps
            .iter()
            .map(|step| match &step.value {
                style::stylesheets::keyframes_rule::KeyframesStepValue::Declarations { block } => {
                    Some(block.clone())
                }
                style::stylesheets::keyframes_rule::KeyframesStepValue::ComputedValues => None,
            })
            .collect(),
        font_size: base_style.font_size,
        root_font_size,
        color: base_style.color,
        parent_color,
    };
    let extract = |kf_anim: &style::stylesheets::keyframes_rule::KeyframesAnimation| {
        let stops = keyframes::extract_keyframe_stops(
            kf_anim,
            base_style,
            parent_color,
            root_font_size,
            guard,
        );
        (!stops.is_empty()).then_some(stops)
    };

    // Match from the end: the last spec of a name takes the last entry of that
    // name, and no entry is taken twice. Matching every spec to the *first*
    // entry of its name turned `k 10s, k 20s` into two copies of the first.
    let mut existing: Vec<Option<ActiveAnimation>> = existing.into_iter().map(Some).collect();
    let mut matched: Vec<Option<ActiveAnimation>> = vec![None; specs.len()];
    for (i, spec) in specs.iter().enumerate().rev() {
        if let Some(j) = existing
            .iter()
            .rposition(|a| a.as_ref().is_some_and(|a| a.name == spec.name))
        {
            matched[i] = existing[j].take();
        }
    }

    let mut new_active = Vec::new();

    for (position, (spec, kept)) in specs.iter().zip(matched).enumerate() {
        let Some(kf_anim) = stylist.lookup_keyframes(&Atom::from(spec.name.as_str()), element)
        else {
            // No such rule: nothing starts, and a kept animation whose rule is
            // gone is cancelled, as in Chrome.
            continue;
        };

        if let Some(mut existing_anim) = kept {
            // Update play state
            match (existing_anim.play_state, spec.play_state) {
                (AnimationPlayState::Running, AnimationPlayState::Paused) => {
                    // Pause: record elapsed time
                    let elapsed = current_time_ms - existing_anim.start_time_ms;
                    existing_anim.paused_elapsed_ms = Some(elapsed);
                    existing_anim.play_state = AnimationPlayState::Paused;
                }
                (AnimationPlayState::Paused, AnimationPlayState::Running) => {
                    // Resume: adjust start time to maintain position
                    if let Some(paused_at) = existing_anim.paused_elapsed_ms {
                        existing_anim.start_time_ms = current_time_ms - paused_at;
                        existing_anim.paused_elapsed_ms = None;
                    }
                    existing_anim.play_state = AnimationPlayState::Running;
                }
                _ => {}
            }

            // The timing comes from the spec; the clock (`start_time_ms`,
            // `paused_elapsed_ms`) is not written here.
            existing_anim.direction = spec.direction;
            existing_anim.fill_mode = spec.fill_mode;
            existing_anim.iteration_count = spec.iteration_count;
            existing_anim.default_timing = spec.timing.clone();
            existing_anim.duration_ms = spec.duration_ms;
            existing_anim.delay_ms = spec.delay_ms;
            existing_anim.position = position;

            let inputs = inputs_of(kf_anim);
            if inputs != existing_anim.stop_inputs {
                let Some(stops) = extract(kf_anim) else {
                    continue;
                };
                existing_anim.keyframe_stops = stops;
                existing_anim.stop_inputs = inputs;
            } else {
                keyframes::refresh_implicit_values(&mut existing_anim.keyframe_stops, base_style);
            }

            // A restyle can leave a finished animation running again (#782,
            // #783), so ask whether it is *still* finished — **after** the
            // timing above is taken, or the question is asked against the
            // timing the animation had before this restyle. A restyle that
            // lengthens a finished animation is exactly that case: it runs
            // again, and a stale `settled` would leave
            // `has_running_animations()` answering `false` for it and the tick
            // that finishes it a second time not dirtying its node (measured:
            // the box keeps the last running sample instead of the fill).
            // `has_ended`, not `is_filling`: a finished animation without a
            // fill is settled too, and stays so across a restyle that changes
            // nothing about its timing — that is what keeps it from replaying.
            // And not `is_complete`, which answers `false` for a paused
            // animation: pausing a finished one would clear `settled`, and the
            // resume would then count it as running for one frame and restyle
            // its node once more (review of #2000, F4).
            existing_anim.settled =
                existing_anim.settled && existing_anim.has_ended(current_time_ms);

            new_active.push(existing_anim);
        } else if let Some(stops) = extract(kf_anim) {
            // New animation
            new_active.push(ActiveAnimation {
                name: spec.name.clone(),
                keyframe_stops: stops,
                default_timing: spec.timing.clone(),
                duration_ms: spec.duration_ms,
                delay_ms: spec.delay_ms,
                direction: spec.direction,
                iteration_count: spec.iteration_count,
                fill_mode: spec.fill_mode,
                play_state: spec.play_state,
                start_time_ms: current_time_ms,
                paused_elapsed_ms: if spec.play_state == AnimationPlayState::Paused {
                    Some(0.0)
                } else {
                    None
                },
                settled: false,
                stop_inputs: inputs_of(kf_anim),
                position,
            });
        }
    }

    // A settled animation that holds no fill writes nothing, on a tick or in a
    // cascade, so it goes back to `finished_animations`, where no tick walks
    // it (review of #2000, F1).
    let (finished, running): (Vec<_>, Vec<_>) = new_active
        .into_iter()
        .partition(|a| a.settled && !a.fills_forwards());
    if !running.is_empty() {
        active_animations.insert(node_id, running);
    }
    if !finished.is_empty() {
        finished_animations.insert(node_id, finished);
    }
}

/// Advance all active animations by one frame. Returns true if any running
/// animation is still active — i.e. whether the caller should keep polling.
///
/// For each active animation:
/// 1. Compute interpolated values
/// 2. Write to node's computed_style
/// 3. Mark node dirty (PAINT, and LAYOUT if layout-affecting)
/// 4. Keep completed animations as settled entries (see below)
///
/// **A paused animation is kept, and is not counted and not marked dirty**
/// (#763). Its elapsed time is frozen, so its sample is the one the cascade
/// already wrote into `computed_style` when it paused, and the tick has nothing
/// to advance. Counting it made the frame clock schedule a frame every frame,
/// forever; marking its node dirty did the same by a second route, since a
/// `LAYOUT`-dirty node is resolved and repainted. The sample is still
/// re-applied, so a paused animation keeps the same precedence over a
/// transition on the same property that a running one has: a transition that
/// finishes on this tick writes its end value first, and has marked the node
/// dirty itself.
///
/// **A finished `forwards`/`both` animation is the same shape once its fill is
/// written** (#782): the tick that finishes it writes the fill and dirties the
/// node, and every later tick re-applies it the same quiet way — see
/// [`ActiveAnimation::settled`]. **A finished one without a fill is kept too**
/// (#783), as a settled marker with no sample: the tick that finishes it
/// pushes its node onto [`NodeTree::ended_animations`], which
/// `RinchDocument::tick_animations` turns into a restyle of the node so its
/// cascaded values replace the last sample, and moves the entry to
/// [`NodeTree::finished_animations`], which no tick walks. A caller of this
/// function alone owns that restyle.
pub fn tick_animations(tree: &mut NodeTree, current_time_ms: f64) -> bool {
    let node_ids: Vec<RawNodeId> = tree.active_animations.keys().copied().collect();
    let mut any_active = false;

    for node_id in node_ids {
        if !tree.nodes.contains(node_id) {
            tree.active_animations.remove(&node_id);
            continue;
        }

        let animations = match tree.active_animations.get(&node_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        let mut needs_layout = false;
        let mut needs_paint = false;
        let mut ended = Vec::new();
        let mut kept_animations = Vec::new();
        let hit_key = crate::hit_cache::HitStyleKey::of(&tree.nodes[node_id].computed_style);

        for anim in &animations {
            if anim.is_paused() {
                if let AnimationResult::Values(values) = anim.values_at(current_time_ms) {
                    for (prop, value) in &values {
                        apply_value_to_style(&mut tree.nodes[node_id].computed_style, *prop, value);
                    }
                }
                kept_animations.push(anim.clone());
                continue;
            }

            // A finished animation that fills has a constant sample, exactly
            // like a paused one (#782). The tick that finishes it writes the
            // fill and dirties the node, because that frame shows the end;
            // every later tick re-applies it quietly. It is never counted: the
            // finishing tick is presented through K23's "was there anything to
            // tick" guard, as a finishing animation without a fill is.
            if anim.is_filling(current_time_ms) {
                let newly_finished = !anim.settled;
                if let AnimationResult::Values(values) = anim.values_at(current_time_ms) {
                    for (prop, value) in &values {
                        apply_value_to_style(&mut tree.nodes[node_id].computed_style, *prop, value);
                        if newly_finished {
                            needs_paint = true;
                            if prop.affects_layout() {
                                needs_layout = true;
                            }
                        }
                    }
                }
                let mut settled = anim.clone();
                settled.settled = true;
                kept_animations.push(settled);
                continue;
            }

            let result = anim.values_at(current_time_ms);

            match result {
                AnimationResult::Values(values) => {
                    for (prop, value) in &values {
                        apply_value_to_style(&mut tree.nodes[node_id].computed_style, *prop, value);
                        needs_paint = true;
                        if prop.affects_layout() {
                            needs_layout = true;
                        }
                    }

                    // Keep this animation if it's not complete (a filling one
                    // was handled above)
                    if !anim.is_complete(current_time_ms) {
                        kept_animations.push(anim.clone());
                        any_active = true;
                    }
                }
                AnimationResult::NoValues => {
                    // Still in delay — keep it
                    kept_animations.push(anim.clone());
                    any_active = true;
                }
                AnimationResult::Complete => {
                    // Finished with no fill to hold (#783). The entry is kept,
                    // settled, as a marker in `finished_animations`: the next
                    // cascade of the node finds it by name, applies nothing
                    // and does not start the still-declared animation again
                    // from t=0. The tick that finishes it cannot put the
                    // cascaded values back itself — `computed_style` holds the
                    // last sample and nothing under it — so it asks for a
                    // restyle of the node. Later ticks never see it.
                    let mut settled = anim.clone();
                    settled.settled = true;
                    ended.push(settled);
                }
            }
        }

        if !ended.is_empty() {
            tree.finished_animations
                .entry(node_id)
                .or_default()
                .extend(ended);
            if !tree.ended_animations.contains(&node_id) {
                tree.ended_animations.push(node_id);
            }
        }

        // A colour or opacity animation (a `Loader`, a shimmer) leaves every
        // hit-test input alone, so it must not cost every pointer move its
        // memo; a `transform` or `visibility` step does (`HitStyleKey`).
        if hit_key != crate::hit_cache::HitStyleKey::of(&tree.nodes[node_id].computed_style) {
            tree.hit_cache.invalidate();
        }

        // Mark dirty
        if needs_paint {
            tree.nodes[node_id].dirty.insert(DirtyFlags::PAINT);
            tree.paint_dirty_nodes.push(node_id);
        }
        if needs_layout {
            tree.nodes[node_id]
                .dirty
                .insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            tree.dirty_nodes.insert(node_id);
        }

        // Update or remove
        if kept_animations.is_empty() {
            tree.active_animations.remove(&node_id);
        } else {
            tree.active_animations.insert(node_id, kept_animations);
        }
    }

    any_active
}
