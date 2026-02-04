//! Mark widget.
//!
//! Highlighted/marked text.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Marked/highlighted text component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     p {
///         "This is " Mark { "important" } " text."
///     }
///     Mark { color: "yellow", "highlighted" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Mark {
    /// Background color.
    pub color: Option<String>,
}

impl Widget for Mark {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { mark { class: "rinch-mark" } };

        if let Some(c) = &self.color {
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-mark-color: {}", c)
            } else {
                format!("--rinch-mark-color: var(--rinch-color-{}-2)", c)
            };
            container.set_attribute("style", &style);
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
