//! # Tabler Icons for Rinch
//!
//! This crate provides 5000+ free SVG icons from [Tabler Icons](https://tabler.io/icons)
//! for use in Rinch applications.
//!
//! ## Features
//!
//! - **5000+ icons** in both Outline and Filled styles
//! - **Type-safe** - Use enum variants instead of strings
//! - **Tree-shaking friendly** - Only icons you use are included in the binary
//! - **Consistent styling** - All icons render at 24x24 with `currentColor`
//!
//! ## Usage
//!
//! ```rust,ignore
//! use rinch::prelude::*;
//! use rinch_tabler_icons::{TablerIcon, render_tabler_icon};
//!
//! #[component]
//! fn my_component() -> NodeHandle {
//!     rsx! {
//!         div {
//!             {render_tabler_icon(__scope, TablerIcon::AlertCircle, Default::default())}
//!             "Alert!"
//!         }
//!     }
//! }
//! ```
//!
//! ## Styles
//!
//! Icons come in two styles:
//! - **Outline** (default) - Stroke-based line icons
//! - **Filled** - Solid filled icons
//!
//! ```rust,ignore
//! use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
//!
//! // Outline style (default)
//! render_tabler_icon(__scope, TablerIcon::Heart, Default::default());
//!
//! // Filled style
//! render_tabler_icon(__scope, TablerIcon::Heart, TablerIconStyle::Filled);
//! ```
//!
//! ## Credits
//!
//! Icons are from [Tabler Icons](https://tabler.io/icons) by Paweł Kuna,
//! licensed under the MIT License.

use rinch_core::dom::{NodeHandle, RenderScope};

// Include the generated code from build.rs
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

/// Options for rendering a Tabler icon.
#[derive(Debug, Clone, Default)]
pub struct TablerIconOptions {
    /// The style variant (Outline or Filled).
    pub style: TablerIconStyle,
    /// Optional CSS class to add to the SVG element.
    pub class: Option<String>,
    /// Size in pixels (default: 24).
    pub size: Option<u32>,
    /// Stroke width for outline icons (default: 2).
    pub stroke_width: Option<f32>,
}

/// Renders a Tabler icon to a NodeHandle.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `icon` - The icon to render
/// * `style` - The style variant (Outline or Filled)
///
/// # Returns
///
/// A NodeHandle containing the SVG element.
///
/// # Example
///
/// ```rust,ignore
/// let icon = render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline);
/// container.append_child(&icon);
/// ```
pub fn render_tabler_icon(
    scope: &mut RenderScope,
    icon: TablerIcon,
    style: TablerIconStyle,
) -> NodeHandle {
    render_tabler_icon_with_options(
        scope,
        icon,
        TablerIconOptions {
            style,
            ..Default::default()
        },
    )
}

/// Renders a Tabler icon with custom options.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `icon` - The icon to render
/// * `options` - Rendering options (style, class, size, stroke width)
///
/// # Returns
///
/// A NodeHandle containing the SVG element.
pub fn render_tabler_icon_with_options(
    scope: &mut RenderScope,
    icon: TablerIcon,
    options: TablerIconOptions,
) -> NodeHandle {
    let size = options.size.unwrap_or(24);
    let stroke_width = options.stroke_width.unwrap_or(2.0);

    // Get the paths for the requested style, falling back to the other style
    let (paths, is_filled) = match options.style {
        TablerIconStyle::Outline => {
            if let Some(p) = icon.outline_paths() {
                (p, false)
            } else if let Some(p) = icon.filled_paths() {
                (p, true)
            } else {
                // Return empty SVG if no paths available
                let svg = scope.create_element("svg");
                svg.set_attribute("viewBox", "0 0 24 24");
                svg.set_attribute("width", &size.to_string());
                svg.set_attribute("height", &size.to_string());
                svg.set_attribute(
                    "style",
                    &format!("width: {}px; height: {}px; flex-shrink: 0;", size, size),
                );
                return svg;
            }
        }
        TablerIconStyle::Filled => {
            if let Some(p) = icon.filled_paths() {
                (p, true)
            } else if let Some(p) = icon.outline_paths() {
                (p, false)
            } else {
                // Return empty SVG if no paths available
                let svg = scope.create_element("svg");
                svg.set_attribute("viewBox", "0 0 24 24");
                svg.set_attribute("width", &size.to_string());
                svg.set_attribute("height", &size.to_string());
                svg.set_attribute(
                    "style",
                    &format!("width: {}px; height: {}px; flex-shrink: 0;", size, size),
                );
                return svg;
            }
        }
    };

    // Create SVG element
    let svg = scope.create_element("svg");
    svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("width", &size.to_string());
    svg.set_attribute("height", &size.to_string());
    // Also set inline styles for width/height - SVG attributes alone don't affect CSS layout
    svg.set_attribute(
        "style",
        &format!("width: {}px; height: {}px; flex-shrink: 0;", size, size),
    );

    if is_filled {
        // Filled style: use fill, no stroke
        svg.set_attribute("fill", "currentColor");
        svg.set_attribute("stroke", "none");
    } else {
        // Outline style: use stroke, no fill
        svg.set_attribute("fill", "none");
        svg.set_attribute("stroke", "currentColor");
        svg.set_attribute("stroke-width", &stroke_width.to_string());
        svg.set_attribute("stroke-linecap", "round");
        svg.set_attribute("stroke-linejoin", "round");
    }

    // Add optional class
    if let Some(class) = &options.class {
        svg.set_attribute("class", class);
    }

    // Add paths
    for path_data in paths {
        let path = scope.create_element("path");
        path.set_attribute("d", path_data);
        svg.append_child(&path);
    }

    svg
}

/// Macro to create an icon with inline rendering.
///
/// This macro is a convenience wrapper around `render_tabler_icon`.
///
/// # Example
///
/// ```rust,ignore
/// // Basic usage
/// tabler_icon!(__scope, Home)
///
/// // With style
/// tabler_icon!(__scope, Home, Filled)
/// ```
#[macro_export]
macro_rules! tabler_icon {
    ($scope:expr, $icon:ident) => {
        $crate::render_tabler_icon(
            $scope,
            $crate::TablerIcon::$icon,
            $crate::TablerIconStyle::default(),
        )
    };
    ($scope:expr, $icon:ident, $style:ident) => {
        $crate::render_tabler_icon(
            $scope,
            $crate::TablerIcon::$icon,
            $crate::TablerIconStyle::$style,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_count() {
        const { assert!(ICON_COUNT > 4000) };
    }

    #[test]
    fn test_icon_name() {
        assert_eq!(TablerIcon::AlertCircle.name(), "alert-circle");
        assert_eq!(TablerIcon::Home.name(), "home");
    }

    #[test]
    fn test_icon_has_styles() {
        // Most icons have outline
        assert!(TablerIcon::AlertCircle.has_outline());
        assert!(TablerIcon::Home.has_outline());

        // Some icons have filled variants
        assert!(TablerIcon::Heart.has_filled());
    }

    #[test]
    fn test_icon_paths() {
        let paths = TablerIcon::AlertCircle.outline_paths();
        assert!(paths.is_some());
        assert!(!paths.unwrap().is_empty());
    }

    #[test]
    fn test_all_icons_iteration() {
        assert_eq!(ALL_ICONS.len(), ICON_COUNT);
    }
}
