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
use rinch_core::reactive::Effect;
use std::rc::Rc;

mod controls_layout;

pub use controls_layout::{WindowControl, WindowControlsLayout};

/// The default [`BorderlessWindow::titlebar_height`], in px. The stylesheet's
/// fallback for `--rinch-titlebar-height` (`styles/borderless_window.rs`) is
/// the same number, for a titlebar rendered without the component.
pub const DEFAULT_TITLEBAR_HEIGHT: u32 = 36;

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
    /// Window title displayed in the titlebar, as plain text in the titlebar's
    /// text colour (`.rinch-borderlesswindow__title`, or
    /// `.rinch-borderlesswindow__brand` in the inline menu row).
    pub title: String,
    /// Renders the title as nodes, in place of [`title`](BorderlessWindow::title)
    /// (issue #2196): a logo, styled or coloured text. It goes in the same
    /// element the title text would.
    pub title_section: Option<SectionRenderer>,
    /// Which window controls go at which end of the titlebar, in order
    /// (issue #2196). `None`, the default, follows the platform
    /// ([`WindowControlsLayout::platform`]): the traffic lights on the left on
    /// macOS, the desktop's own layout on Linux, minimize/maximize/close on the
    /// right elsewhere. A control left out of the layout is not drawn, and
    /// `show_*` still hides one that is in it.
    pub controls_layout: Option<WindowControlsLayout>,
    /// Titlebar height in px. Default: 36 ([`DEFAULT_TITLEBAR_HEIGHT`]).
    ///
    /// Written as `--rinch-titlebar-height` on the window, which sizes the
    /// titlebar, its control buttons and the inline menu row, and counted into
    /// `--rinch-window-top-inset`; a below-titlebar menu bar is placed under
    /// it. Change the height here rather than in CSS, or the inset and the
    /// menu bar do not follow.
    pub titlebar_height: u32,
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
            .field("title_section", &self.title_section.as_ref().map(|_| "..."))
            .field("controls_layout", &self.controls_layout)
            .field("titlebar_height", &self.titlebar_height)
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
            title_section: None,
            controls_layout: None,
            titlebar_height: DEFAULT_TITLEBAR_HEIGHT,
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

/// An empty `.rinch-borderlesswindow__controls` box for one end.
fn controls_container(__scope: &mut RenderScope, side: &str) -> NodeHandle {
    let controls = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__controls" } };
    controls.set_attribute(
        "class",
        &format!("rinch-borderlesswindow__controls rinch-borderlesswindow__controls--{side}"),
    );
    controls
}

impl BorderlessWindow {
    /// The controls box for one end of the titlebar, or `None` when no control
    /// in `which` is shown.
    fn render_controls(
        &self,
        __scope: &mut RenderScope,
        which: &[WindowControl],
        side: &str,
    ) -> Option<NodeHandle> {
        let buttons: Vec<NodeHandle> = which
            .iter()
            .filter_map(|&control| self.render_control(__scope, control))
            .collect();
        if buttons.is_empty() {
            return None;
        }
        let controls = controls_container(__scope, side);
        for button in &buttons {
            controls.append_child(button);
        }
        Some(controls)
    }

    /// One control button, or `None` when its `show_*` is off.
    fn render_control(
        &self,
        __scope: &mut RenderScope,
        control: WindowControl,
    ) -> Option<NodeHandle> {
        let (shown, modifier, label, callback) = match control {
            WindowControl::Minimize => (
                self.show_minimize,
                "minimize",
                "Minimize",
                &self.on_minimize,
            ),
            WindowControl::Maximize => (
                self.show_maximize,
                "maximize",
                "Maximize",
                &self.on_maximize,
            ),
            WindowControl::Close => (self.show_close, "close", "Close", &self.on_close),
        };
        if !shown {
            return None;
        }
        let btn = rinch_macros::rsx! { button {} };
        btn.set_attribute(
            "class",
            &format!("rinch-borderlesswindow__control rinch-borderlesswindow__control--{modifier}"),
        );
        btn.set_attribute("aria-label", label);

        let icon = rinch_macros::rsx! { svg {} };
        icon.set_attribute("width", "10");
        icon.set_attribute("height", "10");
        icon.set_attribute("viewBox", "0 0 10 10");
        match control {
            // A horizontal line.
            WindowControl::Minimize => {
                let line = rinch_macros::rsx! { path {} };
                line.set_attribute("d", "M0 5h10");
                line.set_attribute("stroke", "currentColor");
                line.set_attribute("stroke-width", "1");
                icon.append_child(&line);
            }
            // A square.
            WindowControl::Maximize => {
                let rect = rinch_macros::rsx! { rect {} };
                rect.set_attribute("x", "1");
                rect.set_attribute("y", "1");
                rect.set_attribute("width", "8");
                rect.set_attribute("height", "8");
                rect.set_attribute("fill", "none");
                rect.set_attribute("stroke", "currentColor");
                rect.set_attribute("stroke-width", "1");
                icon.append_child(&rect);
            }
            // An X.
            WindowControl::Close => {
                let path = rinch_macros::rsx! { path {} };
                path.set_attribute("d", "M1 1l8 8M9 1l-8 8");
                path.set_attribute("stroke", "currentColor");
                path.set_attribute("stroke-width", "1.5");
                icon.append_child(&path);
            }
        }
        btn.append_child(&icon);

        if let Some(cb) = callback {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }
        Some(btn)
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

        // How much window chrome sits above the content: the titlebar always,
        // plus the menu bar when it renders below the titlebar (inline mode
        // draws it *inside* the titlebar, so it adds nothing). Flow content
        // clears this via the titlebar being in-flow and the content
        // padding-top below, but a `position: fixed` overlay resolves against
        // the real viewport — as it does in a browser and on rinch-web — so it
        // has no way to know. Custom properties inherit, so a fixed descendant
        // can opt in with `top: var(--rinch-window-top-inset, 0px)`.
        let titlebar_height = self.titlebar_height;
        let top_inset = titlebar_height
            + if is_inline {
                0
            } else {
                menu_ctx.as_ref().map(|ctx| ctx.bar_height).unwrap_or(0)
            };
        let inset_decl = format!(
            "--rinch-window-top-inset: {top_inset}px; --rinch-titlebar-height: {titlebar_height}px;"
        );
        container.set_attribute("style", &inset_decl);
        // Also publish it on `body`. Overlays are commonly rendered as a
        // root-level sibling of the window chrome rather than inside it (UI Zoo
        // does exactly this), so a declaration on the container alone would
        // never reach them — custom properties inherit down the DOM, not across
        // it. `set_style` writes a single property, so body's other styles are
        // left alone.
        __scope
            .body_handle()
            .set_style("--rinch-window-top-inset", &format!("{top_inset}px"));

        // Create titlebar (draggable for window movement)
        let titlebar = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__titlebar" } };
        titlebar.set_attribute("data-drag-window", "true");

        let layout = self
            .controls_layout
            .clone()
            .unwrap_or_else(WindowControlsLayout::platform);
        let left_controls = self.render_controls(__scope, &layout.left, "left");
        // Always present, even empty, so the right end has a box to hold.
        let right_controls = self
            .render_controls(__scope, &layout.right, "right")
            .or_else(|| Some(controls_container(__scope, "right")));
        if !is_inline && let Some(controls) = &left_controls {
            titlebar.append_child(controls);
        }

        // Left section — only render in titlebar when NOT inline
        // (inline layout moves it to the menu layer)
        let mut menu_spacer: Option<NodeHandle> = None;
        if !is_inline {
            let left = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__left" } };
            if let Some(ref render_left) = self.left_section {
                let content = render_left(__scope);
                left.append_child(&content);
            }
            titlebar.append_child(&left);
        } else {
            // Inline mode: add a spacer to reserve titlebar space for the
            // absolutely-positioned menu items that overlay the titlebar.
            // `spacer_width` is the shell's estimate, made before any
            // document existed; the effect below writes it, and it holds only
            // until the row has been laid out, after which the spacer follows
            // the row's measured box.
            let spacer =
                rinch_macros::rsx! { div { class: "rinch-borderlesswindow__menu-spacer" } };
            titlebar.append_child(&spacer);
            menu_spacer = Some(spacer);
        }

        // Title — hidden in inline mode (title is rendered in the inline menu row instead)
        let title_el = rinch_macros::rsx! { div {} };
        if is_inline {
            title_el.set_attribute("class", "rinch-borderlesswindow__title");
            // Empty spacer — title lives in the inline row
        } else {
            title_el.set_attribute("class", "rinch-borderlesswindow__title");
            if let Some(render_title) = &self.title_section {
                title_el.append_child(&render_title(__scope));
            } else if !self.title.is_empty() {
                let title_text = rinch_core::IntoNode::into_node(self.title.clone(), __scope);
                title_el.append_child(&title_text);
            }
        }
        titlebar.append_child(&title_el);

        // Right section (custom content before controls)
        let right = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__right" } };
        if let Some(ref render_right) = self.right_section {
            let content = render_right(__scope);
            right.append_child(&content);
        }
        titlebar.append_child(&right);

        // Window controls, at whichever ends the layout puts them (issue
        // #2196). The right-hand ones close the titlebar row; the left-hand
        // ones open it (or, inline, open the menu row drawn over it).
        if let Some(controls) = &right_controls {
            titlebar.append_child(controls);
        }
        container.append_child(&titlebar);

        // Content area
        let content = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__content" } };
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
            container.set_attribute("style", &format!("{inset_decl}position: relative;"));

            let menu_layer =
                rinch_macros::rsx! { div { class: "rinch-app-menu-bar__inline-layer" } };

            // Overlay first, matching its lower `z-index` — within one
            // stacking context DOM order only breaks ties at the same level,
            // so the `199`/`201` pair is what actually orders these two.
            if let Some(ref overlay_renderer) = menu_ctx.overlay_renderer {
                let overlay = overlay_renderer(__scope);
                menu_layer.append_child(&overlay);
            }

            // Items-row last, at the higher `z-index`.
            let items_row = rinch_macros::rsx! { div { class: "rinch-app-menu-bar__inline-row" } };
            // Left-hand window controls lead the row, as they lead the
            // titlebar outside inline mode: the row is drawn over the
            // titlebar's left end, so that is where they can be seen.
            if let Some(controls) = &left_controls {
                items_row.append_child(controls);
            }
            // Render left_section into the items-row (hamburger button)
            if let Some(ref render_left) = self.left_section {
                let left_content = render_left(__scope);
                items_row.append_child(&left_content);
            }
            // The title, as plain text (issue #2196: it used to be a fixed
            // six-colour rainbow in inline styles no stylesheet could
            // override). `title_section` replaces it with the app's own nodes.
            if let Some(render_title) = &self.title_section {
                let brand = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__brand" } };
                brand.append_child(&render_title(__scope));
                items_row.append_child(&brand);
            } else if !self.title.is_empty() {
                let brand = rinch_macros::rsx! { div { class: "rinch-borderlesswindow__brand", {self.title.clone()} } };
                items_row.append_child(&brand);
            }
            // Render menu items
            if let Some(ref items_renderer) = menu_ctx.items_renderer {
                let items = items_renderer(__scope);
                items_row.append_child(&items);
            }
            menu_layer.append_child(&items_row);

            // Size the spacer from the row as laid out (issue #529): every
            // label, the branded title and the left section, in whatever
            // script and font they shaped in. The shell can only estimate
            // that before the document exists, and no per-character count is
            // a text advance — UTF-8 bytes over-reserved Cyrillic 2x, a
            // narrow/wide table could not see a combining mark or a
            // proportional font, and neither counted the title.
            //
            // The spacer ends where the row ends: the row's right edge minus
            // the spacer's own left edge, both in the window root's **layout**
            // frame (issue #1375). The result is written as a CSS width, so
            // it has to be worked out in the frame CSS lays out in: the
            // painted boxes the `bounds_signal`s carry are scaled by any
            // transform on the window or above it, and a width taken from
            // them reserved half the row under `scale(0.5)`. The signals are
            // read only to re-run this when either box moves on screen.
            //
            // Neither edge depends on the spacer's width (the row is out of
            // flow in its own layer; the spacer's left edge is the titlebar's
            // padding), so this settles in one extra layout and re-runs only
            // when the row or the titlebar actually moves. Until the row has
            // a box the estimate stands, and this effect's first run — during
            // this render, before any layout — is what writes it.
            if let Some(spacer) = menu_spacer {
                let row_bounds = items_row.bounds_signal();
                let spacer_bounds = spacer.bounds_signal();
                let estimate = menu_ctx.spacer_width;
                let layer = menu_layer.clone();
                let row = items_row.clone();
                let titlebar = titlebar.clone();
                Effect::new(move || {
                    row_bounds.get();
                    spacer_bounds.get();
                    // Parent-relative layout boxes, summed up to the window
                    // root: root > layer > row, and root > titlebar > spacer.
                    let layout = |n: &NodeHandle| n.get_layout_bounds().unwrap_or_default();
                    let (layer_x, ..) = layout(&layer);
                    let (row_x, _, row_width, _) = layout(&row);
                    let (titlebar_x, ..) = layout(&titlebar);
                    let (spacer_x, ..) = layout(&spacer);
                    let width = if row_width > 0.0 {
                        (layer_x + row_x + row_width - (titlebar_x + spacer_x)).max(0.0)
                    } else {
                        estimate as f32
                    };
                    spacer.set_style("width", &format!("{width}px"));
                });
            }

            container.append_child(&menu_layer);
        } else if let Some(ctx) = menu_ctx {
            // BelowTitlebar: menu bar as LAST child with absolute positioning
            container.set_attribute("style", &format!("{inset_decl}position: relative;"));
            let menu_bar = (ctx.renderer)(__scope);
            // The shell placed the bar under a titlebar of the default height;
            // put it under this one.
            menu_bar.set_style("top", &format!("{titlebar_height}px"));
            container.append_child(&menu_bar);
        }

        container
    }
}
