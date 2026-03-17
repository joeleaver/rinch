//! Highlight component.
//!
//! Highlights matching substrings within text.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Highlights matching parts of text.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Highlight {
///         text: "Search results for query",
///         highlight: "query",
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Highlight {
    /// The full text to display.
    pub text: String,
    /// The substring(s) to highlight.
    pub highlight: String,
    /// Background color for highlighted parts.
    pub color: String,
    /// Whether to do case-insensitive matching.
    pub ignore_case: bool,
}

impl Default for Highlight {
    fn default() -> Self {
        Self {
            text: String::new(),
            highlight: String::new(),
            color: String::new(),
            ignore_case: true,
        }
    }
}

impl Component for Highlight {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let text = &self.text;
        let highlight = &self.highlight;

        let container = rinch_macros::rsx! { span { class: "rinch-highlight" } };

        // Color style
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-highlight-color: {}", c)
            } else {
                format!("--rinch-highlight-color: var(--rinch-color-{}-2)", c)
            };
            container.set_attribute("style", &style);
        }

        if highlight.is_empty() {
            let text_node = rinch_core::IntoNode::into_node(text, __scope);
            container.append_child(&text_node);
            return container;
        }

        // Find and highlight matches
        let mut last_end = 0;

        let search_text = if self.ignore_case {
            text.to_lowercase()
        } else {
            text.to_string()
        };

        let search_highlight = if self.ignore_case {
            highlight.to_lowercase()
        } else {
            highlight.to_string()
        };

        for (start, _) in search_text.match_indices(&search_highlight) {
            // Add text before match
            if start > last_end {
                let before_text =
                    rinch_core::IntoNode::into_node(text[last_end..start].to_string(), __scope);
                container.append_child(&before_text);
            }

            // Add highlighted match (preserving original case)
            let end = start + highlight.len();
            let mark = rinch_macros::rsx! { mark { class: "rinch-highlight__match", {text[start..end].to_string()} } };
            container.append_child(&mark);

            last_end = end;
        }

        // Add remaining text
        if last_end < text.len() {
            let remaining_text =
                rinch_core::IntoNode::into_node(text[last_end..].to_string(), __scope);
            container.append_child(&remaining_text);
        }

        container
    }
}
