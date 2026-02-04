//! Badge widget.
//!
//! A small status indicator with text.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Badge variant styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    /// Solid background with primary color.
    #[default]
    Filled,
    /// Light background with darker text.
    Light,
    /// Transparent background with colored border.
    Outline,
    /// Gray with a colored dot indicator.
    Dot,
}

impl BadgeVariant {
    /// Get the CSS class name for this variant.
    pub fn class_name(&self) -> &'static str {
        match self {
            BadgeVariant::Filled => "rinch-badge--filled",
            BadgeVariant::Light => "rinch-badge--light",
            BadgeVariant::Outline => "rinch-badge--outline",
            BadgeVariant::Dot => "rinch-badge--dot",
        }
    }
}

impl std::str::FromStr for BadgeVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "filled" => Ok(BadgeVariant::Filled),
            "light" => Ok(BadgeVariant::Light),
            "outline" => Ok(BadgeVariant::Outline),
            "dot" => Ok(BadgeVariant::Dot),
            _ => Err(()),
        }
    }
}

/// Badge size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl BadgeSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            BadgeSize::Xs => "rinch-badge--xs",
            BadgeSize::Sm => "rinch-badge--sm",
            BadgeSize::Md => "rinch-badge--md",
            BadgeSize::Lg => "rinch-badge--lg",
            BadgeSize::Xl => "rinch-badge--xl",
        }
    }
}

impl std::str::FromStr for BadgeSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(BadgeSize::Xs),
            "sm" => Ok(BadgeSize::Sm),
            "md" => Ok(BadgeSize::Md),
            "lg" => Ok(BadgeSize::Lg),
            "xl" => Ok(BadgeSize::Xl),
            _ => Err(()),
        }
    }
}

/// A small status indicator badge.
#[derive(Debug, Default)]
pub struct Badge {
    /// Badge variant (filled, light, outline, dot).
    pub variant: Option<String>,
    /// Badge size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Override the primary color.
    pub color: Option<String>,
    /// Border radius override.
    pub radius: Option<String>,
    /// Whether the badge should take full width.
    pub full_width: bool,
}

impl Badge {
    /// Generate the CSS class string for this badge.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-badge"];

        // Size class
        let size: BadgeSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        // Variant class
        let variant: BadgeVariant = self
            .variant
            .as_ref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        classes.push(variant.class_name());

        // Full width
        if self.full_width {
            classes.push("rinch-badge--full-width");
        }

        classes.join(" ")
    }
}

impl Widget for Badge {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { span { class: "rinch-badge" } };
        container.set_attribute("class", &self.class_string());

        if let Some(color) = &self.color {
            container.set_attribute("data-color", color);
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
