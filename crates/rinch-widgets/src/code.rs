//! Code widget.
//!
//! Inline and block code display.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A code element for displaying code snippets.
#[derive(Debug, Default)]
pub struct Code {
    /// Whether to display as a block (pre) instead of inline.
    pub block: bool,
    /// Color variant ("primary" for primary color).
    pub color: Option<String>,
}

impl Code {
    /// Generate the CSS class string for this code element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-code"];

        if self.block {
            classes.push("rinch-code--block");
        }

        if let Some(ref color) = self.color {
            if color == "primary" {
                classes.push("rinch-code--primary");
            }
        }

        classes.join(" ")
    }
}

impl Widget for Code {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        if self.block {
            let code_elem = rinch_macros::rsx! { code {} };
            for child in children {
                code_elem.append_child(child);
            }
            let pre = rinch_macros::rsx! { pre { class: "rinch-code" } };
            pre.set_attribute("class", &class);
            pre.append_child(&code_elem);
            pre
        } else {
            let code_elem = rinch_macros::rsx! { code { class: "rinch-code" } };
            code_elem.set_attribute("class", &class);
            for child in children {
                code_elem.append_child(child);
            }
            code_elem
        }
    }
}
