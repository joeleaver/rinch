//! Event handler registry for click, input, file-drop, key and wheel events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
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
    ///
    /// Runs untracked (issue #285): called from inside an effect, the
    /// handler's signal reads do not subscribe that effect.
    pub fn invoke(&self, value: String) {
        crate::reactive::untracked_handler(|| (self.0)(value))
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

/// Cloneable callback for OS file-drop events.
///
/// Receives the list of file paths dropped onto the element.
#[derive(Clone)]
pub struct FileDropCallback(pub Rc<dyn Fn(Vec<PathBuf>)>);

impl FileDropCallback {
    /// Create a new file-drop callback from a function.
    pub fn new<F: Fn(Vec<PathBuf>) + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback with the dropped file paths.
    ///
    /// Runs untracked (issue #285): called from inside an effect, the
    /// handler's signal reads do not subscribe that effect.
    pub fn invoke(&self, paths: Vec<PathBuf>) {
        crate::reactive::untracked_handler(|| (self.0)(paths))
    }
}

impl<F: Fn(Vec<PathBuf>) + 'static> From<F> for FileDropCallback {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for FileDropCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileDropCallback(...)")
    }
}

impl Default for FileDropCallback {
    fn default() -> Self {
        Self(Rc::new(|_| {}))
    }
}

impl crate::element::IntoEventHandler<FileDropCallback> for FileDropCallback {
    fn into_event_handler(self) -> FileDropCallback {
        self
    }
}

impl<F: Fn(Vec<PathBuf>) + 'static> crate::element::IntoEventHandler<FileDropCallback> for F {
    fn into_event_handler(self) -> FileDropCallback {
        FileDropCallback::from(self)
    }
}

/// Where a scroll container sits after it scrolled — the payload handed to a
/// [`ScrollCallback`].
///
/// `onscroll` means "this element scrolled", not "this element scrolled
/// vertically": one gesture can move a container on both axes, and a container
/// can be horizontal-only. Both offsets therefore travel together, so a
/// listener never has to go back to the DOM to learn what the other axis did
/// (issue #177).
///
/// `#[non_exhaustive]`: a field can be added in a minor release without that
/// being a breaking change for downstream code. Construct one with
/// [`ScrollEvent::new`] rather than a struct literal, and read fields by name.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollEvent {
    /// Distance the content is scrolled down, in logical pixels (`scrollTop`).
    pub scroll_top: f64,
    /// Distance the content is scrolled right, in logical pixels
    /// (`scrollLeft`).
    pub scroll_left: f64,
}

impl ScrollEvent {
    /// A scroll event at the given offsets.
    pub fn new(scroll_top: f64, scroll_left: f64) -> Self {
        Self {
            scroll_top,
            scroll_left,
        }
    }
}

/// Cloneable callback for scroll events.
///
/// Receives a [`ScrollEvent`] carrying the container's offset on **both** axes.
#[derive(Clone)]
pub struct ScrollCallback(pub Rc<dyn Fn(ScrollEvent)>);

impl ScrollCallback {
    /// Create a new scroll callback from a function.
    pub fn new<F: Fn(ScrollEvent) + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback with the container's current scroll offsets.
    ///
    /// Runs untracked (issue #285): called from inside an effect, the
    /// handler's signal reads do not subscribe that effect.
    pub fn invoke(&self, event: ScrollEvent) {
        crate::reactive::untracked_handler(|| (self.0)(event))
    }
}

impl<F: Fn(ScrollEvent) + 'static> From<F> for ScrollCallback {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for ScrollCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScrollCallback(...)")
    }
}

impl crate::element::IntoEventHandler<ScrollCallback> for ScrollCallback {
    fn into_event_handler(self) -> ScrollCallback {
        self
    }
}

impl<F: Fn(ScrollEvent) + 'static> crate::element::IntoEventHandler<ScrollCallback> for F {
    fn into_event_handler(self) -> ScrollCallback {
        ScrollCallback::from(self)
    }
}

/// Cloneable callback for an element's `onkeydown` / `onkeyup` (issue #1678).
///
/// Receives the key as [`KeyEventData`](super::KeyEventData) and answers
/// whether it **consumed** the key: `true` is the browser's `preventDefault()`
/// plus `stopPropagation()` — the element's own handling of the key (inserting
/// a line break, submitting, moving focus) does not happen, no ancestor's
/// handler is offered it, and neither the document-level keyboard interceptor
/// nor the dismiss stack sees the press. `false` lets the key go on as if the
/// handler were not there.
#[derive(Clone)]
pub struct KeyCallback(pub Rc<dyn Fn(&super::KeyEventData) -> bool>);

impl KeyCallback {
    /// Create a new key callback from a function.
    pub fn new<F: Fn(&super::KeyEventData) -> bool + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback with a key event; answers whether it consumed it.
    ///
    /// Runs untracked (issue #285), like every other app callback.
    pub fn invoke(&self, key: &super::KeyEventData) -> bool {
        crate::reactive::untracked_handler(|| (self.0)(key))
    }
}

impl std::fmt::Debug for KeyCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyCallback(...)")
    }
}

/// A wheel turn (or trackpad scroll, or pinch) over an element — the payload
/// handed to an element's `onwheel` handler.
///
/// The deltas have the DOM's sign and unit on every backend
/// (`WheelEvent.deltaX/deltaY`): **logical pixels**, positive `delta_y` when
/// the content would move up (the wheel turned towards the user, a two-finger
/// swipe up), positive `delta_x` when it would move left. A notched wheel's
/// lines are 40 px each, as the desktop scrolls them; a page is the window's
/// height.
///
/// A trackpad **pinch** arrives as a wheel with `modifiers.ctrl` set, as a
/// browser reports it: spreading the fingers is a negative `delta_y`, and
/// [`zoom_factor`](Self::zoom_factor) turns either a pinch or a Ctrl+wheel
/// into the same scale.
///
/// `#[non_exhaustive]`: build one with [`WheelEvent::new`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WheelEvent {
    /// Horizontal distance in logical pixels; positive scrolls right.
    pub delta_x: f32,
    /// Vertical distance in logical pixels; positive scrolls down.
    pub delta_y: f32,
    /// The pointer, in logical window pixels (the frame of
    /// [`ClickContext::mouse_x`](super::ClickContext::mouse_x)).
    pub x: f32,
    /// The pointer, in logical window pixels.
    pub y: f32,
    /// Shift / Ctrl / Alt / Meta at the time of the wheel. Ctrl is set for a
    /// trackpad pinch.
    pub modifiers: super::ModifierState,
}

impl WheelEvent {
    /// A wheel event with these deltas at `(x, y)`, no modifiers held.
    pub fn new(delta_x: f32, delta_y: f32, x: f32, y: f32) -> Self {
        Self {
            delta_x,
            delta_y,
            x,
            y,
            modifiers: super::ModifierState::default(),
        }
    }

    /// The same event with these modifiers.
    pub fn with_modifiers(mut self, modifiers: super::ModifierState) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The scale this wheel zooms by when it is read as a zoom (a pinch, or
    /// Ctrl/Cmd+wheel): 100 px of `delta_y` is a factor of *e*, the curve
    /// browsers and most canvas apps use. Above 1 zooms in.
    pub fn zoom_factor(&self) -> f32 {
        (-self.delta_y / 100.0).exp()
    }
}

/// Cloneable callback for an element's `onwheel`.
///
/// Receives the [`WheelEvent`] and answers whether it **consumed** it: `true`
/// stops the wheel — no ancestor's `onwheel` is offered it and no scroll
/// container scrolls (the browser's `preventDefault()`). `false` lets it go on
/// as if the handler were not there.
#[derive(Clone)]
pub struct WheelCallback(pub Rc<dyn Fn(&WheelEvent) -> bool>);

impl WheelCallback {
    /// Create a new wheel callback from a function.
    pub fn new<F: Fn(&WheelEvent) -> bool + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback; answers whether it consumed the wheel.
    ///
    /// Runs untracked (issue #285), like every other app callback.
    pub fn invoke(&self, event: &WheelEvent) -> bool {
        crate::reactive::untracked_handler(|| (self.0)(event))
    }
}

impl std::fmt::Debug for WheelCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WheelCallback(...)")
    }
}

/// Global counter for generating unique event handler IDs.
static NEXT_HANDLER_ID: AtomicUsize = AtomicUsize::new(0);

/// Generate a new unique event handler ID.
pub fn next_handler_id() -> EventHandlerId {
    EventHandlerId(NEXT_HANDLER_ID.fetch_add(1, Ordering::SeqCst))
}

/// Reset the handler ID counter (useful for testing or re-rendering).
///
/// # Warning
///
/// Rewinding a process-global counter aliases any [`EventHandlerId`] still
/// recorded against a live scope (issue #141): a later registration can reuse an
/// id that a surviving scope still believes it owns. This is safe today only
/// because every production caller runs at startup, before any scope exists.
/// Do not call it once a tree is mounted.
pub fn reset_handler_ids() {
    NEXT_HANDLER_ID.store(0, Ordering::SeqCst);
}

/// Remove a single handler, by id, from whichever registry holds it.
///
/// This is how scope disposal reclaims the handlers a scope owns (issue #141).
/// It replaced a `handler_id_watermark`/`remove_handlers_in_range` pair that
/// deleted a contiguous *range* of ids, which was both incomplete and unsound:
/// a range recorded around a synchronous build misses every handler a later
/// effect run registers, and a synchronous effect flush *inside* that build lets
/// an unrelated root allocate an id within the window, which the range would
/// then delete out from under it. A scope's own record has neither problem.
///
/// Each removed callback is moved into a local and dropped **after** its
/// registry borrow is released: the callbacks are `Rc<dyn Fn(..)>` closing over
/// arbitrary user state whose `Drop` may touch these same registries.
pub fn unregister_handler(id: EventHandlerId) {
    let click = EVENT_REGISTRY.with(|r| r.borrow_mut().handlers.remove(&id));
    drop(click);
    let input = INPUT_REGISTRY.with(|r| r.borrow_mut().handlers.remove(&id));
    drop(input);
    let file_drop = FILE_DROP_REGISTRY.with(|r| r.borrow_mut().handlers.remove(&id));
    drop(file_drop);
    let files_drop = FILES_DROP_REGISTRY.with(|r| r.borrow_mut().remove(&id));
    drop(files_drop);
    let scroll = SCROLL_REGISTRY.with(|r| r.borrow_mut().handlers.remove(&id));
    drop(scroll);
    let key = KEY_REGISTRY.with(|r| r.borrow_mut().remove(&id));
    drop(key);
    let wheel = WHEEL_REGISTRY.with(|r| r.borrow_mut().remove(&id));
    drop(wheel);
}

/// Whether a click handler is still registered under this id.
///
/// Scope disposal frees the handlers a scope owns (issue #141), so an id read
/// back out of a `data-rid` attribute can outlive its callback: nothing removes
/// the attribute when the handler goes. Dispatchers that walk an ancestor chain
/// use this to *skip* a dead node and keep walking, rather than consuming the
/// event on its behalf.
pub fn has_click_handler(id: EventHandlerId) -> bool {
    EVENT_REGISTRY.with(|registry| registry.borrow().handlers.contains_key(&id))
}

/// Whether an input handler is still registered under this id.
///
/// The counterpart to [`has_click_handler`] for callers that cache an id across
/// frames (the desktop shell's focused-input state) and must notice when the
/// element it belongs to has been torn down.
pub fn has_input_handler(id: EventHandlerId) -> bool {
    INPUT_REGISTRY.with(|registry| registry.borrow().handlers.contains_key(&id))
}

/// Whether a file-drop handler is still registered under this id.
/// See [`has_click_handler`].
pub fn has_file_drop_handler(id: EventHandlerId) -> bool {
    FILE_DROP_REGISTRY.with(|registry| registry.borrow().handlers.contains_key(&id))
}

/// Whether an `ondropfiles` handler (issue #1771) is still registered under
/// this id. See [`has_click_handler`].
pub fn has_files_drop_handler(id: EventHandlerId) -> bool {
    FILES_DROP_REGISTRY.with(|registry| registry.borrow().contains_key(&id))
}

/// A handler for files dropped from the OS, handed as
/// [`LocalFile`](crate::file::LocalFile)s (`ondropfiles`, issue #1771).
type FilesDropCallback = Rc<dyn Fn(Vec<crate::file::LocalFile>)>;

// Thread-local event handler registry.
thread_local! {
    // None of the four registries below are const-evaluable: each `::new()`
    // builds a `HashMap` with the default `RandomState`, which has no
    // `const fn` constructor.
    static EVENT_REGISTRY: RefCell<EventRegistry> = RefCell::new(EventRegistry::new());
    static INPUT_REGISTRY: RefCell<InputRegistry> = RefCell::new(InputRegistry::new());
    static FILE_DROP_REGISTRY: RefCell<FileDropRegistry> = RefCell::new(FileDropRegistry::new());
    static SCROLL_REGISTRY: RefCell<ScrollRegistry> = RefCell::new(ScrollRegistry::new());
    /// `ondropfiles` handlers (issue #1771).
    static FILES_DROP_REGISTRY: RefCell<HashMap<EventHandlerId, FilesDropCallback>> =
        RefCell::new(HashMap::new());
    /// `onkeydown` / `onkeyup` handlers (issue #1678).
    static KEY_REGISTRY: RefCell<HashMap<EventHandlerId, KeyCallback>> = RefCell::new(HashMap::new());
    /// `onwheel` handlers.
    static WHEEL_REGISTRY: RefCell<HashMap<EventHandlerId, WheelCallback>> =
        RefCell::new(HashMap::new());
    // Not const-evaluable: goes through the non-const `Default` trait method.
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
    pub node_id: u64,
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

/// Registry that maps event handler IDs to file-drop callbacks.
pub struct FileDropRegistry {
    handlers: HashMap<EventHandlerId, FileDropCallback>,
}

impl FileDropRegistry {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}

/// Registry that maps event handler IDs to scroll callbacks.
pub struct ScrollRegistry {
    handlers: HashMap<EventHandlerId, ScrollCallback>,
}

impl ScrollRegistry {
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
#[doc(hidden)]
pub fn register_handler(callback: EventCallback) -> EventHandlerId {
    let id = next_handler_id();
    tracing::debug!("register_handler: Registered handler {:?}", id);
    // Capture the context root current at registration time: dispatch happens
    // from the event loop (no root pushed), so the wrapper re-enters the root
    // the handler was built under — a handler inside a mounted root resolves
    // that root's stores/contexts (issue #136).
    let root = crate::context::current_context_root();
    // Attribute the handler to the scope registering it, and re-enter that scope
    // on dispatch so a `Signal::new` inside the callback belongs to the
    // component that installed it (issue #141). `Owner` is a `Weak`, so the
    // capture does not keep the scope alive — unlike `root`, which is a `Copy`
    // `u64` and retains nothing, this is the edge that would make every desktop
    // scope immortal if it were strong.
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback: EventCallback = Rc::new(move || {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback();
    });
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

/// Set the ancestor chain for the current click, used by [`click_ancestors`].
///
/// Called by the runtime before dispatching a click handler. Pass an empty
/// `Vec` to clear the chain (e.g. for synthetic events without DOM ancestry).
///
/// [`click_ancestors`]: super::click_ancestors
pub fn set_click_ancestors(ancestors: Vec<super::AncestorBounds>) {
    super::CLICK_ANCESTORS.with(|c| {
        *c.borrow_mut() = ancestors;
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
///
/// A `false` is **not** by itself a bug: disposing a scope frees the handlers it
/// owns while their `data-rid` attributes stay on any node that outlives them
/// (issue #141), so a miss is the routine outcome of a teardown race. Callers
/// that walk an ancestor chain should treat it as "keep walking" — see
/// [`has_click_handler`] for probing without dispatching.
pub fn dispatch_event(id: EventHandlerId) -> bool {
    // Clone the handler out of the registry so we can release the borrow
    // before calling it. This allows handlers to register new handlers
    // (e.g., when Show toggles and renders new content with buttons).
    let handler: Option<EventCallback> = EVENT_REGISTRY.with(|registry| {
        let reg = registry.borrow();
        reg.handlers.get(&id).cloned()
    });

    if let Some(h) = handler {
        tracing::trace!("dispatch_event: calling handler {:?}", id);
        // One transaction per handler: every signal it writes lands before any
        // effect runs, and effects flush once, when it returns. See
        // `crate::reactive::batch` and the guide's "Batching" section.
        crate::reactive::batch(|| h());
        tracing::trace!("dispatch_event: handler {:?} completed", id);
        true
    } else {
        // `debug!`, and no registry dump: since #141 a miss is an expected
        // teardown outcome rather than a bug, and this runs on every stale
        // hover/drag-move, where formatting the whole registry would dominate.
        tracing::debug!("dispatch_event: no handler registered for {:?}", id);
        false
    }
}

/// Register an input event handler and return its ID.
///
/// The handler will be called when an element with the corresponding
/// `data-oninput` attribute receives input.
#[doc(hidden)]
pub fn register_input_handler(callback: InputCallback) -> EventHandlerId {
    let id = next_handler_id();
    // Re-enter the registration-time context root and owner on dispatch
    // (issues #136, #141).
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback = InputCallback::new(move |value| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback.invoke(value);
    });
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
    // Clone the handler out of the registry so the borrow is released before
    // calling it — the same reason `dispatch_event` does. An input handler
    // typically writes a signal, and `Signal::set` flushes effects
    // synchronously outside `batch()`; those effects can register new handlers
    // (an `if` flipping to a branch with a button) or re-enter this registry.
    // Holding `borrow()` across `invoke` makes that a BorrowMutError.
    let handler: Option<InputCallback> = INPUT_REGISTRY.with(|registry| {
        let reg = registry.borrow();
        reg.handlers.get(&id).cloned()
    });

    if let Some(h) = handler {
        crate::reactive::batch(|| h.invoke(value));
        // Signal that an input event was handled - caller should re-render
        INPUT_EVENT_HANDLED.with(|flag| *flag.borrow_mut() = true);
        true
    } else {
        false
    }
}

/// Register a file-drop event handler and return its ID.
///
/// The handler will be called when files are dropped from the OS onto an
/// element with the corresponding `data-onfiledrop` attribute.
#[doc(hidden)]
pub fn register_file_drop_handler(callback: FileDropCallback) -> EventHandlerId {
    let id = next_handler_id();
    // Re-enter the registration-time context root and owner on dispatch
    // (issues #136, #141).
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback = FileDropCallback::new(move |paths| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback.invoke(paths);
    });
    FILE_DROP_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.insert(id, callback);
    });
    id
}

/// Dispatch a file-drop event to the handler with the given ID.
///
/// Returns `true` if a handler was found and called.
pub fn dispatch_file_drop_event(id: EventHandlerId, paths: Vec<PathBuf>) -> bool {
    let handler: Option<FileDropCallback> =
        FILE_DROP_REGISTRY.with(|registry| registry.borrow().handlers.get(&id).cloned());
    if let Some(h) = handler {
        crate::reactive::batch(|| h.invoke(paths));
        true
    } else {
        false
    }
}

thread_local! {
    static PRESS_WITHOUT_ACTIVATION: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
}

/// Set by a backend for the length of a click it dispatches from a press
/// that grants the page no user activation: on the web a touch or pen
/// `pointerdown`, whose click runs on contact unless the element can scroll
/// (#1582). `Some(pointer id)`; `None` after. Code that needs activation (the
/// file picker, #1771) asks [`press_without_activation`] and waits for that
/// pointer's release, which does grant it.
#[doc(hidden)]
pub fn set_press_without_activation(pointer: Option<i32>) {
    PRESS_WITHOUT_ACTIVATION.with(|p| p.set(pointer));
}

/// The pointer whose press is being dispatched without user activation, if
/// any. See [`set_press_without_activation`].
#[doc(hidden)]
pub fn press_without_activation() -> Option<i32> {
    PRESS_WITHOUT_ACTIVATION.with(std::cell::Cell::get)
}

/// Register an `ondropfiles` handler (issue #1771) and return its id: called
/// with the files dropped from the OS onto an element carrying
/// `data-ondropfiles`, as [`LocalFile`](crate::file::LocalFile)s, on the
/// desktop and on the web. While it runs, [`get_click_context`] holds the drop
/// point and that element's box on both (#1822).
#[doc(hidden)]
pub fn register_files_drop_handler(
    callback: impl Fn(Vec<crate::file::LocalFile>) + 'static,
) -> EventHandlerId {
    let id = next_handler_id();
    // Re-enter the registration-time context root and owner on dispatch
    // (issues #136, #141), as every other handler does.
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback: FilesDropCallback = Rc::new(move |files| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        crate::reactive::untracked_handler(|| callback(files));
    });
    FILES_DROP_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(id, callback);
    });
    id
}

/// Dispatch dropped files to the `ondropfiles` handler with the given id, in
/// one batch. Returns `true` if a handler was found and called.
pub fn dispatch_files_drop_event(id: EventHandlerId, files: Vec<crate::file::LocalFile>) -> bool {
    let handler = FILES_DROP_REGISTRY.with(|registry| registry.borrow().get(&id).cloned());
    if let Some(h) = handler {
        crate::reactive::batch(|| h(files));
        true
    } else {
        false
    }
}

/// Register a scroll event handler and return its ID.
///
/// The handler will be called when an element with the corresponding
/// `data-onscroll` attribute is scrolled, passing a [`ScrollEvent`] with the
/// container's offset on both axes.
#[doc(hidden)]
pub fn register_scroll_handler(callback: ScrollCallback) -> EventHandlerId {
    let id = next_handler_id();
    // Re-enter the registration-time context root and owner on dispatch
    // (issues #136, #141).
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback = ScrollCallback::new(move |event| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback.invoke(event);
    });
    SCROLL_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.insert(id, callback);
    });
    id
}

/// Dispatch a scroll event to the handler with the given ID.
///
/// Returns `true` if a handler was found and called.
pub fn dispatch_scroll_event(id: EventHandlerId, event: ScrollEvent) -> bool {
    let handler: Option<ScrollCallback> =
        SCROLL_REGISTRY.with(|registry| registry.borrow().handlers.get(&id).cloned());
    if let Some(h) = handler {
        crate::reactive::batch(|| h.invoke(event));
        true
    } else {
        false
    }
}

/// Register an element key handler (`onkeydown` / `onkeyup`, issue #1678)
/// and return its ID, to be written to the element's `data-onkeydown` or
/// `data-onkeyup` attribute.
///
/// Like every other handler it is owned by the scope registering it and
/// re-enters that scope and context root on dispatch (issues #136, #141).
#[doc(hidden)]
pub fn register_key_handler(callback: KeyCallback) -> EventHandlerId {
    let id = next_handler_id();
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback = KeyCallback::new(move |key| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback.invoke(key)
    });
    KEY_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(id, callback);
    });
    id
}

/// Whether a key handler is still registered under this id. See
/// [`has_click_handler`].
pub fn has_key_handler(id: EventHandlerId) -> bool {
    KEY_REGISTRY.with(|registry| registry.borrow().contains_key(&id))
}

/// Offer a key to the element key handler with the given ID, as one
/// transaction (`batch`). Answers whether the handler **consumed** it; a
/// missing handler (freed with its scope, #141) consumed nothing.
pub fn dispatch_key_handler(id: EventHandlerId, key: &super::KeyEventData) -> bool {
    let handler: Option<KeyCallback> =
        KEY_REGISTRY.with(|registry| registry.borrow().get(&id).cloned());
    match handler {
        Some(h) => crate::reactive::batch(|| h.invoke(key)),
        None => false,
    }
}

/// Offer a key to a chain of element key handlers, nearest first — the
/// `onkeydown` / `onkeyup` ids a backend read off the focused element and its
/// ancestors, **before** calling any of them (a handler may change the tree,
/// and the path is the one the key was pressed in, as a browser fixes an
/// event's path at dispatch). Stops at the first handler that consumes the
/// key and answers whether one did (issue #1678).
pub fn dispatch_key_handler_chain(
    ids: impl IntoIterator<Item = EventHandlerId>,
    key: &super::KeyEventData,
) -> bool {
    ids.into_iter().any(|id| dispatch_key_handler(id, key))
}

/// Register an element wheel handler (`onwheel`) and return its ID, to be
/// written to the element's `data-onwheel` attribute.
///
/// Owned by the scope registering it, and re-enters that scope and context
/// root on dispatch (issues #136, #141), like every other handler.
#[doc(hidden)]
pub fn register_wheel_handler(callback: WheelCallback) -> EventHandlerId {
    let id = next_handler_id();
    let root = crate::context::current_context_root();
    crate::reactive::record_handler(id);
    let owner = crate::reactive::Owner::current();
    let callback = WheelCallback::new(move |event| {
        let _root = crate::context::push_context_root(root);
        let _owner = owner.push();
        callback.invoke(event)
    });
    WHEEL_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(id, callback);
    });
    id
}

/// Whether a wheel handler is still registered under this id. See
/// [`has_click_handler`].
pub fn has_wheel_handler(id: EventHandlerId) -> bool {
    WHEEL_REGISTRY.with(|registry| registry.borrow().contains_key(&id))
}

/// Offer a wheel to the `onwheel` handler with the given ID, as one
/// transaction (`batch`). Answers whether it **consumed** the wheel; a missing
/// handler (freed with its scope, #141) consumed nothing.
pub fn dispatch_wheel_handler(id: EventHandlerId, event: &WheelEvent) -> bool {
    let handler: Option<WheelCallback> =
        WHEEL_REGISTRY.with(|registry| registry.borrow().get(&id).cloned());
    match handler {
        Some(h) => crate::reactive::batch(|| h.invoke(event)),
        None => false,
    }
}

/// Offer a wheel to a chain of `onwheel` handlers, nearest first — the ids a
/// backend read off the element under the pointer and its ancestors **before**
/// calling any of them (a handler may change the tree; the path is the one the
/// wheel turned over). Stops at the first handler that consumes it and answers
/// whether one did; a consumed wheel scrolls nothing.
pub fn dispatch_wheel_handler_chain(
    ids: impl IntoIterator<Item = EventHandlerId>,
    event: &WheelEvent,
) -> bool {
    ids.into_iter().any(|id| dispatch_wheel_handler(id, event))
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
    FILE_DROP_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.clear();
    });
    SCROLL_REGISTRY.with(|registry| {
        registry.borrow_mut().handlers.clear();
    });
    FILES_DROP_REGISTRY.with(|registry| {
        registry.borrow_mut().clear();
    });
    KEY_REGISTRY.with(|registry| {
        registry.borrow_mut().clear();
    });
    WHEEL_REGISTRY.with(|registry| {
        registry.borrow_mut().clear();
    });
    reset_handler_ids();
}

/// Get the number of registered handlers (for debugging).
pub fn handler_count() -> usize {
    EVENT_REGISTRY.with(|registry| registry.borrow().handlers.len())
}

#[cfg(test)]
mod owner_tests {
    //! A signal created inside an event handler belongs to the scope that
    //! *registered* the handler (issue #141, maintainer decision 1).
    //!
    //! Dispatch happens from the event loop with an empty owner stack, so
    //! without the re-entry in the registration wrapper these signals would have
    //! app lifetime and never be reclaimed.
    //!
    //! # Harness note
    //!
    //! `NEXT_HANDLER_ID` is a **process-global** `AtomicUsize` while the
    //! registries are thread-local, and `clear_handlers()` — which other tests
    //! call from other threads — rewinds it. A test that registers two handlers
    //! into the *same* registry could therefore see the second overwrite the
    //! first. Hence: one handler per registry per test.

    use super::*;
    use crate::reactive::{Scope, Signal, current_owner};

    /// Register one handler under `scope`, dispatch it from an empty owner
    /// stack, and report what the callback saw.
    fn check_handler_owner(
        scope: &Scope,
        register: impl FnOnce() -> EventHandlerId,
        dispatch: impl FnOnce(EventHandlerId) -> bool,
    ) {
        let id = {
            let _owner = scope.push_owner();
            register()
        };
        assert_eq!(
            scope.owned_counts().handlers,
            1,
            "the handler is attributed at registration time"
        );

        assert!(
            current_owner().is_none(),
            "dispatch must run from an empty owner stack for this to prove anything"
        );
        assert!(dispatch(id), "the handler must have been found");

        assert!(
            current_owner().is_none(),
            "the wrapper's owner push must be RAII"
        );
        assert_eq!(
            scope.owned_counts().signals,
            1,
            "a signal created inside the handler belongs to the registering scope"
        );
    }

    #[test]
    fn a_signal_created_in_a_click_handler_belongs_to_the_registering_scope() {
        let scope = Scope::new();
        check_handler_owner(
            &scope,
            || {
                register_handler(Rc::new(|| {
                    Signal::new(0);
                }))
            },
            dispatch_event,
        );
    }

    #[test]
    fn a_signal_created_in_an_input_handler_belongs_to_the_registering_scope() {
        let scope = Scope::new();
        check_handler_owner(
            &scope,
            || {
                register_input_handler(InputCallback::new(|_value: String| {
                    Signal::new(0);
                }))
            },
            |id| dispatch_input_event(id, "hello".into()),
        );
    }

    #[test]
    fn a_signal_created_in_a_file_drop_handler_belongs_to_the_registering_scope() {
        let scope = Scope::new();
        check_handler_owner(
            &scope,
            || {
                register_file_drop_handler(FileDropCallback::new(|_paths: Vec<PathBuf>| {
                    Signal::new(0);
                }))
            },
            |id| dispatch_file_drop_event(id, Vec::new()),
        );
    }

    #[test]
    fn a_signal_created_in_a_scroll_handler_belongs_to_the_registering_scope() {
        let scope = Scope::new();
        check_handler_owner(
            &scope,
            || {
                register_scroll_handler(ScrollCallback::new(|_ev: ScrollEvent| {
                    Signal::new(0);
                }))
            },
            |id| dispatch_scroll_event(id, ScrollEvent::new(12.0, 0.0)),
        );
    }

    /// A handler that dispatches another handler nests the owner pushes, so each
    /// callback's allocations land in its own registering scope.
    ///
    /// The two handlers deliberately live in **different registries**: that makes
    /// the test immune to the process-global id counter being rewound between
    /// the two registrations (see the module note).
    #[test]
    fn a_re_entrant_dispatch_nests_owner_pushes() {
        let outer = Scope::new();
        let inner = Scope::new();

        let inner_id = {
            let _owner = inner.push_owner();
            register_input_handler(InputCallback::new(|_value: String| {
                Signal::new(0);
            }))
        };

        let outer_id = {
            let _owner = outer.push_owner();
            register_handler(Rc::new(move || {
                Signal::new(0); // before the nested dispatch
                assert!(dispatch_input_event(inner_id, "x".into()));
                Signal::new(0); // after it — the outer owner must be restored
            }))
        };

        assert!(dispatch_event(outer_id));

        assert_eq!(
            outer.owned_counts().signals,
            2,
            "both of the outer handler's signals belong to the outer scope"
        );
        assert_eq!(
            inner.owned_counts().signals,
            1,
            "the nested handler's signal belongs to the inner scope"
        );
        assert!(current_owner().is_none(), "the stack drains completely");
    }
}

#[cfg(test)]
mod batch_tests {
    //! Every handler dispatch is one reactive transaction: the signals a
    //! handler writes all land before any effect runs, effects flush once when
    //! it returns, and a memo it reads after a write is already current.
    //!
    //! One handler per registry per test, for the reason `owner_tests` gives.

    use super::*;
    use crate::events::{
        KeyEventData, clear_keyboard_interceptor, dispatch_keyboard_event, set_keyboard_interceptor,
    };
    use crate::reactive::{Effect, Memo, Signal, reactive_counters, subscribe_signal_change};
    use std::cell::{Cell, RefCell};

    /// How many signals the multi-write handlers write. Several, so "one flush"
    /// and "one flush per write" are different numbers.
    const WRITES: usize = 5;

    /// Counts the flushes that reach the host — the signal-change callback is
    /// what the desktop shell turns into a `ReRender`.
    fn count_flushes() -> (Rc<Cell<u32>>, crate::reactive::SignalChangeSubscription) {
        let flushes = Rc::new(Cell::new(0));
        let f = Rc::clone(&flushes);
        let sub = subscribe_signal_change(move || f.set(f.get() + 1));
        (flushes, sub)
    }

    /// K writes in one click handler: one flush, one run of the effect that
    /// reads all of them, and that run sees every write (no intermediate
    /// state, which is what an effect re-run per write would glitch through).
    #[test]
    fn a_click_handler_that_writes_k_signals_flushes_once() {
        let signals: Vec<Signal<u32>> = (0..WRITES).map(|_| Signal::new(0)).collect();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = Rc::clone(&seen);
        let sigs = signals.clone();
        let _effect = Effect::new(move || {
            let sum: u32 = sigs.iter().map(|sig| sig.get()).sum();
            s.borrow_mut().push(sum);
        });
        seen.borrow_mut().clear();

        let sigs = signals.clone();
        let id = register_handler(Rc::new(move || {
            for (i, sig) in sigs.iter().enumerate() {
                sig.set(i as u32 + 1);
            }
        }));
        let (flushes, _sub) = count_flushes();
        let runs_before = reactive_counters().effect_runs;

        assert!(dispatch_event(id));

        assert_eq!(flushes.get(), 1, "one flush for the whole handler");
        assert_eq!(reactive_counters().effect_runs - runs_before, 1);
        assert_eq!(*seen.borrow(), vec![15], "1+2+3+4+5, never a partial sum");
    }

    /// A click whose handler writes no signal requests no re-render (issue
    /// #234): the batch the dispatch runs in wrote nothing, so the host's
    /// signal-change callback — a desktop `ReRender`, an embedded context's
    /// dirty flag — is not called. The handler did run (positive control).
    #[test]
    fn a_click_handler_that_writes_nothing_requests_no_rerender() {
        let ran = Rc::new(Cell::new(0));
        let r = Rc::clone(&ran);
        let id = register_handler(Rc::new(move || r.set(r.get() + 1)));
        let (flushes, _sub) = count_flushes();

        assert!(dispatch_event(id));

        assert_eq!(ran.get(), 1, "the handler ran");
        assert_eq!(
            flushes.get(),
            0,
            "and wrote nothing, so nothing is requested"
        );
    }

    /// A handler that writes and then reads a memo of what it wrote gets the
    /// new value, although the flush has not happened yet.
    #[test]
    fn a_handler_reads_a_fresh_memo_after_its_own_write() {
        let n = Signal::new(3i32);
        let squared = Memo::new(move || n.get() * n.get());
        let effect_runs = Rc::new(Cell::new(0));
        let e = Rc::clone(&effect_runs);
        let _effect = Effect::new(move || {
            let _ = squared.get();
            e.set(e.get() + 1);
        });

        let observed = Rc::new(RefCell::new(Vec::new()));
        let o = Rc::clone(&observed);
        let runs_inside = Rc::clone(&effect_runs);
        let id = register_input_handler(InputCallback::new(move |value: String| {
            n.set(value.parse().unwrap());
            o.borrow_mut().push((squared.get(), runs_inside.get()));
            n.set(n.get() + 1);
            o.borrow_mut().push((squared.get(), runs_inside.get()));
        }));

        assert!(dispatch_input_event(id, "7".to_string()));
        assert_eq!(
            *observed.borrow(),
            vec![(49, 1), (64, 1)],
            "fresh memo values inside the handler, and no effect has run yet"
        );
        assert_eq!(effect_runs.get(), 2, "the effect runs once, afterwards");
    }

    /// The keyboard path is batched too: the interceptor and the dismiss scan
    /// share one transaction.
    #[test]
    fn a_keyboard_interceptor_that_writes_k_signals_flushes_once() {
        let signals: Vec<Signal<u32>> = (0..WRITES).map(|_| Signal::new(0)).collect();
        let runs = Rc::new(Cell::new(0));
        let r = Rc::clone(&runs);
        let sigs = signals.clone();
        let _effect = Effect::new(move || {
            for sig in &sigs {
                let _ = sig.get();
            }
            r.set(r.get() + 1);
        });
        runs.set(0);

        let sigs = signals.clone();
        set_keyboard_interceptor(move |_| {
            for sig in &sigs {
                sig.update(|v| *v += 1);
            }
            true
        });
        let (flushes, _sub) = count_flushes();
        assert!(dispatch_keyboard_event(&KeyEventData::new("a", "KeyA")));
        clear_keyboard_interceptor();

        assert_eq!((flushes.get(), runs.get()), (1, 1));
    }
}
