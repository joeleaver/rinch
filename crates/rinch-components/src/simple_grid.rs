//! SimpleGrid component.
//!
//! A responsive grid layout that distributes children in equal-width columns.

use rinch_core::Component;
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
    /// The custom properties the grid's props set, for its inline style.
    ///
    /// Only these go inline. The grid itself (`display`, the template, the
    /// gaps) is the `.rinch-simple-grid` rule, which reads them with today's
    /// defaults as fallbacks, so an app stylesheet can still change the
    /// layout: a breakpoint rule on the class (`.my-grid.rinch-simple-grid {
    /// grid-template-columns: 1fr }` in an `@media`), or on the variables when
    /// the prop is unset. They used to be written inline, defaults included,
    /// where no stylesheet could reach them (#2207).
    fn style_vars(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();

        if !self.min_child_width.is_empty() {
            vars.push((
                "--rinch-simple-grid-template",
                format!("repeat(auto-fill, minmax({}, 1fr))", self.min_child_width),
            ));
        } else if let Some(cols) = self.cols {
            vars.push(("--rinch-simple-grid-cols", cols.to_string()));
        }

        if !self.spacing.is_empty() {
            vars.push((
                "--rinch-simple-grid-gap",
                self.spacing_to_css(&self.spacing),
            ));
        }
        if !self.vertical_spacing.is_empty() {
            vars.push((
                "--rinch-simple-grid-row-gap",
                self.spacing_to_css(&self.vertical_spacing),
            ));
        }

        vars
    }

    fn spacing_to_css(&self, spacing: &str) -> String {
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

impl Component for SimpleGrid {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-simple-grid" }
        };
        let vars = self.style_vars();
        if !vars.is_empty() {
            let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            container.set_styles(&pairs);
        }
        for child in children {
            container.append_child(child);
        }
        container
    }
}
