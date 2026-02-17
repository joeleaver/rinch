//! Kbd component.
//!
//! Keyboard key display element.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A keyboard key indicator.
#[derive(Debug, Default)]
pub struct Kbd {
    /// Size (xs, sm, md, lg).
    pub size: String,
}

impl Kbd {
    /// Generate the CSS class string for this kbd element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-kbd"];

        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-kbd--xs"),
                "sm" => classes.push("rinch-kbd--sm"),
                "md" => classes.push("rinch-kbd--md"),
                "lg" => classes.push("rinch-kbd--lg"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Component for Kbd {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { kbd { class: "rinch-kbd" } };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
