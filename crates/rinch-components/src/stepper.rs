//! Stepper component.
//!
//! Step-by-step progress indicator.

use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::ValueCallback;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Stepper size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepperSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl StepperSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            StepperSize::Xs => "rinch-stepper--xs",
            StepperSize::Sm => "rinch-stepper--sm",
            StepperSize::Md => "rinch-stepper--md",
            StepperSize::Lg => "rinch-stepper--lg",
            StepperSize::Xl => "rinch-stepper--xl",
        }
    }
}

impl std::str::FromStr for StepperSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(StepperSize::Xs),
            "sm" => Ok(StepperSize::Sm),
            "md" => Ok(StepperSize::Md),
            "lg" => Ok(StepperSize::Lg),
            "xl" => Ok(StepperSize::Xl),
            _ => Err(()),
        }
    }
}

/// Stepper orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepperOrientation {
    #[default]
    Horizontal,
    Vertical,
}

impl StepperOrientation {
    pub fn class_name(&self) -> &'static str {
        match self {
            StepperOrientation::Horizontal => "rinch-stepper--horizontal",
            StepperOrientation::Vertical => "rinch-stepper--vertical",
        }
    }
}

impl std::str::FromStr for StepperOrientation {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "horizontal" => Ok(StepperOrientation::Horizontal),
            "vertical" => Ok(StepperOrientation::Vertical),
            _ => Err(()),
        }
    }
}

/// Stepper component for multi-step processes.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Stepper { active: 1, completed_icon: TablerIcon::CircleCheck,
///         StepperStep { label: "Step 1", description: "Create account" }
///         StepperStep { label: "Step 2", description: "Verify email" }
///         StepperStep { label: "Step 3", description: "Complete profile" }
///     }
/// }
/// ```
///
/// Each step's state and index come from [`Stepper::active`] and the step's own
/// position: the steps before `active` are completed, the one at it is in
/// progress, and the ones after it are inactive (issue #709). So the example
/// above draws a `CircleCheck` on step 1, the number `2` on step 2, and `3` on
/// step 3, with no `state:` or `step:` written by hand.
///
/// A step may still name its own [`state`](StepperStep::state) or
/// [`step`](StepperStep::step), and what it names it keeps — that is the escape
/// hatch for a stepper whose states are not a simple prefix of the list.
#[derive(Debug, Default)]
pub struct Stepper {
    /// Currently active step (0-indexed).
    ///
    /// Each step takes its state from its position against this: before it
    /// completed, at it in progress, after it inactive. A step that named a
    /// `state` of its own keeps it. A step that arrives **later**, by a `for`
    /// reconcile or a `show_dom` branch, is stated as it lands — and so are the
    /// steps it displaced, since an insertion moves everything after it (issue
    /// #716). A step *removed*, or moved out to another stepper, renumbers and
    /// restates the steps behind it the same way (issue #745).
    ///
    /// A closure — `active: {|| signal.get()}` — re-renders the whole stepper,
    /// children included, so the derivation runs again on every change.
    pub active: u32,
    /// Size variant (xs, sm, md, lg, xl).
    pub size: String,
    /// Orientation (horizontal, vertical).
    pub orientation: String,
    /// Color for completed/active steps.
    pub color: String,
    /// Border radius for step icons.
    pub radius: String,
    /// Icon size.
    pub icon_size: String,
    /// Whether a step *after* the active one may be selected.
    ///
    /// When true, each step past [`Stepper::active`] that did not ask to be
    /// clickable itself is made clickable, a step that arrives later included
    /// (issue #716). A step that set
    /// `allow_step_click` or `allow_step_select` of its own is already clickable
    /// and is left alone, so this only ever grants — turning it off does not
    /// take a step's own ask away.
    ///
    /// This prop alone is still decorative — the class carries a cursor and a
    /// hover state, and granting it adds no handler. What makes a grant *act*
    /// is [`Stepper::on_step_click`] (issue #737): with no callback set, every
    /// behaviour this prop had before #737 is unchanged (the class, and only
    /// the class).
    pub allow_next_steps_select: bool,
    /// Fired with a step's 0-based position when the user clicks or activates
    /// (Enter/Space) a step the stepper considers clickable (issue #737).
    ///
    /// Mantine's default: a strictly **completed** step — `position <
    /// Stepper::active` — is clickable without [`allow_next_steps_select`];
    /// the active step itself and every step past it need that flag, or the
    /// step's own
    /// [`allow_step_click`](StepperStep::allow_step_click) /
    /// [`allow_step_select`](StepperStep::allow_step_select) (Mantine's
    /// `shouldAllowSelect`: `state === 'stepCompleted' || allowNextStepsSelect`,
    /// and `state` for the active step is `'stepProgress'`, not
    /// `'stepCompleted'`). With no callback set, nothing is wired — a step's
    /// `cursor: pointer` from its own ask or from `allow_next_steps_select` is
    /// unchanged, decorative, as it always was.
    ///
    /// `Stepper` does not move `active` itself: the caller does, from this
    /// callback, exactly as [`Tabs`](crate::tabs::Tabs) moves its own selection
    /// from a tab click.
    ///
    /// The index is read from the clicked step's own position **at click
    /// time**, not baked into the handler at the render pass that wired it
    /// (the #714 pattern): a step's position can move after it registers its
    /// handler, through a later insertion or removal (issues #716, #745), and
    /// a captured index would then fire the position the step *used to* be at.
    pub on_step_click: Option<ValueCallback<u32>>,
    /// Default completed-step icon.
    ///
    /// Used by the [`StepperStep`]s in the completed state that set no
    /// `completed_icon` of their own — the step's icon wins, whether the step
    /// named its own state or this stepper derived it. Steps are patched after
    /// they have rendered, since a parent component renders *after* its
    /// children, and a step that arrives later is patched as it lands (issue
    /// #716).
    ///
    /// A step **moved in from another stepper** keeps the icon that stepper gave
    /// it: the box records that it holds a real glyph for the state, not which
    /// stepper put it there (issue #745).
    pub completed_icon: Option<TablerIcon>,
    /// Default in-progress-step icon.
    ///
    /// Used by the [`StepperStep`]s in the progress state that set no
    /// `progress_icon` of their own, in place of that step's `icon` or its
    /// number. The step's own `progress_icon` wins, and a step that arrives
    /// later takes this one as it lands (issue #716).
    pub progress_icon: Option<TablerIcon>,
}

/// Which of the three states a step is in.
///
/// A step is in exactly one, and carries exactly one of the three modifier
/// classes for it. `StepperStep` reads the caller's string into this and
/// `Stepper` derives it from a position, so the two agree by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepState {
    Completed,
    Progress,
    Inactive,
}

impl StepState {
    /// The caller's spelling. Anything but the two named states is inactive,
    /// **including the empty string** — which is why an unset `state` cannot be
    /// told from an explicit `"inactive"` by the outcome alone, and why the step
    /// records the ask separately as `data-state`.
    fn parse(s: &str) -> Self {
        match s {
            "completed" => StepState::Completed,
            "progress" => StepState::Progress,
            _ => StepState::Inactive,
        }
    }

    /// The state of the step at `index` in a stepper on `active`.
    fn derive(index: u32, active: u32) -> Self {
        match index.cmp(&active) {
            Ordering::Less => StepState::Completed,
            Ordering::Equal => StepState::Progress,
            Ordering::Greater => StepState::Inactive,
        }
    }

    fn class_name(self) -> &'static str {
        match self {
            StepState::Completed => "rinch-stepper__step--completed",
            StepState::Progress => "rinch-stepper__step--progress",
            StepState::Inactive => "rinch-stepper__step--inactive",
        }
    }
}

impl Stepper {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-stepper"];

        if !self.size.is_empty()
            && let Ok(size) = self.size.parse::<StepperSize>()
        {
            classes.push(size.class_name());
        }

        if !self.orientation.is_empty() {
            if let Ok(orientation) = self.orientation.parse::<StepperOrientation>() {
                classes.push(orientation.class_name());
            }
        } else {
            classes.push(StepperOrientation::Horizontal.class_name());
        }

        let radius_cls = crate::class_utils::radius_class("rinch-stepper", &self.radius);
        classes.extend(radius_cls.as_deref());

        classes.join(" ")
    }
}

impl Component for Stepper {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if !self.color.is_empty() {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                style_parts.push(format!("--rinch-stepper-color: {}", c));
            } else {
                style_parts.push(format!("--rinch-stepper-color: var(--rinch-color-{}-6)", c));
            }
        }
        if !self.icon_size.is_empty() {
            style_parts.push(format!("--rinch-stepper-icon-size: {}", self.icon_size));
        }

        let container = rinch_macros::rsx! { div { class: "rinch-stepper" } };
        container.set_attribute("class", &self.class_string());
        container.set_attribute("data-active", &self.active.to_string());

        if !style_parts.is_empty() {
            container.set_attribute("style", &style_parts.join("; "));
        }

        let steps_container = rinch_macros::rsx! { div { class: "rinch-stepper__steps" } };

        for child in children {
            steps_container.append_child(child);
        }

        let derivation = Derivation {
            active: self.active,
            allow_next_steps_select: self.allow_next_steps_select,
            completed_icon: self.completed_icon,
            progress_icon: self.progress_icon,
            on_step_click: self.on_step_click.clone(),
            settler: next_settler(),
        };
        settle_steps(__scope, &steps_container, &derivation);

        // A step that arrives *after* this render — a `for` reconcile, a
        // `show_dom` branch, a hand-rolled `append_child` — needs the same pass,
        // and so do the steps it displaces: an insertion in front of a step
        // moves it, which changes its number and can change its state (issue
        // #716). So the pass runs again over the whole list, and it is written
        // to be idempotent for exactly that reason. It derives only the steps
        // the change moved (issue #748): one this stepper settled at the
        // position it still holds is passed over.
        let watched = steps_container.clone();
        let d = derivation.clone();
        crate::late_children::adopt_late_children(
            __scope,
            &steps_container,
            STEP_BOUNDARY,
            move |_inserted, scope| {
                // The subtree that landed is deliberately ignored. What has to
                // be found is every step's *position*, and only the whole list
                // gives that — an insertion in front of a step renumbers it and
                // can restate it, so patching the newcomer alone would leave the
                // stepper saying two different things.
                settle_steps(scope, &watched, &d);
            },
        );

        // And a step that *leaves* (issue #745). A removal is the same event
        // read the other way: every step behind the one that went moves
        // backwards, which renumbers it and can restate it. `List` and
        // `RadioGroup` register no such observer, because neither of their
        // defaults depends on a sibling's position; this stepper's whole
        // derivation does.
        let watched = steps_container.clone();
        let d = derivation.clone();
        crate::late_children::adopt_child_removals(
            __scope,
            &steps_container,
            STEP_BOUNDARY,
            move |scope| {
                settle_steps(scope, &watched, &d);
            },
        );

        container.append_child(&steps_container);

        container
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
struct Derivation {
    active: u32,
    allow_next_steps_select: bool,
    completed_icon: Option<TablerIcon>,
    progress_icon: Option<TablerIcon>,
    on_step_click: Option<ValueCallback<u32>>,
    /// Which render of which stepper this is, as the value of
    /// [`SETTLED_BY_ATTR`] (issue #748). Everything else in this struct is
    /// fixed for the life of that render, so a step carrying this value was
    /// settled under exactly these props.
    settler: Rc<str>,
}

thread_local! {
    /// The next [`Derivation::settler`] on this thread.
    static NEXT_SETTLER: Cell<u64> = const { Cell::new(1) };

    /// **Test-only.** Makes [`settle_steps`] derive every step on every pass,
    /// as it did before #748 — the oracle the differential test compares the
    /// skipping pass with.
    #[cfg(test)]
    static DERIVE_EVERY_STEP: Cell<bool> = const { Cell::new(false) };
}

/// A value no other `Stepper` render on this thread has: a counter, never
/// reused. Per **render** and not per stepper, because a re-render can hand the
/// same step nodes to a stepper whose props have changed.
fn next_settler() -> Rc<str> {
    NEXT_SETTLER.with(|next| {
        let id = next.get();
        next.set(id + 1);
        Rc::from(id.to_string())
    })
}

/// Whether a pass may pass over a step it finds already settled.
fn skips_settled_steps() -> bool {
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
    fn default_for(&self, key: &str) -> Option<TablerIcon> {
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
fn settle_steps(scope: &mut RenderScope, steps_container: &NodeHandle, d: &Derivation) {
    let skips = skips_settled_steps();
    for (position, step) in collect_steps(steps_container).iter().enumerate() {
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
/// one to release itself).
///
/// `tabindex="0"` plus `data-rid` is the same shape [`Tree`](crate::tree::Tree)
/// wires for its own rows — generic keyboard activation (Enter/Space) reads
/// any focused node's `data-rid`, with no `register_focus_target` needed —
/// and `role="button"` says what the reach is for.
fn settle_step_clickability(
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
        let handler_id = match step
            .get_attribute(CLICK_HANDLER_ID_ATTR)
            .and_then(|s| s.parse::<usize>().ok())
        {
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
                step.set_attribute(CLICK_HANDLER_ID_ATTR, &id.to_string());
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
fn settle_step_icon(
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
fn push_key(has: &mut Vec<String>, key: &str) {
    if !has.iter().any(|k| k == key) {
        has.push(key.to_owned());
    }
}

/// The built-in glyph for a content key: the tick for completed, the step number
/// for anything else.
fn built_in_glyph(scope: &mut RenderScope, key: &str, number: u32) -> NodeHandle {
    if key == KEY_COMPLETED {
        crate::icons::check_dom(scope)
    } else {
        rinch_core::IntoNode::into_node(number.to_string(), scope)
    }
}

/// The class a clickable step carries.
const CLICKABLE_CLASS: &str = "rinch-stepper__step--clickable";

/// The class a loading step carries.
const LOADING_CLASS: &str = "rinch-stepper__step--loading";

/// The class a [`StepperStep::disabled`] step carries.
const DISABLED_CLASS: &str = "rinch-stepper__step--disabled";

/// A step's derivation position, 0-based, set by [`settle_steps`] on every
/// pass regardless of clickability (issue #737).
///
/// Distinct from [`STEP_ATTR`], which a step's own `step` prop can override
/// for the *label* only — `allow_next_steps_select` and
/// [`Stepper::on_step_click`] both count positions, and a second notion of
/// index would disagree with them the same way the state derivation would.
/// A click handler reads this back at **click** time rather than closing
/// over the position it saw when it registered (the #714 pattern), because a
/// later insertion or removal can renumber the step after the handler is
/// wired (issues #716, #745).
const POSITION_ATTR: &str = "data-step-position";

/// Set by [`settle_steps`] on a step once it is fully derived: the
/// [`Derivation::settler`] of the stepper render that derived it (issue #748).
///
/// With [`POSITION_ATTR`] it is what lets a later pass tell a step it has
/// nothing to do for. The position alone cannot: a step moved in from another
/// stepper can land at the position it left, under different props.
const SETTLED_BY_ATTR: &str = "data-stepper-settled";

/// The plain HTML `disabled` boolean attribute, read tag-agnostically
/// (`node_is_disabled` honours it on any tag, not only the ones a browser
/// does) to mean exactly what it means on a `<button>`: never clickable,
/// never focusable, whatever else granted it the class.
const DISABLED_ATTR: &str = "disabled";

/// Set by [`StepperStep::render`] when the step asked for its own
/// clickability (`allow_step_click` or `allow_step_select`), so
/// [`settle_step_clickability`] can tell that ask apart from a class an
/// earlier pass of its own granted — a class already on the node cannot say
/// who put it there.
const OWN_CLICKABLE_ATTR: &str = "data-step-own-clickable";

/// Set by [`settle_step_clickability`] once a step's click handler is
/// registered, and never removed while the step lives: the registration
/// itself is paid once, and `data-rid` is set from this record — not
/// re-derived — whenever the step becomes reachable again after a position
/// shift made it briefly unreachable.
const CLICK_HANDLER_ID_ATTR: &str = "data-stepper-click-handler";

/// The three **content keys** a step's icon box deals in.
///
/// A key is not a state: `completed` and `progress` name the two glyphs a step
/// (or its [`Stepper`]) can supply for a particular state, and `base` names the
/// one glyph that serves every state neither of those covers — the step's own
/// `icon`, or failing that its number. Two states therefore share a key
/// whenever the step has no progress icon, which is the common case, and moving
/// such a step between them costs nothing but a renumber.
const KEY_COMPLETED: &str = "completed";
/// See [`KEY_COMPLETED`].
const KEY_PROGRESS: &str = "progress";
/// See [`KEY_COMPLETED`].
const KEY_BASE: &str = "base";

/// Set on a step's icon box: the content keys the box holds a **real** glyph
/// for, space separated.
///
/// Written by [`StepperStep`] from its own icon props. A key that is absent is
/// one where what is showing (or parked) is a built-in placeholder — the tick, or
/// the number — which a [`Stepper`]'s own default for that key may replace;
/// [`Stepper`] then adds the key, so the substitution happens once however many
/// times the pass re-runs (issue #716).
const ICON_HAS_ATTR: &str = "data-icon-has";

/// Set on a step's icon box: which content key the live glyph stands for.
///
/// The box's other children are hidden alternates, each labelled by
/// [`ICON_ALT_ATTR`]. A box with no `data-icon-live` was not built by
/// [`StepperStep`] and [`Stepper`] leaves its contents alone.
const ICON_LIVE_ATTR: &str = "data-icon-live";

/// Set by [`StepperStep`] on itself when the caller named a `state`.
///
/// Presence is the whole value, and it is the only thing that can carry it:
/// `state: String` renders an explicit `"inactive"` and an unset field into the
/// same class, so [`Stepper`] cannot tell a step that chose its state from one
/// waiting to be told. `Radio` records its `size` the same way, for the same
/// reason (issue #707).
const STATE_ATTR: &str = "data-state";

/// The step's index, set by [`StepperStep`] when the caller named one and by
/// [`Stepper`] from the step's position otherwise.
const STEP_ATTR: &str = "data-step";

/// Set by [`Stepper`] beside [`STEP_ATTR`] on a step it numbered itself.
///
/// Presence is the whole value, and the derivation cannot re-run without it:
/// once the first pass has numbered a step, `data-step` is present whoever wrote
/// it, so a later pass would read a caller's ask where there was none and leave
/// the step at a position it no longer occupies (issue #716).
const STEP_DERIVED_ATTR: &str = "data-step-derived";

/// The wrapper class of an alternate icon parked in a step's icon box.
const ICON_ALT_CLASS: &str = "rinch-stepper__step-icon-alt";

/// Which [content key](KEY_COMPLETED) an alternate icon is for.
const ICON_ALT_ATTR: &str = "data-icon-for";

/// One step of a stepper.
const STEP_CLASS: &str = "rinch-stepper__step";

/// A [`StepperCompleted`] block, which is **not a position** — see
/// [`collect_steps`] for why it is skipped rather than stopped at.
const COMPLETED_CLASS: &str = "rinch-stepper__completed";

/// Does `node` carry `class` as a whole class token?
fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

/// The classes [`collect_steps`] stops descending at, read back upwards by the
/// late-child observers so the two halves name one boundary.
///
/// [`COMPLETED_CLASS`] is here as a **cost** rule, not a correctness one, and a
/// mutant narrowing this back to `[STEP_CLASS]` therefore survives the suite: a
/// change inside a completed block cannot alter any step's position, because
/// `collect_steps` does not look in there, so the pass it declines would have
/// re-derived identical answers over an identical list. It is here so that
/// editing content inside a completed block does not re-walk the whole stepper.
const STEP_BOUNDARY: &[&str] = &[STEP_CLASS, COMPLETED_CLASS];

/// Every step in `node`'s subtree, in document order.
///
/// Steps are found by class rather than taken as `children` directly: a `for`
/// loop or a conditional puts wrapper nodes between the stepper and its steps.
/// The walk stops at each step, so a nested stepper inside a step's content is
/// not this one's to renumber.
///
/// It stops at a [`StepperCompleted`] too, and for the same reason (issue #741):
/// that block is what a stepper shows *instead of* its steps once they are all
/// done, so it is not a position and neither is anything inside it. The walk
/// used to descend into it, which re-derived a nested stepper's steps at the
/// outer stepper's indices.
///
/// **It is a skip, not a stop.** Mantine's `Stepper.Completed` is conventionally
/// written last but is not required to be, and Mantine never stops counting
/// steps at it. Reading #741 as "stop the walk" — which this did before review —
/// makes a stepper whose completed block is written *first* derive nothing at
/// all: no step numbered, every step inactive, and two steps both drawing the
/// number `1`. So the block is skipped and the walk goes on, and a step after it
/// is a position like any other.
fn collect_steps(node: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        // One read for both questions: this walk runs over every step on every
        // change to the list (issue #748).
        let class = node.get_attribute("class").unwrap_or_default();
        let carries = |wanted: &str| class.split_whitespace().any(|c| c == wanted);
        // Asked first, so a node somehow carrying both classes is not a
        // position — the narrower answer.
        if carries(COMPLETED_CLASS) {
            return;
        }
        if carries(STEP_CLASS) {
            out.push(node.clone());
            return;
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

/// A step's icon box, which is its first child.
fn step_icon_box(step: &NodeHandle) -> Option<NodeHandle> {
    step.children()
        .into_iter()
        .find(|child| has_class(child, "rinch-stepper__step-icon"))
}

/// Individual step in the stepper.
#[derive(Debug, Default)]
pub struct StepperStep {
    /// Step label text.
    pub label: String,
    /// Step description text.
    pub description: String,
    /// Custom icon for this step.
    pub icon: Option<TablerIcon>,
    /// Custom completed icon.
    pub completed_icon: Option<TablerIcon>,
    /// Custom progress icon.
    pub progress_icon: Option<TablerIcon>,
    /// Whether this step can be clicked.
    ///
    /// Grants the step the clickable class regardless of its position against
    /// the parent [`Stepper`]'s `active`. With [`Stepper::on_step_click`] set
    /// (issue #737), it also grants the wiring — a handler, `tabindex="0"` and
    /// `data-rid` — unless [`disabled`](StepperStep::disabled) is set, which
    /// always wins.
    pub allow_step_click: bool,
    /// Whether this step allows selecting next step.
    ///
    /// The same grant as [`allow_step_click`](StepperStep::allow_step_click);
    /// Mantine keeps the two names, so rinch does too, and neither means
    /// anything the other does not.
    pub allow_step_select: bool,
    /// Loading state.
    pub loading: bool,
    /// Never clickable, never focusable, whatever else grants it — the plain
    /// HTML `disabled` attribute (issue #737), read the way rinch's own focus
    /// arbiter reads it on any tag, not only the ones a browser does.
    pub disabled: bool,
    /// Step state — `"completed"`, `"progress"`, or anything else for inactive.
    ///
    /// Leave it unset and the parent [`Stepper`] derives it from this step's
    /// position against its `active` (issue #709), which is what a stepper
    /// normally wants. Set it to override that for this one step: what a step
    /// names, it keeps.
    pub state: String,
    /// Step index, 0-based.
    ///
    /// Set by the parent [`Stepper`] from this step's position unless the caller
    /// names one, in which case that is the index shown (as `data-step`, and as
    /// the number in the icon box, which is one higher). An index named here
    /// does **not** move the step: its state still derives from where it sits.
    pub step: Option<u32>,
}

impl Component for StepperStep {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // State classes
        let state = StepState::parse(&self.state);
        let state_class = state.class_name();

        let loading_class = if self.loading { LOADING_CLASS } else { "" };

        let own_clickable = !self.disabled && (self.allow_step_click || self.allow_step_select);
        let clickable = if own_clickable { CLICKABLE_CLASS } else { "" };
        let disabled_class = if self.disabled { DISABLED_CLASS } else { "" };

        let class =
            format!("{STEP_CLASS} {state_class} {loading_class} {clickable} {disabled_class}");

        let step_el = rinch_macros::rsx! { div { class: "rinch-stepper__step" } };
        step_el.set_attribute("class", &class);

        // What the caller asked for, as distinct from what it rendered as. Only
        // presence carries this: a `Stepper` cannot otherwise tell a step that
        // chose to be inactive from one that simply said nothing (issue #709).
        if !self.state.is_empty() {
            step_el.set_attribute(STATE_ATTR, &self.state);
        }
        if let Some(step) = self.step {
            step_el.set_attribute(STEP_ATTR, &step.to_string());
        }

        // Recorded separately from the class above (issue #737):
        // `settle_step_clickability` has to tell "this step asked for its own
        // clickability" apart from "an earlier pass of mine already granted
        // the class", which the class alone cannot say.
        if own_clickable {
            step_el.set_attribute(OWN_CLICKABLE_ATTR, "");
        }
        // The plain HTML boolean attribute, read tag-agnostically by rinch's
        // own focus arbiter and by `settle_step_clickability` alike — never
        // clickable, never focusable, whatever else grants it.
        if self.disabled {
            step_el.set_attribute(DISABLED_ATTR, "");
        }

        // Step number/icon
        let step_number = self.step.map(|n| n + 1).unwrap_or(1);
        let icon_container = rinch_macros::rsx! { div { class: "rinch-stepper__step-icon" } };

        // Which content keys this step supplies a glyph of its own for. Every
        // other key the box ever shows is a built-in placeholder, which a parent
        // `Stepper`'s own default may replace (issue #707). This is a record of
        // the step's *props*, not of what it happens to be drawing, which is what
        // lets the parent's pass re-run over it (issue #716).
        let mut has: Vec<&str> = Vec::new();
        if !self.loading {
            if self.completed_icon.is_some() {
                has.push(KEY_COMPLETED);
            }
            if self.progress_icon.is_some() {
                has.push(KEY_PROGRESS);
            }
            if self.icon.is_some() {
                has.push(KEY_BASE);
            }
        }

        // The key this step draws live. `base` covers every state the two
        // specific keys do not, so a step with no progress icon draws its base
        // glyph while in progress.
        let live_key = match state {
            _ if self.loading => None,
            StepState::Completed => Some(KEY_COMPLETED),
            StepState::Progress if self.progress_icon.is_some() => Some(KEY_PROGRESS),
            _ => Some(KEY_BASE),
        };

        let icon_content = match live_key {
            None => rinch_macros::rsx! { span { class: "rinch-stepper__loader" } },
            Some(KEY_COMPLETED) => match self.completed_icon {
                Some(custom_icon) => {
                    render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
                }
                None => crate::icons::check_dom(__scope),
            },
            Some(KEY_PROGRESS) => {
                let icon = self
                    .progress_icon
                    .expect("KEY_PROGRESS implies a progress icon");
                render_tabler_icon(__scope, icon, TablerIconStyle::Outline)
            }
            Some(_) => match self.icon {
                Some(custom_icon) => {
                    render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
                }
                None => rinch_core::IntoNode::into_node(step_number.to_string(), __scope),
            },
        };

        icon_container.append_child(&icon_content);
        if let Some(live_key) = live_key {
            icon_container.set_attribute(ICON_LIVE_ATTR, live_key);
        }
        if !has.is_empty() {
            icon_container.set_attribute(ICON_HAS_ATTR, &has.join(" "));
        }

        // A parent `Stepper` may put this step in a state it did not render, and
        // the icon for that state is a `TablerIcon` in this step's props — not
        // something a patch of the rendered tree could recover. So render it now
        // and leave it in the box, hidden, for the parent to promote; the parent
        // parks the live glyph in its place and drops only the alternates no
        // later move could want (issues #709, #716).
        //
        // Only where a parent could act on one: a step that named its own state
        // is already in the state it will stay in, and a loading step draws a
        // spinner whatever state it ends up in. `display: none` inline rather
        // than from the component stylesheet, because a step rendered outside a
        // `Stepper` has nobody to take these away and must not show them even if
        // the sheet never loads.
        if self.state.is_empty() && !self.loading {
            for (key, icon) in [
                (KEY_COMPLETED, self.completed_icon),
                (KEY_PROGRESS, self.progress_icon),
            ] {
                let Some(icon) = icon else { continue };
                let alt = rinch_macros::rsx! { span { class: "rinch-stepper__step-icon-alt" } };
                alt.set_attribute(ICON_ALT_ATTR, key);
                alt.set_style("display", "none");
                alt.append_child(&render_tabler_icon(__scope, icon, TablerIconStyle::Outline));
                icon_container.append_child(&alt);
            }
        }

        step_el.append_child(&icon_container);

        // Body (label + description)
        let body = rinch_macros::rsx! { div { class: "rinch-stepper__step-body" } };

        if !self.label.is_empty() {
            let label_span = rinch_macros::rsx! { span { class: "rinch-stepper__step-label", {self.label.clone()} } };
            body.append_child(&label_span);
        }

        if !self.description.is_empty() {
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-stepper__step-description" } };
            let desc_text = rinch_core::IntoNode::into_node(self.description.clone(), __scope);
            desc_span.append_child(&desc_text);
            body.append_child(&desc_span);
        }

        step_el.append_child(&body);

        // Content
        let content_el = rinch_macros::rsx! { div { class: "rinch-stepper__step-content" } };

        for child in children {
            content_el.append_child(child);
        }

        step_el.append_child(&content_el);

        // Separator
        let separator = rinch_macros::rsx! { div { class: "rinch-stepper__separator" } };
        step_el.append_child(&separator);

        step_el
    }
}

/// Content shown for the completed state.
///
/// **Not a position** (issue #741): the parent [`Stepper`]'s derivation skips
/// this block, so a [`StepperStep`] placed inside it keeps the index and state
/// it rendered itself with, and a nested `Stepper` keeps its own. The skip does
/// not stop the count — a step placed *after* this block is numbered like any
/// other, so writing it first, last or between the steps all work.
#[derive(Debug, Default)]
pub struct StepperCompleted;

impl Component for StepperCompleted {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-stepper__completed" } };

        for child in children {
            container.append_child(child);
        }

        container
    }
}

#[cfg(test)]
mod differential_tests {
    //! The skipping pass against the pass that derives every step (issue #748).
    //!
    //! Two documents get the same history — two steppers each, then a seeded
    //! sequence of insertions, removals, reorders and moves between the two
    //! steppers — one with [`DERIVE_EVERY_STEP`] set for every change. After
    //! each change the two trees must be the same tree: every attribute this
    //! file writes, every text node, every child, in order.

    use super::*;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::dom::traits::DomDocument;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Every attribute a stepper, a step or an icon box can carry.
    const ATTRS: &[&str] = &[
        "class",
        "style",
        "tabindex",
        "role",
        "data-rid",
        "data-active",
        "data-icon-for",
        DISABLED_ATTR,
        POSITION_ATTR,
        OWN_CLICKABLE_ATTR,
        CLICK_HANDLER_ID_ATTR,
        ICON_HAS_ATTR,
        ICON_LIVE_ATTR,
        STATE_ATTR,
        STEP_ATTR,
        STEP_DERIVED_ATTR,
        SETTLED_BY_ATTR,
    ];

    /// Attributes whose value is an id minted from a thread-wide counter, which
    /// the two documents therefore draw different numbers from. Compared by
    /// order of first appearance instead.
    const MINTED: &[&str] = &["data-rid", CLICK_HANDLER_ID_ATTR, SETTLED_BY_ATTR];

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn one_in(&mut self, n: usize) -> bool {
            self.below(n) == 0
        }
    }

    struct World {
        _doc: Rc<RefCell<MockDomDocument>>,
        scope: RenderScope,
        root: NodeHandle,
        containers: Vec<NodeHandle>,
        every_step: bool,
    }

    impl World {
        fn new(every_step: bool, rng: &mut Rng) -> Self {
            let doc = Rc::new(RefCell::new(MockDomDocument::new()));
            let body = doc.borrow().body();
            let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
            let root = scope.create_element("div");
            let mut world = Self {
                _doc: doc,
                scope,
                root,
                containers: Vec::new(),
                every_step,
            };
            world.with_mode(|world| {
                for stepper in [
                    Stepper {
                        active: 2,
                        on_step_click: Some(ValueCallback::new(|_: u32| {})),
                        completed_icon: Some(TablerIcon::CircleCheck),
                        ..Default::default()
                    },
                    Stepper {
                        active: 4,
                        allow_next_steps_select: rng.one_in(2),
                        progress_icon: Some(TablerIcon::Bell),
                        ..Default::default()
                    },
                ] {
                    let mut children: Vec<NodeHandle> = (0..3).map(|_| world.step(rng)).collect();
                    // A wrapper between the stepper and some of its steps, as a
                    // `for` loop puts one, and a completed block mid-list.
                    let wrapper = world.scope.create_element("div");
                    for _ in 0..2 {
                        let step = world.step(rng);
                        wrapper.append_child(&step);
                    }
                    children.insert(1, wrapper);
                    children.insert(3, StepperCompleted.render(&mut world.scope, &[]));
                    let rendered = stepper.render(&mut world.scope, &children);
                    let container = rendered
                        .children()
                        .into_iter()
                        .find(|c| has_class(c, "rinch-stepper__steps"))
                        .expect("steps row");
                    world.root.append_child(&rendered);
                    world.containers.push(container);
                }
            });
            world
        }

        fn with_mode<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
            DERIVE_EVERY_STEP.with(|flag| flag.set(self.every_step));
            let out = run(self);
            DERIVE_EVERY_STEP.with(|flag| flag.set(false));
            out
        }

        fn step(&mut self, rng: &mut Rng) -> NodeHandle {
            let icon = |rng: &mut Rng, icon| rng.one_in(3).then_some(icon);
            StepperStep {
                icon: icon(rng, TablerIcon::Home),
                completed_icon: icon(rng, TablerIcon::Check),
                progress_icon: icon(rng, TablerIcon::Clock),
                allow_step_click: rng.one_in(5),
                loading: rng.one_in(9),
                disabled: rng.one_in(7),
                state: match rng.below(8) {
                    0 => "completed".into(),
                    1 => "progress".into(),
                    2 => "inactive".into(),
                    _ => String::new(),
                },
                step: rng.one_in(6).then(|| rng.below(20) as u32),
                ..Default::default()
            }
            .render(&mut self.scope, &[])
        }

        /// Every node a step can be put into: the two steps rows and the
        /// wrappers inside them.
        fn parents(&self) -> Vec<NodeHandle> {
            let mut out = Vec::new();
            for container in &self.containers {
                out.push(container.clone());
                for child in container.children() {
                    if !has_class(&child, STEP_CLASS) && !has_class(&child, COMPLETED_CLASS) {
                        out.push(child);
                    }
                }
            }
            out
        }

        fn steps(&self) -> Vec<NodeHandle> {
            self.containers.iter().flat_map(collect_steps).collect()
        }

        /// Put `step` into parent `p` in front of that parent's child `at`, or
        /// at its end.
        fn place(&self, step: &NodeHandle, p: usize, at: usize) {
            let parents = self.parents();
            let parent = &parents[p % parents.len()];
            let siblings: Vec<NodeHandle> = parent
                .children()
                .into_iter()
                .filter(|c| c.node_id() != step.node_id())
                .collect();
            match siblings.get(at % (siblings.len() + 1)) {
                Some(reference) => parent.insert_before(step, reference),
                None => parent.append_child(step),
            }
        }

        fn apply(&mut self, op: Op, rng: &mut Rng) {
            self.with_mode(|world| match op {
                Op::Insert { p, at } => {
                    let step = world.step(rng);
                    world.place(&step, p, at);
                }
                Op::Remove { which } => {
                    let steps = world.steps();
                    if !steps.is_empty() {
                        steps[which % steps.len()].remove();
                    }
                }
                Op::Move { which, p, at } => {
                    let steps = world.steps();
                    if !steps.is_empty() {
                        world.place(&steps[which % steps.len()], p, at);
                    }
                }
                Op::Replace { which } => {
                    // What a `for` reconcile does to a changed row.
                    let steps = world.steps();
                    if !steps.is_empty() {
                        let old = &steps[which % steps.len()];
                        let new = world.step(rng);
                        old.insert_after(&new);
                        old.discard();
                    }
                }
            });
        }

        fn dump(&self) -> String {
            let mut out = String::new();
            let mut minted = HashMap::new();
            dump(&self.root, 0, &mut minted, &mut out);
            out
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Op {
        Insert { p: usize, at: usize },
        Remove { which: usize },
        Move { which: usize, p: usize, at: usize },
        Replace { which: usize },
    }

    fn dump(
        node: &NodeHandle,
        depth: usize,
        minted: &mut HashMap<(&'static str, String), usize>,
        out: &mut String,
    ) {
        out.push_str(&"  ".repeat(depth));
        match node.tag_name() {
            Some(tag) => {
                out.push_str(&tag);
                for name in ATTRS {
                    let Some(value) = node.get_attribute(name) else {
                        continue;
                    };
                    let value = if MINTED.contains(name) {
                        let next = minted.len();
                        format!("#{}", minted.entry((name, value)).or_insert(next))
                    } else {
                        value
                    };
                    out.push_str(&format!(" {name}={value:?}"));
                }
                out.push('\n');
                for child in node.children() {
                    dump(&child, depth + 1, minted, out);
                }
            }
            None => {
                out.push_str(&format!("{:?}\n", node.text_content()));
            }
        }
    }

    fn run(seed: u64, ops: usize) {
        // One generator per world, seeded alike, so the steps the two build are
        // the same steps; a third draws the history.
        let mut rng_a = Rng(seed);
        let mut rng_b = Rng(seed);
        let mut history = Rng(seed ^ 0x9e3779b97f4a7c15);
        let mut skipping = World::new(false, &mut rng_a);
        let mut reference = World::new(true, &mut rng_b);
        assert_eq!(skipping.dump(), reference.dump(), "seed {seed}: at render");

        for n in 0..ops {
            let op = match history.below(10) {
                0..=3 => Op::Insert {
                    p: history.below(64),
                    at: history.below(64),
                },
                4..=5 => Op::Remove {
                    which: history.below(64),
                },
                6..=8 => Op::Move {
                    which: history.below(64),
                    p: history.below(64),
                    at: history.below(64),
                },
                _ => Op::Replace {
                    which: history.below(64),
                },
            };
            skipping.apply(op, &mut rng_a);
            reference.apply(op, &mut rng_b);
            let (got, want) = (skipping.dump(), reference.dump());
            assert!(
                got == want,
                "seed {seed}, change {n} ({op:?}): the skipping pass left a \
                 different tree from the pass that derives every step.\n\
                 --- skipping ---\n{got}\n--- every step ---\n{want}"
            );
        }
    }

    #[test]
    fn the_skipping_pass_leaves_the_tree_the_whole_pass_leaves() {
        let seeds: u64 = std::env::var("RINCH_TWIN_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        for seed in 1..=seeds {
            run(seed, 60);
        }
    }

    #[test]
    fn the_reference_pass_really_derives_every_step() {
        // The oracle's own control: with the flag set, an append to a settled
        // stepper reads far more than three attributes per step it leaves alone.
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
        let steps: Vec<NodeHandle> = (0..20)
            .map(|_| StepperStep::default().render(&mut scope, &[]))
            .collect();
        let rendered = Stepper {
            active: 9,
            ..Default::default()
        }
        .render(&mut scope, &steps);
        let container = rendered.children().into_iter().next().expect("steps row");

        let mut reads = Vec::new();
        for every_step in [false, true] {
            let step = StepperStep::default().render(&mut scope, &[]);
            DERIVE_EVERY_STEP.with(|flag| flag.set(every_step));
            let before = doc.borrow().__op_counts();
            container.append_child(&step);
            reads.push(doc.borrow().__op_counts().since(before).attribute_reads);
            DERIVE_EVERY_STEP.with(|flag| flag.set(false));
        }
        assert!(
            reads[1] > reads[0] + 20 * 10,
            "skipping {} reads, every step {} reads",
            reads[0],
            reads[1]
        );
    }
}

#[cfg(test)]
mod extended_differential_tests {
    //! The skipping pass against the pass that derives every step, over the
    //! shapes [`differential_tests`] does not sample (from the review of PR
    //! #1423): a stepper with every prop set at once, a nested stepper whose
    //! steps move to and from the outer ones, a step removed and put back
    //! later, a wrapper or completed block moved with its steps, and the same
    //! step nodes handed to a new render with different props.
    use super::*;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::dom::traits::DomDocument;
    use std::cell::RefCell;

    const ATTRS: &[&str] = &[
        "class",
        "style",
        "tabindex",
        "role",
        "data-rid",
        "data-active",
        "data-icon-for",
        DISABLED_ATTR,
        POSITION_ATTR,
        OWN_CLICKABLE_ATTR,
        CLICK_HANDLER_ID_ATTR,
        ICON_HAS_ATTR,
        ICON_LIVE_ATTR,
        ICON_ALT_ATTR,
        STATE_ATTR,
        STEP_ATTR,
        STEP_DERIVED_ATTR,
    ];
    const MINTED: &[&str] = &["data-rid", CLICK_HANDLER_ID_ATTR];

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn one_in(&mut self, n: usize) -> bool {
            self.below(n) == 0
        }
    }

    struct Props {
        active: u32,
        next: bool,
        click: bool,
        ci: bool,
        pi: bool,
    }
    impl Props {
        fn stepper(&self) -> Stepper {
            Stepper {
                active: self.active,
                allow_next_steps_select: self.next,
                on_step_click: self.click.then(|| ValueCallback::new(|_: u32| {})),
                completed_icon: self.ci.then_some(TablerIcon::CircleCheck),
                progress_icon: self.pi.then_some(TablerIcon::Bell),
                ..Default::default()
            }
        }
    }

    struct World {
        _doc: Rc<RefCell<MockDomDocument>>,
        scope: RenderScope,
        root: NodeHandle,
        steppers: Vec<(NodeHandle, NodeHandle, Props)>, // rendered root, container, props
        stash: Vec<NodeHandle>,
        every_step: bool,
    }

    fn container_of(rendered: &NodeHandle) -> NodeHandle {
        rendered
            .children()
            .into_iter()
            .find(|c| has_class(c, "rinch-stepper__steps"))
            .expect("steps row")
    }

    impl World {
        fn new(every_step: bool, rng: &mut Rng) -> Self {
            let doc = Rc::new(RefCell::new(MockDomDocument::new()));
            let body = doc.borrow().body();
            let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
            let root = scope.create_element("div");
            let mut world = Self {
                _doc: doc,
                scope,
                root,
                steppers: Vec::new(),
                stash: Vec::new(),
                every_step,
            };
            world.with_mode(|world| {
                // 0: everything on. 1: nothing. 2: nested in a step of 0. 3: bare under 1's row (double claim).
                let nested_props = Props {
                    active: 1,
                    next: rng.one_in(2),
                    click: true,
                    ci: false,
                    pi: true,
                };
                let nested_steps: Vec<NodeHandle> = (0..3).map(|_| world.step(rng, &[])).collect();
                let nested = nested_props
                    .stepper()
                    .render(&mut world.scope, &nested_steps);
                let nested_c = container_of(&nested);
                for (i, props) in [
                    Props {
                        active: 2,
                        next: true,
                        click: true,
                        ci: true,
                        pi: true,
                    },
                    Props {
                        active: 3,
                        next: rng.one_in(2),
                        click: rng.one_in(2),
                        ci: rng.one_in(2),
                        pi: rng.one_in(2),
                    },
                ]
                .into_iter()
                .enumerate()
                {
                    let mut children: Vec<NodeHandle> =
                        (0..3).map(|_| world.step(rng, &[])).collect();
                    if i == 0 {
                        let host = world.step(rng, std::slice::from_ref(&nested));
                        children.push(host);
                    }
                    let wrapper = world.scope.create_element("div");
                    for _ in 0..2 {
                        let st = world.step(rng, &[]);
                        wrapper.append_child(&st);
                    }
                    children.insert(1, wrapper);
                    children.insert(3, StepperCompleted.render(&mut world.scope, &[]));
                    let rendered = props.stepper().render(&mut world.scope, &children);
                    let c = container_of(&rendered);
                    world.root.append_child(&rendered);
                    world.steppers.push((rendered, c, props));
                }
                world.steppers.push((nested, nested_c, nested_props));
            });
            world
        }

        fn with_mode<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
            DERIVE_EVERY_STEP.with(|flag| flag.set(self.every_step));
            let out = run(self);
            DERIVE_EVERY_STEP.with(|flag| flag.set(false));
            out
        }

        fn step(&mut self, rng: &mut Rng, children: &[NodeHandle]) -> NodeHandle {
            let icon = |rng: &mut Rng, icon| rng.one_in(3).then_some(icon);
            StepperStep {
                icon: icon(rng, TablerIcon::Home),
                completed_icon: icon(rng, TablerIcon::Check),
                progress_icon: icon(rng, TablerIcon::Clock),
                allow_step_click: rng.one_in(5),
                loading: rng.one_in(9),
                disabled: rng.one_in(7),
                state: match rng.below(8) {
                    0 => "completed".into(),
                    1 => "progress".into(),
                    2 => "inactive".into(),
                    _ => String::new(),
                },
                step: rng.one_in(6).then(|| rng.below(20) as u32),
                ..Default::default()
            }
            .render(&mut self.scope, children)
        }

        /// Rows, wrappers and completed blocks.
        fn parents(&self) -> Vec<NodeHandle> {
            let mut out = Vec::new();
            for (_, container, _) in &self.steppers {
                out.push(container.clone());
                for child in container.children() {
                    if !has_class(&child, STEP_CLASS) {
                        out.push(child);
                    }
                }
            }
            out
        }

        /// Every step node anywhere under the root, completed blocks included.
        fn steps(&self) -> Vec<NodeHandle> {
            fn walk(n: &NodeHandle, out: &mut Vec<NodeHandle>) {
                if has_class(n, STEP_CLASS) {
                    out.push(n.clone());
                }
                for c in n.children() {
                    walk(&c, out);
                }
            }
            let mut out = Vec::new();
            walk(&self.root, &mut out);
            out
        }

        fn is_under(node: &NodeHandle, anc: &NodeHandle) -> bool {
            let mut cur = Some(node.clone());
            while let Some(n) = cur {
                if n.node_id() == anc.node_id() {
                    return true;
                }
                cur = n.parent_node();
            }
            false
        }

        fn place(&self, node: &NodeHandle, p: usize, at: usize) -> bool {
            let parents = self.parents();
            let parent = &parents[p % parents.len()];
            if Self::is_under(parent, node) {
                return false;
            }
            let siblings: Vec<NodeHandle> = parent
                .children()
                .into_iter()
                .filter(|c| c.node_id() != node.node_id())
                .collect();
            match siblings.get(at % (siblings.len() + 1)) {
                Some(reference) => parent.insert_before(node, reference),
                None => parent.append_child(node),
            }
            true
        }

        fn apply(&mut self, op: Op, rng: &mut Rng) {
            self.with_mode(|world| match op {
                Op::Insert { p, at } => {
                    let step = world.step(rng, &[]);
                    world.place(&step, p, at);
                }
                Op::Remove { which } => {
                    let steps = world.steps();
                    if steps.len() > 1 {
                        steps[which % steps.len()].remove();
                    }
                }
                Op::Move { which, p, at } => {
                    let steps = world.steps();
                    if !steps.is_empty() {
                        world.place(&steps[which % steps.len()], p, at);
                    }
                }
                Op::Stash { which } => {
                    let steps = world.steps();
                    if steps.len() > 1 {
                        let s = steps[which % steps.len()].clone();
                        s.remove();
                        world.stash.push(s);
                    }
                }
                Op::Unstash { p, at } => {
                    if let Some(s) = world.stash.pop() {
                        world.place(&s, p, at);
                    }
                }
                Op::MoveGroup { which, p, at } => {
                    // a wrapper or a completed block, with whatever it holds
                    let groups: Vec<NodeHandle> = world
                        .parents()
                        .into_iter()
                        .filter(|n| !has_class(n, "rinch-stepper__steps"))
                        .collect();
                    if !groups.is_empty() {
                        let g = groups[which % groups.len()].clone();
                        world.place(&g, p, at);
                    }
                }
                Op::Rerender { which, active } => {
                    // The same step nodes handed to a new render with other props.
                    let idx = which % 2;
                    let (old_root, old_c, _) = &world.steppers[idx];
                    let (old_root, old_c) = (old_root.clone(), old_c.clone());
                    let children = old_c.children();
                    world.steppers[idx].2.active = active as u32;
                    if active % 2 == 0 {
                        world.steppers[idx].2.next = !world.steppers[idx].2.next;
                    }
                    let stepper = world.steppers[idx].2.stepper();
                    let rendered = stepper.render(&mut world.scope, &children);
                    let c = container_of(&rendered);
                    old_root.insert_after(&rendered);
                    old_root.discard();
                    world.steppers[idx].0 = rendered;
                    world.steppers[idx].1 = c;
                }
            });
        }

        fn dump(&self) -> String {
            let mut out = String::new();
            let mut minted = HashMap::new();
            dump(&self.root, 0, &mut minted, &mut out);
            for s in &self.stash {
                dump(s, 1, &mut minted, &mut out);
            }
            out
        }
    }
    use std::collections::HashMap;

    #[derive(Clone, Copy, Debug)]
    enum Op {
        Insert { p: usize, at: usize },
        Remove { which: usize },
        Move { which: usize, p: usize, at: usize },
        Stash { which: usize },
        Unstash { p: usize, at: usize },
        MoveGroup { which: usize, p: usize, at: usize },
        Rerender { which: usize, active: usize },
    }

    fn dump(
        node: &NodeHandle,
        depth: usize,
        minted: &mut HashMap<(&'static str, String), usize>,
        out: &mut String,
    ) {
        out.push_str(&"  ".repeat(depth));
        match node.tag_name() {
            Some(tag) => {
                out.push_str(&tag);
                for name in ATTRS {
                    let Some(value) = node.get_attribute(name) else {
                        continue;
                    };
                    let value = if MINTED.contains(name) {
                        let next = minted.len();
                        format!("#{}", minted.entry((name, value)).or_insert(next))
                    } else {
                        value
                    };
                    out.push_str(&format!(" {name}={value:?}"));
                }
                out.push('\n');
                for child in node.children() {
                    dump(&child, depth + 1, minted, out);
                }
            }
            None => out.push_str(&format!("{:?}\n", node.text_content())),
        }
    }

    fn run(seed: u64, ops: usize) -> usize {
        let mut rng_a = Rng(seed);
        let mut rng_b = Rng(seed);
        let mut history = Rng(seed ^ 0x9e3779b97f4a7c15);
        let mut skipping = World::new(false, &mut rng_a);
        let mut reference = World::new(true, &mut rng_b);
        assert_eq!(skipping.dump(), reference.dump(), "seed {seed}: at render");
        let mut applied = 0;
        for n in 0..ops {
            let op = match history.below(16) {
                0..=3 => Op::Insert {
                    p: history.below(64),
                    at: history.below(64),
                },
                4..=5 => Op::Remove {
                    which: history.below(64),
                },
                6..=8 => Op::Move {
                    which: history.below(64),
                    p: history.below(64),
                    at: history.below(64),
                },
                9..=10 => Op::Stash {
                    which: history.below(64),
                },
                11..=12 => Op::Unstash {
                    p: history.below(64),
                    at: history.below(64),
                },
                13..=14 => Op::MoveGroup {
                    which: history.below(64),
                    p: history.below(64),
                    at: history.below(64),
                },
                _ => Op::Rerender {
                    which: history.below(64),
                    active: history.below(7),
                },
            };
            skipping.apply(op, &mut rng_a);
            reference.apply(op, &mut rng_b);
            applied += 1;
            let (got, want) = (skipping.dump(), reference.dump());
            assert!(
                got == want,
                "seed {seed}, change {n} ({op:?}):\n--- skipping ---\n{got}\n--- every step ---\n{want}"
            );
        }
        applied
    }

    #[test]
    fn the_skipping_pass_matches_the_whole_pass_over_the_shapes_the_review_added() {
        let seeds: u64 = std::env::var("RINCH_REVIEW_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40);
        let mut total = 0;
        for seed in 1..=seeds {
            total += run(seed, 80);
        }
        assert!(total > 0);
        eprintln!("extended differential: {seeds} seeds, {total} changes");
    }
}
