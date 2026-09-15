//! Stepper component.
//!
//! Step-by-step progress indicator.

use std::cmp::Ordering;

use rinch_core::Component;
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
    /// #716). A step *removed* does not renumber its siblings (issue #745).
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
    /// Clickable is currently **decorative**: the class carries a cursor and a
    /// hover state, and this component registers no click handler and takes no
    /// callback (issue #737), as `allow_step_click` and `allow_step_select`
    /// already did.
    pub allow_next_steps_select: bool,
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
        };
        settle_steps(__scope, &steps_container, derivation);

        // A step that arrives *after* this render — a `for` reconcile, a
        // `show_dom` branch, a hand-rolled `append_child` — needs the same pass,
        // and so do the steps it displaces: an insertion in front of a step
        // moves it, which changes its number and can change its state (issue
        // #716). So the whole pass runs again, over every step, and it is
        // written to be idempotent for exactly that reason.
        let watched = steps_container.clone();
        crate::late_children::adopt_late_children(
            __scope,
            &steps_container,
            "rinch-stepper__step",
            move |_inserted, scope| {
                // The subtree that landed is deliberately ignored. What has to
                // be recomputed is every step's *position*, and only the whole
                // list gives that — an insertion in front of a step renumbers
                // it and can restate it, so patching the newcomer alone would
                // leave the stepper saying two different things.
                settle_steps(scope, &watched, derivation);
            },
        );

        container.append_child(&steps_container);

        container
    }
}

/// Everything a [`Stepper`] tells its steps that a step cannot know about
/// itself.
///
/// `Copy` and prop-shaped rather than a borrow of the component, because the
/// late-arrival observer outlives the `render` call that installed it (issue
/// #716).
#[derive(Debug, Clone, Copy)]
struct Derivation {
    active: u32,
    allow_next_steps_select: bool,
    completed_icon: Option<TablerIcon>,
    progress_icon: Option<TablerIcon>,
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
/// **Idempotent, and that is load-bearing** (issue #716). It runs once at the
/// stepper's own render and again every time a step lands beneath the container
/// afterwards, because an insertion renumbers the steps after it and can move
/// them between states. Everything it reads is therefore a *stable* record of
/// what the caller asked for — `data-state` for a named state,
/// [`STEP_DERIVED_ATTR`] for an index this pass assigned, [`ICON_HAS_ATTR`] for
/// the glyphs the box holds — never a record of what the last pass happened to
/// draw.
fn settle_steps(scope: &mut RenderScope, steps_container: &NodeHandle, d: Derivation) {
    for (position, step) in collect_steps(steps_container).iter().enumerate() {
        let position = position as u32;

        if d.allow_next_steps_select && position > d.active {
            // A step that asked to be clickable itself already carries the
            // class. `add_class` is idempotent since #717, so the guard is
            // belt and braces rather than load-bearing.
            if !has_class(step, CLICKABLE_CLASS) {
                step.add_class(CLICKABLE_CLASS);
            }
        }

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
            step.set_attribute(STEP_ATTR, &position.to_string());
            step.set_attribute(STEP_DERIVED_ATTR, "");
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
                // freshly rendered by *this* stepper: it descends into a
                // `StepperCompleted`, so a nested `Stepper` there has its steps
                // re-derived at the outer stepper's positions, already carrying
                // whatever their own stepper gave them. Measured: one such step
                // ended up with `--progress` and `--inactive` at once. It is
                // also what a *re-run* needs, since the class this pass is
                // replacing is the one the last pass wrote. `remove_class`
                // writes nothing when the class is absent (#730), so the two
                // extra calls are free.
                for state in [
                    StepState::Completed,
                    StepState::Progress,
                    StepState::Inactive,
                ] {
                    step.remove_class(state.class_name());
                }
                step.add_class(derived.class_name());
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
/// nowhere else to get its own glyph from. Only the alternates a *forward* move
/// could still need are kept — see [`reachable_keys`].
fn settle_step_icon(
    scope: &mut RenderScope,
    step: &NodeHandle,
    state: StepState,
    named_its_own_state: bool,
    number: u32,
    d: Derivation,
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
            if has.iter().any(|k| *k == live_key) {
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
                    has.push(target.to_owned());
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
        has.push(target.to_owned());
        live = Some(glyph);
    }

    // The number, which the step could only have drawn as its own index plus
    // one — and which an insertion in front of it changes.
    if live_key == KEY_BASE
        && !has.iter().any(|k| k == KEY_BASE)
        && let Some(live) = &live
    {
        live.set_text(&number.to_string());
    }

    // Alternates a forward move could still want are kept; the rest are markup
    // nobody will look at again. A step that named its own state is in the state
    // it will stay in, so for it that is *every* alternate.
    let keep: &[&str] = if named_its_own_state {
        &[]
    } else {
        reachable_keys(state)
    };
    for (key, wrapper) in alternates {
        if !keep.contains(&key.as_str()) {
            wrapper.discard();
        }
    }

    if has.is_empty() {
        icon_box.remove_attribute(ICON_HAS_ATTR);
    } else {
        icon_box.set_attribute(ICON_HAS_ATTR, &has.join(" "));
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

/// The content keys a step in `state` could still be moved into.
///
/// **A step's position only ever grows**, because this pass is re-run on an
/// *insertion* and nothing else: a step gains siblings in front of it, never
/// loses them. So state only moves forward — completed → progress → inactive —
/// and an alternate for a state already behind the step is dead. A removal that
/// shifts a step backwards would not re-run this pass at all (issue #745); if it
/// ever does, every key has to be kept.
fn reachable_keys(state: StepState) -> &'static [&'static str] {
    match state {
        StepState::Completed => &[KEY_COMPLETED, KEY_PROGRESS, KEY_BASE],
        StepState::Progress => &[KEY_PROGRESS, KEY_BASE],
        StepState::Inactive => &[KEY_BASE],
    }
}

/// The class a clickable step carries.
const CLICKABLE_CLASS: &str = "rinch-stepper__step--clickable";

/// The class a loading step carries.
const LOADING_CLASS: &str = "rinch-stepper__step--loading";

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

/// Does `node` carry `class` as a whole class token?
fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

/// Every step in `node`'s subtree, in document order.
///
/// Steps are found by class rather than taken as `children` directly: a `for`
/// loop or a conditional puts wrapper nodes between the stepper and its steps.
/// The walk stops at each step, so a nested stepper inside a step's content is
/// not this one's to renumber.
///
/// It descends into every *other* child, though — a [`StepperCompleted`]
/// included — so a `Stepper` placed there does have its steps collected at this
/// stepper's positions. That reach predates the derivation (it granted only the
/// clickable class); the state swap above is written to survive it rather than
/// to assume it away.
fn collect_steps(node: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        if has_class(node, "rinch-stepper__step") {
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
    pub allow_step_click: bool,
    /// Whether this step allows selecting next step.
    pub allow_step_select: bool,
    /// Loading state.
    pub loading: bool,
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

        let clickable = if self.allow_step_click || self.allow_step_select {
            CLICKABLE_CLASS
        } else {
            ""
        };

        let class = format!("rinch-stepper__step {state_class} {loading_class} {clickable}");

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
