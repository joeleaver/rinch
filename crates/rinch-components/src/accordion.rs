//! Accordion component.
//!
//! Collapsible content sections.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

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
    pub value: String,
    /// Default expanded item (uncontrolled).
    pub default_value: String,
    /// Visual variant (default, contained, filled, separated).
    pub variant: String,
    /// Border radius.
    pub radius: String,
    /// Whether multiple items can be open.
    pub multiple: bool,
    /// Chevron position (left, right).
    pub chevron_position: String,
    /// Whether to disable chevron rotation.
    pub disable_chevron_rotation: bool,
}

impl Accordion {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-accordion"];

        if !self.variant.is_empty() {
            if let Ok(variant) = self.variant.parse::<AccordionVariant>() {
                classes.push(variant.class_name());
            }
        } else {
            classes.push(AccordionVariant::Default.class_name());
        }

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-accordion--radius-xs"),
                "sm" => classes.push("rinch-accordion--radius-sm"),
                "md" => classes.push("rinch-accordion--radius-md"),
                "lg" => classes.push("rinch-accordion--radius-lg"),
                "xl" => classes.push("rinch-accordion--radius-xl"),
                _ => {}
            }
        }

        if !self.chevron_position.is_empty()
            && let Ok(ChevronPosition::Left) = self.chevron_position.parse()
        {
            classes.push("rinch-accordion--chevron-left");
        }

        classes.join(" ")
    }
}

impl Component for Accordion {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-accordion" } };
        container.set_attribute("class", &self.class_string());

        // Store active value as data attribute so AccordionItem can read it
        let active_value = if !self.value.is_empty() {
            &self.value
        } else {
            &self.default_value
        };
        if !active_value.is_empty() {
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
///
/// Wires up toggle behavior: finds the AccordionControl (button) and
/// AccordionPanel among its children and connects them via a Signal.
#[derive(Debug, Default)]
pub struct AccordionItem {
    /// Item identifier value.
    pub value: String,
}

impl Component for AccordionItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let item = rinch_macros::rsx! { div { class: "rinch-accordion__item" } };

        if !self.value.is_empty() {
            item.set_attribute("data-item-value", &self.value);
        }

        let is_open = Signal::new(false);

        // Find the control button and panel among children
        let mut control_btn: Option<NodeHandle> = None;
        let mut panel: Option<NodeHandle> = None;

        for child in children {
            if let Some(cls) = child.get_attribute("class") {
                if cls.contains("rinch-accordion__control") {
                    control_btn = Some(child.clone());
                } else if cls.contains("rinch-accordion__panel") {
                    panel = Some(child.clone());
                }
            }
            item.append_child(child);
        }

        // Wire up click handler on the control button
        if let Some(ref btn) = control_btn {
            let handler_id = __scope.register_handler(move || {
                is_open.update(|v| *v = !*v);
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Toggle panel display
        if let Some(ref p) = panel {
            let p = p.clone();
            __scope.create_effect(move || {
                if is_open.get() {
                    p.set_style("display", "");
                } else {
                    p.set_style("display", "none");
                }
            });
        }

        // Rotate chevron on control — find the SVG child with the chevron class
        if let Some(ref btn) = control_btn {
            let chevron = btn.children().into_iter().find(|child| {
                child
                    .get_attribute("class")
                    .is_some_and(|c| c.contains("rinch-accordion__chevron"))
            });
            if let Some(chevron) = chevron {
                __scope.create_effect(move || {
                    if is_open.get() {
                        chevron.set_style("transform", "rotate(180deg)");
                    } else {
                        chevron.set_style("transform", "");
                    }
                });
            }
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
    pub icon: Option<TablerIcon>,
    /// Click callback.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for AccordionControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccordionControl")
            .field("disabled", &self.disabled)
            .field("icon", &self.icon)
            .finish_non_exhaustive()
    }
}

impl Component for AccordionControl {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-accordion__control"];

        if self.disabled {
            classes.push("rinch-accordion__control--disabled");
        }

        let btn = rinch_macros::rsx! { button { class: "rinch-accordion__control" } };
        btn.set_attribute("class", &classes.join(" "));

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // If user provided a click callback, register it.
        // The toggle behavior is wired by AccordionItem, which sets data-rid on this button.
        if let Some(ref cb) = self.onclick
            && !self.disabled
        {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Label span
        let label = rinch_macros::rsx! { span { class: "rinch-accordion__label" } };
        for child in children {
            label.append_child(child);
        }
        btn.append_child(&label);

        // Chevron icon
        let chevron_el = if let Some(icon) = self.icon {
            render_tabler_icon(__scope, icon, TablerIconStyle::Outline)
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

impl Component for AccordionPanel {
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
