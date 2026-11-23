//! CloseButton component.
//!
//! A button with a close/X icon for dismissing elements.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{
    TablerIcon, TablerIconOptions, TablerIconStyle, render_tabler_icon_with_options,
};

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
///     CloseButton { size: "lg", title: "Dismiss" }
/// }
/// ```
#[derive(Default)]
pub struct CloseButton {
    /// Button size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius override (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Custom icon size in pixels. It is published as the custom property
    /// `--rinch-close-button-icon-size` on the glyph, which the stylesheet's
    /// `.rinch-icon--close` rule reads, so an app stylesheet can still resize
    /// the glyph (`.rinch-close-button svg { width: …; height: … }`).
    pub icon_size: Option<u32>,
    /// A Tabler icon drawn instead of the built-in X. Children, when given,
    /// win over both.
    pub icon: Option<TablerIcon>,
    /// The button's accessible name (`aria-label`). Empty (the default) is
    /// `"Close"`.
    pub aria_label: String,
    /// The HTML `type` attribute. Empty (the default) is `"button"`, so a
    /// close button inside a form never submits it.
    pub r#type: String,
    /// The button's `title` attribute — its tooltip. Empty (the default)
    /// writes none (issue #1653); the accessible name stays `aria_label`
    /// (by default "Close"). A reactive value re-renders the button; for a surgical update
    /// write the attribute itself, `"title": {|| …}`.
    pub title: String,
    /// Click handler.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for CloseButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloseButton")
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("disabled", &self.disabled)
            .field("icon_size", &self.icon_size)
            .field("icon", &self.icon)
            .field("aria_label", &self.aria_label)
            .field("type", &self.r#type)
            .field("title", &self.title)
            .field("onclick", &self.onclick.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl CloseButton {
    /// The size step this button renders at — the default when `size` is unset
    /// or names nothing.
    fn resolved_size(&self) -> CloseButtonSize {
        if self.size.is_empty() {
            CloseButtonSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        }
    }

    /// The glyph's edge length in px: `icon_size` when the caller gave one,
    /// otherwise the size step's own (#474).
    fn resolved_icon_size(&self) -> String {
        match self.icon_size {
            Some(px) => px.to_string(),
            None => self.resolved_size().icon_size().to_string(),
        }
    }

    /// Generate the CSS class string for this close button.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-close-button"];

        // Size class
        classes.push(self.resolved_size().class_name());

        // Radius class
        let radius_cls = crate::class_utils::radius_class("rinch-close-button", &self.radius);
        classes.extend(radius_cls.as_deref());

        if self.disabled {
            classes.push("rinch-close-button--disabled");
        }

        classes.join(" ")
    }
}

/// The accessible name a close button has when its `aria_label` is empty.
pub const DEFAULT_CLOSE_LABEL: &str = "Close";

impl Component for CloseButton {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            button { class: "rinch-close-button", r#type: "button" }
        };
        container.set_attribute("class", &self.class_string());
        if !self.r#type.is_empty() {
            container.set_attribute("type", &self.r#type);
        }
        let label = if self.aria_label.is_empty() {
            DEFAULT_CLOSE_LABEL
        } else {
            &self.aria_label
        };
        container.set_attribute("aria-label", label);

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        if !self.title.is_empty() {
            container.set_attribute("title", &self.title);
        }

        if children.is_empty() {
            // The glyph, sized by `icon_size` or by the size step (#474). The
            // size is the svg's attributes and a custom property the
            // stylesheet's `.rinch-icon--close` rule spends, not an inline
            // width/height, so an app's own rule can resize it (#2199).
            let icon_px = self.resolved_icon_size();
            let glyph = match self.icon {
                Some(icon) => {
                    let px = icon_px.parse().unwrap_or(16);
                    render_tabler_icon_with_options(
                        __scope,
                        icon,
                        TablerIconOptions {
                            style: TablerIconStyle::Outline,
                            size: Some(px),
                            ..Default::default()
                        },
                    )
                }
                None => crate::icons::close_icon_dom(__scope, &icon_px),
            };
            glyph.set_attribute("class", "rinch-icon rinch-icon--close");
            glyph.set_attribute(
                "style",
                &format!("--rinch-close-button-icon-size: {icon_px}px;"),
            );
            container.append_child(&glyph);
        } else {
            for child in children {
                container.append_child(child);
            }
        }

        // Click handler
        if let Some(cb) = &self.onclick {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            container.set_attribute("data-rid", &handler_id.0.to_string());
        }

        container
    }
}
