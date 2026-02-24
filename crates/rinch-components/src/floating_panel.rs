//! FloatingPanel component.
//!
//! A draggable, resizable floating panel for desktop app patterns —
//! tool panels, property inspectors, floating toolbars.

use rinch_core::{
    Callback, Component, Signal,
    dom::{NodeHandle, RenderScope},
    get_click_context, start_drag_absolute,
};

/// A draggable, resizable floating panel.
///
/// Position and size are controlled via `Signal<f32>` props, enabling
/// both programmatic control and reactive updates. The panel writes
/// back to these signals during drag/resize operations.
///
/// # Example
///
/// ```ignore
/// let x = Signal::new(100.0);
/// let y = Signal::new(100.0);
/// let w = Signal::new(300.0);
/// let h = Signal::new(200.0);
///
/// rsx! {
///     FloatingPanel {
///         title: "Properties",
///         x: x,
///         y: y,
///         width: w,
///         height: h,
///         on_close: move || { /* hide panel */ },
///         div { "Panel content here" }
///     }
/// }
/// ```
pub struct FloatingPanel {
    /// Title displayed in the panel header.
    pub title: String,
    /// X position signal (viewport pixels).
    pub x: Option<Signal<f32>>,
    /// Y position signal (viewport pixels).
    pub y: Option<Signal<f32>>,
    /// Width signal (viewport pixels).
    pub width: Option<Signal<f32>>,
    /// Height signal (viewport pixels).
    pub height: Option<Signal<f32>>,
    /// Minimum width constraint (default: 200.0).
    pub min_width: Option<f32>,
    /// Minimum height constraint (default: 100.0).
    pub min_height: Option<f32>,
    /// Whether the panel can be resized (default: true).
    pub resizable: bool,
    /// Callback when close button is clicked.
    pub on_close: Option<Callback>,
}

impl Default for FloatingPanel {
    fn default() -> Self {
        Self {
            title: String::new(),
            x: None,
            y: None,
            width: None,
            height: None,
            min_width: None,
            min_height: None,
            resizable: true,
            on_close: None,
        }
    }
}

impl std::fmt::Debug for FloatingPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloatingPanel")
            .field("title", &self.title)
            .field("min_width", &self.min_width)
            .field("min_height", &self.min_height)
            .field("resizable", &self.resizable)
            .field("on_close", &self.on_close.as_ref().map(|_| "<callback>"))
            .finish_non_exhaustive()
    }
}

impl Component for FloatingPanel {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Read initial values from signals (or use defaults)
        let x_sig = self.x.unwrap_or_else(|| Signal::new(50.0));
        let y_sig = self.y.unwrap_or_else(|| Signal::new(50.0));
        let w_sig = self.width.unwrap_or_else(|| Signal::new(300.0));
        let h_sig = self.height.unwrap_or_else(|| Signal::new(200.0));

        let init_x = x_sig.get();
        let init_y = y_sig.get();
        let init_w = w_sig.get();
        let init_h = h_sig.get();

        let min_w = self.min_width.unwrap_or(200.0);
        let min_h = self.min_height.unwrap_or(100.0);
        let resizable = self.resizable;

        // Root container — position: absolute
        let root = scope.create_element("div");
        root.set_attribute("class", "rinch-floating-panel");
        root.set_style("left", &format!("{}px", init_x));
        root.set_style("top", &format!("{}px", init_y));
        root.set_style("width", &format!("{}px", init_w));
        root.set_style("height", &format!("{}px", init_h));

        // === Header (drag handle) ===
        let header = scope.create_element("div");
        header.set_attribute("class", "rinch-floating-panel__header");

        // Title
        if !self.title.is_empty() {
            let title_span = scope.create_element("span");
            title_span.set_attribute("class", "rinch-floating-panel__title");
            let title_text = scope.create_text(&self.title);
            title_span.append_child(&title_text);
            header.append_child(&title_span);
        }

        // Close button
        if let Some(ref cb) = self.on_close {
            let close_btn = scope.create_element("button");
            close_btn.set_attribute("class", "rinch-floating-panel__close");
            close_btn.set_attribute("aria-label", "Close panel");
            let close_text = scope.create_text("\u{00D7}"); // × character
            close_btn.append_child(&close_text);

            let handler_id = scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            close_btn.set_attribute("data-rid", &handler_id.to_string());
            header.append_child(&close_btn);
        }

        // Drag-to-move handler on header
        {
            let handler_id = scope.register_handler(move || {
                let ctx = get_click_context();
                let offset_x = ctx.mouse_x - x_sig.get();
                let offset_y = ctx.mouse_y - y_sig.get();
                start_drag_absolute(move |mx, my| {
                    x_sig.set(mx - offset_x);
                    y_sig.set(my - offset_y);
                });
            });
            header.set_attribute("data-rid", &handler_id.to_string());
        }

        root.append_child(&header);

        // === Body (children go here) ===
        let body = scope.create_element("div");
        body.set_attribute("class", "rinch-floating-panel__body");
        for child in children {
            body.append_child(child);
        }
        root.append_child(&body);

        // === Resize footer + handle (bottom-right corner) ===
        if resizable {
            // Footer spacer keeps the scrollbar above the resize handle
            let footer = scope.create_element("div");
            footer.set_attribute("class", "rinch-floating-panel__footer");
            root.append_child(&footer);

            let resize_handle = scope.create_element("div");
            resize_handle.set_attribute("class", "rinch-floating-panel__resize");

            let handler_id = scope.register_handler(move || {
                let ctx = get_click_context();
                let start_w = w_sig.get();
                let start_h = h_sig.get();
                let start_mx = ctx.mouse_x;
                let start_my = ctx.mouse_y;
                start_drag_absolute(move |mx, my| {
                    w_sig.set((start_w + mx - start_mx).max(min_w));
                    h_sig.set((start_h + my - start_my).max(min_h));
                });
            });
            resize_handle.set_attribute("data-rid", &handler_id.to_string());
            root.append_child(&resize_handle);
        }

        // === Surgical style Effects ===

        // Position effect
        scope.create_effect({
            let root = root.clone();
            move || {
                root.set_style("left", &format!("{}px", x_sig.get()));
                root.set_style("top", &format!("{}px", y_sig.get()));
            }
        });

        // Size effect
        scope.create_effect({
            let root = root.clone();
            move || {
                root.set_style("width", &format!("{}px", w_sig.get()));
                root.set_style("height", &format!("{}px", h_sig.get()));
            }
        });

        root
    }
}
