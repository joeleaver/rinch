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
/// Compares new specs against existing active animations, **by name**:
/// - New animation name: create and start
/// - Same name: keep running (don't restart)
/// - Removed name: cancel
/// - Play state change: pause/resume
///
/// A kept animation normally keeps everything it had except the fields the new
/// spec restates in place (direction, fill, iteration count, timing function):
/// its keyframe stops and its `duration_ms` / `delay_ms` stay as they were
/// minted. **During `recompute_all_styles_full`** (`tree.refreshing_animations`)
/// it keeps only its clock — `start_time_ms`, `paused_elapsed_ms` and
/// `play_state` — and takes the rest afresh: the `@keyframes` rule is looked up
/// again and the animation dropped if it is gone, the stops are re-extracted
/// from `base_style`, and the duration and delay come from the spec. A re-timed
/// paused animation keeps `paused_elapsed_ms` as elapsed *time*, so its
/// progress moves and its `currentTime` does not, as in Chrome. Outside that
/// pass an edited `@keyframes` body, a changed duration, and a stop derived from
/// the base style all stay stale (#766, #780, #781).
#[allow(clippy::too_many_arguments)]
pub fn start_animations(
    active_animations: &mut HashMap<RawNodeId, Vec<ActiveAnimation>>,
    node_id: RawNodeId,
    specs: &[AnimationSpec],
    base_style: &ComputedStyle,
    stylist: &Stylist,
    guard: &SharedRwLockReadGuard,
    tree: &NodeTree,
    current_time_ms: f64,
) {
    let existing = active_animations.remove(&node_id).unwrap_or_default();

    if specs.is_empty() {
        // No animations — remove all
        return;
    }

    let element = RinchNode::new(node_id, tree);

    // The keyframe stops for `name` against this node's current base style, or
    // `None` when no `@keyframes` rule of that name exists (or it yields no
    // stops).
    let extract_stops = |name: &str| -> Option<Vec<KeyframeStop>> {
        let atom = Atom::from(name);
        let kf_anim = stylist.lookup_keyframes(&atom, element)?;
        // A `color: currentcolor` stop inherits the parent's colour.
        let parent_color = tree
            .get(node_id)
            .and_then(|n| n.parent)
            .and_then(|parent| tree.get(parent))
            .and_then(|parent| parent.computed_style.color);
        // `rem` in a keyframe stop resolves against the root's font
        // size, the same base the cascade uses.
        let root_font_size = tree
            .get(tree.root_id)
            .map_or(16.0, |root| root.computed_style.font_size);
        let stops = keyframes::extract_keyframe_stops(
            kf_anim,
            base_style,
            parent_color,
            root_font_size,
            guard,
        );
        (!stops.is_empty()).then_some(stops)
    };

    let mut new_active = Vec::new();

    for spec in specs {
        // Look for an existing animation with the same name
        if let Some(mut existing_anim) = existing.iter().find(|a| a.name == spec.name).cloned() {
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

            // Update mutable properties (direction, fill, iteration count)
            existing_anim.direction = spec.direction;
            existing_anim.fill_mode = spec.fill_mode;
            existing_anim.iteration_count = spec.iteration_count;
            existing_anim.default_timing = spec.timing;

            // The full restyle keeps the clock and nothing else (see the doc
            // above). Neither `start_time_ms` nor `paused_elapsed_ms` nor
            // `play_state` is written here.
            if tree.refreshing_animations {
                let Some(stops) = extract_stops(&spec.name) else {
                    // The rule is gone: cancelled, as in Chrome.
                    continue;
                };
                existing_anim.keyframe_stops = stops;
                existing_anim.duration_ms = spec.duration_ms;
                existing_anim.delay_ms = spec.delay_ms;
            }

            // A restyle can leave a finished animation running again (#782), so
            // ask whether it is *still* filling — **after** the refresh block
            // above, or the question is asked against the timing the animation
            // had before this restyle. A full restyle that lengthens a finished
            // animation is exactly that case: it runs again, and a stale
            // `fill_settled` would leave `has_running_animations()` answering
            // `false` for it and the tick that finishes it a second time not
            // dirtying its node (measured: the box keeps the last running
            // sample instead of the fill).
            existing_anim.fill_settled =
                existing_anim.fill_settled && existing_anim.is_filling(current_time_ms);

            new_active.push(existing_anim);
        } else {
            // New animation — look up keyframes and create
            if let Some(stops) = extract_stops(&spec.name) {
                new_active.push(ActiveAnimation {
                    name: spec.name.clone(),
                    keyframe_stops: stops,
                    default_timing: spec.timing,
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
                    fill_settled: false,
                });
            }
        }
    }

    if !new_active.is_empty() {
        active_animations.insert(node_id, new_active);
    }
}

/// Advance all active animations by one frame. Returns true if any running
/// animation is still active — i.e. whether the caller should keep polling.
///
/// For each active animation:
/// 1. Compute interpolated values
/// 2. Write to node's computed_style
/// 3. Mark node dirty (PAINT, and LAYOUT if layout-affecting)
/// 4. Remove completed animations (unless filling)
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
/// [`ActiveAnimation::fill_settled`].
pub fn tick_animations(tree: &mut NodeTree, current_time_ms: f64) -> bool {
    tree.hit_cache.invalidate();
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
        let mut kept_animations = Vec::new();

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
                let newly_finished = !anim.fill_settled;
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
                settled.fill_settled = true;
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
                    // Completed with no fill — drop it
                }
            }
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
