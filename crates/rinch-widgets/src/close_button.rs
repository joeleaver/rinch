//! CloseButton widget.
//!
//! A button with a close/X icon for dismissing elements.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// CloseButton size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseButtonSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl CloseButtonSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            CloseButtonSize::Xs => "rinch-close-button--xs",
            CloseButtonSize::Sm => "rinch-close-button--sm",
            CloseButtonSize::Md => "rinch-close-button--md",
            CloseButtonSize::Lg => "rinch-close-button--lg",
            CloseButtonSize::Xl => "rinch-close-button--xl",
        }
    }

    /// Get the icon size for this button size.
    pub fn icon_size(&self) -> &'static str {
        match self {
            CloseButtonSize::Xs => "12",
            CloseButtonSize::Sm => "14",
            CloseButtonSize::Md => "16",
            CloseButtonSize::Lg => "20",
            CloseButtonSize::Xl => "24",
        }
    }
}

impl std::str::FromStr for CloseButtonSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(CloseButtonSize::Xs),
            "sm" => Ok(CloseButtonSize::Sm),
            "md" => Ok(CloseButtonSize::Md),
            "lg" => Ok(CloseButtonSize::Lg),
            "xl" => Ok(CloseButtonSize::Xl),
            _ => Err(()),
        }
    }
}

/// A close/dismiss button component.
///
/// Displays an X icon button typically used to close modals, alerts, etc.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     CloseButton {}
///     CloseButton { size: "lg" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct CloseButton {
    /// Button size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Border radius override (xs, sm, md, lg, xl).
    pub radius: Option<String>,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Custom icon size in pixels.
    pub icon_size: Option<u32>,
}

impl CloseButton {
    /// Generate the CSS class string for this close button.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-close-button"];

        // Size class
        let size: CloseButtonSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        // Radius class
        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-close-button--radius-xs"),
                "sm" => classes.push("rinch-close-button--radius-sm"),
                "md" => classes.push("rinch-close-button--radius-md"),
                "lg" => classes.push("rinch-close-button--radius-lg"),
                "xl" => classes.push("rinch-close-button--radius-xl"),
                _ => {}
            }
        }

        if self.disabled {
            classes.push("rinch-close-button--disabled");
        }

        classes.join(" ")
    }
}

impl Widget for CloseButton {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let icon_span = rinch_macros::rsx! { span { class: "rinch-icon rinch-icon--close" } };
        let container = rinch_macros::rsx! {
            button { class: "rinch-close-button", r#type: "button" }
        };
        container.set_attribute("class", &self.class_string());
        container.set_attribute("aria-label", "Close");

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        container.append_child(&icon_span);
        container
    }
}
