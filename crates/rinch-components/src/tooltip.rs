//! Tooltip component.
//!
//! Shows additional information on hover.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Tooltip position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl TooltipPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            TooltipPosition::Top => "rinch-tooltip--top",
            TooltipPosition::Bottom => "rinch-tooltip--bottom",
            TooltipPosition::Left => "rinch-tooltip--left",
            TooltipPosition::Right => "rinch-tooltip--right",
        }
    }
}

impl std::str::FromStr for TooltipPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "top" => Ok(TooltipPosition::Top),
            "bottom" => Ok(TooltipPosition::Bottom),
            "left" => Ok(TooltipPosition::Left),
            "right" => Ok(TooltipPosition::Right),
            _ => Err(()),
        }
    }
}

/// A tooltip component that shows on hover.
///
/// Note: This is a CSS-only tooltip. For more complex tooltips with
/// JavaScript-based positioning, additional runtime support is needed.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Tooltip { label: "This is helpful information",
///         button { "Hover me" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Tooltip {
    /// Tooltip text content.
    pub label: String,
    /// Position (top, bottom, left, right).
    pub position: String,
    /// Background color.
    pub color: String,
    /// Whether tooltip is initially open.
    pub opened: bool,
    /// Whether tooltip is disabled.
    pub disabled: bool,
    /// Whether to show arrow.
    pub with_arrow: bool,
    /// Multiline tooltip (wraps text).
    pub multiline: bool,
    /// Width for multiline tooltips.
    pub width: String,
}

impl Tooltip {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-tooltip"];

        let position: TooltipPosition = if self.position.is_empty() {
            TooltipPosition::default()
        } else {
            self.position.parse().unwrap_or_default()
        };
        classes.push(position.class_name());

        if self.with_arrow {
            classes.push("rinch-tooltip--with-arrow");
        }

        if self.multiline {
            classes.push("rinch-tooltip--multiline");
        }

        if self.opened {
            classes.push("rinch-tooltip--opened");
        }

        if self.disabled {
            classes.push("rinch-tooltip--disabled");
        }

        classes.join(" ")
    }
}

impl Component for Tooltip {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let label = if self.label.is_empty() {
            ""
        } else {
            &self.label
        };

        let mut styles = Vec::new();

        if !self.color.is_empty() {
            if self.color.starts_with('#') || self.color.starts_with("rgb") || self.color.starts_with("hsl") {
                styles.push(format!("--rinch-tooltip-color: {}", self.color));
            } else {
                styles.push(format!(
                    "--rinch-tooltip-color: var(--rinch-color-{}-9)",
                    self.color
                ));
            }
        }

        if !self.width.is_empty() {
            styles.push(format!("--rinch-tooltip-width: {}", self.width));
        }

        // Create root element
        let root = rinch_macros::rsx! { div { class: "rinch-tooltip" } };
        root.set_attribute("class", &self.class_string());
        if !styles.is_empty() {
            root.set_attribute("style", &styles.join("; "));
        }

        // Target element
        let target = rinch_macros::rsx! { div { class: "rinch-tooltip__target" } };
        for child in children {
            target.append_child(child);
        }
        root.append_child(&target);

        // Content element
        let content =
            rinch_macros::rsx! { div { class: "rinch-tooltip__content", role: "tooltip" } };
        let label_text = __scope.create_text(label);
        content.append_child(&label_text);
        root.append_child(&content);

        root
    }
}
