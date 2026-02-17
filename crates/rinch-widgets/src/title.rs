//! Title widget.
//!
//! Heading component with semantic h1-h6 levels.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A heading element (h1-h6).
#[derive(Debug, Default)]
pub struct Title {
    /// Heading level (1-6). Default is 1.
    pub order: Option<u8>,
    /// Text alignment ("left", "center", "right").
    pub align: String,
    /// Size override (1-6, independent of order).
    pub size: String,
}

impl Title {
    /// Generate the CSS class string for this title.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-title"];

        // Order/size class
        let order = self.order.unwrap_or(1).clamp(1, 6);
        let size = if self.size.is_empty() {
            order
        } else {
            self.size.parse::<u8>().unwrap_or(order)
        };
        classes.push(match size {
            1 => "rinch-title--1",
            2 => "rinch-title--2",
            3 => "rinch-title--3",
            4 => "rinch-title--4",
            5 => "rinch-title--5",
            _ => "rinch-title--6",
        });

        // Alignment
        if !self.align.is_empty() {
            let align = &self.align;
            match align.as_str() {
                "left" => classes.push("rinch-title--left"),
                "center" => classes.push("rinch-title--center"),
                "right" => classes.push("rinch-title--right"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Widget for Title {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let order = self.order.unwrap_or(1).clamp(1, 6);
        let class = self.class_string();
        let container = match order {
            1 => rinch_macros::rsx! { h1 { class: "rinch-title" } },
            2 => rinch_macros::rsx! { h2 { class: "rinch-title" } },
            3 => rinch_macros::rsx! { h3 { class: "rinch-title" } },
            4 => rinch_macros::rsx! { h4 { class: "rinch-title" } },
            5 => rinch_macros::rsx! { h5 { class: "rinch-title" } },
            _ => rinch_macros::rsx! { h6 { class: "rinch-title" } },
        };
        container.set_attribute("class", &class);
        for child in children {
            container.append_child(child);
        }
        container
    }
}
