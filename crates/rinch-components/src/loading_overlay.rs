//! LoadingOverlay component.
//!
//! An overlay with a loader that covers its parent container.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A loading overlay that covers its parent container.
///
/// Unlike Modal/Drawer, this doesn't use a portal - it overlays
/// its immediate parent container.
///
/// # The parent must be positioned
///
/// The overlay is `position: absolute; inset: 0`, so it covers **its containing
/// block** — which per CSS is the nearest *positioned* ancestor. Give the
/// container `position: relative` (as the example does). Without it the
/// containing block is the viewport and the overlay covers the whole window,
/// on desktop as in the browser (issue #204 — before it, desktop covered the
/// unpositioned parent instead, and a missing `position: relative` went
/// unnoticed there while already misbehaving on `rinch-web`).
///
/// # Example
///
/// ```ignore
/// let loading = Signal::new(true);
///
/// rsx! {
///     div { style: "position: relative;",   // required
///         LoadingOverlay {
///             visible: loading.get(),
///             // Content is still rendered but obscured
///         }
///         // Your content here
///         Text { "Content that will be covered" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct LoadingOverlay {
    /// Whether the overlay is visible.
    pub visible: bool,
    /// Overlay background color/opacity.
    pub overlay_opacity: Option<f32>,
    /// Overlay blur amount.
    pub overlay_blur: String,
    /// Loader type (oval, bars, dots).
    pub loader_type: String,
    /// Loader size.
    pub loader_size: String,
    /// Loader color.
    pub loader_color: String,
    /// Border radius (matches parent container).
    pub radius: String,
    /// Z-index relative to siblings.
    pub z_index: Option<i32>,
    /// Transition duration.
    pub transition_duration: Option<u32>,
}

impl LoadingOverlay {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-loading-overlay"];

        if self.visible {
            classes.push("rinch-loading-overlay--visible");
        }

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-loading-overlay--radius-xs"),
                "sm" => classes.push("rinch-loading-overlay--radius-sm"),
                "md" => classes.push("rinch-loading-overlay--radius-md"),
                "lg" => classes.push("rinch-loading-overlay--radius-lg"),
                "xl" => classes.push("rinch-loading-overlay--radius-xl"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Component for LoadingOverlay {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        let mut style_parts = Vec::new();
        if let Some(opacity) = self.overlay_opacity {
            style_parts.push(format!("--rinch-loading-overlay-opacity: {}", opacity));
        }
        if !self.overlay_blur.is_empty() {
            style_parts.push(format!(
                "--rinch-loading-overlay-blur: {}",
                self.overlay_blur
            ));
        }
        if !self.loader_color.is_empty() {
            let color = &self.loader_color;
            if color.starts_with('#') || color.starts_with("rgb") || color.starts_with("hsl") {
                style_parts.push(format!("--rinch-loading-overlay-loader-color: {}", color));
            } else {
                style_parts.push(format!(
                    "--rinch-loading-overlay-loader-color: var(--rinch-color-{}-6)",
                    color
                ));
            }
        }
        if let Some(z) = self.z_index {
            style_parts.push(format!("z-index: {}", z));
        }
        if let Some(duration) = self.transition_duration {
            style_parts.push(format!(
                "--rinch-loading-overlay-transition: {}ms",
                duration
            ));
        }

        // Determine loader class
        let loader_type = if self.loader_type.is_empty() {
            "oval"
        } else {
            &self.loader_type
        };
        let loader_class = match loader_type {
            "bars" => "rinch-loader--bars",
            "dots" => "rinch-loader--dots",
            _ => "rinch-loader--oval",
        };

        let loader_size_class = if self.loader_size.is_empty() {
            ""
        } else {
            match self.loader_size.as_str() {
                "xs" => " rinch-loader--xs",
                "sm" => " rinch-loader--sm",
                "lg" => " rinch-loader--lg",
                "xl" => " rinch-loader--xl",
                _ => "",
            }
        };

        let root = rinch_macros::rsx! { div { class: "rinch-loading-overlay" } };
        root.set_attribute("class", &class);

        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        let overlay_el = rinch_macros::rsx! { div { class: "rinch-loading-overlay__overlay" } };
        root.append_child(&overlay_el);

        let loader_el = rinch_macros::rsx! { div { class: "rinch-loader" } };
        loader_el.set_attribute(
            "class",
            &format!("rinch-loader {}{}", loader_class, loader_size_class),
        );
        root.append_child(&loader_el);

        root
    }
}
