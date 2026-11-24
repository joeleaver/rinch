//! Avatar component.
//!
//! Displays a user avatar with image or initials fallback.

use std::cell::Cell;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

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
/// Displays an image, or in its place a placeholder: the children if there
/// are any, else initials (`initials`, or the first letters of the first two
/// words of `name`), else an icon (`placeholder_icon`, a person by default).
/// An image that fails to load fires `on_error` and gives way to the
/// placeholder, as Mantine's does; `keep_failed_image` keeps it (showing its
/// `alt` text, as a browser does).
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Avatar { src: "/user.jpg", alt: "John Doe", name: "John Doe" }  // "JD" if the image fails
///     Avatar { name: "John Doe" }  // Shows "JD"
///     Avatar { initials: "J" }  // Shows "J"
///     Avatar { "?" }  // Shows the children
///     Avatar {}  // Shows the placeholder icon
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
    /// Initials to show, in place of the ones `name` gives.
    pub initials: String,
    /// The placeholder's icon, when there are no children and no initials.
    /// `None` is a person (`TablerIcon::User`).
    pub placeholder_icon: Option<TablerIcon>,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Color for initials background.
    pub color: String,
    /// Variant (filled, light, outline).
    pub variant: String,
    /// Runs when the image fails to load (the browser's `error` event).
    pub on_error: Option<Callback>,
    /// Keep an image that failed to load (its `alt` text shows) rather than
    /// replacing it with the placeholder.
    pub keep_failed_image: bool,
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

    /// The initials shown: `initials` as given, else the first letters of the
    /// first two words of `name`, uppercased.
    fn get_initials(&self) -> Option<String> {
        if !self.initials.is_empty() {
            return Some(self.initials.clone());
        }
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

    /// What stands in for the image: the children, else the initials, else
    /// the icon.
    fn placeholder(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        if !children.is_empty() {
            let span = rinch_macros::rsx! { span { class: "rinch-avatar__placeholder" } };
            for child in children {
                span.append_child(child);
            }
            span
        } else if let Some(initials) = self.get_initials() {
            rinch_macros::rsx! { span { class: "rinch-avatar__placeholder", {initials} } }
        } else {
            // It used to be an empty span, so `Avatar {}` showed a blank disc
            // where its docs promised an icon (#2202).
            let span = rinch_macros::rsx! { span { class: "rinch-avatar__placeholder-icon" } };
            let icon = self.placeholder_icon.unwrap_or(TablerIcon::User);
            let svg = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            // The glyph is 70% of the disc (the icon's own size is `1em`).
            svg.set_style("width", "70%");
            svg.set_style("height", "70%");
            span.append_child(&svg);
            span
        }
    }
}

impl Component for Avatar {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-avatar" } };
        container.set_attribute("class", &self.class_string());

        if !self.color.is_empty() {
            let style = crate::color_utils::accent_vars("--rinch-avatar", &self.color);
            container.set_attribute("style", &style);
        }

        let placeholder = self.placeholder(__scope, children);
        if !self.src.is_empty() {
            let src = &self.src;
            let alt = if self.alt.is_empty() { "" } else { &self.alt };
            let img = rinch_macros::rsx! { img { class: "rinch-avatar__image" } };
            img.set_attribute("src", src);
            img.set_attribute("alt", alt);
            // A load failure (#2202): `onerror` on the `<img>`.
            let fall_back = !self.keep_failed_image;
            if self.on_error.is_some() || fall_back {
                let on_error = self.on_error.clone();
                let img_handle = img.clone();
                let replaced = Cell::new(false);
                let handler = __scope.register_handler(move || {
                    if let Some(cb) = &on_error {
                        cb.invoke();
                    }
                    if fall_back && !replaced.get() {
                        replaced.set(true);
                        img_handle.replace_with(&placeholder);
                    }
                });
                img.set_attribute("data-onerror", &handler.0.to_string());
            }
            container.append_child(&img);
        } else {
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
