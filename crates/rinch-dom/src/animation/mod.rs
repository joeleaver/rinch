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
            existing_anim.default_timing = spec.timing.clone();

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

            // A restyle can leave a finished animation running again (#782,
            // #783), so ask whether it is *still* finished — **after** the
            // refresh block above, or the question is asked against the timing
            // the animation had before this restyle. A full restyle that
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
        } else {
            // New animation — look up keyframes and create
            if let Some(stops) = extract_stops(&spec.name) {
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
                });
            }
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
