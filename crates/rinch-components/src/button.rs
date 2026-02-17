//! Button component.
//!
//! A clickable button with multiple variants and sizes.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, Callback};

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
#[derive(Debug, Default)]
pub struct Button {
    /// Button variant (filled, outline, light, subtle, default).
    pub variant: String,
    /// Button size (xs, sm, md, lg, xl).
    pub size: String,
    /// Override the primary color (e.g., "red", "green").
    pub color: String,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Whether the button shows a loading spinner.
    pub loading: bool,
    /// Whether the button should take full width.
    pub full_width: bool,
    /// Override border radius.
    pub radius: String,
    /// Callback when button is clicked.
    pub onclick: Option<Callback>,
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
        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-button--radius-xs"),
                "sm" => classes.push("rinch-button--radius-sm"),
                "md" => classes.push("rinch-button--radius-md"),
                "lg" => classes.push("rinch-button--radius-lg"),
                "xl" => classes.push("rinch-button--radius-xl"),
                _ => {}
            }
        }

        classes.join(" ")
    }

    /// Generate inline style for custom color.
    pub fn style_string(&self) -> Option<String> {
        if self.color.is_empty() {
            None
        } else {
            Some(format!(
                "--rinch-button-color: var(--rinch-color-{}-6); \
                 --rinch-button-color-hover: var(--rinch-color-{}-7); \
                 --rinch-button-color-light: var(--rinch-color-{}-0); \
                 --rinch-button-color-light-hover: var(--rinch-color-{}-1);",
                self.color, self.color, self.color, self.color
            ))
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

        // Click handler
        if let Some(cb) = &self.onclick {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            container.set_attribute("data-rid", &handler_id.0.to_string());
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
