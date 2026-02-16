//! ActionIcon widget.
//!
//! An icon-only button for actions.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Widget, WidgetCallback};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// ActionIcon variant styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionIconVariant {
    /// Solid background with primary color.
    #[default]
    Filled,
    /// Light background.
    Light,
    /// Transparent background with border.
    Outline,
    /// Transparent background.
    Subtle,
    /// Completely transparent.
    Transparent,
    /// Default gray style.
    Default,
}

impl ActionIconVariant {
    /// Get the CSS class name for this variant.
    pub fn class_name(&self) -> &'static str {
        match self {
            ActionIconVariant::Filled => "rinch-action-icon--filled",
            ActionIconVariant::Light => "rinch-action-icon--light",
            ActionIconVariant::Outline => "rinch-action-icon--outline",
            ActionIconVariant::Subtle => "rinch-action-icon--subtle",
            ActionIconVariant::Transparent => "rinch-action-icon--transparent",
            ActionIconVariant::Default => "rinch-action-icon--default",
        }
    }
}

impl std::str::FromStr for ActionIconVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "filled" => Ok(ActionIconVariant::Filled),
            "light" => Ok(ActionIconVariant::Light),
            "outline" => Ok(ActionIconVariant::Outline),
            "subtle" => Ok(ActionIconVariant::Subtle),
            "transparent" => Ok(ActionIconVariant::Transparent),
            "default" => Ok(ActionIconVariant::Default),
            _ => Err(()),
        }
    }
}

/// ActionIcon size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionIconSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl ActionIconSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            ActionIconSize::Xs => "rinch-action-icon--xs",
            ActionIconSize::Sm => "rinch-action-icon--sm",
            ActionIconSize::Md => "rinch-action-icon--md",
            ActionIconSize::Lg => "rinch-action-icon--lg",
            ActionIconSize::Xl => "rinch-action-icon--xl",
        }
    }

    /// Get the icon size in pixels for this button size.
    pub fn icon_size(&self) -> &'static str {
        match self {
            ActionIconSize::Xs => "12",
            ActionIconSize::Sm => "14",
            ActionIconSize::Md => "18",
            ActionIconSize::Lg => "22",
            ActionIconSize::Xl => "28",
        }
    }
}

impl std::str::FromStr for ActionIconSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ActionIconSize::Xs),
            "sm" => Ok(ActionIconSize::Sm),
            "md" => Ok(ActionIconSize::Md),
            "lg" => Ok(ActionIconSize::Lg),
            "xl" => Ok(ActionIconSize::Xl),
            _ => Err(()),
        }
    }
}

/// An icon-only button component.
///
/// Used for compact actions where only an icon is needed.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     ActionIcon { icon: Icon::Menu, variant: "subtle", onclick: || toggle_menu() }
///     ActionIcon { icon: Icon::Close, size: "lg", variant: "outline" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct ActionIcon {
    /// Icon to display.
    pub icon: Option<TablerIcon>,
    /// Button variant (filled, light, outline, subtle, transparent, default).
    pub variant: Option<String>,
    /// Button size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Color override.
    pub color: Option<String>,
    /// Border radius override (xs, sm, md, lg, xl).
    pub radius: Option<String>,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Whether to show loading state.
    pub loading: bool,
    /// Click handler.
    pub onclick: Option<WidgetCallback>,
}

impl ActionIcon {
    /// Generate the CSS class string for this action icon.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-action-icon"];

        // Variant class
        let variant: ActionIconVariant = self
            .variant
            .as_ref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        classes.push(variant.class_name());

        // Size class
        let size: ActionIconSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        // Radius class
        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-action-icon--radius-xs"),
                "sm" => classes.push("rinch-action-icon--radius-sm"),
                "md" => classes.push("rinch-action-icon--radius-md"),
                "lg" => classes.push("rinch-action-icon--radius-lg"),
                "xl" => classes.push("rinch-action-icon--radius-xl"),
                _ => {}
            }
        }

        // State classes
        if self.disabled {
            classes.push("rinch-action-icon--disabled");
        }
        if self.loading {
            classes.push("rinch-action-icon--loading");
        }

        classes.join(" ")
    }
}

impl Widget for ActionIcon {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container =
            rinch_macros::rsx! { button { class: "rinch-action-icon", r#type: "button" } };
        container.set_attribute("class", &self.class_string());

        // Color style
        if let Some(c) = &self.color {
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-action-icon-color: {}", c)
            } else {
                format!("--rinch-action-icon-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        // Register click handler
        if let Some(cb) = &self.onclick {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            container.set_attribute("data-rid", &handler_id.0.to_string());
        }

        if self.loading {
            let loader = rinch_macros::rsx! { span { class: "rinch-action-icon__loader" } };
            container.append_child(&loader);
        } else if let Some(icon) = self.icon {
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            container.append_child(&icon_el);
        } else {
            for child in children {
                container.append_child(child);
            }
        }

        container
    }
}
