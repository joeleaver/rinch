//! The derivation pass: every step's state, index and clickability, worked
//! out from the stepper's props and the step's position (issues #716, #748).

use super::*;

/// Run `f` as the stepper render that `owner` was the ambient owner of.
pub(super) fn as_owned_by(owner: &Option<rinch_core::reactive::Owner>, f: impl FnOnce()) {
    match owner {
        Some(owner) => owner.run(f),
        None => rinch_core::reactive::unowned(f),
    }
}

/// Everything a [`Stepper`] tells its steps that a step cannot know about
/// itself.
///
/// `Clone` and prop-shaped rather than a borrow of the component, because the
/// late-arrival observer outlives the `render` call that installed it (issue
/// #716). No longer `Copy` as of #737: `on_step_click` is an `Rc` underneath,
/// so every call site that used to move or copy a `Derivation` into a closure
/// now clones it instead.
#[derive(Debug, Clone)]
pub(super) struct Derivation {
    pub(super) active: u32,
    pub(super) allow_next_steps_select: bool,
    pub(super) completed_icon: Option<TablerIcon>,
    pub(super) progress_icon: Option<TablerIcon>,
    pub(super) on_step_click: Option<ValueCallback<u32>>,
    /// Which render of which stepper this is, as the value of
    /// [`SETTLED_BY_ATTR`] (issue #748). Everything else in this struct is
    /// fixed for the life of that render, so a step carrying this value was
    /// settled under exactly these props.
    pub(super) settler: Rc<str>,
    /// The region below the steps that shows the active step's content, or the
    /// completed block (issue #2195).
    pub(super) content: ContentRegion,
}

thread_local! {
    /// The next [`Derivation::settler`] on this thread.
    pub(super) static NEXT_SETTLER: Cell<u64> = const { Cell::new(1) };

    /// **Test-only.** Makes [`settle_steps`] derive every step on every pass,
    /// as it did before #748 — the oracle the differential test compares the
    /// skipping pass with.
    #[cfg(test)]
    pub(super) static DERIVE_EVERY_STEP: Cell<bool> = const { Cell::new(false) };
}

/// A value no other `Stepper` render on this thread has: a counter, never
/// reused. Per **render** and not per stepper, because a re-render can hand the
/// same step nodes to a stepper whose props have changed.
pub(super) fn next_settler() -> Rc<str> {
    NEXT_SETTLER.with(|next| {
        let id = next.get();
        next.set(id + 1);
        Rc::from(id.to_string())
    })
}

/// Whether a pass may pass over a step it finds already settled.
pub(super) fn skips_settled_steps() -> bool {
    #[cfg(test)]
    {
        !DERIVE_EVERY_STEP.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        true
    }
}

impl Derivation {
    /// This stepper's default glyph for a content key, if it has one.
    pub(super) fn default_for(&self, key: &str) -> Option<TablerIcon> {
        match key {
            KEY_COMPLETED => self.completed_icon,
            KEY_PROGRESS => self.progress_icon,
            // There is no stepper-wide default for the base glyph: a step's
            // `icon`, or its number, is the step's own business.
            _ => None,
        }
    }
}

/// Give every step under `steps_container` the position, state, index and glyph
/// `d` puts it in.
///
/// **Idempotent, and that is load-bearing** (issues #716 and #745). It runs once
/// at the stepper's own render and again every time a step lands beneath the
/// container or leaves it, because either one renumbers the steps after it and
/// can move them between states.
///
/// **A step already settled where it stands is passed over** (issue #748).
/// Everything this pass gives a step is a function of three things: the
/// stepper's props, which are fixed for a render and named by
/// [`Derivation::settler`]; the step's position; and what the step asked for
/// itself, which [`StepperStep`] writes once when it renders. So a step that
/// carries this render's [`SETTLED_BY_ATTR`] and still stands at its
/// [`POSITION_ATTR`] would be given exactly what it has, and costs three
/// attribute reads — its `class`, read by [`collect_steps`], and those two —
/// and no write. Only the steps a change moved are derived: one for an append,
/// none for a removal from the end, the steps behind it for anything else.
/// Every step used to be derived on every change, about twenty-four reads and
/// three writes each.
///
/// The walk itself still visits every step per change, so growing a stepper one
/// step at a time is still quadratic in those three reads. Not visiting them
/// would rest on every arrival having been announced, and the late-child
/// dispatch is suppressed for the length of any observer's callback.
///
/// What the skip gives up: a step's own asks are read when it is derived, not
/// on every pass. Code that edits a settled step's `disabled`, `data-state` or
/// `data-step` attribute by hand is not noticed until the step next moves. It
/// was noticed before only at the next unrelated change to the list, never when
/// it happened; a [`StepperStep`] whose props change re-renders into a new node,
/// which is derived as any arrival is.
///
/// Everything a derivation reads is a *stable* record of what the caller asked
/// for — `data-state` for a named state, [`STEP_DERIVED_ATTR`] for an index this
/// pass assigned, [`ICON_HAS_ATTR`] for the glyphs the box holds — never a
/// record of what the last pass happened to draw.
pub(super) fn settle_steps(scope: &mut RenderScope, steps_container: &NodeHandle, d: &Derivation) {
    let skips = skips_settled_steps();
    let (steps, completed) = collect_steps(steps_container);
    for (position, step) in steps.iter().enumerate() {
        let position = position as u32;
        let position_str = position.to_string();

        let settled_here = step.get_attribute(SETTLED_BY_ATTR).as_deref() == Some(&*d.settler);
        // The derivation's own position, distinct from `STEP_ATTR` — which a
        // step's own `step` prop can override, for the *label* only (see
        // below). A click handler reads this one back at click time, never a
        // value baked into its closure, because a step's position can move
        // after the handler is wired, through a later insertion or removal
        // (issues #716, #745) — the same reason `data-step` itself has to be
        // re-read rather than captured (#714's pattern).
        let stands_at = step.get_attribute(POSITION_ATTR);
        let unmoved = stands_at.as_deref() == Some(position_str.as_str());
        if skips && settled_here && unmoved {
            continue;
        }
        if !unmoved {
            step.set_attribute(POSITION_ATTR, &position_str);
        }

        let disabled = step.get_attribute(DISABLED_ATTR).is_some();

        if d.allow_next_steps_select && position > d.active && !disabled {
            // A step that asked to be clickable itself already carries the
            // class. `add_class` is idempotent since #717, so the guard is
            // belt and braces rather than load-bearing.
            if !has_class(step, CLICKABLE_CLASS) {
                step.add_class(CLICKABLE_CLASS);
            }
        }

        settle_step_clickability(scope, step, position, disabled, d);

        // The index the step shows. Its own wins, so a caller may number a
        // stepper however they like; that changes the *label*, not the position
        // the state derives from — `allow_next_steps_select` counts positions,
        // and a second notion of index would disagree with it.
        //
        // `STEP_DERIVED_ATTR` is what keeps this re-runnable: after the first
        // pass every step carries `data-step`, so its presence alone can no
        // longer say who wrote it.
        let numbered_here = step.get_attribute(STEP_ATTR).is_none()
            || step.get_attribute(STEP_DERIVED_ATTR).is_some();
        if numbered_here {
            // Compared before writing. `set_attribute` does not early-out on an
            // unchanged value, and a step that only changed stepper (see
            // `SETTLED_BY_ATTR`) is derived again where it stands.
            let position = position.to_string();
            if step.get_attribute(STEP_ATTR).as_deref() != Some(position.as_str()) {
                step.set_attribute(STEP_ATTR, &position);
            }
            if step.get_attribute(STEP_DERIVED_ATTR).is_none() {
                step.set_attribute(STEP_DERIVED_ATTR, "");
            }
        }

        // The state. Its own wins here too, and `data-state` is the only thing
        // that can say it named one: `state: String` renders an explicit
        // `"inactive"` and an unset field into the same class.
        let named_state = step.get_attribute(STATE_ATTR);
        let state = match named_state.as_deref() {
            Some(named) => StepState::parse(named),
            None => {
                let derived = StepState::derive(position, d.active);
                // Swap the state class rather than rewriting `class`: the
                // clickable grant above, and the rsx `class:` prop the macro
                // merges onto a component's root, are both already on this node
                // (issue #717).
                //
                // **All three come off, not just `--inactive`.** A step that
                // named no state rendered as inactive, so dropping that one
                // looks sufficient — but this walk reaches steps that are not
                // freshly rendered by *this* stepper: a step moved in from
                // another stepper arrives carrying whatever that one gave it,
                // and a step already settled here is re-derived on every
                // insertion and every removal. Measured: one such step ended up
                // with `--progress` and `--inactive` at once. It is also what a
                // *re-run* needs, since the class this pass is replacing is the
                // one the last pass wrote. `remove_class` writes nothing when
                // the class is absent (#730), so the two extra calls are free.
                //
                // And none of it when the step already carries the derived
                // class and neither of the others, which is every step a change
                // renumbered without restating (issue #748).
                const STATES: [StepState; 3] = [
                    StepState::Completed,
                    StepState::Progress,
                    StepState::Inactive,
                ];
                let class = step.get_attribute("class").unwrap_or_default();
                let stated = STATES.iter().all(|state| {
                    class.split_whitespace().any(|c| c == state.class_name()) == (*state == derived)
                });
                if !stated {
                    for state in STATES {
                        step.remove_class(state.class_name());
                    }
                    step.add_class(derived.class_name());
                }
                derived
            }
        };

        // The number a step draws is one higher than its index, whoever set it.
        let number = step
            .get_attribute(STEP_ATTR)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(position)
            + 1;

        settle_step_icon(scope, step, state, named_state.is_some(), number, d);

        // Last, so a step carries it only once everything above has run.
        if !settled_here {
            step.set_attribute(SETTLED_BY_ATTR, &d.settler);
        }
    }

    settle_content(&steps, &completed, d);
}

/// Bring one step's clickability — the class, the keyboard reach, and the
/// wired handler — into line with its current position (issue #737).
///
/// **Mantine's default**: a strictly **completed** step — `position <
/// Derivation::active` — is clickable without
/// [`Derivation::allow_next_steps_select`]; the active step itself and every
/// step past it need that flag, or its own ask (recorded by
/// [`StepperStep::render`] under [`OWN_CLICKABLE_ATTR`], since a class
/// already on the node cannot be told from one *this* function granted on an
/// earlier pass). Mantine's own rule is `shouldAllowSelect`:
/// `state === 'stepCompleted' || allowNextStepsSelect`, and the active step's
/// `state` is `'stepProgress'`, not `'stepCompleted'` — so `active` itself is
/// *not* reachable by default, only genuinely prior steps are. All of that is
/// gated on [`Derivation::on_step_click`] being set at all — with no
/// callback, nothing here does anything, and every behaviour
/// `allow_next_steps_select` and `allow_step_click`/`allow_step_select` had
/// before #737 (cursor and hover, nothing else) is unchanged; that is why the
/// `allow_next_steps_select` class grant above lives in [`settle_steps`]
/// rather than here, untouched. A `disabled` step (the plain HTML attribute,
/// read tag-agnostically the way rinch's own focus arbiter reads it) is never
/// clickable, whatever else grants it. And **the class always follows
/// `reachable`, not only `disabled`**: a step that loses its wiring purely
/// because a sibling insertion or removal shifted it past `active` must lose
/// the cursor too, or it goes on looking clickable with nothing behind it —
/// the one exception is `own`, since a step's own ask keeps the class
/// regardless of position (and such a step never becomes unreachable, so the
/// two branches never fight over it).
///
/// **The handler is registered once per step, lazily, the first pass that
/// finds it reachable** — never baked with the step's position, which the
/// handler instead re-reads from [`POSITION_ATTR`] at **click** time (the
/// #714 pattern: a thread-local registry's lookup moved from render time to
/// dispatch time because a render-time snapshot could not see a late
/// arrival). [`CLICK_HANDLER_ID_ATTR`] is the registration's own record,
/// kept even while the step is temporarily unreachable, so a position shift
/// that makes it reachable again restores `data-rid` from it rather than
/// registering a second handler and leaking the first (issue #141's handler
/// side: a scope frees what it owns, and nothing here would ever ask the old
/// one to release itself). **Once per step per stepper render**, to be exact
/// (issue #1428): the handler closes over this render's `on_step_click`, so
/// the record names who registered each id, and a step that arrives carrying
/// another stepper's handler is given one of this stepper's — it used to keep
/// calling the stepper it came from. In a stepper with no callback such a step
/// also loses the clickable class, unless this stepper grants it one.
///
/// `tabindex="0"` plus `data-rid` is the same shape [`Tree`](crate::tree::Tree)
/// wires for its own rows — generic keyboard activation (Enter/Space) reads
/// any focused node's `data-rid`, with no `register_focus_target` needed —
/// and `role="button"` says what the reach is for.
pub(super) fn settle_step_clickability(
    scope: &mut RenderScope,
    step: &NodeHandle,
    position: u32,
    disabled: bool,
    d: &Derivation,
) {
    let own = step.get_attribute(OWN_CLICKABLE_ATTR).is_some();
    let reachable = own || position < d.active || d.allow_next_steps_select;
    let should_wire = !disabled && reachable && d.on_step_click.is_some();

    if should_wire {
        if !has_class(step, CLICKABLE_CLASS) {
            step.add_class(CLICKABLE_CLASS);
        }
        if step.get_attribute("tabindex").is_none() {
            step.set_attribute("tabindex", "0");
        }
        if step.get_attribute("role").as_deref() != Some("button") {
            step.set_attribute("role", "button");
        }

        // SAFETY of the `expect`: `should_wire` just checked `is_some()`.
        let cb = d.on_step_click.clone().expect("on_step_click is Some");
        // The handler **this render** registered for the step, if it has one
        // (issue #1428). A record left by another stepper, or by an earlier
        // render of this one, names a handler that calls *that* render's
        // callback, so it is not ours to restore `data-rid` from.
        let record = step.get_attribute(CLICK_HANDLER_ID_ATTR);
        let handler_id = match recorded_handler(record.as_deref(), &d.settler) {
            Some(id) => id,
            None => {
                let watched = step.clone();
                let id = scope
                    .register_handler(move || {
                        if let Some(pos) = watched
                            .get_attribute(POSITION_ATTR)
                            .and_then(|s| s.parse::<u32>().ok())
                        {
                            cb.invoke(pos);
                        }
                    })
                    .0;
                // Added to the record, not written over it: the step may be
                // moved back, and the stepper it came from should find the
                // handler it already paid for.
                // Entries whose handler is gone go: a stepper render frees
                // its handlers with its scope, and its settler is never used
                // again, so such an entry can only grow the record.
                let mut record: Vec<&str> = record
                    .as_deref()
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter(|entry| handler_of(entry).is_some_and(|(_, id)| is_registered(id)))
                    .collect();
                let entry = format!("{}:{id}", d.settler);
                record.push(&entry);
                let record = record.join(" ");
                step.set_attribute(CLICK_HANDLER_ID_ATTR, &record);
                id
            }
        };
        let rid = handler_id.to_string();
        if step.get_attribute("data-rid").as_deref() != Some(rid.as_str()) {
            step.set_attribute("data-rid", &rid);
        }
    } else {
        // The class follows reachability whenever there is a callback to
        // wire at all — a step that was reachable-and-wired and then shifts
        // past `active` (a sibling insertion or removal) must lose the
        // cursor along with `data-rid`, or it keeps looking clickable with
        // nothing behind it (issue #737's review, Finding 2). With no
        // callback there is nothing to wire in the first place, so the class
        // here is purely whatever `allow_next_steps_select`'s own grant (in
        // `settle_steps`) or `StepperStep::render`'s own-ask grant put there,
        // and must be left alone — removing it would undo the no-callback
        // decorative behaviour #737 was explicit about preserving.
        if d.on_step_click.is_some() {
            step.remove_class(CLICKABLE_CLASS);
        } else if !own
            && !(d.allow_next_steps_select && position > d.active && !disabled)
            && step.get_attribute(CLICK_HANDLER_ID_ATTR).is_some()
        {
            // The one class a callback-less stepper does take off (issue
            // #1428): the step was wired by a stepper that had a callback
            // and has been moved here, so the class may be that wiring's,
            // and nothing in this stepper grants it — not the step's own
            // ask, not the `allow_next_steps_select` grant in
            // `settle_steps`. Left on, the step shows a pointer with nothing
            // behind it. A step no stepper ever wired carries no record and
            // is not touched.
            step.remove_class(CLICKABLE_CLASS);
        }
        if step.get_attribute("data-rid").is_some() {
            step.remove_attribute("data-rid");
        }
        if step.get_attribute("tabindex").is_some() {
            step.remove_attribute("tabindex");
        }
        if step.get_attribute("role").as_deref() == Some("button") {
            step.remove_attribute("role");
        }
    }
}
