//! Drawer component.
//!
//! A slide-out panel that renders in a portal.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive opened state without re-rendering, use the `opened_fn` prop:
//!
//! ```ignore
//! let drawer_opened = Signal::new(false);
//!
//! rsx! {
//!     Drawer {
//!         opened_fn: Some(Rc::new(move || drawer_opened.get())),
//!         onclose: move || drawer_opened.set(false),
//!         title: "Menu",
//!         "Drawer content"
//!     }
//! }
//! ```

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::rc::Rc;

/// Drawer position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawerPosition {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl DrawerPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            DrawerPosition::Left => "rinch-drawer--left",
            DrawerPosition::Right => "rinch-drawer--right",
            DrawerPosition::Top => "rinch-drawer--top",
            DrawerPosition::Bottom => "rinch-drawer--bottom",
        }
    }
}

impl std::str::FromStr for DrawerPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "left" => Ok(DrawerPosition::Left),
            "right" => Ok(DrawerPosition::Right),
            "top" => Ok(DrawerPosition::Top),
            "bottom" => Ok(DrawerPosition::Bottom),
            _ => Err(()),
        }
    }
}

/// Drawer size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawerSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
    Full,
}

impl DrawerSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            DrawerSize::Xs => "rinch-drawer--xs",
            DrawerSize::Sm => "rinch-drawer--sm",
            DrawerSize::Md => "rinch-drawer--md",
            DrawerSize::Lg => "rinch-drawer--lg",
            DrawerSize::Xl => "rinch-drawer--xl",
            DrawerSize::Full => "rinch-drawer--full",
        }
    }
}

impl std::str::FromStr for DrawerSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(DrawerSize::Xs),
            "sm" => Ok(DrawerSize::Sm),
            "md" => Ok(DrawerSize::Md),
            "lg" => Ok(DrawerSize::Lg),
            "xl" => Ok(DrawerSize::Xl),
            "full" => Ok(DrawerSize::Full),
            _ => Err(()),
        }
    }
}

/// Reactive callback type for opened state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// A drawer (slide-out panel) overlay.
///
/// Renders in a portal, sliding in from the specified edge.
///
/// # Example
///
/// ```ignore
/// let show_drawer = Signal::new(false);
///
/// rsx! {
///     Button { onclick: move || show_drawer.set(true), "Open Drawer" }
///
///     Drawer {
///         opened: show_drawer.get(),
///         onclose: move || show_drawer.set(false),
///         position: "left",
///         title: "Navigation",
///
///         NavLink { label: "Home", href: "/" }
///         NavLink { label: "About", href: "/about" }
///     }
/// }
/// ```
pub struct Drawer {
    /// Whether the drawer is open (static, for initial render or non-reactive use).
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    /// When provided, the drawer updates automatically when the signal changes.
    pub opened_fn: Option<ReactiveBool>,
    /// Drawer title.
    pub title: String,
    /// Position (left, right, top, bottom).
    pub position: String,
    /// Size variant (xs, sm, md, lg, xl, full).
    pub size: String,
    /// Whether to show overlay backdrop.
    pub with_overlay: bool,
    /// Overlay opacity (0-1).
    pub overlay_opacity: Option<f32>,
    /// Whether clicking overlay closes drawer.
    pub close_on_click_outside: bool,
    /// Whether pressing Escape closes drawer.
    pub close_on_escape: bool,
    /// Whether to show close button.
    pub with_close_button: bool,
    /// Padding inside drawer.
    pub padding: String,
    /// Z-index for the drawer.
    pub z_index: Option<i32>,
    /// Whether to lock scroll when open.
    pub lock_scroll: bool,
    /// Whether to trap focus inside drawer.
    pub trap_focus: bool,
    /// Callback when drawer should close.
    pub onclose: Option<rinch_core::Callback>,
}

impl std::fmt::Debug for Drawer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drawer")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("title", &self.title)
            .field("position", &self.position)
            .field("size", &self.size)
            .field("with_overlay", &self.with_overlay)
            .field("overlay_opacity", &self.overlay_opacity)
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

impl Default for Drawer {
    fn default() -> Self {
        Self {
            opened: false,
            opened_fn: None,
            title: String::new(),
            position: String::new(),
            size: String::new(),
            with_overlay: true,
            overlay_opacity: None,
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

impl Drawer {
    pub fn class_string(&self) -> String {
        self.class_string_with_opened(self.opened)
    }

    pub fn class_string_with_opened(&self, opened: bool) -> String {
        let mut classes = vec!["rinch-drawer"];

        if !self.position.is_empty() {
            if let Ok(pos) = self.position.parse::<DrawerPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(DrawerPosition::Left.class_name());
        }

        if !self.size.is_empty() {
            if let Ok(size) = self.size.parse::<DrawerSize>() {
                classes.push(size.class_name());
            }
        } else {
            classes.push(DrawerSize::Md.class_name());
        }

        if opened {
            classes.push("rinch-drawer--opened");
        }

        classes.join(" ")
    }
}

impl Component for Drawer {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Determine initial opened state
        let is_opened = if let Some(ref opened_fn) = self.opened_fn {
            opened_fn()
        } else {
            self.opened
        };

        let class = self.class_string_with_opened(is_opened);

        // Build overlay styles
        let overlay_style = self
            .overlay_opacity
            .map(|o| format!("--rinch-drawer-overlay-opacity: {}", o));

        // Build content styles
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
            "rinch-drawer__root".to_string()
        } else {
            "rinch-drawer__root rinch-drawer__root--hidden".to_string()
        };

        let root = rinch_macros::rsx! { div { class: "rinch-drawer__root" } };
        root.set_attribute("class", &root_class);

        // Build the overlay
        if self.with_overlay {
            let overlay = rinch_macros::rsx! { div { class: "rinch-drawer__overlay" } };
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

        // Build drawer div
        let drawer_div = rinch_macros::rsx! { div { class: "rinch-drawer" } };
        drawer_div.set_attribute("class", &class);

        // Header section with title and close button inside
        if !self.title.is_empty() || self.with_close_button {
            let header = rinch_macros::rsx! { div { class: "rinch-drawer__header" } };

            // Title takes up remaining space
            if !self.title.is_empty() {
                let title_el = rinch_macros::rsx! { h2 { class: "rinch-drawer__title" } };
                let title_text = __scope.create_text(&self.title);
                title_el.append_child(&title_text);
                header.append_child(&title_el);
            } else {
                // Spacer when no title
                let spacer = rinch_macros::rsx! { div { class: "rinch-drawer__spacer" } };
                header.append_child(&spacer);
            }

            // Close button at the end
            if self.with_close_button {
                let btn = rinch_macros::rsx! { button { class: "rinch-drawer__close" } };
                if let Some(handler_id) = close_handler_id {
                    btn.set_attribute("data-rid", &handler_id.to_string());
                }
                let close_icon = crate::icons::close_icon_lines_dom(__scope);
                btn.append_child(&close_icon);
                header.append_child(&btn);
            }

            drawer_div.append_child(&header);
        }

        // Build drawer body
        let body = rinch_macros::rsx! { div { class: "rinch-drawer__body" } };
        if let Some(ref style) = content_style {
            body.set_attribute("style", style);
        }
        for child in children {
            body.append_child(child);
        }
        drawer_div.append_child(&body);

        // If reactive opened_fn is provided, create an Effect to toggle visibility via class
        if let Some(ref opened_fn) = self.opened_fn {
            let opened_fn = opened_fn.clone();
            let root_clone = root.clone();
            let drawer_clone = drawer_div.clone();
            let body_clone = body.clone();
            let base_class = class.replace(" rinch-drawer--opened", "");

            __scope.create_effect(move || {
                let is_open = opened_fn();
                if is_open {
                    root_clone.set_attribute("class", "rinch-drawer__root");
                    drawer_clone
                        .set_attribute("class", &format!("{} rinch-drawer--opened", base_class));
                    // Reset scroll position on drawer body when opening
                    body_clone.set_scroll_top(0.0);
                } else {
                    root_clone
                        .set_attribute("class", "rinch-drawer__root rinch-drawer__root--hidden");
                    drawer_clone.set_attribute("class", &base_class);
                }
            });
        }

        root.append_child(&drawer_div);

        root
    }
}
