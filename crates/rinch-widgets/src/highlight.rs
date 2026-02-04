//! Highlight widget.
//!
//! Highlights matching substrings within text.

use rinch_core::Widget;
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
    pub text: Option<String>,
    /// The substring(s) to highlight.
    pub highlight: Option<String>,
    /// Background color for highlighted parts.
    pub color: Option<String>,
    /// Whether to do case-insensitive matching.
    pub ignore_case: bool,
}

impl Default for Highlight {
    fn default() -> Self {
        Self {
            text: None,
            highlight: None,
            color: None,
            ignore_case: true,
        }
    }
}

impl Widget for Highlight {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let text = self.text.as_deref().unwrap_or("");
        let highlight = self.highlight.as_deref().unwrap_or("");

        let container = rinch_macros::rsx! { span { class: "rinch-highlight" } };

        // Color style
        if let Some(c) = &self.color {
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-highlight-color: {}", c)
            } else {
                format!("--rinch-highlight-color: var(--rinch-color-{}-2)", c)
            };
            container.set_attribute("style", &style);
        }

        if highlight.is_empty() {
            let text_node = __scope.create_text(text);
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
                let before_text = __scope.create_text(&text[last_end..start]);
                container.append_child(&before_text);
            }

            // Add highlighted match (preserving original case)
            let end = start + highlight.len();
            let mark = rinch_macros::rsx! { mark { class: "rinch-highlight__match" } };
            let match_text = __scope.create_text(&text[start..end]);
            mark.append_child(&match_text);
            container.append_child(&mark);

            last_end = end;
        }

        // Add remaining text
        if last_end < text.len() {
            let remaining_text = __scope.create_text(&text[last_end..]);
            container.append_child(&remaining_text);
        }

        container
    }
}
