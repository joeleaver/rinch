//! Loader component.
//!
//! A loading indicator with multiple animation types.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Loader type (animation style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoaderType {
    /// Spinning oval/circle (default).
    #[default]
    Oval,
    /// Animated bars.
    Bars,
    /// Animated dots.
    Dots,
}

impl LoaderType {
    /// Get the CSS class name for this type.
    pub fn class_name(&self) -> &'static str {
        match self {
            LoaderType::Oval => "rinch-loader--oval",
            LoaderType::Bars => "rinch-loader--bars",
            LoaderType::Dots => "rinch-loader--dots",
        }
    }
}

impl std::str::FromStr for LoaderType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "oval" | "spinner" | "circle" => Ok(LoaderType::Oval),
            "bars" => Ok(LoaderType::Bars),
            "dots" => Ok(LoaderType::Dots),
            _ => Err(()),
        }
    }
}

/// Loader size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoaderSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl LoaderSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            LoaderSize::Xs => "rinch-loader--xs",
            LoaderSize::Sm => "rinch-loader--sm",
            LoaderSize::Md => "rinch-loader--md",
            LoaderSize::Lg => "rinch-loader--lg",
            LoaderSize::Xl => "rinch-loader--xl",
        }
    }
}

impl std::str::FromStr for LoaderSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(LoaderSize::Xs),
            "sm" => Ok(LoaderSize::Sm),
            "md" => Ok(LoaderSize::Md),
            "lg" => Ok(LoaderSize::Lg),
            "xl" => Ok(LoaderSize::Xl),
            _ => Err(()),
        }
    }
}

/// A loading indicator component.
///
/// Displays an animated spinner to indicate loading state.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Loader {}
///     Loader { r#type: "bars", size: "lg" }
///     Loader { r#type: "dots", color: "red" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Loader {
    /// Loader type (oval, bars, dots).
    pub r#type: String,
    /// Loader size (xs, sm, md, lg, xl).
    pub size: String,
    /// Color (uses theme colors or custom).
    pub color: String,
}

impl Loader {
    /// Generate the CSS class string for this loader.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-loader"];

        // Type class
        let loader_type: LoaderType = self.r#type.parse().unwrap_or_default();
        classes.push(loader_type.class_name());

        // Size class
        let size: LoaderSize = self.size.parse().unwrap_or_default();
        classes.push(size.class_name());

        classes.join(" ")
    }
}

impl Component for Loader {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { span { class: "rinch-loader", role: "status" } };
        container.set_attribute("class", &self.class_string());
        container.set_attribute("aria-label", "Loading");

        // Color style attribute
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-loader-color: {}", c)
            } else {
                format!("--rinch-loader-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        let loader_type: LoaderType = self.r#type.parse().unwrap_or_default();

        match loader_type {
            LoaderType::Oval => {
                let svg_placeholder = rinch_macros::rsx! { span { class: "rinch-loader__oval" } };
                container.append_child(&svg_placeholder);
            }
            LoaderType::Bars => {
                for _ in 0..4 {
                    let bar = rinch_macros::rsx! { span { class: "rinch-loader__bar" } };
                    container.append_child(&bar);
                }
            }
            LoaderType::Dots => {
                for _ in 0..3 {
                    let dot = rinch_macros::rsx! { span { class: "rinch-loader__dot" } };
                    container.append_child(&dot);
                }
            }
        }

        container
    }
}
