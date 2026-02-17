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
//! #[component]
//! fn app() -> NodeHandle {
//!     // Create persistent state with use_signal
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::from("World"));
//!
//!     rsx! {
//!         div {
//!             h1 { "Hello, " {|| name.get()} "!" }
//!             p { "Count: " {|| count.get().to_string()} }
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
//! | [`use_derived`] | Auto-tracking computed values (uses reactive Memo) |
//! | [`create_context`] / [`use_context`] | Shared state across components |
//!
//! # Before and After
//!
//! Hooks dramatically simplify state management. Compare the old approach:
//!
//! ```ignore
//! // OLD (obsolete): Verbose thread_local! pattern (DON'T DO THIS)
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
//!         *c.as_ref().unwrap()
//!     })
//! }
//!
//! fn get_text() -> Signal<String> {
//!     TEXT.with(|t| {
//!         let mut t = t.borrow_mut();
//!         if t.is_none() {
//!             *t = Some(Signal::new(String::from("Hello")));
//!         }
//!         *t.as_ref().unwrap()
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
//! #[component]
//! fn app() -> NodeHandle {
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
//! ## DO: Call hooks at the top level
//!
//! ```ignore
//! #[component]
//! fn app() -> NodeHandle {
//!     // Good: hooks called unconditionally at the top
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::new());
//!     let items = use_signal(|| Vec::<String>::new());
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## DON'T: Call hooks conditionally
//!
//! ```ignore
//! #[component]
//! fn app() -> NodeHandle {
//!     let show_extra = use_signal(|| false);
//!
//!     // BAD: Hook inside a conditional!
//!     if show_extra.get() {
//!         let extra = use_signal(|| "extra data");  // WRONG!
//!     }
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## DON'T: Call hooks in loops
//!
//! ```ignore
//! #[component]
//! fn app() -> NodeHandle {
//!     let items = vec!["a", "b", "c"];
//!
//!     // BAD: Hook inside a loop!
//!     for item in &items {
//!         let signal = use_signal(|| item.to_string());  // WRONG!
//!     }
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## DON'T: Call hooks after early returns
//!
//! ```ignore
//! #[component]
//! fn app() -> NodeHandle {
//!     let loading = use_signal(|| true);
//!
//!     if loading.get() {
//!         return rsx! { p { "Loading..." } };
//!     }
//!
//!     // BAD: Hook after an early return!
//!     let data = use_signal(|| fetch_data());  // WRONG!
//!
//!     rsx! { /* ... */ }
//! }
//! ```
//!
//! ## DON'T: Call hooks in event handlers
//!
//! ```ignore
//! #[component]
//! fn app() -> NodeHandle {
//!     let count = use_signal(|| 0);
//!
//!     rsx! {
//!         button {
//!             onclick: move || {
//!                 // BAD: Hook inside an event handler!
//!                 let other = use_signal(|| 0);  // WRONG!
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
//! #[component]
//! fn app() -> NodeHandle {
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
//!                 button { onclick: move || count.update(|n| *n -= 1), "-" }
//!                 button { onclick: move || count.update(|n| *n += 1), "+" }
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
//!     run("Hooks Demo", 800, 600, app);
//! }
//! ```

pub mod context;
pub mod effect_hooks;
pub mod state_hooks;

use std::any::Any;
use std::cell::RefCell;

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
pub struct HookEntry {
    value: Box<dyn Any>,
    meta: HookMeta,
}

/// Saved hook scope state for nesting (e.g., For item hooks).
struct SavedHookScope {
    hooks: Vec<HookEntry>,
    current_index: usize,
    expected_count: Option<usize>,
    render_count: usize,
    was_rendering: bool,
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
    pub(crate) is_rendering: bool,
    /// Expected hook count from previous render (for mismatch detection)
    expected_count: Option<usize>,
    /// Number of completed renders (for debugging)
    render_count: usize,
    /// Stack of saved scopes for nested hook contexts (e.g., For item hooks)
    scope_stack: Vec<SavedHookScope>,
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
            scope_stack: Vec::new(),
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
    pub(crate) fn use_hook<T: Clone + 'static>(
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

    /// Push the current hook state onto the scope stack and start a fresh (or restored) scope.
    ///
    /// Used by For loop to give each item its own isolated hook state.
    /// Pass `saved` from a previous `pop_hook_scope()` call to restore hooks for re-renders,
    /// or `None` for a fresh item.
    fn push_hook_scope(&mut self, saved: Option<Vec<HookEntry>>) {
        self.scope_stack.push(SavedHookScope {
            hooks: std::mem::take(&mut self.hooks),
            current_index: self.current_index,
            expected_count: self.expected_count.take(),
            render_count: self.render_count,
            was_rendering: self.is_rendering,
        });

        if let Some(hooks) = saved {
            self.hooks = hooks;
            self.expected_count = Some(self.hooks.len());
        } else {
            self.hooks = Vec::new();
            self.expected_count = None;
        }
        self.current_index = 0;
        self.is_rendering = true;
        self.render_count = 0;
    }

    /// Pop the scope stack, saving the current item's hooks and restoring the parent state.
    ///
    /// Returns the item's hook entries for storage and later restoration.
    fn pop_hook_scope(&mut self) -> Vec<HookEntry> {
        let item_hooks = std::mem::take(&mut self.hooks);

        if let Some(saved) = self.scope_stack.pop() {
            self.hooks = saved.hooks;
            self.current_index = saved.current_index;
            self.expected_count = saved.expected_count;
            self.render_count = saved.render_count;
            self.is_rendering = saved.was_rendering;
        }

        item_hooks
    }

    /// Clear all hooks (for app restart).
    pub(crate) fn clear(&mut self) {
        self.hooks.clear();
        self.current_index = 0;
        self.is_rendering = false;
        self.expected_count = None;
        self.render_count = 0;
        self.scope_stack.clear();
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local hook registry
thread_local! {
    pub(crate) static HOOK_REGISTRY: RefCell<HookRegistry> = RefCell::new(HookRegistry::new());
}

/// Push a new hook scope for isolated hook state (e.g., For loop items).
///
/// Pass saved hooks from a previous `pop_hook_scope()` to restore state,
/// or `None` for a fresh scope.
pub fn push_hook_scope(saved: Option<Vec<HookEntry>>) {
    HOOK_REGISTRY.with(|registry| {
        registry.borrow_mut().push_hook_scope(saved);
    });
}

/// Pop the current hook scope, returning the item's hooks for storage.
pub fn pop_hook_scope() -> Vec<HookEntry> {
    HOOK_REGISTRY.with(|registry| registry.borrow_mut().pop_hook_scope())
}

/// Execute a function with render context enabled.
/// This allows hooks to be called from within Effects that need to render new content.
pub fn with_render_context<T>(f: impl FnOnce() -> T) -> T {
    HOOK_REGISTRY.with(|registry| {
        let was_rendering = registry.borrow().is_rendering;
        registry.borrow_mut().is_rendering = true;
        let result = f();
        registry.borrow_mut().is_rendering = was_rendering;
        result
    })
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
    context::clear_context();
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

// Re-export all public items from submodules
pub use context::{create_context, try_use_context, use_context};
pub use effect_hooks::{use_callback, use_effect, use_effect_cleanup, use_memo, use_mount};
pub use state_hooks::{RefHandle, use_derived, use_ref, use_signal, use_state};
