//! ColorSwatch component.
//!
//! Displays a colored square with a checkerboard background for alpha visibility.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};

/// A colored square that shows transparency via a checkerboard pattern.
#[derive(Default)]
pub struct ColorSwatch {
    /// CSS color value (e.g., "#ff0000", "rgba(255,0,0,0.5)").
    pub color: String,
    /// Size of the swatch (e.g., "28px"). Defaults to "28px".
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl, or CSS value). Defaults to "sm".
    pub radius: String,
    /// Whether to show a box shadow.
    pub with_shadow: bool,
    /// Click handler.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for ColorSwatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorSwatch")
            .field("color", &self.color)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("with_shadow", &self.with_shadow)
            .field("onclick", &self.onclick.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Component for ColorSwatch {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let size = if self.size.is_empty() {
            "28px"
        } else {
            &self.size
        };

        let radius = match self.radius.as_str() {
            "" | "sm" => "var(--rinch-radius-sm)",
            "xs" => "var(--rinch-radius-xs)",
            "md" => "var(--rinch-radius-md)",
            "lg" => "var(--rinch-radius-lg)",
            "xl" => "var(--rinch-radius-xl)",
            other => other,
        };

        let mut class = String::from("rinch-color-swatch");
        if self.with_shadow {
            class.push_str(" rinch-color-swatch--shadow");
        }
        if self.onclick.is_some() {
            class.push_str(" rinch-color-swatch--clickable");
        }

        let root = scope.create_element("div");
        root.set_attribute("class", &class);
        root.set_attribute(
            "style",
            &format!(
                "width: {}; height: {}; border-radius: {}",
                size, size, radius
            ),
        );

        if let Some(ref cb) = self.onclick {
            let cb = cb.clone();
            let handler_id = scope.register_handler(move || cb.invoke());
            root.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Inner overlay for the actual color
        let overlay = scope.create_element("div");
        overlay.set_attribute("class", "rinch-color-swatch__overlay");
        overlay.set_attribute(
            "style",
            &format!(
                "background-color: {}; border-radius: {}",
                self.color, radius
            ),
        );
        root.append_child(&overlay);

        for child in children {
            root.append_child(child);
        }

        root
    }
}
