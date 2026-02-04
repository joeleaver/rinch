//! Notification widget.
//!
//! Toast notifications that render in a portal.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive opened state without re-rendering, use the `opened_fn` prop:
//!
//! ```ignore
//! let notification_visible = use_signal(|| false);
//!
//! rsx! {
//!     Notification {
//!         opened_fn: Some(Rc::new(move || notification_visible.get())),
//!         onclose: move || notification_visible.set(false),
//!         title: "Success!",
//!         "Your changes have been saved."
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Icon, Widget};
use std::rc::Rc;

/// Reactive callback type for opened state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Notification position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationPosition {
    TopLeft,
    TopCenter,
    #[default]
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl NotificationPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            NotificationPosition::TopLeft => "rinch-notification--top-left",
            NotificationPosition::TopCenter => "rinch-notification--top-center",
            NotificationPosition::TopRight => "rinch-notification--top-right",
            NotificationPosition::BottomLeft => "rinch-notification--bottom-left",
            NotificationPosition::BottomCenter => "rinch-notification--bottom-center",
            NotificationPosition::BottomRight => "rinch-notification--bottom-right",
        }
    }
}

impl std::str::FromStr for NotificationPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "top_left" | "topleft" => Ok(NotificationPosition::TopLeft),
            "top_center" | "topcenter" => Ok(NotificationPosition::TopCenter),
            "top_right" | "topright" => Ok(NotificationPosition::TopRight),
            "bottom_left" | "bottomleft" => Ok(NotificationPosition::BottomLeft),
            "bottom_center" | "bottomcenter" => Ok(NotificationPosition::BottomCenter),
            "bottom_right" | "bottomright" => Ok(NotificationPosition::BottomRight),
            _ => Err(()),
        }
    }
}

/// A toast notification.
///
/// Renders in a portal at a fixed position.
///
/// # Example
///
/// ```ignore
/// let show_notification = use_signal(|| false);
///
/// rsx! {
///     Button { onclick: move || show_notification.set(true), "Show Notification" }
///
///     Notification {
///         opened: show_notification.get(),
///         onclose: move || show_notification.set(false),
///         title: "Success!",
///         color: "green",
///         icon: Icon::CheckCircle,
///
///         "Your changes have been saved."
///     }
/// }
/// ```
pub struct Notification {
    /// Whether the notification is visible (static, for initial render or non-reactive use).
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    /// When provided, the notification updates automatically when the signal changes.
    pub opened_fn: Option<ReactiveBool>,
    /// Notification title.
    pub title: Option<String>,
    /// Color variant.
    pub color: Option<String>,
    /// Position (top-left, top-center, top-right, bottom-left, bottom-center, bottom-right).
    pub position: Option<String>,
    /// Border radius.
    pub radius: Option<String>,
    /// Whether to show close button.
    pub with_close_button: bool,
    /// Whether notification has a colored border on the left.
    pub with_border: bool,
    /// Icon to display.
    pub icon: Option<Icon>,
    /// Auto-close delay in milliseconds (0 = no auto-close).
    pub auto_close: u32,
    /// Loading state.
    pub loading: bool,
    /// Z-index for the notification.
    pub z_index: Option<i32>,
    /// Callback when notification should close.
    pub onclose: Option<rinch_core::WidgetCallback>,
}

impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("title", &self.title)
            .field("color", &self.color)
            .field("position", &self.position)
            .field("radius", &self.radius)
            .field("with_close_button", &self.with_close_button)
            .field("with_border", &self.with_border)
            .field("icon", &self.icon)
            .field("auto_close", &self.auto_close)
            .field("loading", &self.loading)
            .field("z_index", &self.z_index)
            .field("onclose", &self.onclose.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            opened: false,
            opened_fn: None,
            title: None,
            color: None,
            position: None,
            radius: None,
            with_close_button: true,
            with_border: false,
            icon: None,
            auto_close: 0,
            loading: false,
            z_index: None,
            onclose: None,
        }
    }
}

impl Notification {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-notification"];

        if let Some(ref p) = self.position {
            if let Ok(pos) = p.parse::<NotificationPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(NotificationPosition::TopRight.class_name());
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-notification--radius-xs"),
                "sm" => classes.push("rinch-notification--radius-sm"),
                "md" => classes.push("rinch-notification--radius-md"),
                "lg" => classes.push("rinch-notification--radius-lg"),
                "xl" => classes.push("rinch-notification--radius-xl"),
                _ => {}
            }
        }

        if self.with_border {
            classes.push("rinch-notification--with-border");
        }

        if self.loading {
            classes.push("rinch-notification--loading");
        }

        classes.join(" ")
    }
}

impl Widget for Notification {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Determine initial opened state
        let is_opened = if let Some(ref opened_fn) = self.opened_fn {
            opened_fn()
        } else {
            self.opened
        };

        // Color style
        let color_style = self.color.as_ref().map(|c| {
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-notification-color: {}", c)
            } else {
                format!("--rinch-notification-color: var(--rinch-color-{}-6)", c)
            }
        });

        // Close handler
        let close_handler_id = self.onclose.as_ref().map(|cb| {
            __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            })
        });

        // Create root element with visibility class
        let base_class = self.class_string();
        let root_class = if is_opened {
            base_class.clone()
        } else {
            format!("{} rinch-notification--hidden", base_class)
        };
        let root = rinch_macros::rsx! { div { class: "rinch-notification" } };
        root.set_attribute("class", &root_class);
        if let Some(ref style) = color_style {
            root.set_attribute("style", style);
        }

        // If reactive opened_fn is provided, create an Effect to toggle visibility via class
        if let Some(ref opened_fn) = self.opened_fn {
            let opened_fn = opened_fn.clone();
            let root_clone = root.clone();
            let base_class_clone = base_class;

            __scope.create_effect(move || {
                let is_open = opened_fn();
                if is_open {
                    root_clone.set_attribute("class", &base_class_clone);
                } else {
                    root_clone.set_attribute(
                        "class",
                        &format!("{} rinch-notification--hidden", base_class_clone),
                    );
                }
            });
        }

        // Icon element
        if self.loading {
            let icon_div = rinch_macros::rsx! {
                div { class: "rinch-notification__icon rinch-notification__loader" }
            };
            root.append_child(&icon_div);
        } else if let Some(icon) = &self.icon {
            let icon_div = rinch_macros::rsx! { div { class: "rinch-notification__icon" } };
            let icon_svg = crate::icons::render_icon(__scope, *icon);
            icon_div.append_child(&icon_svg);
            root.append_child(&icon_div);
        }

        // Build body content
        let body = rinch_macros::rsx! { div { class: "rinch-notification__body" } };

        // Title element
        if let Some(ref t) = self.title {
            let title_div = rinch_macros::rsx! { div { class: "rinch-notification__title" } };
            let title_text = __scope.create_text(t);
            title_div.append_child(&title_text);
            body.append_child(&title_div);
        }

        // Description
        let desc_div = rinch_macros::rsx! { div { class: "rinch-notification__description" } };
        for child in children {
            desc_div.append_child(child);
        }
        body.append_child(&desc_div);

        root.append_child(&body);

        // Close button
        if self.with_close_button {
            let btn = rinch_macros::rsx! { button { class: "rinch-notification__close" } };
            if let Some(handler_id) = close_handler_id {
                btn.set_attribute("data-rid", &handler_id.to_string());
            }
            let close_icon = crate::icons::close_icon_lines_dom(__scope);
            btn.append_child(&close_icon);
            root.append_child(&btn);
        }

        root
    }
}
