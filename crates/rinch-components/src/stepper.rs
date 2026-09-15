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
    /// Each step **present at this stepper's own render** takes its state from
    /// its position against this: before it completed, at it in progress, after
    /// it inactive. A step that named a `state` of its own keeps it, and a step
    /// appended later keeps whatever it rendered as (issue #716).
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
    /// When true, each step past [`Stepper::active`] **present at this
    /// stepper's own render** that did not ask to be clickable itself is made
    /// clickable; a step appended later is not (issue #716). A step that set
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
    /// Used by the [`StepperStep`]s in the completed state **present at this
    /// stepper's own render** that set no `completed_icon` of their own — the
    /// step's icon wins, whether the step named its own state or this stepper
    /// derived it. Steps are patched after they have rendered, since a parent
    /// component renders *after* its children, and that patch runs once: a step
    /// appended later keeps the default tick (issue #716).
    pub completed_icon: Option<TablerIcon>,
    /// Default in-progress-step icon.
    ///
    /// Used by the [`StepperStep`]s in the progress state **present at this
    /// stepper's own render** that set no `progress_icon` of their own, in place
    /// of that step's `icon` or its number. The step's own `progress_icon` wins,
    /// and a step appended later keeps whatever it drew (issue #716).
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

    fn as_str(self) -> &'static str {
        match self {
            StepState::Completed => "completed",
            StepState::Progress => "progress",
            StepState::Inactive => "inactive",
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

        let steps = collect_steps(&steps_container);

        // Everything a step cannot know about itself, in one pass in document
        // order: which position it sits at, and therefore which state `active`
        // puts it in (issue #709). A step renders before this component does, so
        // none of it could have travelled as a prop.
        for (position, step) in steps.iter().enumerate() {
            let position = position as u32;

            if self.allow_next_steps_select && position > self.active {
                // A step that asked to be clickable itself already carries the
                // class. `add_class` is idempotent since #717, so the guard is
                // belt and braces rather than load-bearing.
                if !has_class(step, CLICKABLE_CLASS) {
                    step.add_class(CLICKABLE_CLASS);
                }
            }

            // The index the step shows. Its own wins, so a caller may number a
            // stepper however they like; that changes the *label*, not the
            // position the state derives from — `allow_next_steps_select` counts
            // positions, and a second notion of index would disagree with it.
            let numbered_here = step.get_attribute(STEP_ATTR).is_none();
            if numbered_here {
                step.set_attribute(STEP_ATTR, &position.to_string());
            }

            // The state. Its own wins here too, and `data-state` is the only
            // thing that can say it named one: `state: String` renders an
            // explicit `"inactive"` and an unset field into the same class.
            let named_state = step.get_attribute(STATE_ATTR);
            let state = match named_state.as_deref() {
                Some(named) => StepState::parse(named),
                None => {
                    let derived = StepState::derive(position, self.active);
                    // Swap the one class rather than rewriting `class`: the
                    // clickable grant above, and the rsx `class:` prop the macro
                    // merges onto a component's root, are both already on this
                    // node (issue #717).
                    step.remove_class(StepState::Inactive.class_name());
                    step.add_class(derived.class_name());
                    derived
                }
            };

            self.settle_step_icon(
                __scope,
                step,
                state,
                named_state.is_some(),
                numbered_here.then_some(position + 1),
            );
        }

        container.append_child(&steps_container);

        container
    }
}

impl Stepper {
    /// Bring one step's icon box into line with the state and index it has just
    /// been given.
    ///
    /// Three things can be wrong with what the step drew, and only this
    /// component knows about any of them:
    ///
    /// - it drew the glyph for its *own* state, which may not be the state it is
    ///   now in;
    /// - it drew the built-in default for a state this stepper has an icon for
    ///   (`data-icon-fallback`, issue #707);
    /// - it drew the number `1`, because it had no index to draw.
    ///
    /// A step it did not put into a new state is left exactly where #707 left
    /// it, bar the number.
    fn settle_step_icon(
        &self,
        __scope: &mut RenderScope,
        step: &NodeHandle,
        state: StepState,
        named_its_own_state: bool,
        number: Option<u32>,
    ) {
        let Some(icon_box) = step_icon_box(step) else {
            return;
        };

        // A loading step draws a spinner, which is the right content in every
        // state and carries no number. It leaves no alternates and marks no
        // fallback, so there is nothing here that could apply to it.
        if has_class(step, LOADING_CLASS) {
            return;
        }

        // The alternates a step with no state of its own left behind, for the
        // states it could be moved into but did not draw. Nothing goes yet —
        // see the note on `discard` below for why the order matters.
        let mut alternate = None;
        let mut alternates = Vec::new();
        for child in icon_box.children() {
            if !has_class(&child, ICON_ALT_CLASS) {
                continue;
            }
            if child.get_attribute(ICON_ALT_ATTR).as_deref() == Some(state.as_str()) {
                alternate = child.children().into_iter().next();
            }
            alternates.push(child);
        }

        // The stepper's own default for this state, which applies only where the
        // step supplied nothing of its own — `data-icon-fallback` says so for a
        // step that drew this state itself, and an absent alternate says so for
        // one this component moved here.
        let stepper_default = match state {
            StepState::Completed => self.completed_icon,
            StepState::Progress => self.progress_icon,
            StepState::Inactive => None,
        };
        let step_asked_for_the_default = if named_its_own_state {
            icon_box.get_attribute(ICON_FALLBACK_ATTR).as_deref() == Some(state.as_str())
        } else {
            alternate.is_none()
        };

        let replacement: Option<NodeHandle> = if let Some(glyph) = alternate {
            Some(glyph)
        } else if let Some(icon) = stepper_default.filter(|_| step_asked_for_the_default) {
            Some(render_tabler_icon(__scope, icon, TablerIconStyle::Outline))
        } else if named_its_own_state || state != StepState::Completed {
            // Whatever the step drew still stands — except the number, which it
            // could only have drawn as `1`.
            number
                .filter(|_| icon_box.get_attribute(ICON_NUMBER_ATTR).is_some())
                .map(|n| rinch_core::IntoNode::into_node(n.to_string(), __scope))
        } else {
            // Moved to completed with no icon anywhere: the built-in tick, which
            // is what a step that had rendered completed would have drawn.
            Some(crate::icons::check_dom(__scope))
        };

        let Some(replacement) = replacement else {
            // Nothing to redraw, but the alternates are markup nobody will look
            // at again. `discard`, as below.
            for alternate in alternates {
                alternate.discard();
            }
            return;
        };

        // **Attach first, clear second**, and this order is load-bearing rather
        // than tidy. What goes here goes for good — the icon this step drew for
        // a state it is no longer in, and every alternate it is not using — so
        // it leaves by `discard` and not by `remove` (issue #719): `remove` is a
        // detach that keeps a subtree re-insertable on both backends, while
        // `discard` is what releases `rinch-web`'s strong `web_sys::Node` from
        // its two page-global maps. `rinch-web` compiles this crate, and a
        // `Stepper` re-renders per step per signal change, so a `remove` here
        // would strand an SVG subtree in the browser every time.
        //
        // That is exactly why the promoted alternate has to leave its wrapper
        // *before* the wrapper is discarded: `discard` retires the whole
        // subtree, so discarding a wrapper that still held the glyph would
        // retire the glyph with it. `append_child` re-parents, so promoting
        // first takes the glyph out of reach of the loop below.
        icon_box.append_child(&replacement);
        for child in icon_box.children() {
            if child.node_id() != replacement.node_id() {
                child.discard();
            }
        }
        // The two markers described what the step drew, and it no longer does.
        // Leaving them would be a lie in the rendered DOM.
        icon_box.remove_attribute(ICON_FALLBACK_ATTR);
        icon_box.remove_attribute(ICON_NUMBER_ATTR);
    }
}

/// The class a clickable step carries.
const CLICKABLE_CLASS: &str = "rinch-stepper__step--clickable";

/// The class a loading step carries.
const LOADING_CLASS: &str = "rinch-stepper__step--loading";

/// Set by [`StepperStep`] on its icon box to say which of the parent's default
/// icons, if any, may replace what it drew there.
///
/// The step's own icon props are resolved before this is written, so a box that
/// carries the attribute is one whose step supplied nothing for that state —
/// which is exactly the case where the parent's default applies.
const ICON_FALLBACK_ATTR: &str = "data-icon-fallback";

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

/// Set by [`StepperStep`] on its icon box when what it drew there is the plain
/// step number.
///
/// A step with no index of its own can only draw `1`, so [`Stepper`] has to
/// redraw it once it has numbered the step. This says whether there is a number
/// there to redraw: a glyph and a number are both "whatever the step drew", and
/// nothing shallower than sniffing the node type tells them apart.
const ICON_NUMBER_ATTR: &str = "data-icon-number";

/// The wrapper class of an alternate icon left in a step's icon box.
const ICON_ALT_CLASS: &str = "rinch-stepper__step-icon-alt";

/// Which state an alternate icon is for.
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

        // Which of the stepper's default icons, if any, may replace what this
        // step is about to draw. Set only where the step supplied nothing of its
        // own for the state it is in.
        match state {
            StepState::Completed if !self.loading && self.completed_icon.is_none() => {
                icon_container.set_attribute(ICON_FALLBACK_ATTR, "completed");
            }
            StepState::Progress if !self.loading && self.progress_icon.is_none() => {
                icon_container.set_attribute(ICON_FALLBACK_ATTR, "progress");
            }
            _ => {}
        }

        let mut drew_the_number = false;
        let icon_content = if self.loading {
            rinch_macros::rsx! { span { class: "rinch-stepper__loader" } }
        } else if state == StepState::Completed {
            if let Some(custom_icon) = self.completed_icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else {
                crate::icons::check_dom(__scope)
            }
        } else if state == StepState::Progress {
            if let Some(custom_icon) = self.progress_icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else if let Some(custom_icon) = self.icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else {
                drew_the_number = true;
                rinch_core::IntoNode::into_node(step_number.to_string(), __scope)
            }
        } else if let Some(custom_icon) = self.icon {
            render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
        } else {
            drew_the_number = true;
            rinch_core::IntoNode::into_node(step_number.to_string(), __scope)
        };

        icon_container.append_child(&icon_content);
        if drew_the_number {
            icon_container.set_attribute(ICON_NUMBER_ATTR, "");
        }

        // A parent `Stepper` may put this step in a state it did not render, and
        // the icon for that state is a `TablerIcon` in this step's props — not
        // something a patch of the rendered tree could recover. So render it now
        // and leave it in the box, hidden, for the parent to promote; the parent
        // drops every alternate it did not need (issue #709).
        //
        // Only where a parent could act on one: a step that named its own state
        // is already in the state it will stay in, and a loading step draws a
        // spinner whatever state it ends up in. `display: none` inline rather
        // than from the component stylesheet, because a step rendered outside a
        // `Stepper` has nobody to take these away and must not show them even if
        // the sheet never loads.
        if self.state.is_empty() && !self.loading {
            for (which, icon) in [
                (StepState::Completed, self.completed_icon),
                (StepState::Progress, self.progress_icon),
            ] {
                let Some(icon) = icon else { continue };
                let alt = rinch_macros::rsx! { span { class: "rinch-stepper__step-icon-alt" } };
                alt.set_attribute(ICON_ALT_ATTR, which.as_str());
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
