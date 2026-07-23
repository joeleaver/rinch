//! Context system for sharing state across components.
//!
//! Context provides a way to share values across your component tree without
//! explicitly passing them through props.
//!
//! # Root scoping (issue #136)
//!
//! The store is keyed by `(root, TypeId)`. Root `0` is the **thread-global
//! fallback root**: everything created with no root pushed — shell startups,
//! menu/tray callbacks, timers, cross-thread-marshalled closures — lives
//! there, so single-root apps behave exactly as before. A mounted embed root
//! (`RinchContext`) pushes its own root key (its document's `doc_key`) around
//! mount, and effects/event handlers capture the current root at creation
//! time, so each context resolves its own namespace first and falls back to
//! root `0` for anything it didn't create itself.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// The thread-global fallback root (see module docs).
const GLOBAL_ROOT: u64 = 0;

// Thread-local context store for sharing state across components, keyed by
// (root, TypeId) — each mounted root gets its own namespace (issue #136).
thread_local! {
    static CONTEXT_STORE: RefCell<HashMap<(u64, TypeId), Box<dyn Any>>> =
        RefCell::new(HashMap::new());
    /// The root whose namespace create/use_context resolve right now.
    static CURRENT_ROOT: Cell<u64> = const { Cell::new(GLOBAL_ROOT) };
}

/// The context root currently in effect on this thread (`0` = the
/// thread-global fallback root).
pub fn current_context_root() -> u64 {
    CURRENT_ROOT.with(|r| r.get())
}

/// RAII guard returned by [`push_context_root`]; restores the previous root
/// when dropped.
pub struct ContextRootGuard {
    prev: u64,
}

impl Drop for ContextRootGuard {
    fn drop(&mut self) {
        CURRENT_ROOT.with(|r| r.set(self.prev));
    }
}

/// Make `root` the current context root until the returned guard drops.
///
/// Used by the framework around a mounted root's build (`RinchContext` mount),
/// effect re-runs, and event-handler dispatch so `create_context`/`use_context`
/// resolve that root's namespace. Closures that run with no root pushed
/// resolve the thread-global root `0`.
pub fn push_context_root(root: u64) -> ContextRootGuard {
    let prev = CURRENT_ROOT.with(|r| r.replace(root));
    ContextRootGuard { prev }
}

/// Create a context value accessible by any component.
///
/// Context provides a way to share values across your component tree without
/// explicitly passing them through props. This is useful for global state like
/// themes, user preferences, or authentication data.
///
/// The value is stored under the current context root — the mounted root being
/// built, or the thread-global root when called outside any root (see module
/// docs).
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
/// #[component]
/// fn app() -> NodeHandle {
///     // Create the context at the top of your app
///     let theme = create_context(Theme {
///         primary_color: "#007bff".into(),
///         font_size: 16,
///     });
///
///     rsx! {
///         div {
///             // Child components can access the theme via use_context
///         }
///     }
/// }
///
/// #[component]
/// fn themed_button() -> NodeHandle {
///     // Access the theme from anywhere in the component tree
///     let theme = use_context::<Theme>();
///
///     rsx! {
///         button { style: {|| format!("color: {}", theme.primary_color)},
///             "Click me"
///         }
///     }
/// }
/// ```
pub fn create_context<T: Clone + 'static>(value: T) -> T {
    let root = current_context_root();
    CONTEXT_STORE.with(|store| {
        store
            .borrow_mut()
            .insert((root, TypeId::of::<T>()), Box::new(value.clone()));
    });
    value
}

/// Retrieve a context value by type.
///
/// Returns the value directly. Panics with a helpful message if no context
/// of the given type has been created.
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
/// #[component]
/// fn user_info() -> NodeHandle {
///     let user = use_context::<UserContext>();
///     rsx! { p { "Welcome, " {user.username} } }
/// }
/// ```
///
/// # Panics
///
/// Panics if no context of type `T` has been created via `create_context`.
pub fn use_context<T: Clone + 'static>() -> T {
    try_use_context::<T>().unwrap_or_else(|| {
        panic!(
            "Context not found: {}\nDid you forget to call create_context() in a parent component?",
            std::any::type_name::<T>()
        )
    })
}

/// Try to retrieve a context value by type.
///
/// Returns `Some(value)` if a context of the given type has been created,
/// or `None` if no such context exists.
///
/// The current context root's namespace is checked first; when the value is
/// not found there, the lookup falls back to the thread-global root `0` (see
/// module docs).
///
/// Use this when the context may not be present and you want to handle
/// the `None` case explicitly. For most uses, prefer [`use_context`] which
/// panics with a helpful message if the context is missing.
///
/// # Example
///
/// ```ignore
/// #[component]
/// fn optional_theme() -> NodeHandle {
///     let theme = try_use_context::<Theme>();
///     match theme {
///         Some(t) => rsx! { p { style: {format!("color: {}", t.color)}, "Themed" } },
///         None => rsx! { p { "No theme" } },
///     }
/// }
/// ```
pub fn try_use_context<T: Clone + 'static>() -> Option<T> {
    let tid = TypeId::of::<T>();
    let root = current_context_root();
    CONTEXT_STORE.with(|store| {
        let store = store.borrow();
        store
            .get(&(root, tid))
            .or_else(|| {
                (root != GLOBAL_ROOT)
                    .then(|| store.get(&(GLOBAL_ROOT, tid)))
                    .flatten()
            })
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
    })
}

/// Create a store — a shared state container accessible from any component.
///
/// Stores are the recommended way to manage application state in rinch.
/// A store is a struct with [`Signal`] fields and action methods that
/// encapsulate state mutations and side effects.
///
/// This is an alias for [`create_context`] with store-oriented naming.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[derive(Clone)]
/// struct CounterStore {
///     count: Signal<i32>,
/// }
///
/// impl CounterStore {
///     fn new() -> Self {
///         Self { count: Signal::new(0) }
///     }
///
///     fn increment(&self) {
///         self.count.update(|n| *n += 1);
///     }
/// }
///
/// #[component]
/// fn app() -> NodeHandle {
///     create_store(CounterStore::new());
///     rsx! { Counter {} }
/// }
///
/// #[component]
/// fn counter() -> NodeHandle {
///     let store = use_store::<CounterStore>();
///     rsx! {
///         p { {|| store.count.get().to_string()} }
///         button { onclick: move || store.increment(), "+" }
///     }
/// }
/// ```
pub fn create_store<T: Clone + 'static>(value: T) -> T {
    create_context(value)
}

/// Retrieve a store by type.
///
/// Returns the store value directly. Panics with a helpful message if no store
/// of the given type has been created via [`create_store`].
///
/// This is an alias for [`use_context`] with store-oriented naming.
pub fn use_store<T: Clone + 'static>() -> T {
    try_use_store::<T>().unwrap_or_else(|| {
        panic!(
            "Store not found: {}\nDid you forget to call create_store() in a parent component?",
            std::any::type_name::<T>()
        )
    })
}

/// Try to retrieve a store by type, returning `None` if not found.
///
/// This is an alias for [`try_use_context`] with store-oriented naming.
pub fn try_use_store<T: Clone + 'static>() -> Option<T> {
    try_use_context::<T>()
}

/// Clear the thread-global root's context (called during app reset).
///
/// Per-root namespaces are untouched — they are cleared by
/// [`clear_context_for_root`] when their root is dropped.
pub fn clear_context() {
    CONTEXT_STORE.with(|store| store.borrow_mut().retain(|(r, _), _| *r != GLOBAL_ROOT));
}

/// Clear one root's context namespace.
///
/// Called when a mounted root (e.g. an embed `RinchContext`) is dropped, so
/// its stores/contexts do not outlive it (issue #136).
pub fn clear_context_for_root(root: u64) {
    CONTEXT_STORE.with(|store| store.borrow_mut().retain(|(r, _), _| *r != root));
}
