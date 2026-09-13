//! CloseButton component.
//!
//! A button with a close/X icon for dismissing elements.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};

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
#[derive(Default)]
pub struct CloseButton {
    /// Button size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius override (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// Custom icon size in pixels.
    pub icon_size: Option<u32>,
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

impl Component for CloseButton {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        // The X itself, sized by `icon_size` or by the size step (#474). What
        // this replaces was an empty `<span class="rinch-icon
        // rinch-icon--close">` — and no rule anywhere in the workspace matches
        // that class, so the button drew no glyph at all. The class stays on the
        // svg as a styling hook; the size is written as both attributes (via
        // `close_icon_dom`) and an inline style, which is what
        // `render_tabler_icon_with_options` does.
        let icon_px = self.resolved_icon_size();
        let icon_span = crate::icons::close_icon_dom(__scope, &icon_px);
        icon_span.set_attribute("class", "rinch-icon rinch-icon--close");
        icon_span.set_attribute(
            "style",
            &format!("width: {icon_px}px; height: {icon_px}px; flex-shrink: 0;"),
        );
        let container = rinch_macros::rsx! {
            button { class: "rinch-close-button", r#type: "button" }
        };
        container.set_attribute("class", &self.class_string());
        container.set_attribute("aria-label", "Close");

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        container.append_child(&icon_span);

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
