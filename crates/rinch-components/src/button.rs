//! Button component.
//!
//! A clickable button with multiple variants and sizes.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};

use crate::borderless_window::SectionRenderer;

/// Button variant styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    /// Solid background with primary color.
    #[default]
    Filled,
    /// Transparent background with colored border.
    Outline,
    /// Light background with darker text.
    Light,
    /// Transparent background, no border.
    Subtle,
    /// Gray/neutral styling.
    Default,
}

impl ButtonVariant {
    /// Get the CSS class name for this variant.
    pub fn class_name(&self) -> &'static str {
        match self {
            ButtonVariant::Filled => "rinch-button--filled",
            ButtonVariant::Outline => "rinch-button--outline",
            ButtonVariant::Light => "rinch-button--light",
            ButtonVariant::Subtle => "rinch-button--subtle",
            ButtonVariant::Default => "rinch-button--default",
        }
    }
}

impl std::str::FromStr for ButtonVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "filled" => Ok(ButtonVariant::Filled),
            "outline" => Ok(ButtonVariant::Outline),
            "light" => Ok(ButtonVariant::Light),
            "subtle" => Ok(ButtonVariant::Subtle),
            "default" => Ok(ButtonVariant::Default),
            _ => Err(()),
        }
    }
}

/// Button size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl ButtonSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            ButtonSize::Xs => "rinch-button--xs",
            ButtonSize::Sm => "rinch-button--sm",
            ButtonSize::Md => "rinch-button--md",
            ButtonSize::Lg => "rinch-button--lg",
            ButtonSize::Xl => "rinch-button--xl",
        }
    }
}

impl std::str::FromStr for ButtonSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ButtonSize::Xs),
            "sm" => Ok(ButtonSize::Sm),
            "md" => Ok(ButtonSize::Md),
            "lg" => Ok(ButtonSize::Lg),
            "xl" => Ok(ButtonSize::Xl),
            _ => Err(()),
        }
    }
}

/// A clickable button with multiple variants and sizes.
#[derive(Default)]
pub struct Button {
    /// Button variant (filled, outline, light, subtle, default).
    pub variant: String,
    /// Button size (xs, sm, md, lg, xl).
    pub size: String,
    /// Override the primary color: a theme palette name (`"red"`, `"green"`)
    /// or a raw CSS colour (`"#ff0000"`, `"rgb(…)"`, `"hsl(…)"`).
    pub color: String,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Whether the button shows a loading spinner. The label stays in place,
    /// hidden, so the button keeps its width, and a [`Loader`](crate::Loader)
    /// is drawn over it; the button stops taking clicks.
    pub loading: bool,
    /// The loader type drawn while `loading` (`oval`, `bars`, `dots`). Empty
    /// uses the app's [`LoaderDefaults`](crate::LoaderDefaults), then oval.
    pub loader_type: String,
    /// A custom loader drawn while `loading`, in place of the built-in kinds.
    pub loader: Option<SectionRenderer>,
    /// The HTML `type` attribute: `"submit"`, `"button"` or `"reset"`. Empty
    /// (the default) writes none, which HTML reads as `submit`, as Chrome does.
    pub r#type: String,
    /// Whether the button should take full width.
    pub full_width: bool,
    /// Override border radius.
    pub radius: String,
    /// Callback when button is clicked.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("variant", &self.variant)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("disabled", &self.disabled)
            .field("loading", &self.loading)
            .field("loader_type", &self.loader_type)
            .field("loader", &self.loader.as_ref().map(|_| "<renderer>"))
            .field("type", &self.r#type)
            .field("full_width", &self.full_width)
            .field("radius", &self.radius)
            .field("onclick", &self.onclick)
            .finish()
    }
}

impl Button {
    /// Generate the CSS class string for this button.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-button"];

        // Size class
        let size: ButtonSize = if self.size.is_empty() {
            ButtonSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        // Variant class
        let variant: ButtonVariant = if self.variant.is_empty() {
            ButtonVariant::default()
        } else {
            self.variant.parse().unwrap_or_default()
        };
        classes.push(variant.class_name());

        // Full width
        if self.full_width {
            classes.push("rinch-button--full-width");
        }

        // Disabled
        if self.disabled {
            classes.push("rinch-button--disabled");
        }

        // Loading
        if self.loading {
            classes.push("rinch-button--loading");
        }

        // Custom color
        if !self.color.is_empty() {
            classes.push("rinch-button--colored");
        }

        // Custom radius
        let radius_cls = crate::class_utils::radius_class("rinch-button", &self.radius);
        classes.extend(radius_cls.as_deref());

        classes.join(" ")
    }

    /// Generate inline style for custom color.
    ///
    /// A palette name reads that palette's shades; a raw CSS colour is used
    /// as the base, with its hover and light shades mixed from it.
    pub fn style_string(&self) -> Option<String> {
        if self.color.is_empty() {
            return None;
        }
        let c = &self.color;
        if crate::color_utils::is_raw_css_color(c) {
            Some(format!(
                "--rinch-button-color: {c}; \
                 --rinch-button-color-hover: color-mix(in srgb, {c} 90%, black); \
                 --rinch-button-color-light: color-mix(in srgb, {c} 10%, transparent); \
                 --rinch-button-color-light-hover: color-mix(in srgb, {c} 15%, transparent); \
                 --rinch-button-color-light-color: {c}; \
                 --rinch-button-color-contrast: {};",
                crate::color_utils::contrast_for(c)
            ))
        } else {
            Some(format!(
                "--rinch-button-color: var(--rinch-color-{c}-6); \
                 --rinch-button-color-hover: var(--rinch-color-{c}-7); \
                 --rinch-button-color-light: var(--rinch-color-{c}-light); \
                 --rinch-button-color-light-hover: var(--rinch-color-{c}-light-hover); \
                 --rinch-button-color-light-color: var(--rinch-color-{c}-light-color); \
                 --rinch-button-color-contrast: var(--rinch-color-{c}-contrast);"
            ))
        }
    }

    /// The loader size that fits this button's size step.
    fn loader_size(&self) -> &'static str {
        match self.size.parse().unwrap_or_default() {
            ButtonSize::Lg | ButtonSize::Xl => "sm",
            _ => "xs",
        }
    }
}

impl Component for Button {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { button { class: "rinch-button" } };
        container.set_attribute("class", &self.class_string());

        if let Some(style) = self.style_string() {
            container.set_attribute("style", &style);
        }

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        if !self.r#type.is_empty() {
            container.set_attribute("type", &self.r#type);
        }

        // Click handler
        if let Some(cb) = &self.onclick {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            container.set_attribute("data-rid", &handler_id.0.to_string());
        }

        if self.loading {
            // The label keeps the button's width (the stylesheet hides it while
            // loading) and the loader sits over it (issue #1334).
            let label = rinch_macros::rsx! { span { class: "rinch-button__label" } };
            for child in children {
                label.append_child(child);
            }
            container.append_child(&label);
            let slot = rinch_macros::rsx! { span { class: "rinch-button__loader" } };
            let loader = crate::loader::render_component_loader(
                __scope,
                &self.loader_type,
                self.loader.as_ref(),
                self.loader_size(),
            );
            slot.append_child(&loader);
            container.append_child(&slot);
        } else {
            for child in children {
                container.append_child(child);
            }
        }
        container
    }
}
