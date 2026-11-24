//! ActionIcon component.
//!
//! An icon-only button for actions.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon_with_options};

use crate::borderless_window::SectionRenderer;

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
///     ActionIcon { icon: TablerIcon::Menu2, variant: "subtle", onclick: || toggle_menu() }
///     ActionIcon { icon: TablerIcon::X, size: "lg", variant: "outline", title: "Close" }
/// }
/// ```
#[derive(Default)]
pub struct ActionIcon {
    /// Icon to display.
    pub icon: Option<TablerIcon>,
    /// Button variant (filled, light, outline, subtle, transparent, default).
    pub variant: String,
    /// Button size (xs, sm, md, lg, xl).
    pub size: String,
    /// Color override.
    pub color: String,
    /// Border radius override (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Whether to show loading state: a [`Loader`](crate::Loader) in place of
    /// the icon, and the button stops taking clicks.
    pub loading: bool,
    /// The loader type drawn while `loading` (`oval`, `bars`, `dots`). Empty
    /// uses the app's [`LoaderDefaults`](crate::LoaderDefaults), then oval.
    pub loader_type: String,
    /// A custom loader drawn while `loading`, in place of the built-in kinds.
    pub loader: Option<SectionRenderer>,
    /// The icon's edge length in px. `None` (the default) takes the size
    /// step's own: 12, 14, 18, 22, 28 for xs–xl.
    pub icon_size: Option<u32>,
    /// The icon's style: outline (the default) or filled.
    pub icon_style: TablerIconStyle,
    /// The button's accessible name (`aria-label`), without the tooltip
    /// `title` shows. Empty (the default) writes none.
    pub aria_label: String,
    /// The HTML `type` attribute. Empty (the default) is `"button"`: an icon
    /// button inside a form is almost never its submit button, so this is the
    /// one place rinch does not take HTML's `submit` default. Say `"submit"`
    /// or `"reset"` to make it one.
    pub r#type: String,
    /// The button's `title` attribute — its tooltip, and the accessible name
    /// an icon-only button otherwise lacks. Empty (the default) writes none
    /// (issue #1653). A reactive value re-renders the button; for a surgical
    /// update write the attribute itself, `"title": {|| …}`.
    pub title: String,
    /// Click handler.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for ActionIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionIcon")
            .field("icon", &self.icon)
            .field("variant", &self.variant)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("radius", &self.radius)
            .field("disabled", &self.disabled)
            .field("loading", &self.loading)
            .field("loader_type", &self.loader_type)
            .field("loader", &self.loader.as_ref().map(|_| "<renderer>"))
            .field("icon_size", &self.icon_size)
            .field("icon_style", &self.icon_style)
            .field("aria_label", &self.aria_label)
            .field("type", &self.r#type)
            .field("title", &self.title)
            .field("onclick", &self.onclick)
            .finish()
    }
}

impl ActionIcon {
    /// Generate the CSS class string for this action icon.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-action-icon"];

        // Variant class
        let variant: ActionIconVariant = if self.variant.is_empty() {
            ActionIconVariant::default()
        } else {
            self.variant.parse().unwrap_or_default()
        };
        classes.push(variant.class_name());

        // Size class
        let size: ActionIconSize = if self.size.is_empty() {
            ActionIconSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        // Radius class
        let radius_cls = crate::class_utils::radius_class("rinch-action-icon", &self.radius);
        classes.extend(radius_cls.as_deref());

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

impl Component for ActionIcon {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container =
            rinch_macros::rsx! { button { class: "rinch-action-icon", r#type: "button" } };
        container.set_attribute("class", &self.class_string());
        if !self.r#type.is_empty() {
            container.set_attribute("type", &self.r#type);
        }
        if !self.aria_label.is_empty() {
            container.set_attribute("aria-label", &self.aria_label);
        }

        // Color style
        if !self.color.is_empty() {
            let style = crate::color_utils::accent_vars("--rinch-action-icon", &self.color);
            container.set_attribute("style", &style);
        }

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        if !self.title.is_empty() {
            container.set_attribute("title", &self.title);
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
            let slot = rinch_macros::rsx! { span { class: "rinch-action-icon__loader" } };
            let loader = crate::loader::render_component_loader(
                __scope,
                &self.loader_type,
                self.loader.as_ref(),
                "",
            );
            slot.append_child(&loader);
            container.append_child(&slot);
        } else if let Some(icon) = self.icon {
            let size: ActionIconSize = if self.size.is_empty() {
                ActionIconSize::default()
            } else {
                self.size.parse().unwrap_or_default()
            };
            let px = self
                .icon_size
                .unwrap_or_else(|| size.icon_size().parse().unwrap_or(18));
            let icon_el = render_tabler_icon_with_options(
                __scope,
                icon,
                rinch_tabler_icons::TablerIconOptions {
                    style: self.icon_style,
                    size: Some(px),
                    ..Default::default()
                },
            );
            container.append_child(&icon_el);
        } else {
            for child in children {
                container.append_child(child);
            }
        }

        container
    }
}
