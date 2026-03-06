//! Drag state management for slider-like and absolute-coordinate dragging.

use std::cell::RefCell;

/// Active drag state for slider-like components.
///
/// When a draggable element (like a slider) receives a mousedown event,
/// it can register itself as the active drag target. The runtime will
/// then call the drag callback on mouse move until mouseup.
pub struct DragState {
    /// Callback to invoke on mouse move with (percent_x, percent_y).
    pub on_drag: Box<dyn Fn(f32, f32) + 'static>,
    /// Element bounds for calculating percentages.
    pub element_x: f32,
    pub element_y: f32,
    pub element_width: f32,
    pub element_height: f32,
}

// Thread-local storage for active drag state.
thread_local! {
    static ACTIVE_DRAG: RefCell<Option<DragState>> = const { RefCell::new(None) };
}

/// Start a drag operation.
///
/// Call this from a mousedown handler to begin tracking mouse movement.
/// The callback will be invoked on each mouse move with (percent_x, percent_y)
/// values between 0.0 and 1.0.
///
/// # Example
///
/// ```ignore
/// // In a slider's onmousedown handler:
/// let value_signal = my_signal;
/// let ctx = get_click_context();
/// start_drag(
///     ctx.element_x,
///     ctx.element_y,
///     ctx.element_width,
///     ctx.element_height,
///     move |px, _py| {
///         value_signal.set((px * 100.0) as f64);
///     }
/// );
/// ```
pub fn start_drag<F>(
    element_x: f32,
    element_y: f32,
    element_width: f32,
    element_height: f32,
    on_drag: F,
) where
    F: Fn(f32, f32) + 'static,
{
    tracing::info!(
        "DRAG: start_drag called with bounds: ({}, {}, {}x{})",
        element_x,
        element_y,
        element_width,
        element_height
    );
    ACTIVE_DRAG.with(|drag| {
        *drag.borrow_mut() = Some(DragState {
            on_drag: Box::new(on_drag),
            element_x,
            element_y,
            element_width,
            element_height,
        });
    });
}

/// Active drag state for absolute-coordinate dragging (panels, windows).
///
/// Unlike `DragState` which normalizes to percentages, this passes raw
/// viewport pixel coordinates to the callback.
pub struct DragStateAbsolute {
    pub on_drag: Box<dyn Fn(f32, f32) + 'static>,
    pub on_end: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
}

thread_local! {
    static ACTIVE_DRAG_ABSOLUTE: RefCell<Option<DragStateAbsolute>> = const { RefCell::new(None) };
}

/// Start an absolute-coordinate drag operation.
///
/// The callback receives raw `(mouse_x, mouse_y)` viewport coordinates on
/// each mouse move. Use this for dragging panels, windows, or other elements
/// where you need pixel-level positioning.
///
/// # Example
///
/// ```ignore
/// let ctx = get_click_context();
/// let offset_x = ctx.mouse_x - panel_x.get();
/// let offset_y = ctx.mouse_y - panel_y.get();
/// start_drag_absolute(move |mx, my| {
///     panel_x.set(mx - offset_x);
///     panel_y.set(my - offset_y);
/// });
/// ```
pub fn start_drag_absolute<F>(on_drag: F)
where
    F: Fn(f32, f32) + 'static,
{
    // Clear percentage-based drag if any
    stop_drag();
    ACTIVE_DRAG_ABSOLUTE.with(|drag| {
        *drag.borrow_mut() = Some(DragStateAbsolute {
            on_drag: Box::new(on_drag),
            on_end: None,
        });
    });
}

/// Start an absolute-coordinate drag with an end callback.
///
/// Same as `start_drag_absolute`, but `on_end` is called once when the
/// drag finishes (mouseup) with the final `(mouse_x, mouse_y)`.
pub fn start_drag_absolute_with_end<F, E>(on_drag: F, on_end: E)
where
    F: Fn(f32, f32) + 'static,
    E: FnOnce(f32, f32) + 'static,
{
    // Clear percentage-based drag if any
    stop_drag();
    ACTIVE_DRAG_ABSOLUTE.with(|drag| {
        *drag.borrow_mut() = Some(DragStateAbsolute {
            on_drag: Box::new(on_drag),
            on_end: Some(Box::new(on_end)),
        });
    });
}

/// Finish and stop the absolute drag, calling `on_end` if set.
///
/// Called by the runtime on mouseup with the final cursor position.
pub fn finish_drag(mouse_x: f32, mouse_y: f32) {
    // Take the on_end callback from absolute drag before clearing.
    let on_end = ACTIVE_DRAG_ABSOLUTE.with(|drag| drag.borrow_mut().take().and_then(|s| s.on_end));
    // Clear percentage-based drag too.
    ACTIVE_DRAG.with(|drag| {
        *drag.borrow_mut() = None;
    });
    // Fire the end callback after clearing state.
    if let Some(cb) = on_end {
        cb(mouse_x, mouse_y);
    }
}

/// Stop the current drag operation (both percentage and absolute).
/// Does NOT call `on_end` — use `finish_drag` for that.
pub fn stop_drag() {
    ACTIVE_DRAG.with(|drag| {
        *drag.borrow_mut() = None;
    });
    ACTIVE_DRAG_ABSOLUTE.with(|drag| {
        *drag.borrow_mut() = None;
    });
}

/// Check if a drag operation is active (either percentage or absolute).
pub fn is_dragging() -> bool {
    ACTIVE_DRAG.with(|drag| drag.borrow().is_some())
        || ACTIVE_DRAG_ABSOLUTE.with(|drag| drag.borrow().is_some())
}

/// Update the drag position (called by the runtime on mouse move).
///
/// Returns true if a drag callback was invoked.
pub fn update_drag(mouse_x: f32, mouse_y: f32) -> bool {
    // Check absolute drag first
    let abs_handled = ACTIVE_DRAG_ABSOLUTE.with(|drag| {
        if let Some(ref state) = *drag.borrow() {
            (state.on_drag)(mouse_x, mouse_y);
            true
        } else {
            false
        }
    });
    if abs_handled {
        return true;
    }

    // Fall back to percentage-based drag
    ACTIVE_DRAG.with(|drag| {
        if let Some(ref state) = *drag.borrow() {
            let percent_x = if state.element_width > 0.0 {
                ((mouse_x - state.element_x) / state.element_width).clamp(0.0, 1.0)
            } else {
                tracing::warn!("DRAG: element_width is 0!");
                0.0
            };
            let percent_y = if state.element_height > 0.0 {
                ((mouse_y - state.element_y) / state.element_height).clamp(0.0, 1.0)
            } else {
                0.0
            };
            tracing::debug!(
                "DRAG: mouse=({}, {}), element=({}, {}, {}x{}), percent=({:.2}, {:.2})",
                mouse_x,
                mouse_y,
                state.element_x,
                state.element_y,
                state.element_width,
                state.element_height,
                percent_x,
                percent_y
            );
            (state.on_drag)(percent_x, percent_y);
            true
        } else {
            false
        }
    })
}
