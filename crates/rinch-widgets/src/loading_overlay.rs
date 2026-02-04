//! LoadingOverlay widget.
//!
//! An overlay with a loader that covers its parent container.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A loading overlay that covers its parent container.
///
/// Unlike Modal/Drawer, this doesn't use a portal - it overlays
/// its immediate parent container.
///
/// # Example
///
/// ```ignore
/// let loading = use_signal(|| true);
///
/// rsx! {
///     div { style: "position: relative;",
///         LoadingOverlay {
///             visible: loading.get(),
///             // Content is still rendered but obscured
///         }
///         // Your content here
///         Text { "Content that will be covered" }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct LoadingOverlay {
    /// Whether the overlay is visible.
    pub visible: bool,
    /// Overlay background color/opacity.
    pub overlay_opacity: Option<f32>,
    /// Overlay blur amount.
    pub overlay_blur: Option<String>,
    /// Loader type (oval, bars, dots).
    pub loader_type: Option<String>,
    /// Loader size.
    pub loader_size: Option<String>,
    /// Loader color.
    pub loader_color: Option<String>,
    /// Border radius (matches parent container).
    pub radius: Option<String>,
    /// Z-index relative to siblings.
    pub z_index: Option<i32>,
    /// Transition duration.
    pub transition_duration: Option<u32>,
}

impl Default for LoadingOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            overlay_opacity: None,
            overlay_blur: None,
            loader_type: None,
            loader_size: None,
            loader_color: None,
            radius: None,
            z_index: None,
            transition_duration: None,
        }
    }
}

impl LoadingOverlay {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-loading-overlay"];

        if self.visible {
            classes.push("rinch-loading-overlay--visible");
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
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

impl Widget for LoadingOverlay {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        let mut style_parts = Vec::new();
        if let Some(opacity) = self.overlay_opacity {
            style_parts.push(format!("--rinch-loading-overlay-opacity: {}", opacity));
        }
        if let Some(ref blur) = self.overlay_blur {
            style_parts.push(format!("--rinch-loading-overlay-blur: {}", blur));
        }
        if let Some(ref color) = self.loader_color {
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
        let loader_type = self.loader_type.as_deref().unwrap_or("oval");
        let loader_class = match loader_type {
            "bars" => "rinch-loader--bars",
            "dots" => "rinch-loader--dots",
            _ => "rinch-loader--oval",
        };

        let loader_size_class = self
            .loader_size
            .as_ref()
            .map(|s| match s.as_str() {
                "xs" => " rinch-loader--xs",
                "sm" => " rinch-loader--sm",
                "lg" => " rinch-loader--lg",
                "xl" => " rinch-loader--xl",
                _ => "",
            })
            .unwrap_or("");

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
