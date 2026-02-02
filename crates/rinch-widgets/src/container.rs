//! Container widget.
//!
//! Centered max-width container.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A centered container with max-width.
#[derive(Debug, Default)]
pub struct Container {
    /// Size (xs, sm, md, lg, xl). Controls max-width.
    pub size: Option<String>,
    /// Whether to add horizontal padding.
    pub fluid: bool,
}

impl Container {
    /// Generate the CSS class string for this container.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-container"];

        if let Some(ref size) = self.size {
            match size.as_str() {
                "xs" => classes.push("rinch-container--xs"),
                "sm" => classes.push("rinch-container--sm"),
                "md" => classes.push("rinch-container--md"),
                "lg" => classes.push("rinch-container--lg"),
                "xl" => classes.push("rinch-container--xl"),
                _ => {}
            }
        }

        if self.fluid {
            classes.push("rinch-container--fluid");
        }

        classes.join(" ")
    }
}

impl Widget for Container {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-container" }
        };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
