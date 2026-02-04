//! SimpleGrid widget.
//!
//! A responsive grid layout that distributes children in equal-width columns.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A responsive grid layout component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     SimpleGrid { cols: 3, spacing: "lg",
///         Paper { "Card 1" }
///         Paper { "Card 2" }
///         Paper { "Card 3" }
///         Paper { "Card 4" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct SimpleGrid {
    /// Number of columns. Defaults to 1.
    pub cols: Option<u32>,
    /// Minimum column width for auto-fill behavior (e.g., "250px", "300px").
    /// When set, cols is ignored and grid uses auto-fill with minmax.
    pub min_child_width: Option<String>,
    /// Gap between grid items (xs, sm, md, lg, xl, or CSS value).
    pub spacing: Option<String>,
    /// Vertical gap between rows (xs, sm, md, lg, xl, or CSS value).
    /// Defaults to spacing if not provided.
    pub vertical_spacing: Option<String>,
}

impl SimpleGrid {
    /// Generate inline style for the grid.
    fn style_string(&self) -> String {
        let mut styles = vec!["display: grid".to_string()];

        // Grid template columns
        if let Some(ref min_width) = self.min_child_width {
            // Auto-fill with minimum width
            styles.push(format!(
                "grid-template-columns: repeat(auto-fill, minmax({}, 1fr))",
                min_width
            ));
        } else {
            // Fixed number of columns
            let cols = self.cols.unwrap_or(1);
            styles.push(format!("grid-template-columns: repeat({}, 1fr)", cols));
        }

        // Gap
        let gap = self.spacing_to_css(self.spacing.as_deref());
        let v_gap = self
            .vertical_spacing
            .as_deref()
            .map(|v| self.spacing_to_css(Some(v)))
            .unwrap_or_else(|| gap.clone());

        if gap == v_gap {
            styles.push(format!("gap: {}", gap));
        } else {
            styles.push(format!("row-gap: {}", v_gap));
            styles.push(format!("column-gap: {}", gap));
        }

        styles.join("; ")
    }

    fn spacing_to_css(&self, spacing: Option<&str>) -> String {
        match spacing {
            Some("xs") => "var(--rinch-spacing-xs)".to_string(),
            Some("sm") => "var(--rinch-spacing-sm)".to_string(),
            Some("md") => "var(--rinch-spacing-md)".to_string(),
            Some("lg") => "var(--rinch-spacing-lg)".to_string(),
            Some("xl") => "var(--rinch-spacing-xl)".to_string(),
            Some(val) => val.to_string(),
            None => "var(--rinch-spacing-md)".to_string(),
        }
    }
}

impl Widget for SimpleGrid {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-simple-grid" }
        };
        container.set_attribute("style", &self.style_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
