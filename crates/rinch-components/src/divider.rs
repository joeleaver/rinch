//! Divider component.
//!
//! Horizontal or vertical separator with optional label.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

use crate::SectionRenderer;

/// A horizontal or vertical divider line.
#[derive(Default)]
pub struct Divider {
    /// Orientation ("horizontal" or "vertical").
    pub orientation: String,
    /// Margin size (xs, sm, md, lg, xl).
    pub size: String,
    /// Optional label text (horizontal only).
    pub label: String,
    /// A custom label node (an icon, an icon and text, ...), drawn in the
    /// `.rinch-divider__label` span in place of `label`'s text (horizontal
    /// only). Pass `Some(renderer)`.
    pub label_node: Option<SectionRenderer>,
    /// Label position ("left", "center", "right").
    pub label_position: String,
}

impl std::fmt::Debug for Divider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Divider")
            .field("orientation", &self.orientation)
            .field("size", &self.size)
            .field("label", &self.label)
            .field(
                "label_node",
                &self.label_node.as_ref().map(|_| "<renderer>"),
            )
            .field("label_position", &self.label_position)
            .finish()
    }
}

impl Divider {
    /// Whether a label (text or node) is drawn.
    fn has_label(&self) -> bool {
        self.label_node.is_some() || !self.label.is_empty()
    }

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
        if self.has_label() {
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
        } else if self.has_label() {
            let label_span = if let Some(render) = &self.label_node {
                let span = rinch_macros::rsx! { span { class: "rinch-divider__label" } };
                let node = render(__scope);
                span.append_child(&node);
                span
            } else {
                rinch_macros::rsx! { span { class: "rinch-divider__label", {self.label.clone()} } }
            };

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
