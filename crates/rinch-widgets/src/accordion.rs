//! Accordion widget.
//!
//! Collapsible content sections.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Icon, Widget, WidgetCallback};

/// Accordion variant style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccordionVariant {
    #[default]
    Default,
    Contained,
    Filled,
    Separated,
}

impl AccordionVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            AccordionVariant::Default => "rinch-accordion--default",
            AccordionVariant::Contained => "rinch-accordion--contained",
            AccordionVariant::Filled => "rinch-accordion--filled",
            AccordionVariant::Separated => "rinch-accordion--separated",
        }
    }
}

impl std::str::FromStr for AccordionVariant {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(AccordionVariant::Default),
            "contained" => Ok(AccordionVariant::Contained),
            "filled" => Ok(AccordionVariant::Filled),
            "separated" => Ok(AccordionVariant::Separated),
            _ => Err(()),
        }
    }
}

/// Chevron position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChevronPosition {
    Left,
    #[default]
    Right,
}

impl std::str::FromStr for ChevronPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "left" => Ok(ChevronPosition::Left),
            "right" => Ok(ChevronPosition::Right),
            _ => Err(()),
        }
    }
}

/// Accordion container for collapsible sections.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Accordion { default_value: "item1",
///         AccordionItem { value: "item1",
///             AccordionControl { "Section 1" }
///             AccordionPanel { "Content for section 1" }
///         }
///         AccordionItem { value: "item2",
///             AccordionControl { "Section 2" }
///             AccordionPanel { "Content for section 2" }
///         }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Accordion {
    /// Currently expanded item(s).
    pub value: Option<String>,
    /// Default expanded item (uncontrolled).
    pub default_value: Option<String>,
    /// Visual variant (default, contained, filled, separated).
    pub variant: Option<String>,
    /// Border radius.
    pub radius: Option<String>,
    /// Whether multiple items can be open.
    pub multiple: bool,
    /// Chevron position (left, right).
    pub chevron_position: Option<String>,
    /// Whether to disable chevron rotation.
    pub disable_chevron_rotation: bool,
}

impl Accordion {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-accordion"];

        if let Some(ref v) = self.variant {
            if let Ok(variant) = v.parse::<AccordionVariant>() {
                classes.push(variant.class_name());
            }
        } else {
            classes.push(AccordionVariant::Default.class_name());
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-accordion--radius-xs"),
                "sm" => classes.push("rinch-accordion--radius-sm"),
                "md" => classes.push("rinch-accordion--radius-md"),
                "lg" => classes.push("rinch-accordion--radius-lg"),
                "xl" => classes.push("rinch-accordion--radius-xl"),
                _ => {}
            }
        }

        if let Some(ref cp) = self.chevron_position {
            if let Ok(ChevronPosition::Left) = cp.parse() {
                classes.push("rinch-accordion--chevron-left");
            }
        }

        classes.join(" ")
    }
}

impl Widget for Accordion {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-accordion" } };
        container.set_attribute("class", &self.class_string());

        if let Some(active_value) = self.value.as_ref().or(self.default_value.as_ref()) {
            container.set_attribute("data-active-item", active_value);
        }

        if self.multiple {
            container.set_attribute("data-multiple", "true");
        }

        for child in children {
            container.append_child(child);
        }

        container
    }
}

/// Individual accordion item container.
#[derive(Debug, Default)]
pub struct AccordionItem {
    /// Item identifier value.
    pub value: Option<String>,
}

impl Widget for AccordionItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let item = rinch_macros::rsx! { div { class: "rinch-accordion__item" } };

        if let Some(ref value) = self.value {
            item.set_attribute("data-item-value", value);
        }

        for child in children {
            item.append_child(child);
        }

        item
    }
}

/// Clickable accordion header/control.
#[derive(Default)]
pub struct AccordionControl {
    /// Whether the control is disabled.
    pub disabled: bool,
    /// Icon to display (replaces default chevron).
    pub icon: Option<Icon>,
    /// Click callback.
    pub onclick: Option<WidgetCallback>,
}

impl std::fmt::Debug for AccordionControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccordionControl")
            .field("disabled", &self.disabled)
            .field("icon", &self.icon)
            .finish_non_exhaustive()
    }
}

impl Widget for AccordionControl {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-accordion__control"];

        if self.disabled {
            classes.push("rinch-accordion__control--disabled");
        }

        let btn = rinch_macros::rsx! { button { class: "rinch-accordion__control" } };
        btn.set_attribute("class", &classes.join(" "));

        // Mark this as an accordion toggle - the shell will handle DOM toggle
        btn.set_attribute("data-accordion-toggle", "true");

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // Register click handler if provided
        if let Some(ref cb) = self.onclick {
            if !self.disabled {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                btn.set_attribute("data-rid", &handler_id.0.to_string());
            }
        }

        // Label span
        let label = rinch_macros::rsx! { span { class: "rinch-accordion__label" } };
        for child in children {
            label.append_child(child);
        }
        btn.append_child(&label);

        // Chevron icon
        let chevron_el = if let Some(icon) = self.icon {
            crate::icons::render_icon(__scope, icon)
        } else {
            crate::icons::chevron_down_dom("rinch-accordion__chevron", __scope)
        };
        btn.append_child(&chevron_el);

        btn
    }
}

/// Content panel for accordion item.
#[derive(Debug, Default)]
pub struct AccordionPanel;

impl Widget for AccordionPanel {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let panel = rinch_macros::rsx! { div { class: "rinch-accordion__panel" } };
        let content = rinch_macros::rsx! { div { class: "rinch-accordion__content" } };

        for child in children {
            content.append_child(child);
        }

        panel.append_child(&content);

        panel
    }
}
