//! CSS Transitions engine.
//!
//! Handles parsing transition specs from Stylo, detecting property changes,
//! interpolating values over time, and writing intermediate values into ComputedStyle.

mod apply;
mod diff;
pub mod transform;
pub mod types;

pub use apply::apply_value_to_style;
pub use diff::diff_animatable;
pub use transform::{
    Affine, TransformOp, compose, compose_about_origin_z, compose_matrices, interpolate_lists,
};
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
/// §3 item 3 — cancel a running transition whose property has stopped
/// matching `transition-property` — is done in two places (#693). Here, a
/// changed property with no matching spec is removed from the map; the
/// cascade additionally sweeps the whole running set with
/// [`cancel_unmatched_transitions`] before it gets here, since the item applies
/// whether or not the property changed and a node that stopped declaring any
/// `transition` never reaches this function at all.
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
            // §3 item 3 (#693): a running transition whose property no longer
            // matches `transition-property` is cancelled, so the after-change
            // value stands. The caller also sweeps the properties that did not
            // change on this restyle ([`cancel_unmatched_transitions`]); this
            // arm keeps a direct caller of this function honest too.
            None => {
                active_transitions.remove(&change.property);
                continue;
            }
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

/// css-transitions-1 §3 item 3 (#693): cancel every running transition whose
/// property no longer matches any of `specs`.
///
/// Swept over the whole running set, not only the properties the restyle found
/// a change in: the item applies whether or not the property also changed. The
/// cascade assigns the after-change style over `computed_style` on the same
/// pass, so a cancelled property lands on its target rather than being left at
/// an interpolated value — and, having left the map, it is not written back by
/// the next tick. Before this, the tick did write it back: measured on a 20px →
/// 30px 150ms transition, 30px on the restyle and 26.67px one tick later.
///
/// This is what makes the canonical overlay spelling safe — a hidden state
/// carrying `transition: visibility 0s linear 300ms` and a shown state carrying
/// no `visibility` transition: reopening before the delay ends cancels the
/// close, where leaving it running hid the reopened overlay 300ms later.
///
/// Returns whether a `visibility` transition was among the cancelled, which the
/// caller needs for [`propagate_inherited_visibility`].
pub fn cancel_unmatched_transitions(
    active_transitions: &mut HashMap<RawNodeId, HashMap<TransitionProperty, ActiveTransition>>,
    node_id: RawNodeId,
    specs: &[TransitionSpec],
) -> bool {
    let Some(map) = active_transitions.get_mut(&node_id) else {
        return false;
    };
    let mut cancelled_visibility = false;
    map.retain(|prop, _| {
        let keep = find_matching_spec(specs, *prop).is_some();
        if !keep && *prop == TransitionProperty::Visibility {
            cancelled_visibility = true;
        }
        keep
    });
    if map.is_empty() {
        active_transitions.remove(&node_id);
    }
    cancelled_visibility
}

/// Whether `node_id`'s `visibility` is **inherited** — no rule it matched
/// declares one, or the winning declaration is `inherit` / `unset` (or a
/// `revert`, which reaches the UA origin, and the UA sheet declares none).
///
/// Read from the node's Stylo rule chain, which lists the matched rules from
/// the highest-priority down: the first `visibility` declaration found at a
/// rule's own importance is the one the cascade used. An element with no
/// style data yet — never cascaded — has nothing of its own and inherits.
///
/// Asked only while a `visibility` transition is being propagated, so it costs
/// nothing to a document with none running.
pub(crate) fn visibility_is_inherited(tree: &NodeTree, node_id: RawNodeId) -> bool {
    use style::properties::{CSSWideKeyword, LonghandId, PropertyDeclarationId};

    let data = tree.nodes[node_id].stylo_element_data.borrow();
    let Some(primary) = data.as_ref().and_then(|d| d.styles.get_primary()) else {
        return true;
    };

    // Fast path: Stylo shares an inherited style struct with the parent until
    // a declaration in it is applied, so a node whose `InheritedBox` *is* its
    // parent's declared nothing in it — `visibility` included. That answers
    // the common case without reading a single rule; the rule walk below is
    // for a node that declared some other property of the struct, or whose
    // parent was re-cascaded into a fresh struct without it.
    if let Some(parent) = tree.nodes[node_id].parent.and_then(|p| tree.nodes.get(p)) {
        let parent_data = parent.stylo_element_data.borrow();
        if let Some(parent_primary) = parent_data.as_ref().and_then(|d| d.styles.get_primary())
            && std::ptr::eq(
                primary.get_inherited_box(),
                parent_primary.get_inherited_box(),
            )
        {
            return true;
        }
    }

    let Some(rules) = primary.rules.as_ref() else {
        return true;
    };
    let guard = tree.guard.read();
    for rule in rules.self_and_ancestors() {
        let Some(source) = rule.style_source() else {
            continue;
        };
        let important = rule.cascade_level().is_important();
        for (decl, importance) in source.read(&guard).declaration_importance_iter() {
            if importance.important() != important
                || decl.id() != PropertyDeclarationId::Longhand(LonghandId::Visibility)
            {
                continue;
            }
            return matches!(
                decl.get_css_wide_keyword(),
                Some(
                    CSSWideKeyword::Inherit
                        | CSSWideKeyword::Unset
                        | CSSWideKeyword::Revert
                        | CSSWideKeyword::RevertLayer
                )
            );
        }
    }
    true
}

/// The `visibility` an inheriting `node_id` inherits right now, **animated**:
/// that of its nearest ancestor that either runs a `visibility` transition or
/// declares a `visibility` of its own — the ancestor the value actually comes
/// from. `None` when no such ancestor exists (the value is Stylo's).
///
/// Walks past inheriting ancestors because their `computed_style` may still
/// hold Stylo's after-change value on the pass that is being cascaded: the
/// hand-down that corrects them runs after the cascade loop. Asked only for a
/// node that declares its own `visibility` transition, while one runs.
pub(crate) fn animated_inherited_visibility(
    tree: &NodeTree,
    node_id: RawNodeId,
) -> Option<crate::computed_style::VisibilityValue> {
    let mut cur = tree.nodes.get(node_id)?.parent;
    while let Some(id) = cur {
        let node = tree.nodes.get(id)?;
        if !node.is_element() {
            return None;
        }
        let transitioning = tree
            .active_transitions
            .get(&id)
            .is_some_and(|m| m.contains_key(&TransitionProperty::Visibility));
        if transitioning || !visibility_is_inherited(tree, id) {
            return Some(node.computed_style.visibility);
        }
        cur = node.parent;
    }
    None
}

/// Hand `from`'s current `visibility` down to every descendant that inherits
/// it (#759).
///
/// `visibility` inherits, and CSS inherits the **animated** value: while a
/// closing overlay's root is held `visible` by its transition, everything
/// under it that does not declare a `visibility` of its own is visible too.
/// rinch's descendants take their style from Stylo, which knows nothing of
/// rinch's transitions and computes them from the root's *after-change* value
/// — so without this the panel of a closing `Drawer` vanished on the first
/// frame of the close, under a root that was still on screen.
///
/// The walk stops at a descendant that declares its own `visibility`
/// ([`visibility_is_inherited`]) — a closed `Popover` inside a closing drawer
/// stays hidden — and at one running a `visibility` transition of its own,
/// whose value is its transition's business. Text nodes are skipped: paint
/// reads a text run's visibility from its parent element.
///
/// Called by the cascade after a pass that touched a node with a `visibility`
/// transition (every cascade of a descendant resets it to Stylo's value), and
/// by [`tick_transitions`] whenever a tick changes such a node's value — which
/// for a closing overlay is once, at the end. When no transition is running the
/// value handed down equals the one Stylo computed, so a call is a no-op in
/// effect; it is simply never made then.
/// **A descendant that declares its own `visibility` transition** (a
/// `Checkbox`'s `transition: all 150ms`) is not written: the change reaches it
/// as a *change of its inherited value*, which is exactly what starts a
/// transition in CSS, so one is started for it here at `now` and its animated
/// value is what its own descendants inherit. That is Chrome's order of events
/// for a closing drawer holding a checkbox: the checkbox inherits the held
/// `visible` for the drawer's 300ms and runs its own 150ms hide only once the
/// drawer's value flips (review of #991, F1). The cascade keeps it from starting
/// one any earlier — see `apply_stylo_styles_to_taffy`, which gives an
/// inheriting node its parent's *animated* visibility, not Stylo's.
pub fn propagate_inherited_visibility(tree: &mut NodeTree, from: RawNodeId, now: f64) {
    use crate::computed_style::VisibilityValue;

    if !tree.nodes.contains(from) {
        return;
    }
    let value = tree.nodes[from].computed_style.visibility;
    let mut stack: Vec<(RawNodeId, VisibilityValue)> = tree.nodes[from]
        .children
        .iter()
        .map(|&c| (c, value))
        .collect();
    let mut changed = false;

    while let Some((id, inherited)) = stack.pop() {
        let Some(node) = tree.nodes.get(id) else {
            continue;
        };
        if !node.is_element() {
            continue;
        }
        // Already at the handed-down value: whether it inherits or declared
        // that same value, what its children inherit is the same, so it needs
        // neither the rule read nor a write. (The common case on an open.)
        if node.computed_style.visibility == inherited {
            // …unless it is running a `visibility` transition of its own toward
            // some other value and the value it now inherits is where it
            // already is: §3 item 4.1, the same cancel the cascade applies, for
            // the case where the new inherited value arrives through this walk
            // rather than a cascade — a two-way `transition: visibility` root
            // reopened while a `Checkbox` under it runs its own hide (round-3
            // review of #991).
            let stale = tree
                .active_transitions
                .get(&id)
                .and_then(|m| m.get(&TransitionProperty::Visibility))
                .is_some_and(|t| {
                    !t.to
                        .same_computed_value(&AnimatableValue::Visibility(inherited))
                });
            if stale
                && visibility_is_inherited(tree, id)
                && let Some(m) = tree.active_transitions.get_mut(&id)
            {
                m.remove(&TransitionProperty::Visibility);
                if m.is_empty() {
                    tree.active_transitions.remove(&id);
                }
            }
            let node = &tree.nodes[id];
            stack.extend(node.children.iter().map(|&c| (c, inherited)));
            continue;
        }
        let own_transition = tree
            .active_transitions
            .get(&id)
            .is_some_and(|m| m.contains_key(&TransitionProperty::Visibility));
        if own_transition || !visibility_is_inherited(tree, id) {
            continue;
        }
        let old = tree.nodes[id].computed_style.visibility;
        if old != inherited {
            let mut value = inherited;
            let node = &tree.nodes[id];
            if tree.transitions_enabled
                && node.computed_style.display != crate::computed_style::DisplayValue::None
                && find_matching_spec(&node.transition_specs, TransitionProperty::Visibility)
                    .is_some()
            {
                let specs = node.transition_specs.clone();
                let change = PropertyChange {
                    property: TransitionProperty::Visibility,
                    old_value: AnimatableValue::Visibility(old),
                    new_value: AnimatableValue::Visibility(inherited),
                };
                let map = tree.active_transitions.entry(id).or_default();
                start_transitions(map, &specs, std::slice::from_ref(&change), now);
                if let Some(AnimatableValue::Visibility(v)) = map
                    .get(&TransitionProperty::Visibility)
                    .and_then(|t| t.value_at(now))
                {
                    value = v;
                }
                if map.is_empty() {
                    tree.active_transitions.remove(&id);
                }
            }
            let node = &mut tree.nodes[id];
            if node.computed_style.visibility != value {
                node.computed_style.visibility = value;
                node.dirty.insert(DirtyFlags::PAINT);
                tree.paint_dirty_nodes.push(id);
                changed = true;
            }
            let value = tree.nodes[id].computed_style.visibility;
            stack.extend(tree.nodes[id].children.iter().map(|&c| (c, value)));
            continue;
        }
        stack.extend(tree.nodes[id].children.iter().map(|&c| (c, inherited)));
    }

    // `visibility` is a hit-test input (`HitStyleKey`), and the ticking node's
    // own key check does not see its descendants.
    if changed {
        tree.hit_cache.invalidate();
    }
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

        let visibility_before = transitions
            .contains_key(&TransitionProperty::Visibility)
            .then(|| tree.nodes[node_id].computed_style.visibility);
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

        // A `visibility` step reaches the descendants that inherit it (#759) —
        // after the removal above, so a finished transition no longer shields
        // its own node from an ancestor's walk.
        if let Some(before) = visibility_before
            && tree.nodes[node_id].computed_style.visibility != before
        {
            propagate_inherited_visibility(tree, node_id, current_time_ms);
        }
    }

    any_active
}
