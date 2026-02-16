//! Alert widget.
//!
//! Displays a message with contextual feedback (info, success, warning, error).

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Widget, WidgetCallback};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Alert variant styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlertVariant {
    /// Light background with colored text (default).
    #[default]
    Light,
    /// Solid background with white text.
    Filled,
    /// Transparent background with colored border.
    Outline,
}

impl AlertVariant {
    /// Get the CSS class name for this variant.
    pub fn class_name(&self) -> &'static str {
        match self {
            AlertVariant::Light => "rinch-alert--light",
            AlertVariant::Filled => "rinch-alert--filled",
            AlertVariant::Outline => "rinch-alert--outline",
        }
    }
}

impl std::str::FromStr for AlertVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "light" => Ok(AlertVariant::Light),
            "filled" => Ok(AlertVariant::Filled),
            "outline" => Ok(AlertVariant::Outline),
            _ => Err(()),
        }
    }
}

/// Alert color (semantic meaning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlertColor {
    /// Blue informational alert.
    #[default]
    Blue,
    /// Green success alert.
    Green,
    /// Yellow warning alert.
    Yellow,
    /// Red error alert.
    Red,
    /// Gray neutral alert.
    Gray,
    /// Use theme primary color.
    Primary,
}

impl AlertColor {
    /// Get the CSS class name for this color.
    pub fn class_name(&self) -> &'static str {
        match self {
            AlertColor::Blue => "rinch-alert--blue",
            AlertColor::Green => "rinch-alert--green",
            AlertColor::Yellow => "rinch-alert--yellow",
            AlertColor::Red => "rinch-alert--red",
            AlertColor::Gray => "rinch-alert--gray",
            AlertColor::Primary => "rinch-alert--primary",
        }
    }
}

impl std::str::FromStr for AlertColor {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "blue" | "info" => Ok(AlertColor::Blue),
            "green" | "success" => Ok(AlertColor::Green),
            "yellow" | "warning" => Ok(AlertColor::Yellow),
            "red" | "error" => Ok(AlertColor::Red),
            "gray" => Ok(AlertColor::Gray),
            "primary" => Ok(AlertColor::Primary),
            _ => Err(()),
        }
    }
}

/// Alert radius size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlertRadius {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl AlertRadius {
    /// Get the CSS class name for this radius.
    pub fn class_name(&self) -> &'static str {
        match self {
            AlertRadius::Xs => "rinch-alert--radius-xs",
            AlertRadius::Sm => "rinch-alert--radius-sm",
            AlertRadius::Md => "rinch-alert--radius-md",
            AlertRadius::Lg => "rinch-alert--radius-lg",
            AlertRadius::Xl => "rinch-alert--radius-xl",
        }
    }
}

impl std::str::FromStr for AlertRadius {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(AlertRadius::Xs),
            "sm" => Ok(AlertRadius::Sm),
            "md" => Ok(AlertRadius::Md),
            "lg" => Ok(AlertRadius::Lg),
            "xl" => Ok(AlertRadius::Xl),
            _ => Err(()),
        }
    }
}

/// A feedback alert message component.
///
/// Displays contextual information to users with different severity levels.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Alert { color: "green", title: "Success", icon: Icon::CheckCircle,
///         "Your changes have been saved."
///     }
///     Alert { color: "red", variant: "filled", title: "Error", icon: Icon::XCircle,
///         "Something went wrong. Please try again."
///     }
/// }
/// ```
#[derive(Default)]
pub struct Alert {
    /// Alert color (blue, green, yellow, red, gray, primary).
    pub color: Option<String>,
    /// Alert variant (light, filled, outline).
    pub variant: Option<String>,
    /// Alert title (optional).
    pub title: Option<String>,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: Option<String>,
    /// Whether to show a close button.
    pub with_close_button: bool,
    /// Icon to display (optional).
    pub icon: Option<TablerIcon>,
    /// Callback when close button is clicked.
    pub onclose: Option<WidgetCallback>,
}

impl std::fmt::Debug for Alert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alert")
            .field("color", &self.color)
            .field("variant", &self.variant)
            .field("title", &self.title)
            .field("radius", &self.radius)
            .field("with_close_button", &self.with_close_button)
            .field("icon", &self.icon)
            .field("onclose", &self.onclose.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Alert {
    /// Generate the CSS class string for this alert.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-alert"];

        // Variant class
        let variant: AlertVariant = self
            .variant
            .as_ref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        classes.push(variant.class_name());

        // Color class
        let color: AlertColor = self
            .color
            .as_ref()
            .and_then(|c| c.parse().ok())
            .unwrap_or_default();
        classes.push(color.class_name());

        // Radius class (only if non-default)
        if let Some(ref r) = self.radius
            && let Ok(radius) = r.parse::<AlertRadius>()
        {
            classes.push(radius.class_name());
        }

        // With title
        if self.title.is_some() {
            classes.push("rinch-alert--with-title");
        }

        // With icon
        if self.icon.is_some() {
            classes.push("rinch-alert--with-icon");
        }

        classes.join(" ")
    }
}

impl Widget for Alert {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-alert" } };
        container.set_attribute("class", &self.class_string());

        // Icon
        if let Some(icon) = &self.icon {
            let icon_div = rinch_macros::rsx! { div { class: "rinch-alert__icon" } };
            let icon_svg = render_tabler_icon(__scope, *icon, TablerIconStyle::Outline);
            icon_div.append_child(&icon_svg);
            container.append_child(&icon_div);
        }

        // Wrapper for content
        let wrapper = rinch_macros::rsx! { div { class: "rinch-alert__wrapper" } };

        // Title
        if let Some(title) = &self.title {
            let title_div = rinch_macros::rsx! { div { class: "rinch-alert__title" } };
            let title_text = __scope.create_text(title);
            title_div.append_child(&title_text);
            wrapper.append_child(&title_div);
        }

        // Message/children
        let message_div = rinch_macros::rsx! { div { class: "rinch-alert__message" } };
        for child in children {
            message_div.append_child(child);
        }
        wrapper.append_child(&message_div);

        container.append_child(&wrapper);

        // Close button
        if self.with_close_button {
            let btn =
                rinch_macros::rsx! { button { class: "rinch-alert__close", r#type: "button" } };
            btn.set_attribute("aria-label", "Close");
            let close_icon = crate::icons::close_icon_dom(__scope, "14");
            btn.append_child(&close_icon);

            if let Some(cb) = &self.onclose {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                btn.set_attribute("data-rid", &handler_id.0.to_string());
            }

            container.append_child(&btn);
        }

        container
    }
}
