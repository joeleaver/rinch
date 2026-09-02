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
/// Compares new specs against existing active animations:
/// - New animation name: create and start
/// - Same name + same spec: keep running (don't restart)
/// - Removed name: cancel
/// - Play state change: pause/resume
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

            new_active.push(existing_anim);
        } else {
            // New animation — look up keyframes and create
            let atom = Atom::from(spec.name.as_str());
            let keyframes_anim = stylist.lookup_keyframes(&atom, element);

            if let Some(kf_anim) = keyframes_anim {
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

                if !stops.is_empty() {
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
                    });
                }
            }
        }
    }

    if !new_active.is_empty() {
        active_animations.insert(node_id, new_active);
    }
}

/// Advance all active animations by one frame. Returns true if any are still active.
///
/// For each active animation:
/// 1. Compute interpolated values
/// 2. Write to node's computed_style
/// 3. Mark node dirty (PAINT, and LAYOUT if layout-affecting)
/// 4. Remove completed animations (unless filling)
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
        let mut kept_animations = Vec::new();

        for anim in &animations {
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

                    // Keep this animation if it's not complete or is filling
                    if !anim.is_complete(current_time_ms) || anim.is_filling(current_time_ms) {
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
