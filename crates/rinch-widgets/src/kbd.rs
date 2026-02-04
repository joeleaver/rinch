//! Kbd widget.
//!
//! Keyboard key display element.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A keyboard key indicator.
#[derive(Debug, Default)]
pub struct Kbd {
    /// Size (xs, sm, md, lg).
    pub size: Option<String>,
}

impl Kbd {
    /// Generate the CSS class string for this kbd element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-kbd"];

        if let Some(ref size) = self.size {
            match size.as_str() {
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

impl Widget for Kbd {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { kbd { class: "rinch-kbd" } };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
