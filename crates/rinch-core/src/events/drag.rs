//! Pointer capture drag API.
//!
//! Use `Drag::absolute()` or `Drag::percent()` to start tracking mouse
//! movement from a click handler until mouseup.

use std::cell::RefCell;

use crate::events::get_click_context;

/// How coordinates are delivered to the `on_move` callback.
enum DragMode {
    /// Raw viewport pixel coordinates.
    Absolute,
    /// Normalized 0.0–1.0 relative to element bounds captured at start.
    Percent {
        element_x: f32,
        element_y: f32,
        element_width: f32,
        element_height: f32,
    },
}

/// Internal state for an active drag.
struct ActiveDrag {
    mode: DragMode,
    on_move: Box<dyn Fn(f32, f32) + 'static>,
    on_end: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
}

thread_local! {
    static ACTIVE_DRAG: RefCell<Option<ActiveDrag>> = const { RefCell::new(None) };
}

/// Builder for starting a pointer capture drag.
///
/// # Examples
///
/// ```ignore
/// // Drag a panel with absolute coordinates
/// let ctx = get_click_context();
/// let offset_x = ctx.mouse_x - panel_x.get();
/// Drag::absolute()
///     .on_move(move |x, y| panel_x.set(x - offset_x))
///     .on_end(move |x, y| save_position(x, y))
///     .start();
///
/// // Slider with percentage coordinates
/// Drag::percent()
///     .on_move(move |px, _| slider.set(px * 100.0))
///     .start();
/// ```
pub struct Drag {
    mode: DragMode,
    on_move: Option<Box<dyn Fn(f32, f32) + 'static>>,
    on_end: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
}

impl Drag {
    /// Start building a drag that delivers raw viewport pixel coordinates.
    pub fn absolute() -> Self {
        Self {
            mode: DragMode::Absolute,
            on_move: None,
            on_end: None,
        }
    }

    /// Start building a drag that delivers 0.0–1.0 percentage coordinates.
    ///
    /// Reads element bounds from the current `ClickContext` automatically.
    pub fn percent() -> Self {
        let ctx = get_click_context();
        Self {
            mode: DragMode::Percent {
                element_x: ctx.element_x,
                element_y: ctx.element_y,
                element_width: ctx.element_width,
                element_height: ctx.element_height,
            },
            on_move: None,
            on_end: None,
        }
    }

    /// Set the callback invoked on each mouse move during the drag.
    pub fn on_move<F: Fn(f32, f32) + 'static>(mut self, f: F) -> Self {
        self.on_move = Some(Box::new(f));
        self
    }

    /// Set the callback invoked once when the drag ends (mouseup).
    pub fn on_end<F: FnOnce(f32, f32) + 'static>(mut self, f: F) -> Self {
        self.on_end = Some(Box::new(f));
        self
    }

    /// Activate the drag. Call from a mousedown/click handler.
    pub fn start(self) {
        let on_move = self.on_move.unwrap_or_else(|| Box::new(|_, _| {}));
        ACTIVE_DRAG.with(|drag| {
            *drag.borrow_mut() = Some(ActiveDrag {
                mode: self.mode,
                on_move,
                on_end: self.on_end,
            });
        });
    }

    /// Cancel the active drag without firing `on_end`.
    pub fn cancel() {
        ACTIVE_DRAG.with(|drag| {
            *drag.borrow_mut() = None;
        });
    }

    /// Check if a drag is currently active.
    pub fn is_active() -> bool {
        ACTIVE_DRAG.with(|drag| drag.borrow().is_some())
    }
}

/// Update the drag position. Called by the runtime on mouse move.
/// Returns true if a drag callback was invoked.
pub fn update_drag(mouse_x: f32, mouse_y: f32) -> bool {
    ACTIVE_DRAG.with(|drag| {
        if let Some(ref state) = *drag.borrow() {
            let (x, y) = match &state.mode {
                DragMode::Absolute => (mouse_x, mouse_y),
                DragMode::Percent {
                    element_x,
                    element_y,
                    element_width,
                    element_height,
                } => {
                    let px = if *element_width > 0.0 {
                        ((mouse_x - element_x) / element_width).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let py = if *element_height > 0.0 {
                        ((mouse_y - element_y) / element_height).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    (px, py)
                }
            };
            (state.on_move)(x, y);
            true
        } else {
            false
        }
    })
}

/// Finish the drag, firing `on_end` if set. Called by the runtime on mouseup.
pub fn finish_drag(mouse_x: f32, mouse_y: f32) {
    let on_end = ACTIVE_DRAG.with(|drag| drag.borrow_mut().take().and_then(|s| s.on_end));
    if let Some(cb) = on_end {
        cb(mouse_x, mouse_y);
    }
}
