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
    pub min_child_width: String,
    /// Gap between grid items (xs, sm, md, lg, xl, or CSS value).
    pub spacing: String,
    /// Vertical gap between rows (xs, sm, md, lg, xl, or CSS value).
    /// Defaults to spacing if not provided.
    pub vertical_spacing: String,
}

impl SimpleGrid {
    /// Generate inline style for the grid.
    fn style_string(&self) -> String {
        let mut styles = vec!["display: grid".to_string()];

        // Grid template columns
        if !self.min_child_width.is_empty() {
            // Auto-fill with minimum width
            styles.push(format!(
                "grid-template-columns: repeat(auto-fill, minmax({}, 1fr))",
                self.min_child_width
            ));
        } else {
            // Fixed number of columns
            let cols = self.cols.unwrap_or(1);
            styles.push(format!("grid-template-columns: repeat({}, 1fr)", cols));
        }

        // Gap
        let gap = self.spacing_to_css(&self.spacing);
        let v_gap = if self.vertical_spacing.is_empty() {
            gap.clone()
        } else {
            self.spacing_to_css(&self.vertical_spacing)
        };

        if gap == v_gap {
            styles.push(format!("gap: {}", gap));
        } else {
            styles.push(format!("row-gap: {}", v_gap));
            styles.push(format!("column-gap: {}", gap));
        }

        styles.join("; ")
    }

    fn spacing_to_css(&self, spacing: &str) -> String {
        if spacing.is_empty() {
            return "var(--rinch-spacing-md)".to_string();
        }
        match spacing {
            "xs" => "var(--rinch-spacing-xs)".to_string(),
            "sm" => "var(--rinch-spacing-sm)".to_string(),
            "md" => "var(--rinch-spacing-md)".to_string(),
            "lg" => "var(--rinch-spacing-lg)".to_string(),
            "xl" => "var(--rinch-spacing-xl)".to_string(),
            val => val.to_string(),
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
