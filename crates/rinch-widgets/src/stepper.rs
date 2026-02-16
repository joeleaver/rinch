//! Stepper widget.
//!
//! Step-by-step progress indicator.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;
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
///     Stepper { active: 1, completed_icon: Icon::CheckCircle,
///         StepperStep { label: "Step 1", description: "Create account" }
///         StepperStep { label: "Step 2", description: "Verify email" }
///         StepperStep { label: "Step 3", description: "Complete profile" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Stepper {
    /// Currently active step (0-indexed).
    pub active: u32,
    /// Size variant (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Orientation (horizontal, vertical).
    pub orientation: Option<String>,
    /// Color for completed/active steps.
    pub color: Option<String>,
    /// Border radius for step icons.
    pub radius: Option<String>,
    /// Icon size.
    pub icon_size: Option<String>,
    /// Whether to allow clicking on completed steps.
    pub allow_next_steps_select: bool,
    /// Custom completed step icon.
    pub completed_icon: Option<TablerIcon>,
    /// Custom progress step icon.
    pub progress_icon: Option<TablerIcon>,
}

impl Stepper {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-stepper"];

        if let Some(ref s) = self.size
            && let Ok(size) = s.parse::<StepperSize>()
        {
            classes.push(size.class_name());
        }

        if let Some(ref o) = self.orientation {
            if let Ok(orientation) = o.parse::<StepperOrientation>() {
                classes.push(orientation.class_name());
            }
        } else {
            classes.push(StepperOrientation::Horizontal.class_name());
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-stepper--radius-xs"),
                "sm" => classes.push("rinch-stepper--radius-sm"),
                "md" => classes.push("rinch-stepper--radius-md"),
                "lg" => classes.push("rinch-stepper--radius-lg"),
                "xl" => classes.push("rinch-stepper--radius-xl"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Widget for Stepper {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(ref c) = self.color {
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                style_parts.push(format!("--rinch-stepper-color: {}", c));
            } else {
                style_parts.push(format!("--rinch-stepper-color: var(--rinch-color-{}-6)", c));
            }
        }
        if let Some(ref size) = self.icon_size {
            style_parts.push(format!("--rinch-stepper-icon-size: {}", size));
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

        container.append_child(&steps_container);

        container
    }
}

/// Individual step in the stepper.
#[derive(Debug, Default)]
pub struct StepperStep {
    /// Step label text.
    pub label: Option<String>,
    /// Step description text.
    pub description: Option<String>,
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
    pub state: Option<String>,
    /// Step index (set automatically).
    pub step: Option<u32>,
}

impl Widget for StepperStep {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // State classes
        let state = self.state.as_deref().unwrap_or("inactive");
        let state_class = match state {
            "completed" => "rinch-stepper__step--completed",
            "progress" => "rinch-stepper__step--progress",
            _ => "rinch-stepper__step--inactive",
        };

        let loading_class = if self.loading {
            " rinch-stepper__step--loading"
        } else {
            ""
        };

        let clickable = if self.allow_step_click || self.allow_step_select {
            " rinch-stepper__step--clickable"
        } else {
            ""
        };

        let class = format!(
            "rinch-stepper__step {} {}{}",
            state_class, loading_class, clickable
        );

        let step_el = rinch_macros::rsx! { div { class: "rinch-stepper__step" } };
        step_el.set_attribute("class", &class);

        if let Some(step) = self.step {
            step_el.set_attribute("data-step", &step.to_string());
        }

        // Step number/icon
        let step_number = self.step.map(|n| n + 1).unwrap_or(1);
        let icon_container = rinch_macros::rsx! { div { class: "rinch-stepper__step-icon" } };

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
                __scope.create_text(&step_number.to_string())
            }
        } else if let Some(custom_icon) = self.icon {
            render_tabler_icon(__scope, custom_icon, TablerIconStyle::Outline)
        } else {
            __scope.create_text(&step_number.to_string())
        };

        icon_container.append_child(&icon_content);
        step_el.append_child(&icon_container);

        // Body (label + description)
        let body = rinch_macros::rsx! { div { class: "rinch-stepper__step-body" } };

        if let Some(ref label) = self.label {
            let label_span = rinch_macros::rsx! { span { class: "rinch-stepper__step-label" } };
            let label_text = __scope.create_text(label);
            label_span.append_child(&label_text);
            body.append_child(&label_span);
        }

        if let Some(ref description) = self.description {
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-stepper__step-description" } };
            let desc_text = __scope.create_text(description);
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

impl Widget for StepperCompleted {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-stepper__completed" } };

        for child in children {
            container.append_child(child);
        }

        container
    }
}
