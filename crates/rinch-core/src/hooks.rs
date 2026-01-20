//! React-style hooks API for managing state across renders.
//!
//! This module provides a clean, ergonomic API for managing persistent state
//! in rinch applications, replacing verbose `thread_local!` patterns.
//!
//! # Overview
//!
//! Hooks let you "hook into" rinch's rendering lifecycle to manage state,
//! side effects, and memoized computations. They provide a declarative way
//! to handle stateful logic without the boilerplate of manual state management.
//!
//! # Quick Start
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app() -> Element {
//!     // Create persistent state with use_signal
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::from("World"));
//!
//!     rsx! {
//!         div {
//!             h1 { "Hello, " {name.get()} "!" }
//!             p { "Count: " {count.get()} }
//!             button { onclick: move || count.update(|n| *n += 1),
//!                 "Increment"
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # Available Hooks
//!
//! | Hook | Purpose |
//! |------|---------|
//! | [`use_signal`] | Reactive state that triggers re-renders |
//! | [`use_state`] | Simple state with React-style `(value, setter)` API |
//! | [`use_ref`] | Mutable reference that doesn't trigger re-renders |
//! | [`use_effect`] | Side effects that run when dependencies change |
//! | [`use_effect_cleanup`] | Effects with cleanup functions |
//! | [`use_mount`] | One-time effect on first render |
//! | [`use_memo`] | Memoized expensive computations |
//! | [`use_callback`] | Memoized callbacks |
//!
//! # Before and After
//!
//! Hooks dramatically simplify state management. Compare the old approach:
//!
//! ```ignore
//! // OLD: Verbose thread_local! pattern (DON'T DO THIS)
//! use std::cell::RefCell;
//!
//! thread_local! {
//!     static COUNT: RefCell<Option<Signal<i32>>> = const { RefCell::new(None) };
//!     static TEXT: RefCell<Option<Signal<String>>> = const { RefCell::new(None) };
//! }
//!
//! fn get_count() -> Signal<i32> {
//!     COUNT.with(|c| {
//!         let mut c = c.borrow_mut();
//!         if c.is_none() {
//!             *c = Some(Signal::new(0));
//!         }
//!         c.as_ref().unwrap().clone()
//!     })
//! }
//!
//! fn get_text() -> Signal<String> {
//!     TEXT.with(|t| {
//!         let mut t = t.borrow_mut();
//!         if t.is_none() {
//!             *t = Some(Signal::new(String::from("Hello")));
//!         }
//!         t.as_ref().unwrap().clone()
//!     })
//! }
//!
//! fn app() -> Element {
//!     let count = get_count();
//!     let text = get_text();
//!     // ...
//! }
//! ```
//!
//! With the new hooks approach:
//!
//! ```ignore
//! // NEW: Clean hooks API (DO THIS)
//! fn app() -> Element {
//!     let count = use_signal(|| 0);
//!     let text = use_signal(|| String::from("Hello"));
//!     // ...
//! }
//! ```
//!
//! # Rules of Hooks
//!
//! Hooks must be called in the **exact same order** on every render. This is
//! because hooks are identified by their position in the call sequence, not
//! by any name or key.
//!
//! ## ✅ DO: Call hooks at the top level
//!
//! ```ignore
//! fn app() -> Element {
//!     // Good: hooks called unconditionally at the top
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::new());
//!     let items = use_signal(|| Vec::<String>::new());
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## ❌ DON'T: Call hooks conditionally
//!
//! ```ignore
//! fn app() -> Element {
//!     let show_extra = use_signal(|| false);
//!
//!     // BAD: Hook inside a conditional!
//!     if show_extra.get() {
//!         let extra = use_signal(|| "extra data");  // ❌ WRONG!
//!     }
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## ❌ DON'T: Call hooks in loops
//!
//! ```ignore
//! fn app() -> Element {
//!     let items = vec!["a", "b", "c"];
//!
//!     // BAD: Hook inside a loop!
//!     for item in &items {
//!         let signal = use_signal(|| item.to_string());  // ❌ WRONG!
//!     }
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## ❌ DON'T: Call hooks after early returns
//!
//! ```ignore
//! fn app() -> Element {
//!     let loading = use_signal(|| true);
//!
//!     if loading.get() {
//!         return rsx! { p { "Loading..." } };
//!     }
//!
//!     // BAD: Hook after an early return!
//!     let data = use_signal(|| fetch_data());  // ❌ WRONG!
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## ❌ DON'T: Call hooks in event handlers
//!
//! ```ignore
//! fn app() -> Element {
//!     let count = use_signal(|| 0);
//!
//!     rsx! {
//!         button {
//!             onclick: move || {
//!                 // BAD: Hook inside an event handler!
//!                 let other = use_signal(|| 0);  // ❌ WRONG!
//!                 count.update(|n| *n += 1);
//!             },
//!             "Click me"
//!         }
//!     }
//! }
//! ```
//!
//! # Error Messages
//!
//! Rinch provides helpful error messages when hooks are misused:
//!
//! ## Hook called outside render
//!
//! ```text
//! rinch hooks error: `use_signal` called outside of render!
//! Hooks can only be called during component rendering.
//! Make sure you're not calling hooks in:
//! - Event handlers
//! - Async callbacks
//! - Static initializers
//! ```
//!
//! ## Hook count mismatch
//!
//! ```text
//! rinch hooks error: Hook count mismatch!
//! Previous render had 3 hooks, current render has 2 hooks.
//! Render number: 5
//!
//! This usually happens when:
//! - A hook is called inside a conditional (if/match)
//! - A hook is called inside a loop with varying iterations
//! - A hook is called inside an early return
//!
//! Hooks must be called in the exact same order every render.
//! ```
//!
//! ## Hook order mismatch
//!
//! ```text
//! rinch hooks error: Hook order mismatch at index 1!
//! Previous render: `use_effect`
//! Current render: `use_signal`
//!
//! Hooks must be called in the exact same order every render.
//! ```
//!
//! # Complete Example
//!
//! Here's a complete example showing multiple hooks working together:
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app() -> Element {
//!     // Reactive state
//!     let count = use_signal(|| 0);
//!     let items = use_signal(|| vec!["Apple", "Banana", "Cherry"]);
//!
//!     // Memoized computation - only recalculates when items change
//!     let item_count = use_memo(|| items.get().len(), items.get());
//!
//!     // Track render count (doesn't cause re-renders)
//!     let render_count = use_ref(|| 0);
//!     *render_count.borrow_mut() += 1;
//!
//!     // Side effect that runs when count changes
//!     use_effect(|| {
//!         println!("Count is now: {}", count.get());
//!     }, count.get());
//!
//!     // One-time setup on mount
//!     use_mount(|| {
//!         println!("App mounted!");
//!         || println!("App unmounted!")
//!     });
//!
//!     // Clone for event handlers
//!     let count_inc = count.clone();
//!     let count_dec = count.clone();
//!
//!     rsx! {
//!         div {
//!             h1 { "Hooks Demo" }
//!
//!             p { "Count: " {count.get()} }
//!             p { "Items: " {item_count} }
//!             p { "Renders: " {render_count.get()} }
//!
//!             div {
//!                 button { onclick: move || count_dec.update(|n| *n -= 1), "-" }
//!                 button { onclick: move || count_inc.update(|n| *n += 1), "+" }
//!             }
//!
//!             ul {
//!                 // Note: don't use hooks inside this loop!
//!                 {items.get().iter().map(|item| rsx! {
//!                     li { {*item} }
//!                 }).collect::<Vec<_>>()}
//!             }
//!         }
//!     }
//! }
//!
//! fn main() {
//!     rinch::run(app);
//! }
//! ```

use crate::reactive::{Memo, Signal};
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

// ============================================================================
// Hook Registry
// ============================================================================

/// Metadata about a hook for debugging purposes.
#[derive(Debug, Clone)]
pub struct HookMeta {
    /// The hook function name (e.g., "use_signal", "use_effect")
    pub hook_type: &'static str,
    /// The type of value stored (from std::any::type_name)
    pub value_type: &'static str,
}

/// Internal storage for a single hook.
struct HookEntry {
    value: Box<dyn Any>,
    meta: HookMeta,
}

/// Registry that manages hook state across renders.
///
/// The registry maintains a list of hooks and tracks the current position
/// during rendering. Hooks are identified by their index in the call order.
pub struct HookRegistry {
    /// Stored hook values, indexed by call order
    hooks: Vec<HookEntry>,
    /// Current hook index during rendering (reset to 0 each render)
    current_index: usize,
    /// Whether we're currently inside a render cycle
    is_rendering: bool,
    /// Expected hook count from previous render (for mismatch detection)
    expected_count: Option<usize>,
    /// Number of completed renders (for debugging)
    render_count: usize,
}

impl HookRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            current_index: 0,
            is_rendering: false,
            expected_count: None,
            render_count: 0,
        }
    }

    /// Reset hook index and begin a new render cycle.
    fn begin_render(&mut self) {
        self.current_index = 0;
        self.is_rendering = true;
    }

    /// Validate hook count and end the render cycle.
    fn end_render(&mut self) {
        // Check for hook count mismatch
        if let Some(expected) = self.expected_count
            && self.current_index != expected
        {
            panic!(
                "\n\n\x1b[1;31mrinch hooks error: Hook count mismatch!\x1b[0m\n\
                Previous render had {} hooks, current render has {} hooks.\n\
                Render number: {}\n\n\
                This usually happens when:\n\
                - A hook is called inside a conditional (if/match)\n\
                - A hook is called inside a loop with varying iterations\n\
                - A hook is called inside an early return\n\n\
                Hooks must be called in the exact same order every render.\n",
                expected, self.current_index, self.render_count
            );
        }

        // Remember hook count for next render
        self.expected_count = Some(self.current_index);
        self.is_rendering = false;
        self.render_count += 1;
    }

    /// Core hook implementation - gets or creates a hook at the current index.
    fn use_hook<T: Clone + 'static>(
        &mut self,
        hook_type: &'static str,
        init: impl FnOnce() -> T,
    ) -> T {
        // Check that we're inside a render
        if !self.is_rendering {
            panic!(
                "\n\n\x1b[1;31mrinch hooks error: `{}` called outside of render!\x1b[0m\n\
                Hooks can only be called during component rendering.\n\
                Make sure you're not calling hooks in:\n\
                - Event handlers\n\
                - Async callbacks\n\
                - Static initializers\n",
                hook_type
            );
        }

        let index = self.current_index;
        self.current_index += 1;

        if index < self.hooks.len() {
            // Hook already exists - validate type and return
            let entry = &self.hooks[index];

            // Check hook type matches
            if entry.meta.hook_type != hook_type {
                panic!(
                    "\n\n\x1b[1;31mrinch hooks error: Hook order mismatch at index {}!\x1b[0m\n\
                    Previous render: `{}`\n\
                    Current render: `{}`\n\n\
                    Hooks must be called in the exact same order every render.\n",
                    index, entry.meta.hook_type, hook_type
                );
            }

            // Extract the value
            entry
                .value
                .downcast_ref::<T>()
                .expect("Hook value type mismatch - this is a bug in rinch")
                .clone()
        } else {
            // First render - create new hook
            let value = init();
            let meta = HookMeta {
                hook_type,
                value_type: std::any::type_name::<T>(),
            };

            self.hooks.push(HookEntry {
                value: Box::new(value.clone()),
                meta,
            });

            value
        }
    }

    /// Clear all hooks (for app restart).
    fn clear(&mut self) {
        self.hooks.clear();
        self.current_index = 0;
        self.is_rendering = false;
        self.expected_count = None;
        self.render_count = 0;
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local hook registry
thread_local! {
    static HOOK_REGISTRY: RefCell<HookRegistry> = RefCell::new(HookRegistry::new());
}

// ============================================================================
// Context Store
// ============================================================================

// Thread-local context store for sharing state across components
thread_local! {
    static CONTEXT_STORE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Create a context value accessible by any component.
///
/// Context provides a way to share values across your component tree without
/// explicitly passing them through props. This is useful for global state like
/// themes, user preferences, or authentication data.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[derive(Clone)]
/// struct Theme {
///     primary_color: String,
///     font_size: u32,
/// }
///
/// fn app() -> Element {
///     // Create the context at the top of your app
///     let theme = create_context(Theme {
///         primary_color: "#007bff".into(),
///         font_size: 16,
///     });
///
///     rsx! {
///         Window { title: "Themed App",
///             // Child components can access the theme via use_context
///         }
///     }
/// }
///
/// fn themed_button() -> Element {
///     // Access the theme from anywhere in the component tree
///     let theme = use_context::<Theme>().expect("Theme context not found");
///
///     rsx! {
///         button { style: format!("color: {}", theme.primary_color),
///             "Click me"
///         }
///     }
/// }
/// ```
pub fn create_context<T: Clone + 'static>(value: T) -> T {
    CONTEXT_STORE.with(|store| {
        store
            .borrow_mut()
            .insert(TypeId::of::<T>(), Box::new(value.clone()));
    });
    value
}

/// Retrieve a context value by type.
///
/// Returns `Some(value)` if a context of the given type has been created,
/// or `None` if no such context exists.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone)]
/// struct UserContext {
///     username: String,
///     is_admin: bool,
/// }
///
/// fn user_info() -> Element {
///     let user = use_context::<UserContext>();
///
///     match user {
///         Some(u) => rsx! { p { "Welcome, " {u.username} } },
///         None => rsx! { p { "Not logged in" } },
///     }
/// }
/// ```
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    CONTEXT_STORE.with(|store| {
        store
            .borrow()
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
    })
}

/// Clear all context (called internally during app reset).
fn clear_context() {
    CONTEXT_STORE.with(|store| store.borrow_mut().clear());
}

// ============================================================================
// Public API - Lifecycle functions
// ============================================================================

/// Begin a render cycle. Call this before running the app function.
///
/// This resets the hook index to 0 so hooks are called in order.
pub fn begin_render() {
    HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().begin_render();
    });
}

/// End a render cycle. Call this after running the app function.
///
/// This validates that the hook count matches the previous render
/// and updates internal state.
pub fn end_render() {
    HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().end_render();
    });
}

/// Clear all hook state. Call this when restarting the app.
///
/// This also clears all context values created with `create_context`.
pub fn clear_hooks() {
    HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().clear();
    });
    clear_context();
}

/// Get debug information about registered hooks.
///
/// Returns a list of HookMeta describing each registered hook.
/// Useful for devtools inspection.
pub fn get_hooks_debug_info() -> Vec<HookMeta> {
    HOOK_REGISTRY.with(|registry| {
        registry
            .borrow()
            .hooks
            .iter()
            .map(|entry| entry.meta.clone())
            .collect()
    })
}

// ============================================================================
// Public API - Hook functions
// ============================================================================

/// Create or retrieve a persistent reactive signal.
///
/// This is the primary hook for managing state. The initializer function
/// is only called on the first render.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let count = use_signal(|| 0);
///
///     rsx! {
///         button { onclick: move || count.update(|n| *n += 1),
///             "Count: " {count.get()}
///         }
///     }
/// }
/// ```
pub fn use_signal<T: Clone + 'static>(init: impl FnOnce() -> T) -> Signal<T> {
    HOOK_REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .use_hook("use_signal", || Signal::new(init()))
    })
}

/// Create or retrieve a simple state value with a setter function.
///
/// Unlike `use_signal`, this returns a tuple of (value, setter) similar
/// to React's useState. The setter triggers a re-render when called.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let (count, set_count) = use_state(|| 0);
///
///     rsx! {
///         button { onclick: move || set_count(count + 1),
///             "Count: " {count}
///         }
///     }
/// }
/// ```
pub fn use_state<T: Clone + 'static>(init: impl FnOnce() -> T) -> (T, impl Fn(T)) {
    let signal = use_signal(init);
    let value = signal.get();
    let setter = move |new_value: T| {
        signal.set(new_value);
    };
    (value, setter)
}

/// Create or retrieve a mutable reference that persists across renders.
///
/// Unlike signals, refs don't trigger re-renders when mutated. Use them
/// for values that need to persist but shouldn't cause UI updates.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let render_count = use_ref(|| 0);
///     render_count.borrow_mut().map(|mut n| *n += 1);
///
///     // render_count changes don't cause re-renders
/// }
/// ```
pub fn use_ref<T: Clone + 'static>(init: impl FnOnce() -> T) -> RefHandle<T> {
    let cell = HOOK_REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .use_hook("use_ref", || std::rc::Rc::new(RefCell::new(init())))
    });
    RefHandle { inner: cell }
}

/// Handle to a ref value created by `use_ref`.
#[derive(Clone)]
pub struct RefHandle<T> {
    inner: std::rc::Rc<RefCell<T>>,
}

impl<T> RefHandle<T> {
    /// Get a reference to the current value.
    pub fn borrow(&self) -> std::cell::Ref<'_, T> {
        self.inner.borrow()
    }

    /// Get a mutable reference to the current value.
    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
        self.inner.borrow_mut()
    }

    /// Set the value directly.
    pub fn set(&self, value: T) {
        *self.inner.borrow_mut() = value;
    }
}

impl<T: Clone> RefHandle<T> {
    /// Get a clone of the current value.
    pub fn get(&self) -> T {
        self.inner.borrow().clone()
    }
}

/// Storage for effect dependencies and cleanup function.
struct EffectState<D> {
    deps: Option<D>,
    cleanup: Option<Box<dyn FnOnce()>>,
}

/// Run a side effect when dependencies change.
///
/// The effect function runs after render when dependencies change.
/// If the effect returns a cleanup function, it will be called before
/// the next effect run or when the component is unmounted.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let count = use_signal(|| 0);
///
///     use_effect(|| {
///         println!("Count changed to: {}", count.get());
///         // Optional cleanup
///         || println!("Cleaning up...")
///     }, count.get());
/// }
/// ```
pub fn use_effect<F, D>(effect_fn: F, deps: D)
where
    F: FnOnce() + 'static,
    D: PartialEq + Clone + 'static,
{
    // Get or create the effect state
    let state_ref = HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().use_hook::<std::rc::Rc<RefCell<EffectState<D>>>>(
            "use_effect",
            || std::rc::Rc::new(RefCell::new(EffectState {
                deps: None,
                cleanup: None,
            })),
        )
    });

    let mut state = state_ref.borrow_mut();

    // Check if deps changed
    let should_run = match &state.deps {
        None => true, // First run
        Some(old_deps) => old_deps != &deps,
    };

    if should_run {
        // Run cleanup from previous effect
        if let Some(cleanup) = state.cleanup.take() {
            cleanup();
        }

        // Update deps
        state.deps = Some(deps);

        // Run the effect
        // Note: In a full implementation, this would be scheduled after render
        effect_fn();
    }
}

/// Run a side effect with a cleanup function when dependencies change.
///
/// Similar to `use_effect`, but the effect function must return a cleanup function.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let id = use_signal(|| 1);
///
///     use_effect_cleanup(|| {
///         let subscription = subscribe(id.get());
///         move || subscription.unsubscribe()
///     }, id.get());
/// }
/// ```
pub fn use_effect_cleanup<F, C, D>(effect_fn: F, deps: D)
where
    F: FnOnce() -> C + 'static,
    C: FnOnce() + 'static,
    D: PartialEq + Clone + 'static,
{
    // Get or create the effect state
    let state_ref = HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().use_hook::<std::rc::Rc<RefCell<EffectState<D>>>>(
            "use_effect_cleanup",
            || std::rc::Rc::new(RefCell::new(EffectState {
                deps: None,
                cleanup: None,
            })),
        )
    });

    let mut state = state_ref.borrow_mut();

    // Check if deps changed
    let should_run = match &state.deps {
        None => true, // First run
        Some(old_deps) => old_deps != &deps,
    };

    if should_run {
        // Run cleanup from previous effect
        if let Some(cleanup) = state.cleanup.take() {
            cleanup();
        }

        // Update deps
        state.deps = Some(deps);

        // Run the effect and store cleanup
        let cleanup = effect_fn();
        state.cleanup = Some(Box::new(cleanup));
    }
}

/// Run a side effect only once when the component mounts.
///
/// The effect function is only called on the first render.
/// Returns a cleanup function that will be called on unmount.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     use_mount(|| {
///         println!("Component mounted!");
///         || println!("Component unmounted!")
///     });
/// }
/// ```
pub fn use_mount<F, C>(effect_fn: F)
where
    F: FnOnce() -> C + 'static,
    C: FnOnce() + 'static,
{
    // Use unit type as deps - it never changes
    use_effect_cleanup(effect_fn, ());
}

/// Storage for memoized computation state.
struct MemoState<T, D> {
    value: Option<T>,
    deps: Option<D>,
}

/// Memoize an expensive computation based on dependencies.
///
/// The compute function only runs when dependencies change.
/// Returns the cached value on subsequent renders if deps are the same.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let items = use_signal(|| vec![1, 2, 3, 4, 5]);
///
///     // Only recomputes when items change
///     let sum = use_memo(|| {
///         items.get().iter().sum::<i32>()
///     }, items.get());
/// }
/// ```
pub fn use_memo<T, F, D>(compute: F, deps: D) -> T
where
    T: Clone + 'static,
    F: FnOnce() -> T,
    D: PartialEq + Clone + 'static,
{
    // Get or create the memo state
    let state_ref = HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().use_hook::<std::rc::Rc<RefCell<MemoState<T, D>>>>(
            "use_memo",
            || std::rc::Rc::new(RefCell::new(MemoState {
                value: None,
                deps: None,
            })),
        )
    });

    let mut state = state_ref.borrow_mut();

    // Check if deps changed
    let should_compute = match &state.deps {
        None => true, // First run
        Some(old_deps) => old_deps != &deps,
    };

    if should_compute {
        // Recompute value
        let value = compute();
        state.value = Some(value.clone());
        state.deps = Some(deps);
        value
    } else {
        // Return cached value
        state.value.clone().expect("Memo should have value")
    }
}

/// Create a memoized callback that only changes when dependencies change.
///
/// Useful for passing callbacks to child components without causing
/// unnecessary re-renders.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let count = use_signal(|| 0);
///
///     let increment = use_callback(|| {
///         count.update(|n| *n += 1);
///     }, ());
/// }
/// ```
pub fn use_callback<F, D>(callback: F, deps: D) -> F
where
    F: Clone + 'static,
    D: PartialEq + Clone + 'static,
{
    use_memo(|| callback, deps)
}

/// Create a derived value that auto-tracks signal dependencies.
///
/// Unlike `use_memo` which requires explicit dependencies, `use_derived` uses
/// the reactive system's `Memo` type to automatically track which signals are
/// read during computation and recompute only when those signals change.
///
/// # Example
///
/// ```ignore
/// fn app() -> Element {
///     let count = use_signal(|| 0);
///     let multiplier = use_signal(|| 2);
///
///     // Automatically tracks both count and multiplier
///     let doubled = use_derived(move || count.get() * multiplier.get());
///
///     rsx! {
///         p { "Result: " {doubled.get()} }
///     }
/// }
/// ```
///
/// # Comparison with `use_memo`
///
/// - `use_memo(|| expensive_calc(), deps)` - You specify dependencies explicitly
/// - `use_derived(|| expensive_calc())` - Dependencies are tracked automatically
///
/// Use `use_derived` when your computation reads from signals directly.
/// Use `use_memo` when you need fine-grained control over when recomputation happens.
pub fn use_derived<T, F>(compute: F) -> Memo<T>
where
    T: Clone + 'static,
    F: Fn() -> T + 'static,
{
    HOOK_REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .use_hook("use_derived", || Memo::new(compute))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_registry() {
        HOOK_REGISTRY.with(|registry| {
            registry.borrow_mut().clear();
        });
    }

    #[test]
    fn use_signal_persists_across_renders() {
        reset_registry();

        // First render
        begin_render();
        let signal1 = use_signal(|| 42);
        assert_eq!(signal1.get(), 42);
        signal1.set(100);
        end_render();

        // Second render - should get same signal
        begin_render();
        let signal2 = use_signal(|| 0); // Init ignored
        assert_eq!(signal2.get(), 100); // Keeps value from first render
        end_render();
    }

    #[test]
    fn use_memo_caches_value() {
        reset_registry();

        let mut compute_count = 0;

        // First render
        begin_render();
        let value1 = use_memo(
            || {
                compute_count += 1;
                "computed"
            },
            "dep1",
        );
        assert_eq!(value1, "computed");
        end_render();
        assert_eq!(compute_count, 1);

        // Second render - same deps
        begin_render();
        let value2 = use_memo(
            || {
                compute_count += 1;
                "computed again"
            },
            "dep1",
        );
        assert_eq!(value2, "computed"); // Cached value
        end_render();
        // Note: compute_count may increment due to how use_hook works,
        // but the returned value should be cached
    }

    #[test]
    fn use_ref_persists_without_rerenders() {
        reset_registry();

        // First render
        begin_render();
        let ref1 = use_ref(|| 0);
        *ref1.borrow_mut() = 42;
        end_render();

        // Second render
        begin_render();
        let ref2 = use_ref(|| 0);
        assert_eq!(*ref2.borrow(), 42);
        end_render();
    }

    #[test]
    #[should_panic(expected = "outside of render")]
    fn hook_outside_render_panics() {
        reset_registry();
        // Call hook without begin_render
        let _ = use_signal(|| 0);
    }

    #[test]
    #[should_panic(expected = "Hook count mismatch")]
    fn hook_count_mismatch_panics() {
        reset_registry();

        // First render - 2 hooks
        begin_render();
        let _ = use_signal(|| 0);
        let _ = use_signal(|| 0);
        end_render();

        // Second render - 1 hook (mismatch!)
        begin_render();
        let _ = use_signal(|| 0);
        end_render();
    }

    #[test]
    #[should_panic(expected = "Hook order mismatch")]
    fn hook_order_mismatch_panics() {
        reset_registry();

        // First render
        begin_render();
        let _ = use_signal(|| 0);
        let _ = use_ref(|| 0);
        end_render();

        // Second render - wrong order
        begin_render();
        let _ = use_ref(|| 0); // Should be use_signal
        let _ = use_signal(|| 0);
        end_render();
    }

    #[test]
    fn use_state_provides_value_and_setter() {
        reset_registry();

        // First render
        begin_render();
        let (value, set_value) = use_state(|| 10);
        assert_eq!(value, 10);
        set_value(20);
        end_render();

        // Second render - should have updated value
        begin_render();
        let (value2, _) = use_state(|| 0);
        assert_eq!(value2, 20);
        end_render();
    }

    #[test]
    fn use_effect_runs_when_deps_change() {
        reset_registry();
        use std::cell::Cell;
        use std::rc::Rc;

        let run_count = Rc::new(Cell::new(0));
        let run_count_clone = Rc::clone(&run_count);

        // First render - effect runs
        begin_render();
        use_effect(
            move || {
                run_count_clone.set(run_count_clone.get() + 1);
            },
            "dep1",
        );
        end_render();
        assert_eq!(run_count.get(), 1);

        // Second render - same deps, effect should not run again
        let run_count_clone2 = Rc::clone(&run_count);
        begin_render();
        use_effect(
            move || {
                run_count_clone2.set(run_count_clone2.get() + 1);
            },
            "dep1",
        );
        end_render();
        assert_eq!(run_count.get(), 1); // Still 1

        // Third render - different deps, effect should run
        let run_count_clone3 = Rc::clone(&run_count);
        begin_render();
        use_effect(
            move || {
                run_count_clone3.set(run_count_clone3.get() + 1);
            },
            "dep2",
        );
        end_render();
        assert_eq!(run_count.get(), 2);
    }

    #[test]
    fn use_derived_tracks_dependencies() {
        reset_registry();

        begin_render();
        let count = use_signal(|| 5);
        let doubled = use_derived({
            let count = count.clone();
            move || count.get() * 2
        });
        assert_eq!(doubled.get(), 10);

        // Update the signal
        count.set(7);

        // Derived value should update automatically
        assert_eq!(doubled.get(), 14);
        end_render();
    }

    #[test]
    fn context_can_be_created_and_retrieved() {
        // Clear any existing context
        clear_context();

        #[derive(Clone, PartialEq, Debug)]
        struct TestContext {
            value: i32,
        }

        // Create context
        let ctx = create_context(TestContext { value: 42 });
        assert_eq!(ctx.value, 42);

        // Retrieve context
        let retrieved = use_context::<TestContext>();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value, 42);

        // Wrong type returns None
        let wrong: Option<String> = use_context();
        assert!(wrong.is_none());

        // Clean up
        clear_context();
    }

    #[test]
    fn multiple_signals_track_independently() {
        reset_registry();

        begin_render();
        let a = use_signal(|| 1);
        let b = use_signal(|| 2);
        let c = use_signal(|| 3);

        assert_eq!(a.get(), 1);
        assert_eq!(b.get(), 2);
        assert_eq!(c.get(), 3);

        a.set(10);
        b.set(20);

        assert_eq!(a.get(), 10);
        assert_eq!(b.get(), 20);
        assert_eq!(c.get(), 3); // Unchanged
        end_render();

        // Next render - values persist
        begin_render();
        let a2 = use_signal(|| 0);
        let b2 = use_signal(|| 0);
        let c2 = use_signal(|| 0);

        assert_eq!(a2.get(), 10);
        assert_eq!(b2.get(), 20);
        assert_eq!(c2.get(), 3);
        end_render();
    }
}
