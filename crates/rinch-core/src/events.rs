//! Event handling infrastructure for rinch.
//!
//! This module provides the event handler registry that maps element IDs
//! to Rust callbacks, enabling reactive event handling in the UI.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Text hit testing result from blitz's layout engine.
///
/// When a click lands on text content, blitz can resolve the exact
/// block and byte offset using parley's text layout data.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextHitInfo {
    /// The editor block index (from `data-block-index` attribute).
    pub block_index: usize,
    /// Byte offset within the block's text content.
    pub byte_offset: usize,
    /// The blitz DOM node ID of the inline root element.
    pub inline_root_node_id: usize,
    /// Whether this hit info is valid (was actually resolved from layout).
    pub valid: bool,
}

/// Click event context with mouse position and element bounds.
///
/// This data is available to event handlers during callback execution.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClickContext {
    /// Mouse X position relative to viewport.
    pub mouse_x: f32,
    /// Mouse Y position relative to viewport.
    pub mouse_y: f32,
    /// Clicked element's X position.
    pub element_x: f32,
    /// Clicked element's Y position.
    pub element_y: f32,
    /// Clicked element's width.
    pub element_width: f32,
    /// Clicked element's height.
    pub element_height: f32,
    /// Text hit info from blitz's layout engine (resolved via find_text_position).
    pub text_hit: TextHitInfo,
}

impl ClickContext {
    /// Get the mouse X position relative to the clicked element (0.0 to element_width).
    pub fn relative_x(&self) -> f32 {
        self.mouse_x - self.element_x
    }

    /// Get the mouse Y position relative to the clicked element (0.0 to element_height).
    pub fn relative_y(&self) -> f32 {
        self.mouse_y - self.element_y
    }

    /// Get the click position as a percentage of element width (0.0 to 1.0).
    pub fn percent_x(&self) -> f32 {
        if self.element_width > 0.0 {
            ((self.mouse_x - self.element_x) / self.element_width).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Get the click position as a percentage of element height (0.0 to 1.0).
    pub fn percent_y(&self) -> f32 {
        if self.element_height > 0.0 {
            ((self.mouse_y - self.element_y) / self.element_height).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

// Thread-local storage for the current click context.
thread_local! {
    static CLICK_CONTEXT: RefCell<ClickContext> = RefCell::new(ClickContext::default());
}

/// Active drag state for slider-like widgets.
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
    static ACTIVE_DRAG: RefCell<Option<DragState>> = RefCell::new(None);
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
pub fn start_drag<F>(element_x: f32, element_y: f32, element_width: f32, element_height: f32, on_drag: F)
where
    F: Fn(f32, f32) + 'static,
{
    tracing::info!(
        "DRAG: start_drag called with bounds: ({}, {}, {}x{})",
        element_x, element_y, element_width, element_height
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

/// Stop the current drag operation.
pub fn stop_drag() {
    ACTIVE_DRAG.with(|drag| {
        *drag.borrow_mut() = None;
    });
}

/// Check if a drag operation is active.
pub fn is_dragging() -> bool {
    ACTIVE_DRAG.with(|drag| drag.borrow().is_some())
}

/// Update the drag position (called by the runtime on mouse move).
///
/// Returns true if a drag callback was invoked.
pub fn update_drag(mouse_x: f32, mouse_y: f32) -> bool {
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
                mouse_x, mouse_y,
                state.element_x, state.element_y, state.element_width, state.element_height,
                percent_x, percent_y
            );
            (state.on_drag)(percent_x, percent_y);
            true
        } else {
            false
        }
    })
}

/// Escape HTML special characters in a string.
///
/// This is used at runtime for dynamic content in RSX.
pub fn html_escape_string(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Unique identifier for an event handler.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EventHandlerId(pub usize);

impl std::fmt::Display for EventHandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

use std::rc::Rc;

/// Type alias for event handler callbacks (click events).
/// Uses `Rc` for cloneability, allowing handlers to be extracted from the
/// registry before invocation (prevents RefCell borrow conflicts).
pub type EventCallback = Rc<dyn Fn() + 'static>;

/// Cloneable callback for input events.
///
/// Uses `Rc` for `Clone` support, allowing callbacks to be stored and invoked.
#[derive(Clone)]
pub struct InputCallback(pub Rc<dyn Fn(String)>);

impl InputCallback {
    /// Create a new input callback from a function.
    pub fn new<F: Fn(String) + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback with the input value.
    pub fn invoke(&self, value: String) {
        (self.0)(value)
    }
}

impl std::fmt::Debug for InputCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputCallback(...)")
    }
}

/// Global counter for generating unique event handler IDs.
static NEXT_HANDLER_ID: AtomicUsize = AtomicUsize::new(0);

/// Generate a new unique event handler ID.
pub fn next_handler_id() -> EventHandlerId {
    EventHandlerId(NEXT_HANDLER_ID.fetch_add(1, Ordering::SeqCst))
}

/// Reset the handler ID counter (useful for testing or re-rendering).
pub fn reset_handler_ids() {
    NEXT_HANDLER_ID.store(0, Ordering::SeqCst);
}

// Thread-local event handler registry.
thread_local! {
    static EVENT_REGISTRY: RefCell<EventRegistry> = RefCell::new(EventRegistry::new());
    static INPUT_REGISTRY: RefCell<InputRegistry> = RefCell::new(InputRegistry::new());
    static INPUT_CONTEXT: RefCell<InputContext> = RefCell::new(InputContext::default());
    /// Flag to signal that an input event was handled and a re-render may be needed.
    static INPUT_EVENT_HANDLED: RefCell<bool> = const { RefCell::new(false) };
}

/// Context for input events, providing the current input value.
#[derive(Debug, Clone, Default)]
pub struct InputContext {
    /// The current value of the input field.
    pub value: String,
    /// The node ID of the input element.
    pub node_id: usize,
}

/// Registry that maps event handler IDs to callbacks.
pub struct EventRegistry {
    handlers: HashMap<EventHandlerId, EventCallback>,
}

impl EventRegistry {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}

/// Registry that maps event handler IDs to input callbacks.
pub struct InputRegistry {
    handlers: HashMap<EventHandlerId, InputCallback>,
}

impl InputRegistry {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}

/// Register an event handler and return its ID.
///
/// The handler will be called when an element with the corresponding
/// `data-rid` attribute is clicked.
///
/// # Example
///
/// ```ignore
/// let id = register_handler(std::rc::Rc::new(|| {
///     println!("Button clicked!");
/// }));
/// // The element should have: data-rid="{id}"
/// ```
pub fn register_handler(callback: EventCallback) -> EventHandlerId {
    let id = next_handler_id();
    tracing::debug!("register_handler: Registered handler {:?}", id);
    EVENT_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.insert(id, callback);
    });
    id
}

/// Set the click context before dispatching an event.
///
/// This should be called by the shell before `dispatch_event` to provide
/// mouse position and element bounds to event handlers.
pub fn set_click_context(ctx: ClickContext) {
    CLICK_CONTEXT.with(|c| {
        *c.borrow_mut() = ctx;
    });
}

/// Get the current click context.
///
/// This can be called from event handlers to access mouse position and
/// element bounds for the current click event.
///
/// # Example
///
/// ```ignore
/// use rinch_core::events::get_click_context;
///
/// // In a slider's onclick handler:
/// let ctx = get_click_context();
/// let percent = ctx.percent_x(); // 0.0 to 1.0
/// let value = min + percent * (max - min);
/// ```
pub fn get_click_context() -> ClickContext {
    CLICK_CONTEXT.with(|c| *c.borrow())
}

/// Dispatch an event to the handler with the given ID.
///
/// Returns `true` if a handler was found and called, `false` otherwise.
pub fn dispatch_event(id: EventHandlerId) -> bool {
    // Clone the handler out of the registry so we can release the borrow
    // before calling it. This allows handlers to register new handlers
    // (e.g., when Show toggles and renders new content with buttons).
    let handler: Option<EventCallback> = EVENT_REGISTRY.with(|registry| {
        let reg = registry.borrow();
        reg.handlers.get(&id).cloned()
    });

    if let Some(h) = handler {
        tracing::info!("dispatch_event: Calling handler {:?}", id);
        h();
        tracing::info!("dispatch_event: Handler {:?} completed", id);
        true
    } else {
        let (count, handler_ids) = EVENT_REGISTRY.with(|registry| {
            let reg = registry.borrow();
            let ids: Vec<_> = reg.handlers.keys().cloned().collect();
            (ids.len(), ids)
        });
        tracing::error!("dispatch_event: handler {:?} NOT FOUND. Registry has {} handlers: {:?}",
                  id, count, handler_ids);
        false
    }
}

/// Register an input event handler and return its ID.
///
/// The handler will be called when an element with the corresponding
/// `data-oninput` attribute receives input.
pub fn register_input_handler(callback: InputCallback) -> EventHandlerId {
    let id = next_handler_id();
    INPUT_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.insert(id, callback);
    });
    id
}

/// Set the input context before dispatching an input event.
pub fn set_input_context(ctx: InputContext) {
    INPUT_CONTEXT.with(|c| {
        *c.borrow_mut() = ctx;
    });
}

/// Get the current input context.
pub fn get_input_context() -> InputContext {
    INPUT_CONTEXT.with(|c| c.borrow().clone())
}

/// Dispatch an input event to the handler with the given ID.
///
/// Returns `true` if a handler was found and called, `false` otherwise.
/// Also sets the `INPUT_EVENT_HANDLED` flag which can be checked with
/// `check_and_clear_input_handled()`.
pub fn dispatch_input_event(id: EventHandlerId, value: String) -> bool {
    INPUT_REGISTRY.with(|registry| {
        if let Some(handler) = registry.borrow().handlers.get(&id) {
            handler.invoke(value);
            // Signal that an input event was handled - caller should re-render
            INPUT_EVENT_HANDLED.with(|flag| *flag.borrow_mut() = true);
            true
        } else {
            false
        }
    })
}

/// Check if an input event was handled since last check, and clear the flag.
///
/// This allows the shell to know if a re-render is needed after processing
/// DOM events that might have triggered input handlers.
pub fn check_and_clear_input_handled() -> bool {
    INPUT_EVENT_HANDLED.with(|flag| {
        let handled = *flag.borrow();
        *flag.borrow_mut() = false;
        handled
    })
}

/// Clear all registered event handlers.
///
/// This should be called before re-rendering to avoid stale handlers.
pub fn clear_handlers() {
    EVENT_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.clear();
    });
    INPUT_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.clear();
    });
    reset_handler_ids();
}

/// Get the number of registered handlers (for debugging).
pub fn handler_count() -> usize {
    EVENT_REGISTRY.with(|registry| registry.borrow().handlers.len())
}

/// Keyboard event data for the global keyboard interceptor.
#[derive(Debug, Clone)]
pub struct KeyEventData {
    /// The logical key value (e.g., "a", "Enter", "Backspace")
    pub key: String,
    /// The physical key code (e.g., "KeyA", "Enter")
    pub code: String,
    /// Whether Ctrl/Cmd is pressed
    pub ctrl: bool,
    /// Whether Shift is pressed
    pub shift: bool,
    /// Whether Alt is pressed
    pub alt: bool,
    /// Whether Meta/Super is pressed
    pub meta: bool,
}

/// Type alias for the keyboard interceptor callback.
/// Returns true if the event was handled (should not propagate to blitz).
pub type KeyboardInterceptor = Rc<dyn Fn(&KeyEventData) -> bool>;

thread_local! {
    static KEYBOARD_INTERCEPTOR: RefCell<Option<KeyboardInterceptor>> = RefCell::new(None);
}

/// Set the global keyboard interceptor.
/// Only one interceptor can be active at a time.
pub fn set_keyboard_interceptor<F>(cb: F)
where
    F: Fn(&KeyEventData) -> bool + 'static,
{
    KEYBOARD_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global keyboard interceptor.
pub fn clear_keyboard_interceptor() {
    KEYBOARD_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = None;
    });
}

/// Dispatch a keyboard event to the interceptor.
/// Returns true if the event was handled.
pub fn dispatch_keyboard_event(data: &KeyEventData) -> bool {
    KEYBOARD_INTERCEPTOR.with(|i| {
        if let Some(ref interceptor) = *i.borrow() {
            interceptor(data)
        } else {
            false
        }
    })
}

// ============================================================================
// Text Selection Callback
// ============================================================================

/// Actions for controlling blitz's native text selection.
#[derive(Debug, Clone)]
pub enum SelectionAction {
    /// Set selection with anchor and focus (both node ID + byte offset).
    Set {
        anchor_node: usize,
        anchor_offset: usize,
        focus_node: usize,
        focus_offset: usize,
    },
    /// Extend selection to a viewport point (for drag-to-select).
    ExtendToPoint { x: f32, y: f32 },
    /// Clear the selection.
    Clear,
    /// Query the current selection ranges. Returns Vec<(node_id, start_offset, end_offset)>.
    QueryRanges,
}

/// Type alias for the text selection callback.
/// Returns selection ranges for QueryRanges, empty vec otherwise.
pub type SelectionCallback = Rc<dyn Fn(SelectionAction) -> Vec<(usize, usize, usize)>>;

thread_local! {
    static SELECTION_CALLBACK: RefCell<Option<SelectionCallback>> = RefCell::new(None);
    static SAVED_SELECTION: RefCell<Vec<(usize, usize, usize)>> = RefCell::new(Vec::new());
}

/// Set the global text selection callback.
/// The window manager sets this to delegate selection operations to BaseDocument.
pub fn set_selection_callback<F>(cb: F)
where
    F: Fn(SelectionAction) -> Vec<(usize, usize, usize)> + 'static,
{
    SELECTION_CALLBACK.with(|s| {
        *s.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global text selection callback.
pub fn clear_selection_callback() {
    SELECTION_CALLBACK.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Dispatch a selection action to the callback.
pub fn dispatch_selection(action: SelectionAction) -> Vec<(usize, usize, usize)> {
    SELECTION_CALLBACK.with(|s| {
        if let Some(ref cb) = *s.borrow() {
            cb(action)
        } else {
            Vec::new()
        }
    })
}

/// Save the current selection ranges as a snapshot.
/// Called before PointerDown dispatch to blitz, which may clear selection.
pub fn save_selection_snapshot() {
    let ranges = dispatch_selection(SelectionAction::QueryRanges);
    SAVED_SELECTION.with(|s| {
        *s.borrow_mut() = ranges;
    });
}

/// Clear the saved selection snapshot.
pub fn clear_selection_snapshot() {
    SAVED_SELECTION.with(|s| {
        s.borrow_mut().clear();
    });
}

/// Get the saved selection snapshot.
pub fn get_saved_selection() -> Vec<(usize, usize, usize)> {
    SAVED_SELECTION.with(|s| s.borrow().clone())
}

/// Query the current blitz text selection ranges.
/// Returns Vec<(node_id, start_byte_offset, end_byte_offset)>.
/// Falls back to the saved snapshot if the live selection is empty
/// (e.g., blitz cleared it on mousedown before a toolbar command runs).
pub fn query_selection_ranges() -> Vec<(usize, usize, usize)> {
    let live = dispatch_selection(SelectionAction::QueryRanges);
    if !live.is_empty() {
        return live;
    }
    // Fall back to saved snapshot (before blitz cleared it)
    get_saved_selection()
}

// ============================================================================
// Selection Sync Callback (mouseup drag-to-select → editor)
// ============================================================================

/// Callback invoked on mouseup to notify that drag selection may have changed.
/// The callback receives the current blitz selection ranges (block_index, start, end).
pub type SelectionSyncCallback = Rc<dyn Fn(Vec<(usize, usize, usize)>)>;

thread_local! {
    static SELECTION_SYNC_CALLBACK: RefCell<Option<SelectionSyncCallback>> = RefCell::new(None);
}

/// Set the callback invoked on mouseup to sync drag selection.
pub fn set_selection_sync_callback<F>(cb: F)
where
    F: Fn(Vec<(usize, usize, usize)>) + 'static,
{
    SELECTION_SYNC_CALLBACK.with(|s| {
        *s.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the selection sync callback.
pub fn clear_selection_sync_callback() {
    SELECTION_SYNC_CALLBACK.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Fire the selection sync callback with current LIVE ranges only.
/// Uses dispatch_selection directly instead of query_selection_ranges()
/// to avoid the saved-snapshot fallback (which is only for toolbar commands).
pub fn fire_selection_sync() {
    let ranges = dispatch_selection(SelectionAction::QueryRanges);
    SELECTION_SYNC_CALLBACK.with(|s| {
        if let Some(ref cb) = *s.borrow() {
            cb(ranges);
        }
    });
}

// --- Deferred selection clear ---
// When the DOM is rebuilt (e.g., editor re-render via Effect), blitz's text selection
// may reference nodes that no longer exist. The Effect cannot call dispatch_selection(Clear)
// directly because the blitz doc may be mutably borrowed by the event handler that triggered
// the Effect. Instead, the Effect sets this flag, and the window manager clears the selection
// in redraw() before paint, when no borrows are active.

thread_local! {
    static PENDING_SELECTION_CLEAR: Cell<bool> = const { Cell::new(false) };
}

/// Request that blitz's text selection be cleared before the next paint.
/// Safe to call from Effects (does not borrow the blitz doc).
pub fn request_selection_clear() {
    PENDING_SELECTION_CLEAR.with(|c| c.set(true));
}

/// Check and consume the pending selection clear flag.
/// Called by the window manager in redraw() before paint.
pub fn take_pending_selection_clear() -> bool {
    PENDING_SELECTION_CLEAR.with(|c| {
        if c.get() {
            c.set(false);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn test_register_and_dispatch() {
        clear_handlers();

        let called = Rc::new(Cell::new(false));
        let called_clone = called.clone();

        let id = register_handler(std::rc::Rc::new(move || {
            called_clone.set(true);
        }));

        assert!(!called.get());
        assert!(dispatch_event(id));
        assert!(called.get());
    }

    #[test]
    fn test_dispatch_unknown_id() {
        clear_handlers();

        let unknown_id = EventHandlerId(99999);
        assert!(!dispatch_event(unknown_id));
    }

    #[test]
    fn test_key_event_data_construction() {
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert_eq!(data.key, "a");
        assert_eq!(data.code, "KeyA");
        assert!(!data.ctrl);
    }

    #[test]
    fn test_keyboard_interceptor_set_and_dispatch() {
        clear_keyboard_interceptor();
        let called = Rc::new(Cell::new(false));
        let called_clone = called.clone();
        set_keyboard_interceptor(move |_data| {
            called_clone.set(true);
            true
        });
        let data = KeyEventData {
            key: "Enter".into(),
            code: "Enter".into(),
            ctrl: false, shift: false, alt: false, meta: false,
        };
        assert!(dispatch_keyboard_event(&data));
        assert!(called.get());
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_keyboard_interceptor_cleared() {
        clear_keyboard_interceptor();
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false, shift: false, alt: false, meta: false,
        };
        assert!(!dispatch_keyboard_event(&data));
    }

    #[test]
    fn test_keyboard_interceptor_returns_false() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| false);
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false, shift: false, alt: false, meta: false,
        };
        assert!(!dispatch_keyboard_event(&data));
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_keyboard_interceptor_replaced() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| false);
        set_keyboard_interceptor(|_| true);
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false, shift: false, alt: false, meta: false,
        };
        assert!(dispatch_keyboard_event(&data));
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_clear_handlers() {
        clear_handlers();

        let id = register_handler(std::rc::Rc::new(|| {}));
        assert_eq!(handler_count(), 1);

        clear_handlers();
        assert_eq!(handler_count(), 0);
        assert!(!dispatch_event(id));
    }
}

// ============================================================================
// Modifier State Tracking
// ============================================================================

thread_local! {
    static MODIFIER_STATE: RefCell<ModifierState> = const { RefCell::new(ModifierState {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    }) };
}

/// Current keyboard modifier state (Shift, Ctrl, Alt, Meta).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModifierState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Set the current modifier state.
///
/// This should be called by the shell whenever modifier keys change
/// (typically in the ModifiersChanged event handler).
pub fn set_modifier_state(state: ModifierState) {
    MODIFIER_STATE.with(|ms| {
        *ms.borrow_mut() = state;
    });
}

/// Get the current modifier state.
///
/// This can be called from event handlers to check if modifier keys
/// are currently pressed.
///
/// # Example
///
/// ```ignore
/// // In a click handler:
/// let mods = get_modifier_state();
/// if mods.shift {
///     // Extend selection
/// } else {
///     // Set cursor
/// }
/// ```
pub fn get_modifier_state() -> ModifierState {
    MODIFIER_STATE.with(|ms| *ms.borrow())
}
