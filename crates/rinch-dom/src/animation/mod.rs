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
/// - the stops are re-extracted when the rule's blocks changed, or a
///   base-style input an authored stop **read** at extraction moved
///   ([`StopInputs`]: the element's `font-size` for an `em`, its `color` for a
///   `currentcolor`, the parent's `color` for `color: currentcolor`), and
///   otherwise kept with only their implicit (base) values re-read
///   ([`keyframes::refresh_implicit_values`]).
///
/// So a node cascaded while it animates pays, per animation, one keyframes
/// lookup, a pointer compare per keyframe, and a re-read of its implicit
/// 0%/100% values from the base style; a hover that moves `color` under a
/// `transform` spin re-extracts nothing. A node with no animation never gets
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
    let (parent_color, parent_font_size, root_font_size) = outer_inputs(tree, node_id);
    let base = keyframes_base(tree, node_id, base_style);

    // What the stops of a rule resolve against on this node now.
    let basis =
        keyframes::StopBasis::new(base_style, parent_color, parent_font_size, root_font_size);
    // The stops of a rule against that basis, with the inputs they read.
    let extract = |kf_anim: &style::stylesheets::keyframes_rule::KeyframesAnimation| {
        let basis = basis.clone();
        let stops = keyframes::extract_keyframe_stops_against(kf_anim, base_style, &basis, guard);
        (!stops.is_empty()).then(|| (stops, StopInputs::new(kf_anim, basis)))
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
                    let elapsed = existing_anim.elapsed_ms(current_time_ms);
                    existing_anim.paused_elapsed_ms = Some(elapsed);
                    existing_anim.play_state = AnimationPlayState::Paused;
                }
                (AnimationPlayState::Paused, AnimationPlayState::Running) => {
                    // Resume: adjust start time to maintain position
                    if let Some(paused_at) = existing_anim.paused_elapsed_ms {
                        existing_anim.start_time_ms = current_time_ms - paused_at;
                        existing_anim.paused_elapsed_ms = None;
                        existing_anim.pending = false;
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

            if !existing_anim.stop_inputs.hold_for(kf_anim, &basis) {
                let Some((stops, inputs)) = extract(kf_anim) else {
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
            existing_anim.base = base.for_stops(&existing_anim.keyframe_stops);

            new_active.push(existing_anim);
        } else if let Some((stops, stop_inputs)) = extract(kf_anim) {
            // New animation
            new_active.push(ActiveAnimation {
                name: spec.name.clone(),
                default_timing: spec.timing.clone(),
                duration_ms: spec.duration_ms,
                delay_ms: spec.delay_ms,
                direction: spec.direction,
                iteration_count: spec.iteration_count,
                fill_mode: spec.fill_mode,
                play_state: spec.play_state,
                start_time_ms: current_time_ms,
                pending: true,
                paused_elapsed_ms: if spec.play_state == AnimationPlayState::Paused {
                    Some(0.0)
                } else {
                    None
                },
                settled: false,
                base: base.for_stops(&stops),
                keyframe_stops: stops,
                stop_inputs,
                position,
            });
        }
    }

    // A settled animation that holds no fill writes nothing, on a tick or in a
    // cascade, so it goes back to `finished_animations`, where no tick walks
    // it (review of #2000, F1).
    let (finished, mut running): (Vec<_>, Vec<_>) = new_active
        .into_iter()
        .partition(|a| a.settled && !a.fills_forwards());
    // The caller samples these next: bring their stops to this sample.
    rebase_node_stops(&mut running, tree, node_id, guard, current_time_ms);
    if !running.is_empty() {
        active_animations.insert(node_id, running);
    }
    if !finished.is_empty() {
        finished_animations.insert(node_id, finished);
    }
}

/// What a node's keyframe stops read from outside the node: its parent's
/// `color` (`color: currentcolor`) and `font-size` (a `font-size` `em` or
/// percentage), and the root element's `font-size` (`rem`). The root is
/// `<html>`, not `tree.root_id` — the document node, whose `font-size` is
/// always the initial 16px, so a `rem` stop used to ignore
/// `html { font-size }` (#2058).
fn outer_inputs(tree: &NodeTree, node_id: RawNodeId) -> (Option<peniko::Color>, f32, f32) {
    let parent = tree
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|parent| tree.get_live(parent));
    // Read from `<html>`'s Stylo style, the value the cascade's own `rem`
    // resolves against (`sync_root_font_size`): its `computed_style` is not
    // kept up to date, `<html>` having no layout box of its own to sync.
    let root_font_size = tree
        .get_live(tree.html_id)
        .and_then(|html| {
            let data = html.stylo_element_data.borrow();
            let cv = data.as_ref()?.styles.get_primary()?;
            Some(
                cv.effective_zoom
                    .unzoom(cv.get_font().clone_font_size().computed_size().px()),
            )
        })
        .unwrap_or(16.0);
    (
        parent.and_then(|p| p.computed_style.color),
        parent.map_or(16.0, |p| p.computed_style.font_size),
        root_font_size,
    )
}

/// The node's [`BaseInputs`] at this cascade, before they are narrowed to an
/// animation's stops ([`BaseInputs::for_stops`]).
fn keyframes_base(tree: &NodeTree, node_id: RawNodeId, base_style: &ComputedStyle) -> BaseInputs {
    let mut current_color = Vec::new();
    if let Some(node) = tree.get(node_id)
        && let Some(cv) = node
            .stylo_element_data
            .borrow()
            .as_ref()
            .and_then(|d| d.styles.get_primary())
    {
        use crate::transition::TransitionProperty as P;
        let border = cv.get_border();
        for (p, c) in [
            (P::BorderTopColor, &border.border_top_color),
            (P::BorderRightColor, &border.border_right_color),
            (P::BorderBottomColor, &border.border_bottom_color),
            (P::BorderLeftColor, &border.border_left_color),
            (P::BackgroundColor, &cv.get_background().background_color),
        ] {
            if c.is_currentcolor() {
                current_color.push(p);
            }
        }
    }
    BaseInputs {
        font_size: base_style.font_size,
        color: base_style.color,
        current_color,
    }
}

impl BaseInputs {
    /// These inputs, keeping of the `currentcolor` properties only the ones
    /// some stop takes from the base, so an animation that has none costs a
    /// sample nothing.
    fn for_stops(&self, stops: &[KeyframeStop]) -> BaseInputs {
        BaseInputs {
            font_size: self.font_size,
            color: self.color,
            current_color: self
                .current_color
                .iter()
                .copied()
                .filter(|p| stops.iter().any(|s| s.implicit.contains(p)))
                .collect(),
        }
    }
}

/// Bring a node's keyframe stops to the basis the node has **at this sample**
/// (#2056, #2058): CSS computes a keyframe's values at each sample, so an
/// `em` resolves against the animated `font-size`, a `currentcolor` (authored,
/// or the base value of an implicit stop) against the animated `color`, and a
/// `rem`, a `font-size` `em` or an inherited `currentcolor` against the root's
/// and the parent's values as they are now. Returns whether any stop changed.
///
/// Only the inputs an animation's authored stops read are compared
/// (`StopDeps`), so a `transform` spin returns at the first check, and stops
/// that read only inputs which did not move are not resolved again. An `em`
/// stop beside a `font-size` animation is resolved on every sample, as its
/// value moves on every sample.
///
/// Two passes, because the own `font-size` and `color` of the sample are
/// themselves animated values: the first brings the stops up to date on the
/// parent's and root's inputs and samples `font-size` and `color` (whose own
/// stops read only those — a `font-size` `em` is the parent's, `color:
/// currentcolor` the parent's colour); the second resolves everything that
/// reads the element's own against those samples.
pub(crate) fn rebase_node_stops(
    anims: &mut [ActiveAnimation],
    tree: &NodeTree,
    node_id: RawNodeId,
    guard: &SharedRwLockReadGuard,
    current_time_ms: f64,
) -> bool {
    use crate::transition::{AnimatableValue, TransitionProperty as P};

    let mut deps = StopDeps::default();
    let mut current_color = false;
    for a in anims.iter() {
        let r = a.stop_inputs.basis().read();
        deps.em |= r.em;
        deps.parent_em |= r.parent_em;
        deps.rem |= r.rem;
        deps.color |= r.color;
        deps.parent_color |= r.parent_color;
        current_color |= !a.base.current_color.is_empty();
    }
    let needs_own = deps.em || deps.color || current_color;
    if !needs_own && !deps.parent_em && !deps.rem && !deps.parent_color {
        return false;
    }
    let Some(base) = anims.first().map(|a| a.base.clone()) else {
        return false;
    };
    let (parent_color, parent_font_size, root_font_size) = outer_inputs(tree, node_id);
    let live = keyframes::StopBasis::from_parts(
        base.font_size,
        base.color,
        parent_color,
        parent_font_size,
        root_font_size,
    );

    let rebase = |a: &mut ActiveAnimation, basis: keyframes::StopBasis| {
        // An `Arc` clone: the rule is shared, not copied.
        let rule = std::sync::Arc::clone(a.stop_inputs.animation());
        keyframes::rebase_authored_values(&mut a.keyframe_stops, &rule, &basis, guard);
        a.stop_inputs.set_basis(basis);
    };

    let mut changed = false;
    let outer = StopDeps {
        parent_em: true,
        rem: true,
        parent_color: true,
        ..StopDeps::default()
    };
    let (mut font_size, mut color) = (base.font_size, base.color);
    for a in anims.iter_mut() {
        let held = a.stop_inputs.basis();
        let read = held.read();
        let outer_read = StopDeps {
            parent_em: read.parent_em && outer.parent_em,
            rem: read.rem && outer.rem,
            parent_color: read.parent_color && outer.parent_color,
            ..StopDeps::default()
        };
        if !held.agrees_on(&live, outer_read) {
            // The element's own inputs as they were: the second pass resolves
            // those, so a stop moved here is not resolved twice per sample.
            let (fs, c) = held.own();
            rebase(a, live.at(fs, c));
            changed = true;
        }
        if needs_own
            && (a.animates(P::FontSize) || a.animates(P::Color))
            && let AnimationResult::Values(values) = a.values_at(current_time_ms)
        {
            for (p, v) in values {
                match (p, v) {
                    (P::FontSize, AnimatableValue::Float(px)) => font_size = px,
                    (P::Color, AnimatableValue::Color(c)) => color = Some(c),
                    _ => {}
                }
            }
        }
    }
    if !needs_own {
        return changed;
    }

    let frame = live.at(font_size, color);
    for a in anims.iter_mut() {
        let held = a.stop_inputs.basis();
        if !held.agrees_on(&frame, held.read()) {
            rebase(a, frame.at(font_size, color));
            changed = true;
        }
        if let Some(color) = color
            && !a.base.current_color.is_empty()
        {
            let ActiveAnimation {
                keyframe_stops,
                base,
                ..
            } = a;
            for stop in keyframe_stops.iter_mut() {
                for (p, v) in stop.values.iter_mut() {
                    if stop.implicit.contains(p)
                        && base.current_color.contains(p)
                        && !matches!(v, AnimatableValue::Color(c) if *c == color)
                    {
                        *v = AnimatableValue::Color(color);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
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
    // The twin of `tick_transitions`' (#777, #787).
    let cascade_nodes = tree.animation_cascade_nodes();
    if !cascade_nodes.is_empty() {
        // A style change is owed: a host that skips a clean frame
        // (`RinchApp::resolve_and_repaint`) must still run the cascade.
        tree.styles_dirty = true;
        tree.animated_recascade.extend(cascade_nodes);
    }
    let node_ids: Vec<RawNodeId> = tree.active_animations.keys().copied().collect();
    let mut any_active = false;

    for node_id in node_ids {
        if !tree.nodes.contains(node_id) {
            tree.active_animations.remove(&node_id);
            continue;
        }

        let mut animations = match tree.active_animations.get(&node_id) {
            Some(a) => a.clone(),
            None => continue,
        };
        // Stops computed at this sample (#2056, #2058). A stop that moved
        // dirties the node even for a paused or filling animation, whose
        // sample is otherwise constant.
        let rebased = {
            let guard = tree.guard.read();
            rebase_node_stops(&mut animations, tree, node_id, &guard, current_time_ms)
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

        if rebased {
            needs_paint = true;
            needs_layout |= animations.iter().any(|a| {
                a.keyframe_stops
                    .iter()
                    .any(|k| k.values.iter().any(|(p, _)| p.affects_layout()))
            });
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
