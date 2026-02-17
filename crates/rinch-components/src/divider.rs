//! Divider component.
//!
//! Horizontal or vertical separator with optional label.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A horizontal or vertical divider line.
#[derive(Debug, Default)]
pub struct Divider {
    /// Orientation ("horizontal" or "vertical").
    pub orientation: String,
    /// Margin size (xs, sm, md, lg, xl).
    pub size: String,
    /// Optional label text (horizontal only).
    pub label: String,
    /// Label position ("left", "center", "right").
    pub label_position: String,
}

impl Divider {
    /// Generate the CSS class string for this divider.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-divider"];

        // Orientation
        let orientation = if self.orientation.is_empty() {
            "horizontal"
        } else {
            &self.orientation
        };
        match orientation {
            "vertical" => classes.push("rinch-divider--vertical"),
            _ => classes.push("rinch-divider--horizontal"),
        }

        // Size (margin)
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-divider--xs"),
                "sm" => classes.push("rinch-divider--sm"),
                "md" => classes.push("rinch-divider--md"),
                "lg" => classes.push("rinch-divider--lg"),
                "xl" => classes.push("rinch-divider--xl"),
                _ => {}
            }
        }

        // Label
        if !self.label.is_empty() {
            classes.push("rinch-divider--with-label");

            // Label position
            if !self.label_position.is_empty() {
                match self.label_position.as_str() {
                    "left" => classes.push("rinch-divider--label-left"),
                    "right" => classes.push("rinch-divider--label-right"),
                    _ => {} // center is default
                }
            }
        }

        classes.join(" ")
    }
}

impl Component for Divider {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();
        let orientation = if self.orientation.is_empty() {
            "horizontal"
        } else {
            &self.orientation
        };

        if orientation == "vertical" {
            let container = rinch_macros::rsx! { div { class: "rinch-divider" } };
            container.set_attribute("class", &class);
            container
        } else if !self.label.is_empty() {
            let label_span = rinch_macros::rsx! { span { class: "rinch-divider__label" } };
            let text_node = __scope.create_text(&self.label);
            label_span.append_child(&text_node);

            let container = rinch_macros::rsx! { div { class: "rinch-divider" } };
            container.set_attribute("class", &class);
            container.append_child(&label_span);
            container
        } else {
            let container = rinch_macros::rsx! { hr { class: "rinch-divider" } };
            container.set_attribute("class", &class);
            container
        }
    }
}
