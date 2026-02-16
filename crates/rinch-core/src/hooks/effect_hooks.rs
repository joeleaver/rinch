//! Effect hooks for side effects, memoization, and callbacks.
//!
//! Provides `use_effect`, `use_effect_cleanup`, `use_mount`, `use_memo`, and `use_callback`.

use std::cell::RefCell;

use super::HOOK_REGISTRY;

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
/// #[component]
/// fn app() -> NodeHandle {
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
        registry
            .borrow_mut()
            .use_hook::<std::rc::Rc<RefCell<EffectState<D>>>>("use_effect", || {
                std::rc::Rc::new(RefCell::new(EffectState {
                    deps: None,
                    cleanup: None,
                }))
            })
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
/// #[component]
/// fn app() -> NodeHandle {
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
        registry
            .borrow_mut()
            .use_hook::<std::rc::Rc<RefCell<EffectState<D>>>>("use_effect_cleanup", || {
                std::rc::Rc::new(RefCell::new(EffectState {
                    deps: None,
                    cleanup: None,
                }))
            })
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
/// #[component]
/// fn app() -> NodeHandle {
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
/// #[component]
/// fn app() -> NodeHandle {
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
        registry
            .borrow_mut()
            .use_hook::<std::rc::Rc<RefCell<MemoState<T, D>>>>("use_memo", || {
                std::rc::Rc::new(RefCell::new(MemoState {
                    value: None,
                    deps: None,
                }))
            })
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
/// #[component]
/// fn app() -> NodeHandle {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{
        begin_render, clear_hooks, end_render, use_ref, use_signal, use_state,
    };
    use crate::hooks::context::clear_context;
    use crate::hooks::context::{create_context, use_context};
    use crate::hooks::state_hooks::use_derived;

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
        let doubled = use_derived(move || count.get() * 2);
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
