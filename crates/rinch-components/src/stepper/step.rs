//! [`StepperStep`] and [`StepperCompleted`].

use super::*;

/// Individual step in the stepper.
///
/// Its children are the step's content, which the parent [`Stepper`] shows
/// below the steps while this step is the active one (issue #2195).
#[derive(Default)]
pub struct StepperStep {
    /// Step label text.
    pub label: String,
    /// Step description text.
    pub description: String,
    /// Renders the label as nodes, in place of [`label`](StepperStep::label)
    /// (issue #2195). It goes in the same `.rinch-stepper__step-label` element.
    pub label_section: Option<SectionRenderer>,
    /// Renders the description as nodes, in place of
    /// [`description`](StepperStep::description) (issue #2195). It goes in the
    /// same `.rinch-stepper__step-description` element.
    pub description_section: Option<SectionRenderer>,
    /// Custom icon for this step.
    pub icon: Option<TablerIcon>,
    /// Renders the step's icon as nodes — an avatar, a styled number — in place
    /// of [`icon`](StepperStep::icon) or the step number (issue #2195).
    ///
    /// It fills the same slot `icon` does: drawn while the step is inactive,
    /// and while it is in progress unless a progress icon applies; a completed
    /// step draws its completed icon (the tick by default) instead.
    pub icon_section: Option<SectionRenderer>,
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
    /// Loading state: the icon box draws a [`Loader`](crate::Loader) in place
    /// of its icon.
    pub loading: bool,
    /// The loader type drawn while `loading` (`oval`, `bars`, `dots`). Empty
    /// uses the app's [`LoaderDefaults`](crate::LoaderDefaults) (which can
    /// also supply a custom loader), then oval.
    pub loader_type: String,
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

impl std::fmt::Debug for StepperStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let section = |s: &Option<SectionRenderer>| s.as_ref().map(|_| "...");
        f.debug_struct("StepperStep")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("label_section", &section(&self.label_section))
            .field("description_section", &section(&self.description_section))
            .field("icon", &self.icon)
            .field("icon_section", &section(&self.icon_section))
            .field("completed_icon", &self.completed_icon)
            .field("progress_icon", &self.progress_icon)
            .field("allow_step_click", &self.allow_step_click)
            .field("allow_step_select", &self.allow_step_select)
            .field("loading", &self.loading)
            .field("loader_type", &self.loader_type)
            .field("disabled", &self.disabled)
            .field("state", &self.state)
            .field("step", &self.step)
            .finish()
    }
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
            if self.icon.is_some() || self.icon_section.is_some() {
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
            None => {
                // The shared Loader (issue #2205), in the slot the stepper's
                // stylesheet sizes and colours.
                let slot = rinch_macros::rsx! { span { class: "rinch-stepper__loader" } };
                let loader =
                    crate::loader::render_component_loader(__scope, &self.loader_type, None, "");
                slot.append_child(&loader);
                slot
            }
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
            Some(_) => match (&self.icon_section, self.icon) {
                (Some(render), _) => render(__scope),
                (None, Some(custom_icon)) => {
                    render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
                }
                (None, None) => rinch_core::IntoNode::into_node(step_number.to_string(), __scope),
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

        if let Some(render) = &self.label_section {
            let label_span = rinch_macros::rsx! { span { class: "rinch-stepper__step-label" } };
            label_span.append_child(&render(__scope));
            body.append_child(&label_span);
        } else if !self.label.is_empty() {
            let label_span = rinch_macros::rsx! { span { class: "rinch-stepper__step-label", {self.label.clone()} } };
            body.append_child(&label_span);
        }

        if let Some(render) = &self.description_section {
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-stepper__step-description" } };
            desc_span.append_child(&render(__scope));
            body.append_child(&desc_span);
        } else if !self.description.is_empty() {
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-stepper__step-description" } };
            let desc_text = rinch_core::IntoNode::into_node(self.description.clone(), __scope);
            desc_span.append_child(&desc_text);
            body.append_child(&desc_span);
        }

        // Content: the body's last child, under the label and description.
        // Hidden here, and moved into the stepper's content region below the
        // steps while this step is the active one (issue #2195). In the body
        // rather than beside it so that, shown in place
        // (`content_for_active_only: false`), it lies in the label's column in
        // either orientation: under the label, and in a vertical stepper
        // beside the icon and its connector, as a timeline's entries are.
        let content_el = rinch_macros::rsx! { div { class: "rinch-stepper__step-content" } };

        for child in children {
            content_el.append_child(child);
        }

        body.append_child(&content_el);

        step_el.append_child(&body);

        // Separator
        let separator = rinch_macros::rsx! { div { class: "rinch-stepper__separator" } };
        step_el.append_child(&separator);

        step_el
    }
}

/// Content shown for the completed state.
///
/// Hidden until the parent [`Stepper`]'s `active` is past its last step (`active
/// >= ` the number of steps); then the stepper shows it in its content region
/// below the steps, in place of a step's content — Mantine's `Stepper.Completed`
/// (issue #2195). Outside a `Stepper` it is shown as is.
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
