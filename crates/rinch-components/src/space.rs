//! Space component.
//!
//! Adds horizontal or vertical spacing.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A spacing element.
#[derive(Debug, Default)]
pub struct Space {
    /// Width size (xs, sm, md, lg, xl) for horizontal space.
    pub w: String,
    /// Height size (xs, sm, md, lg, xl) for vertical space.
    pub h: String,
}

impl Space {
    /// Generate the CSS class string for this space element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-space"];

        // Width
        if !self.w.is_empty() {
            match self.w.as_str() {
                "xs" => classes.push("rinch-space--w-xs"),
                "sm" => classes.push("rinch-space--w-sm"),
                "md" => classes.push("rinch-space--w-md"),
                "lg" => classes.push("rinch-space--w-lg"),
                "xl" => classes.push("rinch-space--w-xl"),
                _ => {}
            }
        }

        // Height
        if !self.h.is_empty() {
            match self.h.as_str() {
                "xs" => classes.push("rinch-space--h-xs"),
                "sm" => classes.push("rinch-space--h-sm"),
                "md" => classes.push("rinch-space--h-md"),
                "lg" => classes.push("rinch-space--h-lg"),
                "xl" => classes.push("rinch-space--h-xl"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Component for Space {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-space" }
        };
        container.set_attribute("class", &self.class_string());
        container
    }
}
