//! Space widget.
//!
//! Adds horizontal or vertical spacing.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A spacing element.
#[derive(Debug, Default)]
pub struct Space {
    /// Width size (xs, sm, md, lg, xl) for horizontal space.
    pub w: Option<String>,
    /// Height size (xs, sm, md, lg, xl) for vertical space.
    pub h: Option<String>,
}

impl Space {
    /// Generate the CSS class string for this space element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-space"];

        // Width
        if let Some(ref w) = self.w {
            match w.as_str() {
                "xs" => classes.push("rinch-space--w-xs"),
                "sm" => classes.push("rinch-space--w-sm"),
                "md" => classes.push("rinch-space--w-md"),
                "lg" => classes.push("rinch-space--w-lg"),
                "xl" => classes.push("rinch-space--w-xl"),
                _ => {}
            }
        }

        // Height
        if let Some(ref h) = self.h {
            match h.as_str() {
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

impl Widget for Space {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-space" }
        };
        container.set_attribute("class", &self.class_string());
        container
    }
}
