//! Stepper component.
//!
//! Step-by-step progress indicator.

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
///         StepperStep { step: 0, state: "completed", label: "Step 1", description: "Create account" }
///         StepperStep { step: 1, state: "progress", label: "Step 2", description: "Verify email" }
///         StepperStep { step: 2, label: "Step 3", description: "Complete profile" }
///     }
/// }
/// ```
///
/// Each step's `state` and `step` are the **caller's** to set, despite what
/// `StepperStep`'s own field docs say: this component publishes `active` as
/// `data-active` and does not derive either (issue #709). Without a `state`
/// every step is inactive, which is what made this example draw no custom
/// completed icon whatever `completed_icon` said.
#[derive(Debug, Default)]
pub struct Stepper {
    /// Currently active step (0-indexed).
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
    /// callback (issue #709), as `allow_step_click` and `allow_step_select`
    /// already did.
    pub allow_next_steps_select: bool,
    /// Default completed-step icon.
    ///
    /// Used by the [`StepperStep`]s in the completed state **present at this
    /// stepper's own render** that set no `completed_icon` of their own — the
    /// step's icon wins. Steps are patched after they have rendered, since a
    /// parent component renders *after* its children, and that patch runs once:
    /// a step appended later keeps the default tick (issue #716).
    pub completed_icon: Option<TablerIcon>,
    /// Default in-progress-step icon.
    ///
    /// Used by the [`StepperStep`]s in the progress state **present at this
    /// stepper's own render** that set no `progress_icon` of their own, in place
    /// of that step's `icon` or its number. The step's own `progress_icon` wins,
    /// and a step appended later keeps whatever it drew (issue #716).
    pub progress_icon: Option<TablerIcon>,
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

        if self.allow_next_steps_select {
            for step in steps.iter().skip(self.active as usize + 1) {
                // A step that asked to be clickable itself already carries the
                // class; `add_class` does not deduplicate.
                if !has_class(step, CLICKABLE_CLASS) {
                    step.add_class(CLICKABLE_CLASS);
                }
            }
        }

        for step in &steps {
            if let Some(icon_box) = step_icon_box(step) {
                let fallback = icon_box.get_attribute(ICON_FALLBACK_ATTR);
                let default_icon = match fallback.as_deref() {
                    Some("completed") => self.completed_icon,
                    Some("progress") => self.progress_icon,
                    _ => None,
                };
                if let Some(icon) = default_icon {
                    // `discard`, not `remove` (issue #719): the previous glyph
                    // is replaced two lines below and nothing holds a handle to
                    // it, so the backend should let go. `rinch-web` compiles
                    // this crate, and a `Stepper` re-render happens per step per
                    // signal change — a `remove` here strands the whole SVG
                    // subtree in the browser backend's two node maps every time.
                    for child in icon_box.children() {
                        child.discard();
                    }
                    let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
                    icon_box.append_child(&icon_el);
                }
            }
        }

        container.append_child(&steps_container);

        container
    }
}

/// The class a clickable step carries.
const CLICKABLE_CLASS: &str = "rinch-stepper__step--clickable";

/// Set by [`StepperStep`] on its icon box to say which of the parent's default
/// icons, if any, may replace what it drew there.
///
/// The step's own icon props are resolved before this is written, so a box that
/// carries the attribute is one whose step supplied nothing for that state —
/// which is exactly the case where the parent's default applies.
const ICON_FALLBACK_ATTR: &str = "data-icon-fallback";

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
    /// Step state (will be set by parent based on active index).
    /// Internal use: "completed", "progress", "inactive"
    pub state: String,
    /// Step index (set automatically).
    pub step: Option<u32>,
}

impl Component for StepperStep {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // State classes
        let state = if self.state.is_empty() {
            "inactive"
        } else {
            &self.state
        };
        let state_class = match state {
            "completed" => "rinch-stepper__step--completed",
            "progress" => "rinch-stepper__step--progress",
            _ => "rinch-stepper__step--inactive",
        };

        let loading_class = if self.loading {
            "rinch-stepper__step--loading"
        } else {
            ""
        };

        let clickable = if self.allow_step_click || self.allow_step_select {
            CLICKABLE_CLASS
        } else {
            ""
        };

        let class = format!("rinch-stepper__step {state_class} {loading_class} {clickable}");

        let step_el = rinch_macros::rsx! { div { class: "rinch-stepper__step" } };
        step_el.set_attribute("class", &class);

        if let Some(step) = self.step {
            step_el.set_attribute("data-step", &step.to_string());
        }

        // Step number/icon
        let step_number = self.step.map(|n| n + 1).unwrap_or(1);
        let icon_container = rinch_macros::rsx! { div { class: "rinch-stepper__step-icon" } };

        // Which of the stepper's default icons, if any, may replace what this
        // step is about to draw. Set only where the step supplied nothing of its
        // own for the state it is in.
        match state {
            "completed" if !self.loading && self.completed_icon.is_none() => {
                icon_container.set_attribute(ICON_FALLBACK_ATTR, "completed");
            }
            "progress" if !self.loading && self.progress_icon.is_none() => {
                icon_container.set_attribute(ICON_FALLBACK_ATTR, "progress");
            }
            _ => {}
        }

        let icon_content = if self.loading {
            rinch_macros::rsx! { span { class: "rinch-stepper__loader" } }
        } else if state == "completed" {
            if let Some(custom_icon) = self.completed_icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else {
                crate::icons::check_dom(__scope)
            }
        } else if state == "progress" {
            if let Some(custom_icon) = self.progress_icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else if let Some(custom_icon) = self.icon {
                render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
            } else {
                rinch_core::IntoNode::into_node(step_number.to_string(), __scope)
            }
        } else if let Some(custom_icon) = self.icon {
            render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
        } else {
            rinch_core::IntoNode::into_node(step_number.to_string(), __scope)
        };

        icon_container.append_child(&icon_content);
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
