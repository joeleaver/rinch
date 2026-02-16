//! State hooks for managing persistent state across renders.
//!
//! Provides `use_signal`, `use_state`, `use_ref`, and `use_derived`.

use std::cell::RefCell;

use crate::reactive::{Memo, Signal};

use super::HOOK_REGISTRY;

/// Create or retrieve a persistent reactive signal.
///
/// This is the primary hook for managing state. The initializer function
/// is only called on the first render.
///
/// # Example
///
/// ```ignore
/// #[component]
/// fn app() -> NodeHandle {
///     let count = use_signal(|| 0);
///
///     rsx! {
///         button { onclick: move || count.update(|n| *n += 1),
///             "Count: " {|| count.get().to_string()}
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
/// #[component]
/// fn app() -> NodeHandle {
///     let (count, set_count) = use_state(|| 0);
///
///     rsx! {
///         button { onclick: move || set_count(count + 1),
///             "Count: " {count.to_string()}
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
/// #[component]
/// fn app() -> NodeHandle {
///     let render_count = use_ref(|| 0);
///     *render_count.borrow_mut() += 1;
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

/// Create a derived value that auto-tracks signal dependencies.
///
/// Unlike `use_memo` which requires explicit dependencies, `use_derived` uses
/// the reactive system's `Memo` type to automatically track which signals are
/// read during computation and recompute only when those signals change.
///
/// # Example
///
/// ```ignore
/// #[component]
/// fn app() -> NodeHandle {
///     let count = use_signal(|| 0);
///     let multiplier = use_signal(|| 2);
///
///     // Automatically tracks both count and multiplier
///     let doubled = use_derived(move || count.get() * multiplier.get());
///
///     rsx! {
///         p { "Result: " {|| doubled.get().to_string()} }
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
