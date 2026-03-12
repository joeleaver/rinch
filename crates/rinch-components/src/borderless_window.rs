//! Borderless Window component.
//!
//! A container component for borderless/transparent windows with a custom titlebar,
//! window controls, rounded corners, and proper drag/resize handling.
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! #[component]
//! fn app() -> NodeHandle {
//!     rsx! {
//!         BorderlessWindow {
//!             title: "My App",
//!             on_minimize: || minimize_current_window(),
//!             on_maximize: || toggle_maximize_current_window(),
//!             on_close: || close_current_window(),
//!             // Content goes here
//!             div { "Hello, world!" }
//!         }
//!     }
//! }
//! ```

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::Callback;
use std::rc::Rc;

/// Callback type for section renderers.
pub type SectionRenderer = Rc<dyn Fn(&mut RenderScope) -> NodeHandle>;

/// Corner radius size for the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowRadius {
    /// No rounded corners.
    None,
    /// Extra small radius (4px).
    Xs,
    /// Small radius (6px).
    Sm,
    /// Medium radius (8px).
    #[default]
    Md,
    /// Large radius (12px).
    Lg,
    /// Extra large radius (16px).
    Xl,
}

impl WindowRadius {
    /// Get the CSS class name for this radius.
    pub fn class_name(&self) -> &'static str {
        match self {
            WindowRadius::None => "rinch-borderlesswindow--radius-none",
            WindowRadius::Xs => "rinch-borderlesswindow--radius-xs",
            WindowRadius::Sm => "rinch-borderlesswindow--radius-sm",
            WindowRadius::Md => "rinch-borderlesswindow--radius-md",
            WindowRadius::Lg => "rinch-borderlesswindow--radius-lg",
            WindowRadius::Xl => "rinch-borderlesswindow--radius-xl",
        }
    }
}

impl std::str::FromStr for WindowRadius {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "0" => Ok(WindowRadius::None),
            "xs" => Ok(WindowRadius::Xs),
            "sm" => Ok(WindowRadius::Sm),
            "md" => Ok(WindowRadius::Md),
            "lg" => Ok(WindowRadius::Lg),
            "xl" => Ok(WindowRadius::Xl),
            _ => Err(()),
        }
    }
}

/// A borderless window container with custom titlebar and window controls.
///
/// This component provides a styled container for borderless/transparent windows,
/// including:
/// - Rounded corners
/// - Custom titlebar with drag support
/// - Window control buttons (minimize, maximize, close)
/// - Proper theming via CSS variables
///
/// **Important:** You must provide callbacks for window controls that call the
/// platform functions (minimize_current_window, toggle_maximize_current_window,
/// close_current_window) from the `rinch` crate.
pub struct BorderlessWindow {
    /// Window title displayed in the titlebar.
    pub title: String,
    /// Corner radius (none, xs, sm, md, lg, xl). Default: md.
    pub radius: String,
    /// Whether to show the minimize button. Default: true.
    pub show_minimize: bool,
    /// Whether to show the maximize button. Default: true.
    pub show_maximize: bool,
    /// Whether to show the close button. Default: true.
    pub show_close: bool,
    /// Custom left section content for the titlebar (e.g., menu button).
    pub left_section: Option<SectionRenderer>,
    /// Custom right section content before window controls.
    pub right_section: Option<SectionRenderer>,
    /// Callback when minimize is clicked. Should call minimize_current_window().
    pub on_minimize: Option<Callback>,
    /// Callback when maximize is clicked. Should call toggle_maximize_current_window().
    pub on_maximize: Option<Callback>,
    /// Callback when close is clicked. Should call close_current_window().
    pub on_close: Option<Callback>,
}

impl std::fmt::Debug for BorderlessWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorderlessWindow")
            .field("title", &self.title)
            .field("radius", &self.radius)
            .field("show_minimize", &self.show_minimize)
            .field("show_maximize", &self.show_maximize)
            .field("show_close", &self.show_close)
            .field("left_section", &self.left_section.as_ref().map(|_| "..."))
            .field("right_section", &self.right_section.as_ref().map(|_| "..."))
            .field("on_minimize", &self.on_minimize.as_ref().map(|_| "..."))
            .field("on_maximize", &self.on_maximize.as_ref().map(|_| "..."))
            .field("on_close", &self.on_close.as_ref().map(|_| "..."))
            .finish()
    }
}

impl Default for BorderlessWindow {
    fn default() -> Self {
        Self {
            title: String::new(),
            radius: String::new(),
            show_minimize: true,
            show_maximize: true,
            show_close: true,
            left_section: None,
            right_section: None,
            on_minimize: None,
            on_maximize: None,
            on_close: None,
        }
    }
}

impl BorderlessWindow {
    /// Generate the CSS class string for this window.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-borderlesswindow"];

        // Radius
        if !self.radius.is_empty() {
            if let Ok(r) = self.radius.parse::<WindowRadius>() {
                classes.push(r.class_name());
            }
        } else {
            classes.push(WindowRadius::default().class_name());
        }

        classes.join(" ")
    }
}

impl Component for BorderlessWindow {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Build the window structure
        let container = rinch_macros::rsx! {
            div { class: "rinch-borderlesswindow" }
        };
        container.set_attribute("class", &self.class_string());

        // Check for menu bar context early (needed to decide titlebar layout)
        let menu_ctx = rinch_core::try_use_context::<rinch_core::MenuBarContext>();

        let is_inline = menu_ctx
            .as_ref()
            .map(|ctx| ctx.layout == rinch_core::MenuBarLayout::InlineTitlebar)
            .unwrap_or(false);

        // Create titlebar (draggable for window movement)
        let titlebar = __scope.create_element("div");
        titlebar.set_attribute("class", "rinch-borderlesswindow__titlebar");
        titlebar.set_attribute("data-drag-window", "true");

        // Left section — only render in titlebar when NOT inline
        // (inline layout moves it to the menu layer)
        if !is_inline {
            let left = __scope.create_element("div");
            left.set_attribute("class", "rinch-borderlesswindow__left");
            if let Some(ref render_left) = self.left_section {
                let content = render_left(__scope);
                left.append_child(&content);
            }
            titlebar.append_child(&left);
        } else {
            // Inline mode: add a spacer to reserve titlebar space for the
            // absolutely-positioned menu items that overlay the titlebar.
            let spacer = __scope.create_element("div");
            spacer.set_attribute("class", "rinch-borderlesswindow__menu-spacer");
            if let Some(ref ctx) = menu_ctx {
                spacer.set_attribute("style", &format!("width: {}px;", ctx.spacer_width));
            }
            titlebar.append_child(&spacer);
        }

        // Title — hidden in inline mode (title is rendered in the inline menu row instead)
        let title_el = __scope.create_element("div");
        if is_inline {
            title_el.set_attribute("class", "rinch-borderlesswindow__title");
            // Empty spacer — title lives in the inline row
        } else {
            title_el.set_attribute("class", "rinch-borderlesswindow__title");
            if !self.title.is_empty() {
                let title_text = __scope.create_text(&self.title);
                title_el.append_child(&title_text);
            }
        }
        titlebar.append_child(&title_el);

        // Right section (custom content before controls)
        let right = __scope.create_element("div");
        right.set_attribute("class", "rinch-borderlesswindow__right");
        if let Some(ref render_right) = self.right_section {
            let content = render_right(__scope);
            right.append_child(&content);
        }
        titlebar.append_child(&right);

        // Window controls
        let controls = __scope.create_element("div");
        controls.set_attribute("class", "rinch-borderlesswindow__controls");

        // Minimize button
        if self.show_minimize {
            let min_btn = __scope.create_element("button");
            min_btn.set_attribute(
                "class",
                "rinch-borderlesswindow__control rinch-borderlesswindow__control--minimize",
            );
            min_btn.set_attribute("aria-label", "Minimize");

            // Create minimize icon (horizontal line)
            let min_icon = __scope.create_element("svg");
            min_icon.set_attribute("width", "10");
            min_icon.set_attribute("height", "10");
            min_icon.set_attribute("viewBox", "0 0 10 10");
            let min_line = __scope.create_element("path");
            min_line.set_attribute("d", "M0 5h10");
            min_line.set_attribute("stroke", "currentColor");
            min_line.set_attribute("stroke-width", "1");
            min_icon.append_child(&min_line);
            min_btn.append_child(&min_icon);

            // Register click handler
            if let Some(ref cb) = self.on_minimize {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                min_btn.set_attribute("data-rid", &handler_id.0.to_string());
            }

            controls.append_child(&min_btn);
        }

        // Maximize button
        if self.show_maximize {
            let max_btn = __scope.create_element("button");
            max_btn.set_attribute(
                "class",
                "rinch-borderlesswindow__control rinch-borderlesswindow__control--maximize",
            );
            max_btn.set_attribute("aria-label", "Maximize");

            // Create maximize icon (square)
            let max_icon = __scope.create_element("svg");
            max_icon.set_attribute("width", "10");
            max_icon.set_attribute("height", "10");
            max_icon.set_attribute("viewBox", "0 0 10 10");
            let max_rect = __scope.create_element("rect");
            max_rect.set_attribute("x", "1");
            max_rect.set_attribute("y", "1");
            max_rect.set_attribute("width", "8");
            max_rect.set_attribute("height", "8");
            max_rect.set_attribute("fill", "none");
            max_rect.set_attribute("stroke", "currentColor");
            max_rect.set_attribute("stroke-width", "1");
            max_icon.append_child(&max_rect);
            max_btn.append_child(&max_icon);

            // Register click handler
            if let Some(ref cb) = self.on_maximize {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                max_btn.set_attribute("data-rid", &handler_id.0.to_string());
            }

            controls.append_child(&max_btn);
        }

        // Close button
        if self.show_close {
            let close_btn = __scope.create_element("button");
            close_btn.set_attribute(
                "class",
                "rinch-borderlesswindow__control rinch-borderlesswindow__control--close",
            );
            close_btn.set_attribute("aria-label", "Close");

            // Create close icon (X)
            let close_icon = __scope.create_element("svg");
            close_icon.set_attribute("width", "10");
            close_icon.set_attribute("height", "10");
            close_icon.set_attribute("viewBox", "0 0 10 10");
            let close_path = __scope.create_element("path");
            close_path.set_attribute("d", "M1 1l8 8M9 1l-8 8");
            close_path.set_attribute("stroke", "currentColor");
            close_path.set_attribute("stroke-width", "1.5");
            close_icon.append_child(&close_path);
            close_btn.append_child(&close_icon);

            // Register click handler
            if let Some(ref cb) = self.on_close {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                close_btn.set_attribute("data-rid", &handler_id.0.to_string());
            }

            controls.append_child(&close_btn);
        }

        titlebar.append_child(&controls);
        container.append_child(&titlebar);

        // Content area
        let content = __scope.create_element("div");
        content.set_attribute("class", "rinch-borderlesswindow__content");
        if let Some(ref ctx) = menu_ctx {
            if ctx.bar_height > 0 {
                // Leave space for the absolutely-positioned menu bar (BelowTitlebar)
                content.set_attribute("style", &format!("padding-top: {}px;", ctx.bar_height));
            }
        }
        for child in children {
            content.append_child(child);
        }
        container.append_child(&content);

        if is_inline {
            // InlineTitlebar: build a menu-layer as the LAST child of the container.
            // This layer contains the overlay and an items-row that visually overlaps
            // the titlebar. The left_section (hamburger) is rendered into the items-row
            // so it stays aligned with the menu items.
            let menu_ctx = menu_ctx.unwrap();
            container.set_attribute("style", "position: relative;");

            let menu_layer = __scope.create_element("div");
            menu_layer.set_attribute("class", "rinch-app-menu-bar__inline-layer");

            // Overlay first in DOM (hit-tested last within layer)
            if let Some(ref overlay_renderer) = menu_ctx.overlay_renderer {
                let overlay = overlay_renderer(__scope);
                menu_layer.append_child(&overlay);
            }

            // Items-row last in DOM (hit-tested first within layer)
            let items_row = __scope.create_element("div");
            items_row.set_attribute("class", "rinch-app-menu-bar__inline-row");
            // Render left_section into the items-row (hamburger button)
            if let Some(ref render_left) = self.left_section {
                let left_content = render_left(__scope);
                items_row.append_child(&left_content);
            }
            // Render title as branded text with multi-color gradient effect
            if !self.title.is_empty() {
                let brand = __scope.create_element("div");
                brand.set_attribute("class", "rinch-borderlesswindow__brand");
                // Cycle through gradient colors for each character
                let colors = [
                    "#ff6b6b", "#feca57", "#48dbfb", "#ff9ff3", "#54a0ff", "#5f27cd",
                ];
                for (i, ch) in self.title.chars().enumerate() {
                    if ch == ' ' {
                        let space = __scope.create_text("\u{00A0}");
                        brand.append_child(&space);
                    } else {
                        let span = __scope.create_element("span");
                        span.set_attribute(
                            "style",
                            &format!("color: {};", colors[i % colors.len()]),
                        );
                        let ch_text = __scope.create_text(&ch.to_string());
                        span.append_child(&ch_text);
                        brand.append_child(&span);
                    }
                }
                items_row.append_child(&brand);
            }
            // Render menu items
            if let Some(ref items_renderer) = menu_ctx.items_renderer {
                let items = items_renderer(__scope);
                items_row.append_child(&items);
            }
            menu_layer.append_child(&items_row);

            container.append_child(&menu_layer);
        } else if let Some(ctx) = menu_ctx {
            // BelowTitlebar: menu bar as LAST child with absolute positioning
            container.set_attribute("style", "position: relative;");
            let menu_bar = (ctx.renderer)(__scope);
            container.append_child(&menu_bar);
        }

        container
    }
}
