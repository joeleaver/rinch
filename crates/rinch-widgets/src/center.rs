//! Center widget.
//!
//! Centers content horizontally and vertically.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A centering container.
#[derive(Debug, Default)]
pub struct Center {
    /// Whether to center inline (horizontal only).
    pub inline: bool,
}

impl Center {
    /// Generate the CSS class string for this center element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-center"];

        if self.inline {
            classes.push("rinch-center--inline");
        }

        classes.join(" ")
    }
}

impl Widget for Center {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-center" }
        };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
