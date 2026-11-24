//! A step's icon box: which glyph is live for the step's state.

use super::*;

/// Bring one step's icon box into line with the state and index it has just been
/// given.
///
/// The box holds exactly one **live** glyph plus zero or more hidden
/// *alternates*, each labelled with the [content key](KEY_BASE) it stands for.
/// [`ICON_LIVE_ATTR`] says which key the live one is, so this function's whole
/// job is: work out which key the step's current state wants, put that glyph
/// live, and park the outgoing one as an alternate in case the step moves again.
///
/// Parking rather than discarding is what makes a late arrival work (issue
/// #716): a step's `icon` prop is not recoverable from the rendered tree, so a
/// step that was moved to completed and is later displaced back out of it has
/// nowhere else to get its own glyph from. **Every** alternate is kept for as
/// long as the step's state is this stepper's to derive, because a keyed `for`
/// reorder can move a step in either direction.
pub(super) fn settle_step_icon(
    scope: &mut RenderScope,
    step: &NodeHandle,
    state: StepState,
    named_its_own_state: bool,
    number: u32,
    d: &Derivation,
) {
    let Some(icon_box) = step_icon_box(step) else {
        return;
    };

    // A loading step draws a spinner, which is the right content in every state
    // and carries no number. It leaves no alternates and claims no key, so there
    // is nothing here that could apply to it.
    if has_class(step, LOADING_CLASS) {
        return;
    }

    // A box built by something other than `StepperStep` says nothing about what
    // it holds, and guessing would mean appending a second glyph beside the
    // first. Leave it exactly as it is.
    let Some(mut live_key) = icon_box.get_attribute(ICON_LIVE_ATTR) else {
        return;
    };

    let mut has: Vec<String> = icon_box
        .get_attribute(ICON_HAS_ATTR)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    let mut live = None;
    let mut alternates: Vec<(String, NodeHandle)> = Vec::new();
    for child in icon_box.children() {
        if has_class(&child, ICON_ALT_CLASS) {
            alternates.push((
                child.get_attribute(ICON_ALT_ATTR).unwrap_or_default(),
                child,
            ));
        } else {
            live = Some(child);
        }
    }

    // Which glyph this state wants. `progress` is the only key whose answer
    // depends on the stepper as well as the step: a step with no progress icon
    // of its own draws its base glyph there *unless* this stepper supplies one.
    let target = match state {
        StepState::Completed => KEY_COMPLETED,
        StepState::Progress => {
            if has.iter().any(|k| k == KEY_PROGRESS) || d.progress_icon.is_some() {
                KEY_PROGRESS
            } else {
                KEY_BASE
            }
        }
        StepState::Inactive => KEY_BASE,
    };

    if live_key != target {
        // What happens to the outgoing glyph turns on whether it can be built
        // again. A **built-in** — the tick, or the step number — is recoverable
        // from nothing at all, so it goes for good, by `discard` and not
        // `remove` (issue #719): `rinch-web` keeps a strong `web_sys::Node` for
        // a merely-detached subtree. A **real** glyph came from a `TablerIcon`
        // in the step's props, which no patch of the rendered tree can recover,
        // so it is parked under its own key against a later move (issue #716).
        if let Some(live) = &live {
            if has.contains(&live_key) {
                let parked = scope.create_element("span");
                parked.set_attribute("class", ICON_ALT_CLASS);
                parked.set_attribute(ICON_ALT_ATTR, &live_key);
                parked.set_style("display", "none");
                parked.append_child(live);
                icon_box.append_child(&parked);
                alternates.push((live_key.clone(), parked));
            } else {
                live.discard();
            }
        }

        let wanted = match alternates.iter().position(|(key, _)| key == target) {
            Some(index) => {
                let (_, wrapper) = alternates.remove(index);
                // **Out of the wrapper before the wrapper goes.** `discard`
                // retires a whole subtree, so discarding a wrapper that still
                // held the glyph would retire the glyph with it (issue #719).
                let glyph = wrapper.children().into_iter().next();
                if let Some(glyph) = &glyph {
                    icon_box.append_child(glyph);
                }
                wrapper.discard();
                glyph
            }
            None => match d.default_for(target) {
                Some(icon) => {
                    // The stepper's own default fills a key the step supplied
                    // nothing for, and the box now holds a real glyph there —
                    // which is what stops a later pass filling it twice.
                    //
                    // Guarded because this attribute is rendered DOM. An
                    // unguarded push grew it by a token on every backwards move
                    // (`"completed base completed"`) while alternates were being
                    // pruned; **keeping them closed that path**, since a glyph
                    // whose key is in `has` is always parked, so this branch can
                    // no longer find a key both absent from the box and present
                    // in `has`. No fixture kills the guard for that reason — the
                    // dedupe assertion in
                    // `a_step_a_keyed_reorder_moves_backwards_keeps_its_own_completed_icon`
                    // passes either way now, and is there to catch a future
                    // path, not this one.
                    push_key(&mut has, target);
                    let glyph = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
                    icon_box.append_child(&glyph);
                    Some(glyph)
                }
                None => {
                    let glyph = built_in_glyph(scope, target, number);
                    icon_box.append_child(&glyph);
                    Some(glyph)
                }
            },
        };
        live = wanted;
        live_key = target.to_owned();
        icon_box.set_attribute(ICON_LIVE_ATTR, target);
    } else if !has.iter().any(|k| k == target)
        && let Some(icon) = d.default_for(target)
    {
        // The right key already, but what is showing there is the built-in
        // placeholder — the tick a step with no completed icon draws — and this
        // stepper has something better. That is #707's case, and it is the one
        // path that does not change key.
        let glyph = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
        icon_box.append_child(&glyph);
        if let Some(old) = &live {
            // Gone for good, so it leaves by `discard` and not by `remove`
            // (issue #719): `rinch-web` keeps a strong `web_sys::Node` for a
            // merely-removed subtree, and a `Stepper` re-renders per signal
            // change.
            old.discard();
        }
        push_key(&mut has, target);
        live = Some(glyph);
    }

    // The number, which the step could only have drawn as its own index plus
    // one — and which a step arriving or leaving in front of it changes.
    if live_key == KEY_BASE
        && !has.iter().any(|k| k == KEY_BASE)
        && let Some(live) = &live
    {
        // Compared first: a step that only changed stepper is derived again
        // where it stands, and `set_text` does not early-out on an unchanged
        // value.
        let number = number.to_string();
        if live.text_content().as_deref() != Some(number.as_str()) {
            live.set_text(&number);
        }
    }

    // **Every parked alternate is kept**, for a step whose state this stepper
    // derives. It used to keep only the ones a *forward* move could want, on the
    // reasoning that this pass re-runs on an insertion and an insertion can only
    // give a step more siblings in front of it. That is false twice over: a
    // keyed `for` **reorder** repositions a live node with `insert_before`,
    // which is one of the four notifying verbs, and it moves that node
    // *backwards*; and since #745 a **removal** re-runs this pass too, which
    // moves every step behind the one that went backwards as well. Measured — a
    // step carrying its own `completed_icon`, moved from position 2 to position
    // 0 — the pruned alternate was gone and the step drew the built-in tick, or
    // the *stepper's* default where it had one, inverting the rule that a step's
    // own icon wins.
    //
    // A step that named its own state is in the state it will stay in wherever
    // it is moved to, so for it every alternate really is dead.
    if named_its_own_state {
        for (_, wrapper) in alternates {
            wrapper.discard();
        }
    }

    // Both arms compared first: a renumbered step is derived again, and
    // neither `set_attribute` nor `remove_attribute` is free for an attribute
    // that already says so (issue #748).
    let has = has.join(" ");
    let recorded = icon_box.get_attribute(ICON_HAS_ATTR);
    if has.is_empty() {
        if recorded.is_some() {
            icon_box.remove_attribute(ICON_HAS_ATTR);
        }
    } else if recorded.as_deref() != Some(has.as_str()) {
        icon_box.set_attribute(ICON_HAS_ATTR, &has);
    }
}

/// Record that the box holds a real glyph for `key`, once.
pub(super) fn push_key(has: &mut Vec<String>, key: &str) {
    if !has.iter().any(|k| k == key) {
        has.push(key.to_owned());
    }
}

/// The built-in glyph for a content key: the tick for completed, the step number
/// for anything else.
pub(super) fn built_in_glyph(scope: &mut RenderScope, key: &str, number: u32) -> NodeHandle {
    if key == KEY_COMPLETED {
        crate::icons::check_dom(scope)
    } else {
        rinch_core::IntoNode::into_node(number.to_string(), scope)
    }
}
