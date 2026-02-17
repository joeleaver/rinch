//! Event handler registry for click and input events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::ClickContext;

/// Unique identifier for an event handler.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EventHandlerId(pub usize);

impl std::fmt::Display for EventHandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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

impl<F: Fn(String) + 'static> From<F> for InputCallback {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for InputCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputCallback(...)")
    }
}

impl crate::element::IntoEventHandler<InputCallback> for InputCallback {
    fn into_event_handler(self) -> InputCallback {
        self
    }
}

impl<F: Fn(String) + 'static> crate::element::IntoEventHandler<InputCallback> for F {
    fn into_event_handler(self) -> InputCallback {
        InputCallback::from(self)
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
    super::CLICK_CONTEXT.with(|c| {
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
    super::CLICK_CONTEXT.with(|c| *c.borrow())
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
        tracing::error!(
            "dispatch_event: handler {:?} NOT FOUND. Registry has {} handlers: {:?}",
            id,
            count,
            handler_ids
        );
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
