//! CSS Transitions engine.
//!
//! Handles parsing transition specs from Stylo, detecting property changes,
//! interpolating values over time, and writing intermediate values into ComputedStyle.

pub mod types;
mod apply;
mod diff;

pub use types::*;
pub use apply::apply_value_to_style;
pub use diff::diff_animatable;

use std::collections::HashMap;

use crate::node::{DirtyFlags, NodeTree, RawNodeId};

/// Find a matching TransitionSpec for a property change.
pub fn find_matching_spec(
    specs: &[TransitionSpec],
    property: TransitionProperty,
) -> Option<&TransitionSpec> {
    // First try exact match
    specs
        .iter()
        .find(|spec| spec.property == property)
        // Then try "all"
        .or_else(|| {
            specs
                .iter()
                .find(|spec| spec.property == TransitionProperty::All)
        })
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
                    apply_value_to_style(&mut tree.nodes[node_id].computed_style, *prop, &value);
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
