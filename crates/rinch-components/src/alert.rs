//! Alert component.
//!
//! Displays a message with contextual feedback (info, success, warning, error).

use crate::borderless_window::SectionRenderer;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
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
/// `color` takes any theme palette name (`violet`, `teal`, …), `primary`, the
/// semantic names (`info`, `success`, `warning`, `error`), or a raw CSS colour
/// (`#7048e8`, `rgb(…)`, `hsl(…)`). The root carries `role="alert"`, as
/// Mantine's does; `role` replaces it (`"status"` for a polite live region,
/// `"none"` for no live region).
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Alert { color: "green", title: "Success", icon: TablerIcon::CircleCheck,
///         "Your changes have been saved."
///     }
///     Alert { color: "red", variant: "filled", title: "Error", icon: TablerIcon::CircleX,
///         "Something went wrong. Please try again."
///     }
/// }
/// ```
#[derive(Default)]
pub struct Alert {
    /// Alert color: a palette name, `primary`, a semantic name (`info`,
    /// `success`, `warning`, `error`), or a raw CSS colour. Empty is blue.
    pub color: String,
    /// Alert variant (light, filled, outline).
    pub variant: String,
    /// Alert title (optional).
    pub title: String,
    /// The title as nodes, in place of the `title` text: rendered inside
    /// `.rinch-alert__title`. Wins over `title` when both are set.
    pub title_section: Option<SectionRenderer>,
    /// The root's ARIA `role`. Empty (the default) is `"alert"`.
    pub role: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether to show a close button.
    pub with_close_button: bool,
    /// The close button's accessible name (`aria-label`). Empty (the default)
    /// is `"Close"`.
    pub close_button_label: String,
    /// Icon to display (optional).
    pub icon: Option<TablerIcon>,
    /// Callback when close button is clicked.
    pub onclose: Option<Callback>,
}

impl std::fmt::Debug for Alert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alert")
            .field("color", &self.color)
            .field("variant", &self.variant)
            .field("title", &self.title)
            .field(
                "title_section",
                &self.title_section.as_ref().map(|_| "<renderer>"),
            )
            .field("role", &self.role)
            .field("radius", &self.radius)
            .field("with_close_button", &self.with_close_button)
            .field("close_button_label", &self.close_button_label)
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
        let variant: AlertVariant = if self.variant.is_empty() {
            AlertVariant::default()
        } else {
            self.variant.parse().unwrap_or_default()
        };
        classes.push(variant.class_name());

        // Color class: a built-in name's class sets the colour variables; any
        // other colour sets them inline (`style_string`).
        match self.builtin_color() {
            Some(color) => classes.push(color.class_name()),
            None => classes.push("rinch-alert--colored"),
        }

        // Radius class (only if non-default)
        if !self.radius.is_empty() {
            if let Ok(radius) = self.radius.parse::<AlertRadius>() {
                classes.push(radius.class_name());
            }
        }

        // With title
        if self.has_title() {
            classes.push("rinch-alert--with-title");
        }

        // With icon
        if self.icon.is_some() {
            classes.push("rinch-alert--with-icon");
        }

        classes.join(" ")
    }

    /// The built-in colour `color` names, or `None` for any other colour
    /// (another palette name, an app's token family, a raw CSS colour).
    fn builtin_color(&self) -> Option<AlertColor> {
        if self.color.is_empty() {
            Some(AlertColor::default())
        } else {
            self.color.parse().ok()
        }
    }

    fn has_title(&self) -> bool {
        self.title_section.is_some() || !self.title.is_empty()
    }

    /// The inline colour variables for a colour that is not one of the
    /// built-in names (`None` for those: their class sets the variables).
    /// It sets every variable, so nothing is inherited from an enclosing
    /// Alert.
    pub fn style_string(&self) -> Option<String> {
        if self.builtin_color().is_some() {
            return None;
        }
        let t = crate::color_utils::color_tokens(&self.color);
        Some(format!(
            "--rinch-alert-color: {}; --rinch-alert-light: {}; \
             --rinch-alert-light-text: {}; --rinch-alert-light-strong: {}; \
             --rinch-alert-filled-text: {}; --rinch-alert-outline-text: {};",
            t.base, t.light, t.light_text, t.light_strong, t.contrast, t.base
        ))
    }
}

impl Component for Alert {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-alert" } };
        container.set_attribute("class", &self.class_string());
        if let Some(style) = self.style_string() {
            container.set_attribute("style", &style);
        }
        let role = if self.role.is_empty() {
            "alert"
        } else {
            &self.role
        };
        container.set_attribute("role", role);

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
        if let Some(render) = &self.title_section {
            let title_div = rinch_macros::rsx! { div { class: "rinch-alert__title" } };
            title_div.append_child(&render(__scope));
            wrapper.append_child(&title_div);
        } else if !self.title.is_empty() {
            let title_div =
                rinch_macros::rsx! { div { class: "rinch-alert__title", {self.title.clone()} } };
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
            let label = if self.close_button_label.is_empty() {
                crate::close_button::DEFAULT_CLOSE_LABEL
            } else {
                &self.close_button_label
            };
            btn.set_attribute("aria-label", label);
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
