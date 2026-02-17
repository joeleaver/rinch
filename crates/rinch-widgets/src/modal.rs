//! Modal widget.
//!
//! A dialog overlay that renders in a portal.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive opened state without re-rendering, use the `opened_fn` prop:
//!
//! ```ignore
//! let modal_opened = use_signal(|| false);
//!
//! rsx! {
//!     Modal {
//!         opened_fn: Some(Rc::new(move || modal_opened.get())),
//!         onclose: move || modal_opened.set(false),
//!         title: "My Modal",
//!         "Modal content"
//!     }
//! }
//! ```

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::rc::Rc;

/// Modal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
    Full,
}

impl ModalSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            ModalSize::Xs => "rinch-modal--xs",
            ModalSize::Sm => "rinch-modal--sm",
            ModalSize::Md => "rinch-modal--md",
            ModalSize::Lg => "rinch-modal--lg",
            ModalSize::Xl => "rinch-modal--xl",
            ModalSize::Full => "rinch-modal--full",
        }
    }
}

impl std::str::FromStr for ModalSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ModalSize::Xs),
            "sm" => Ok(ModalSize::Sm),
            "md" => Ok(ModalSize::Md),
            "lg" => Ok(ModalSize::Lg),
            "xl" => Ok(ModalSize::Xl),
            "full" => Ok(ModalSize::Full),
            _ => Err(()),
        }
    }
}

/// Reactive callback type for opened state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// A modal dialog overlay.
///
/// Renders in a portal above all other content with an overlay backdrop.
///
/// # Example
///
/// ```ignore
/// let show_modal = use_signal(|| false);
///
/// rsx! {
///     Button { onclick: move || show_modal.set(true), "Open Modal" }
///
///     Modal {
///         opened: show_modal.get(),
///         onclose: move || show_modal.set(false),
///         title: "My Modal",
///
///         p { "Modal content goes here" }
///
///         Group { justify: "flex-end",
///             Button { onclick: move || show_modal.set(false), "Close" }
///         }
///     }
/// }
/// ```
pub struct Modal {
    /// Whether the modal is open (static, for initial render or non-reactive use).
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    /// When provided, the modal updates automatically when the signal changes.
    pub opened_fn: Option<ReactiveBool>,
    /// Modal title.
    pub title: String,
    /// Size variant (xs, sm, md, lg, xl, full).
    pub size: String,
    /// Border radius.
    pub radius: String,
    /// Whether to show overlay backdrop.
    pub with_overlay: bool,
    /// Overlay opacity (0-1).
    pub overlay_opacity: Option<f32>,
    /// Overlay blur.
    pub overlay_blur: String,
    /// Whether to center the modal vertically.
    pub centered: bool,
    /// Whether clicking overlay closes modal.
    pub close_on_click_outside: bool,
    /// Whether pressing Escape closes modal.
    pub close_on_escape: bool,
    /// Whether to show close button.
    pub with_close_button: bool,
    /// Padding inside modal.
    pub padding: String,
    /// Z-index for the modal.
    pub z_index: Option<i32>,
    /// Whether to lock scroll when open.
    pub lock_scroll: bool,
    /// Whether to trap focus inside modal.
    pub trap_focus: bool,
    /// Callback when modal should close.
    pub onclose: Option<rinch_core::WidgetCallback>,
}

impl std::fmt::Debug for Modal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Modal")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("title", &self.title)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("with_overlay", &self.with_overlay)
            .field("overlay_opacity", &self.overlay_opacity)
            .field("overlay_blur", &self.overlay_blur)
            .field("centered", &self.centered)
            .field("close_on_click_outside", &self.close_on_click_outside)
            .field("close_on_escape", &self.close_on_escape)
            .field("with_close_button", &self.with_close_button)
            .field("padding", &self.padding)
            .field("z_index", &self.z_index)
            .field("lock_scroll", &self.lock_scroll)
            .field("trap_focus", &self.trap_focus)
            .field("onclose", &self.onclose.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for Modal {
    fn default() -> Self {
        Self {
            opened: false,
            opened_fn: None,
            title: String::new(),
            size: String::new(),
            radius: String::new(),
            with_overlay: true,
            overlay_opacity: None,
            overlay_blur: String::new(),
            centered: false,
            close_on_click_outside: true,
            close_on_escape: true,
            with_close_button: true,
            padding: String::new(),
            z_index: None,
            lock_scroll: true,
            trap_focus: true,
            onclose: None,
        }
    }
}

impl Modal {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-modal"];

        if !self.size.is_empty() {
            if let Ok(size) = self.size.parse::<ModalSize>() {
                classes.push(size.class_name());
            }
        }

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-modal--radius-xs"),
                "sm" => classes.push("rinch-modal--radius-sm"),
                "md" => classes.push("rinch-modal--radius-md"),
                "lg" => classes.push("rinch-modal--radius-lg"),
                "xl" => classes.push("rinch-modal--radius-xl"),
                _ => {}
            }
        }

        if self.centered {
            classes.push("rinch-modal--centered");
        }

        classes.join(" ")
    }
}

impl Widget for Modal {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Determine initial opened state
        let is_opened = if let Some(ref opened_fn) = self.opened_fn {
            opened_fn()
        } else {
            self.opened
        };

        // Build overlay styles
        let overlay_style = {
            let mut styles = Vec::new();
            if let Some(opacity) = self.overlay_opacity {
                styles.push(format!("--rinch-modal-overlay-opacity: {}", opacity));
            }
            if !self.overlay_blur.is_empty() {
                styles.push(format!("--rinch-modal-overlay-blur: {}", self.overlay_blur));
            }
            if styles.is_empty() {
                None
            } else {
                Some(styles.join("; "))
            }
        };

        // Build modal content styles
        let content_style = if self.padding.is_empty() {
            None
        } else {
            Some(format!("padding: {}", self.padding))
        };

        // Build close button handler
        let close_handler_id = self.onclose.as_ref().map(|cb| {
            __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            })
        });

        // Create root container with visibility class
        let root_class = if is_opened {
            "rinch-modal__root".to_string()
        } else {
            "rinch-modal__root rinch-modal__root--hidden".to_string()
        };
        let root = rinch_macros::rsx! { div { class: "rinch-modal__root" } };
        root.set_attribute("class", &root_class);

        // If reactive opened_fn is provided, create an Effect to toggle visibility via class
        if let Some(ref opened_fn) = self.opened_fn {
            let opened_fn = opened_fn.clone();
            let root_clone = root.clone();

            __scope.create_effect(move || {
                let is_open = opened_fn();
                if is_open {
                    root_clone.set_attribute("class", "rinch-modal__root");
                } else {
                    root_clone
                        .set_attribute("class", "rinch-modal__root rinch-modal__root--hidden");
                }
            });
        }

        // Build the overlay
        if self.with_overlay {
            let overlay = rinch_macros::rsx! { div { class: "rinch-modal__overlay" } };
            if let Some(ref style) = overlay_style {
                overlay.set_attribute("style", style);
            }
            if self.close_on_click_outside
                && let Some(handler_id) = close_handler_id
            {
                overlay.set_attribute("data-rid", &handler_id.to_string());
            }
            root.append_child(&overlay);
        }

        // Build modal div
        let modal_div = rinch_macros::rsx! { div { class: "rinch-modal" } };
        modal_div.set_attribute("class", &self.class_string());

        // Close button
        if self.with_close_button {
            let btn = rinch_macros::rsx! { button { class: "rinch-modal__close" } };
            if let Some(handler_id) = close_handler_id {
                btn.set_attribute("data-rid", &handler_id.to_string());
            }
            let close_icon = crate::icons::close_icon_lines_dom(__scope);
            btn.append_child(&close_icon);
            modal_div.append_child(&btn);
        }

        // Title section
        if !self.title.is_empty() {
            let header = rinch_macros::rsx! { div { class: "rinch-modal__header" } };

            let title_el = rinch_macros::rsx! { h2 { class: "rinch-modal__title" } };
            let title_text = __scope.create_text(&self.title);
            title_el.append_child(&title_text);
            header.append_child(&title_el);

            modal_div.append_child(&header);
        }

        // Build modal body
        let body = rinch_macros::rsx! { div { class: "rinch-modal__body" } };
        if let Some(ref style) = content_style {
            body.set_attribute("style", style);
        }
        for child in children {
            body.append_child(child);
        }
        modal_div.append_child(&body);

        root.append_child(&modal_div);

        root
    }
}

// We need a wrapper that outputs Element::Portal
// This is handled specially - the Modal widget renders HTML that expects
// to be in a portal. We'll create a ModalPortal helper widget.

/// Internal: Modal content wrapped in a portal.
/// Users should use Modal directly - this handles the portal wrapping.
#[derive(Debug, Default)]
pub struct ModalRoot {
    pub modal: Modal,
}

impl Widget for ModalRoot {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Generate modal element
        self.modal.render(__scope, children)
    }
}
