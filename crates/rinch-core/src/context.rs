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
//!
//! # Entry lifetime (issue #141)
//!
//! An entry lives as long as the **scope that created it**. What that means in
//! practice depends on where `create_store`/`create_context` was called:
//!
//! | called from | entry lives until |
//! |---|---|
//! | outside any render — startup, `main()`, a menu/tray callback, a timer | the thread ends (no ambient owner) |
//! | a root component body | that mount is torn down |
//! | a `match` arm, `for` body or `show` branch | that branch is swapped away |
//! | a component re-rendered by `reactive_component_dom` | its next re-render |
//! | an event handler or effect body | the scope that *registered* it dies |
//!
//! Only the **entry** is owned, never the signals inside it — the caller builds
//! those before `create_store` is entered, so there is nothing to reparent. A
//! store whose entry is gone reports the usual "Store not found" from
//! [`use_store`], rather than handing back dangling handles.
//!
//! To opt out and keep an entry for the life of the thread, create it under
//! [`unowned`](crate::reactive::unowned).

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// The thread-global fallback root (see module docs).
const GLOBAL_ROOT: u64 = 0;

/// One insertion into the store.
///
/// The `epoch` exists for exactly one reason: a scope's cleanup runs *later*,
/// and by then the slot may hold somebody else's entry (issue #141). Removing
/// by key alone would let a dying scope delete a live scope's store.
struct ContextEntry {
    epoch: u64,
    value: Box<dyn Any>,
}

// Thread-local context store for sharing state across components, keyed by
// (root, TypeId) — each mounted root gets its own namespace (issue #136).
thread_local! {
    static CONTEXT_STORE: RefCell<HashMap<(u64, TypeId), ContextEntry>> =
        RefCell::new(HashMap::new());
    /// The root whose namespace create/use_context resolve right now.
    static CURRENT_ROOT: Cell<u64> = const { Cell::new(GLOBAL_ROOT) };
    /// Monotonic insertion counter. Never reused, and deliberately **not** reset
    /// by `clear_context`/`clear_context_for_root` — resetting it would recreate
    /// the exact ABA the epoch exists to prevent.
    static NEXT_EPOCH: Cell<u64> = const { Cell::new(1) };
}

fn next_epoch() -> u64 {
    NEXT_EPOCH.with(|c| {
        let epoch = c.get();
        c.set(epoch + 1);
        epoch
    })
}

/// Remove `(root, tid)` **iff** it still carries `epoch`, returning the entry so
/// the caller drops it outside the store borrow.
///
/// Total by construction: a missing or already-replaced entry is a no-op, never
/// an assert. Both a scope cleanup and [`clear_context_for_root`] can target the
/// same entry, in either order.
#[must_use = "the removed value must be dropped outside the CONTEXT_STORE borrow"]
fn take_context_entry(root: u64, tid: TypeId, epoch: u64) -> Option<ContextEntry> {
    CONTEXT_STORE.with(|store| {
        let mut store = store.borrow_mut();
        match store.get(&(root, tid)) {
            Some(entry) if entry.epoch == epoch => store.remove(&(root, tid)),
            _ => None,
        }
    })
}

/// Remove every entry whose root satisfies `doomed`, dropping the values only
/// after the store borrow is released.
///
/// `HashMap::retain` would drop them *inside* the borrow, and a removed value's
/// `Drop` can now dispose a scope, whose cleanup re-enters this store
/// (issue #141).
fn clear_roots(doomed: impl Fn(u64) -> bool) {
    let removed: Vec<ContextEntry> = CONTEXT_STORE.with(|store| {
        let mut store = store.borrow_mut();
        let keys: Vec<(u64, TypeId)> = store.keys().filter(|(r, _)| doomed(*r)).copied().collect();
        keys.into_iter().filter_map(|k| store.remove(&k)).collect()
    });
    drop(removed);
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
/// # Lifetime
///
/// The entry is owned by the scope that created it and is removed when that
/// scope is disposed; created outside any render it lives for the life of the
/// thread. See the "Entry lifetime" section of the [module docs](self) for the
/// full table, and [`unowned`](crate::reactive::unowned) to opt out.
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
    // Captured HERE rather than read back in the cleanup: the cleanup runs at
    // dispose time with no root guard pushed, so `current_context_root()` would
    // report the thread-global root 0 — a *different* namespace. Same reasoning
    // as the handler wrappers in `crate::events` (issue #136).
    let root = current_context_root();
    let tid = TypeId::of::<T>();
    let epoch = next_epoch();

    // Cloned and boxed BEFORE taking the borrow. `store.borrow_mut().insert(k,
    // Box::new(value.clone()))` evaluates the receiver first, so `T::clone` —
    // arbitrary user code — would run while the store is mutably borrowed.
    let boxed: Box<dyn Any> = Box::new(value.clone());
    let displaced = CONTEXT_STORE.with(|store| {
        store.borrow_mut().insert(
            (root, tid),
            ContextEntry {
                epoch,
                value: boxed,
            },
        )
    });
    // Any value this replaced is dropped out here, after the borrow is released.
    drop(displaced);

    // Tie the ENTRY to the scope that created it (issue #141). Not the signals
    // inside it: the caller built those before this function was entered, so
    // there is nothing to reparent. With no ambient owner — startup code, a menu
    // callback, a timer — nothing is registered and the entry keeps the app
    // lifetime it has always had.
    let _registered = crate::reactive::on_cleanup_for_ambient_owner(move || {
        drop(take_context_entry(root, tid, epoch));
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
            .and_then(|entry| entry.value.downcast_ref::<T>())
            .cloned()
    })
}

/// Create a store — a shared state container accessible from any component.
///
/// Stores are the recommended way to manage application state in rinch.
/// A store is a struct with [`Signal`](crate::reactive::Signal) fields and
/// action methods that encapsulate state mutations and side effects.
///
/// This is an alias for [`create_context`] with store-oriented naming.
///
/// The store entry is owned by the scope that created it — see the "Entry
/// lifetime" section of the [module docs](self). Calling this from a root
/// component body (the usual place) gives it that mount's lifetime.
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
    clear_roots(|r| r == GLOBAL_ROOT);
}

/// Clear one root's context namespace.
///
/// Called when a mounted root (e.g. an embed `RinchContext`) is dropped, so
/// its stores/contexts do not outlive it (issue #136).
pub fn clear_context_for_root(root: u64) {
    clear_roots(move |r| r == root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{Scope, unowned};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Debug, PartialEq)]
    struct S(&'static str);
    #[derive(Clone, Debug, PartialEq)]
    struct T2(&'static str);

    /// The core of PR3: an entry created under a scope dies with it.
    #[test]
    fn a_store_created_under_a_scope_dies_with_it() {
        let scope = Scope::new();
        scope.run(|| create_store(S("owned")));
        assert_eq!(try_use_store::<S>(), Some(S("owned")));

        scope.dispose();
        assert_eq!(
            try_use_store::<S>(),
            None,
            "the entry must not outlive the scope that created it"
        );
    }

    /// The other direction, and the reason this is non-breaking: with no ambient
    /// owner an entry keeps app lifetime. This is what keeps the DevTools store
    /// alive across F12 open/close cycles.
    #[test]
    fn a_store_created_with_no_owner_survives_a_scope_dispose() {
        clear_context();
        create_store(S("global"));

        let scope = Scope::new();
        scope.run(|| create_store(T2("scoped")));
        scope.dispose();

        assert_eq!(
            try_use_store::<S>(),
            Some(S("global")),
            "an unowned entry must survive an unrelated scope's disposal"
        );
        assert_eq!(try_use_store::<T2>(), None, "the scoped one is reclaimed");
        clear_context();
    }

    /// The ABA case the epoch exists for: two sibling scopes create the same
    /// type, then the *earlier* one dies. It must not delete the live entry.
    ///
    /// Reachable for real — two `for` items rendering the same component. The
    /// self-replacement case (one component re-rendering) is safe without an
    /// epoch, because every re-render site disposes the old scope before
    /// building the new one; siblings have no such ordering.
    #[test]
    fn a_later_scopes_entry_survives_the_earlier_scopes_dispose() {
        clear_context();
        let a = Scope::new();
        let b = Scope::new();

        a.run(|| create_store(S("a")));
        b.run(|| create_store(S("b")));
        assert_eq!(try_use_store::<S>(), Some(S("b")), "last write wins");

        a.dispose();
        assert_eq!(
            try_use_store::<S>(),
            Some(S("b")),
            "a stale cleanup must not remove an entry it no longer owns"
        );

        b.dispose();
        assert_eq!(try_use_store::<S>(), None, "the live owner still reclaims");
        clear_context();
    }

    /// Creating the same type twice in one scope leaves two cleanups; the epoch
    /// makes the stale one inert, so no de-duplication is needed.
    #[test]
    fn duplicate_creates_in_one_scope_remove_exactly_the_live_entry() {
        clear_context();
        let a = Scope::new();
        let b = Scope::new();

        a.run(|| {
            create_store(S("first"));
            create_store(S("second"));
        });
        b.run(|| create_store(S("third")));

        a.dispose();
        assert_eq!(
            try_use_store::<S>(),
            Some(S("third")),
            "both of A's cleanups must no-op against B's entry"
        );

        b.dispose();
        assert_eq!(try_use_store::<S>(), None);
        clear_context();
    }

    /// The cleanup must target the root that was current at *creation*. At
    /// dispose time no root guard is pushed, so reading it back would resolve
    /// the thread-global root 0 and delete the wrong namespace.
    #[test]
    fn a_cleanup_removes_only_its_creation_root() {
        clear_context();
        create_store(S("root-zero"));

        let scope = Scope::new();
        {
            let _root = push_context_root(7);
            scope.run(|| create_store(S("root-seven")));
        }

        // Dispose with CURRENT_ROOT back at 0.
        scope.dispose();

        assert_eq!(
            try_use_store::<S>(),
            Some(S("root-zero")),
            "the root-0 entry belongs to nobody and must be untouched"
        );
        {
            let _root = push_context_root(7);
            assert_eq!(
                try_use_store::<S>(),
                Some(S("root-zero")),
                "root 7's own entry is gone, so the lookup falls back to root 0"
            );
        }
        clear_context();
    }

    /// A scope cleanup and `clear_context_for_root` can both target one entry,
    /// in either order, without panicking or removing a later re-creation.
    #[test]
    fn clear_context_for_root_then_dispose_is_idempotent() {
        clear_context();
        let a = Scope::new();
        {
            let _root = push_context_root(9);
            a.run(|| create_store(S("first")));
        }

        clear_context_for_root(9);
        {
            let _root = push_context_root(9);
            unowned(|| create_store(S("recreated")));
        }

        a.dispose(); // A's cleanup is now stale
        {
            let _root = push_context_root(9);
            assert_eq!(
                try_use_store::<S>(),
                Some(S("recreated")),
                "a cleanup whose entry was already cleared must not remove its successor"
            );
        }
        clear_context_for_root(9);
        clear_context();
    }

    /// A store value whose `Drop` re-enters the context store must not
    /// `BorrowMutError`. Pins that removed values are dropped outside the borrow.
    #[test]
    fn a_store_drop_that_re_enters_the_context_store_does_not_panic() {
        clear_context();
        #[derive(Clone)]
        struct Probe(Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Probe {
            fn drop(&mut self) {
                // Re-entrant read while the store is being mutated elsewhere.
                let _ = try_use_context::<S>();
                self.0.borrow_mut().push("dropped");
            }
        }

        let log = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::new();
        scope.run(|| drop(create_context(Probe(log.clone()))));

        scope.dispose();
        assert!(
            log.borrow().contains(&"dropped"),
            "the removed value's Drop must have run"
        );
        assert!(
            try_use_context::<Probe>().is_none(),
            "and the entry must be gone"
        );
        clear_context();
    }

    /// Replacing an entry drops the displaced value outside the store borrow.
    /// Pre-PR3 this panicked: `insert` dropped it while the `RefMut` was live.
    #[test]
    fn replacing_an_entry_drops_the_old_value_outside_the_borrow() {
        clear_context();
        #[derive(Clone)]
        struct Probe;
        impl Drop for Probe {
            fn drop(&mut self) {
                // Would panic "already mutably borrowed" if this ran under the
                // insert's borrow.
                let _ = try_use_context::<S>();
            }
        }

        unowned(|| {
            drop(create_context(Probe));
            drop(create_context(Probe)); // displaces the first
        });
        clear_context();
    }

    /// `clear_context_for_root` drops removed values outside the borrow too.
    /// This is the path `RinchContext::drop` takes on every embed teardown.
    #[test]
    fn clear_context_for_root_drops_values_outside_the_borrow() {
        clear_context();
        #[derive(Clone)]
        struct Probe;
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = try_use_context::<S>();
            }
        }

        {
            let _root = push_context_root(5);
            unowned(|| drop(create_context(Probe)));
        }
        clear_context_for_root(5); // must not panic
        clear_context();
    }

    /// A `create_store` whose ambient owner is a **disposed** scope gets app
    /// lifetime rather than attaching its cleanup to a corpse — where it would
    /// silently never run.
    ///
    /// This is reachable in production even though `dispose` pushes no owner of
    /// its own. `record_effect` declines to record against a disposed ambient
    /// owner while `Owner::current` still captures it, so such an effect is
    /// disposed by nobody, and every later run re-pushes that dead owner —
    /// which is exactly the stack `run_effect` installs here.
    #[test]
    fn a_store_created_under_a_disposed_owner_gets_app_lifetime() {
        use crate::reactive::{Effect, Signal};

        clear_context();
        let scope = Scope::new();
        scope.run(|| create_store(S("doomed")));

        let trigger = Signal::new(0).leak();
        let effect = {
            // Create the effect while the (already disposed) scope is ambient,
            // reproducing the record/capture asymmetry described above.
            let _owner = scope.push_owner();
            scope.dispose();
            Effect::new(move || {
                if trigger.get() > 0 {
                    create_store(T2("born-under-a-corpse"));
                }
            })
        };

        assert_eq!(try_use_store::<S>(), None, "the owned entry is reclaimed");

        trigger.set(1); // re-runs the effect with the dead owner ambient

        assert_eq!(
            try_use_store::<T2>(),
            Some(T2("born-under-a-corpse")),
            "a store whose only candidate owner is disposed falls back to app \
             lifetime instead of attaching to a corpse"
        );

        drop(effect);
        clear_context();
    }
}
