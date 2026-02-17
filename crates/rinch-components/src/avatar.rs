//! Avatar component.
//!
//! Displays a user avatar with image or initials fallback.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Avatar size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl AvatarSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            AvatarSize::Xs => "rinch-avatar--xs",
            AvatarSize::Sm => "rinch-avatar--sm",
            AvatarSize::Md => "rinch-avatar--md",
            AvatarSize::Lg => "rinch-avatar--lg",
            AvatarSize::Xl => "rinch-avatar--xl",
        }
    }
}

impl std::str::FromStr for AvatarSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(AvatarSize::Xs),
            "sm" => Ok(AvatarSize::Sm),
            "md" => Ok(AvatarSize::Md),
            "lg" => Ok(AvatarSize::Lg),
            "xl" => Ok(AvatarSize::Xl),
            _ => Err(()),
        }
    }
}

/// Avatar radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarRadius {
    Xs,
    Sm,
    Md,
    Lg,
    #[default]
    Xl,
}

impl AvatarRadius {
    pub fn class_name(&self) -> &'static str {
        match self {
            AvatarRadius::Xs => "rinch-avatar--radius-xs",
            AvatarRadius::Sm => "rinch-avatar--radius-sm",
            AvatarRadius::Md => "rinch-avatar--radius-md",
            AvatarRadius::Lg => "rinch-avatar--radius-lg",
            AvatarRadius::Xl => "rinch-avatar--radius-xl",
        }
    }
}

impl std::str::FromStr for AvatarRadius {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(AvatarRadius::Xs),
            "sm" => Ok(AvatarRadius::Sm),
            "md" => Ok(AvatarRadius::Md),
            "lg" => Ok(AvatarRadius::Lg),
            "xl" => Ok(AvatarRadius::Xl),
            _ => Err(()),
        }
    }
}

/// Avatar variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarVariant {
    #[default]
    Filled,
    Light,
    Outline,
}

impl AvatarVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            AvatarVariant::Filled => "rinch-avatar--filled",
            AvatarVariant::Light => "rinch-avatar--light",
            AvatarVariant::Outline => "rinch-avatar--outline",
        }
    }
}

impl std::str::FromStr for AvatarVariant {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "filled" => Ok(AvatarVariant::Filled),
            "light" => Ok(AvatarVariant::Light),
            "outline" => Ok(AvatarVariant::Outline),
            _ => Err(()),
        }
    }
}

/// A user avatar component.
///
/// Displays an image, initials, or placeholder icon.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Avatar { src: "/user.jpg", alt: "John Doe" }
///     Avatar { name: "John Doe" }  // Shows "JD"
///     Avatar {}  // Shows placeholder icon
/// }
/// ```
#[derive(Debug, Default)]
pub struct Avatar {
    /// Image source URL.
    pub src: String,
    /// Alt text for the image.
    pub alt: String,
    /// User name (used to generate initials).
    pub name: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Color for initials background.
    pub color: String,
    /// Variant (filled, light, outline).
    pub variant: String,
}

impl Avatar {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-avatar"];

        let size: AvatarSize = if self.size.is_empty() {
            AvatarSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        let radius: AvatarRadius = if self.radius.is_empty() {
            AvatarRadius::default()
        } else {
            self.radius.parse().unwrap_or_default()
        };
        classes.push(radius.class_name());

        let variant: AvatarVariant = if self.variant.is_empty() {
            AvatarVariant::default()
        } else {
            self.variant.parse().unwrap_or_default()
        };
        classes.push(variant.class_name());

        classes.join(" ")
    }

    fn get_initials(&self) -> Option<String> {
        if self.name.is_empty() {
            None
        } else {
            Some(
                self.name
                    .split_whitespace()
                    .filter_map(|word| word.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase(),
            )
        }
    }
}

impl Component for Avatar {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-avatar" } };
        container.set_attribute("class", &self.class_string());

        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-avatar-color: {}", c)
            } else {
                format!("--rinch-avatar-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        if !self.src.is_empty() {
            let src = &self.src;
            let alt = if self.alt.is_empty() { "" } else { &self.alt };
            let img = rinch_macros::rsx! { img { class: "rinch-avatar__image" } };
            img.set_attribute("src", src);
            img.set_attribute("alt", alt);
            container.append_child(&img);
        } else if let Some(initials) = self.get_initials() {
            let span = rinch_macros::rsx! { span { class: "rinch-avatar__placeholder" } };
            let text_node = __scope.create_text(&initials);
            span.append_child(&text_node);
            container.append_child(&span);
        } else {
            let placeholder =
                rinch_macros::rsx! { span { class: "rinch-avatar__placeholder-icon" } };
            container.append_child(&placeholder);
        }

        container
    }
}

/// A group of overlapping avatars.
#[derive(Debug, Default)]
pub struct AvatarGroup {
    /// Spacing between avatars (negative for overlap).
    pub spacing: String,
}

impl Component for AvatarGroup {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-avatar-group" } };

        if !self.spacing.is_empty() {
            let spacing = &self.spacing;
            container.set_attribute(
                "style",
                &format!("--rinch-avatar-group-spacing: {}", spacing),
            );
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
