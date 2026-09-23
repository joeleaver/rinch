//! CSS Transitions engine.
//!
//! Handles parsing transition specs from Stylo, detecting property changes,
//! interpolating values over time, and writing intermediate values into ComputedStyle.

mod apply;
mod diff;
pub mod types;

pub use apply::apply_value_to_style;
pub use diff::diff_animatable;
pub use types::*;

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

/// Start, retarget, reverse or leave alone a transition for each changed
/// property — css-transitions-1 §3, "Starting of transitions".
///
/// Returns the properties that are transitioning, which is what tells the
/// caller to keep the *interpolated* value rather than the after-change one it
/// has just assigned into `computed_style`. A property whose running transition
/// is left untouched is still on that list: the caller assigns the whole
/// after-change style in the same breath, so dropping the property from it
/// would snap the box to its end value for a frame (#489 needed only one).
///
/// **A running transition whose end value still equals the after-change value
/// is left exactly as it is** (#652). It used to be replaced unconditionally,
/// and since the caller diffs `computed_style` — which holds the interpolated
/// value while a transition runs — against the freshly resolved target, *every*
/// restyle of a transitioning node restarted its transition with a new clock.
/// A declared 150ms animation then ran for as long as restyles kept arriving:
/// measured on one UI Zoo navigation, the `--lg` checkbox's width transition
/// restarted nine times and crawled toward its target instead of arriving.
///
/// What is **not** implemented, and is a deliberate scoping rather than an
/// oversight: §3's *transitionability* precondition, which appears in item 1
/// and again in item 4.2. A pair of values that cannot be interpolated (a
/// length against a percentage, which would need a `calc()` `ComputedStyle`
/// cannot hold) still gets an `ActiveTransition` that idles for its whole
/// duration, because `AnimatableValue::interpolate` answers `None` for it and
/// the tick then writes nothing. That is the pre-existing behaviour and it
/// snaps either way; cancelling instead would only save the idle ticks.
///
/// Nor is §3 item 3 — cancel a running transition whose property has stopped
/// matching `transition-property`. `find_matching_spec` answering `None` skips
/// the property here and leaves any running transition running. Also
/// pre-existing, but **not rare**: it needs the same restyle to diff that
/// property, and while a transition runs that is the usual case rather than a
/// narrowing one, for the reason three paragraphs up — the caller diffs the
/// *interpolated* value against the target. Nor is it inert. The property is
/// left out of `transitioning`, so the box snaps to the target, and the
/// transition is still in the map, so the next tick writes its interpolated
/// value back and the box jumps backwards: measured on a 20px → 30px 150ms
/// transition, 30px on the restyle and 26.67px one tick later. Tracked as #693.
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

        // §3 calls `duration + delay` the *combined duration*. A negative
        // `transition-delay` can drive it to zero or below — `transition: width
        // 150ms linear -200ms` is emitted by `extract_from_stylo`, which gates
        // on `duration > 0 || delay > 0` — and the spec then wants no
        // transition at all: item 1 declines to start one, item 4.2 cancels a
        // running one. Both leave the after-change value standing, which is
        // what the property snaps to.
        let combined_duration_ms = spec.duration_ms + spec.delay_ms;

        // Cloned rather than borrowed so the cancelling arms below can remove
        // the entry. One clone per changed property per restyle, of a value
        // `diff_animatable` only ever builds out of scalars.
        let existing = active_transitions.get(&change.property).cloned();

        let started = match existing {
            // §3 item 1: nothing was transitioning this property, so start from
            // the before-change value over the declared duration.
            None if combined_duration_ms <= 0.0 => continue,
            None => ActiveTransition::starting(
                change.property,
                change.old_value.clone(),
                change.new_value.clone(),
                spec,
                current_time_ms,
            ),

            // §3: a running transition whose end value still equals the
            // after-change value is left alone — same target, same clock.
            Some(ref existing) if existing.to.same_computed_value(&change.new_value) => {
                transitioning.push(change.property);
                continue;
            }

            Some(existing) => {
                let current = existing
                    .value_at(current_time_ms)
                    .unwrap_or_else(|| change.old_value.clone());

                // §3 item 4.1: the running transition has already arrived at
                // the new target, so cancel it and start nothing. Leaving the
                // property off `transitioning` lets the caller's after-change
                // value stand — which is the value the box is already at.
                //
                // §3 item 4.2 is the other cancel: no combined duration left in
                // which to reach the new target.
                if current.same_computed_value(&change.new_value) || combined_duration_ms <= 0.0 {
                    active_transitions.remove(&change.property);
                    continue;
                }

                // §3 item 4.3: a reversal — the new target is the value this
                // transition would reverse back to — is shortened in proportion
                // to how far it had got. §3 item 4.4 is everything else: cancel
                // and restart from the current value over the full duration.
                // (4.2 has already taken the zero-combined-duration case, which
                // is the other half of 4.3's precondition.)
                let is_reversal = existing
                    .reversing_adjusted_start_value
                    .same_computed_value(&change.new_value);

                if is_reversal {
                    existing.reversing(current, change.new_value.clone(), spec, current_time_ms)
                } else {
                    ActiveTransition::starting(
                        change.property,
                        current,
                        change.new_value.clone(),
                        spec,
                        current_time_ms,
                    )
                }
            }
        };

        active_transitions.insert(change.property, started);
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
        let hit_key = crate::hit_cache::HitStyleKey::of(&tree.nodes[node_id].computed_style);

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

        // A colour or opacity fade leaves every hit-test input alone; only a
        // write that changes one costs the next pointer move its memo.
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
