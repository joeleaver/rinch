//! Memo: a cached computed value that only recomputes when dependencies change.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use super::effect::{EffectInner, register};
use super::{MEMO_STORE, ObserverId, RUNTIME};

/// A cached computed value that only recomputes when dependencies change.
///
/// Memos are lazily evaluated and cache their result until one of their
/// dependencies changes.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(2);
/// let doubled = Memo::new(move || count.get() * 2);
///
/// doubled.get(); // Returns 4
/// count.set(3);
/// doubled.get(); // Returns 6 (recomputed)
/// doubled.get(); // Returns 6 (cached)
/// ```
pub struct Memo<T: 'static> {
    id: u32,
    generation: u32,
    _phantom: PhantomData<T>,
}

// Manual Copy/Clone because PhantomData<T> would require T: Copy for derive
impl<T: 'static> Copy for Memo<T> {}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

struct MemoInner<T> {
    id: ObserverId,
    value: RefCell<Option<T>>,
    f: RefCell<Box<dyn Fn() -> T>>,
    dirty: Cell<bool>,
    /// The context root current when this memo was created, re-entered around
    /// the recompute in [`Memo::get`].
    ///
    /// A memo is *lazy*: the user computation does not run in the dirty-marker
    /// effect (which carries its own root) but in whichever call site first
    /// reads the memo after invalidation. Without this, a memo created in
    /// context A but first read from context B resolved B's stores (issue #136
    /// follow-up, issue #141). `0` = the thread-global fallback root.
    root: u64,
    /// The scope that owned this memo at creation, re-entered around the lazy
    /// recompute for the same reason `root` is (issue #141). Weak by
    /// construction — see [`Owner`](super::Owner).
    ///
    /// Mutable so that [`Memo::leak`] can clear it. Detaching the memo from its
    /// owner's bucket is only half of leaking: the recompute still re-enters
    /// this owner, so a `Signal` the computation creates would be attributed to
    /// — and freed with — the very scope the memo was leaked out of.
    owner: RefCell<super::Owner>,
    /// Observers to notify, ordered — same contract (and same reason) as
    /// `SignalSlot::subscribers`: a memo's dependents run in registration order.
    ///
    /// Shared with the memo's `MEMO_STORE` slot (an `Rc`, not a plain
    /// `RefCell`), because unsubscribing a disposed observer starts from a
    /// type-erased slot key and cannot downcast to `MemoInner<T>` to reach the
    /// set — see `MemoSlot::subscribers` (issue #171).
    subscribers: Rc<RefCell<BTreeSet<ObserverId>>>,
}

impl<T: Clone + 'static> Memo<T> {
    /// Create a new memo with the given computation function.
    pub fn new<F: Fn() -> T + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        // Shared with the store slot below, so a type-erased unsubscribe can
        // reach it (issue #171).
        let subscribers = Rc::new(RefCell::new(BTreeSet::new()));
        let inner = Rc::new(MemoInner {
            id,
            value: RefCell::new(None),
            f: RefCell::new(Box::new(f)),
            dirty: Cell::new(true),
            root: crate::context::current_context_root(),
            owner: RefCell::new(super::Owner::current()),
            subscribers: Rc::clone(&subscribers),
        });

        // Store memo as an effect so it can be notified. We store a "marker"
        // effect that marks the memo as dirty.
        //
        // Built before the registry is touched, not inside the borrow: the
        // constructor reads two other thread-locals (the context root and the
        // ambient owner), and nothing under an `EFFECTS` borrow should call out
        // to code that could reach back into it.
        let memo_inner = Rc::clone(&inner);
        let marker = Rc::new(EffectInner {
            f: RefCell::new(Box::new(move || {
                memo_inner.dirty.set(true);
                // Notify memo's subscribers
                let subscribers: Vec<_> = memo_inner.subscribers.borrow().iter().copied().collect();
                RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    for observer in subscribers {
                        if rt.pending_effects_set.insert(observer) {
                            rt.pending_effects.push_back(observer);
                        }
                    }
                });
            })),
            disposed: Cell::new(false),
            root: crate::context::current_context_root(),
            // Inert: the marker closure only flips a flag and queues
            // observers, so it allocates nothing to attribute. Set for
            // uniformity with every other `EffectInner`.
            owner: super::Owner::current(),
            // Not inert: this is where the *memo's* own subscriptions are
            // recorded, since the lazy recompute runs under the marker's id
            // (issue #171).
            deps: RefCell::new(Vec::new()),
            // …and the recompute, not this body, is what re-reads them. A run of
            // this marker must therefore leave them alone.
            body_tracks_deps: false,
        });
        register(id, marker);

        // Store in MEMO_STORE and return Copy handle. The marker's ObserverId
        // rides along so freeing the slot can also remove the EFFECTS entry — the marker
        // holds the second strong Rc to this same MemoInner.
        let (store_id, generation) = MEMO_STORE.with(|store| {
            store
                .borrow_mut()
                .alloc(inner as Rc<dyn Any>, id, subscribers)
        });

        // Attribute this memo to the ambient owner, if any (issue #141). The
        // marker effect is deliberately NOT recorded separately: `free_memo`
        // releases the slot and the marker together, so one record covers both.
        super::scope::record_memo(super::scope::MemoKey {
            id: store_id,
            generation,
        });

        Self {
            id: store_id,
            generation,
            _phantom: PhantomData,
        }
    }

    /// Get the current value, recomputing if necessary.
    ///
    /// # Panics
    ///
    /// Panics if the memo has been freed. Use [`try_get`](Memo::try_get) when
    /// the memo may legitimately be gone.
    pub fn get(&self) -> T {
        match self.try_get() {
            Some(value) => value,
            None => panic!(
                "Memo::get() on a freed memo. The scope that owned this memo was \
                 disposed (its component was removed from the tree) while this \
                 handle was still reachable. Guard the read with `memo.is_alive()`, \
                 or use `memo.try_get()` to get an `Option` instead of panicking."
            ),
        }
    }

    /// Get the current value, or `None` if the memo has been freed.
    ///
    /// The non-panicking counterpart to [`get`](Memo::get). Recomputes and
    /// subscribes exactly like `get` when the memo is live.
    pub fn try_get(&self) -> Option<T> {
        // Clone Rc out of store, releasing the borrow immediately
        let inner_any =
            MEMO_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))?;

        let inner = inner_any
            .downcast::<MemoInner<T>>()
            .expect("Memo type mismatch (internal error)");

        // Subscribe the current observer to this memo, and record the
        // subscription on it so a dispose (or its next run) releases it — the
        // memo half of issue #171. Same "only when the insert is new" rule as
        // `Signal::track`.
        let observer = RUNTIME.with(|rt| rt.borrow().observer_stack.last().copied());
        if let Some(observer) = observer
            && inner.subscribers.borrow_mut().insert(observer)
        {
            super::effect::record_dep(
                observer,
                super::DepKey::Memo {
                    id: self.id,
                    generation: self.generation,
                },
            );
        }

        // Recompute if dirty.
        //
        // The computation runs HERE, at the first read after invalidation — not
        // in the dirty-marker effect. So the marker's root is irrelevant to it,
        // and the memo must re-enter its own creation root, or a memo created in
        // one context but first read from another resolves the wrong stores
        // (issue #136 follow-up). Both guards are RAII so a panic in the user
        // computation cannot strand state (issue #141).
        //
        // The owner is restored for the same structural reason as the root: the
        // computation runs in the *reader's* frame, so a memo created in one
        // component but first read from another would otherwise attribute the
        // resources it creates to whatever happened to be rendering.
        if inner.dirty.get() {
            // Clone the owner out before pushing: the user computation below can
            // reach `Memo::leak` on this same memo, which takes `owner` mutably.
            let owner = inner.owner.borrow().clone();
            let _root_guard = crate::context::push_context_root(inner.root);
            let _owner_guard = owner.push();
            // Retracking: this is the pass that reads the memo's dependencies,
            // so it replaces the set the previous recompute took out (#171).
            let _observer_guard = super::effect::ObserverGuard::push_retracking(inner.id);

            let value = (inner.f.borrow())();
            *inner.value.borrow_mut() = Some(value);
            inner.dirty.set(false);
        }

        Some(
            inner
                .value
                .borrow()
                .clone()
                .expect("memo should have value after get"),
        )
    }
}

impl<T: 'static> Memo<T> {
    /// Whether this memo's backing slot is still live.
    ///
    /// A memo is freed when the scope that owns it is disposed, after which
    /// [`get`](Memo::get) panics. Does **not** subscribe the current observer.
    pub fn is_alive(&self) -> bool {
        MEMO_STORE.with(|store| store.borrow().get_inner(self.id, self.generation).is_some())
    }

    /// Detach this memo from its owning scope, giving it app lifetime.
    ///
    /// The memo counterpart of [`Signal::leak`](super::Signal::leak); the same
    /// "call it in the same render that created it" rule applies.
    ///
    /// Detaching is two-sided for a memo, where it is one-sided for a signal.
    /// Dropping the `MemoKey` from the owner's bucket stops the memo itself
    /// being freed, but the *recompute* also re-enters that owner (see
    /// [`MemoInner::owner`]), so anything the computation allocates would still
    /// be attributed to the scope this memo was just leaked out of, and die with
    /// it. Clearing the owner closes that half.
    #[track_caller]
    pub fn leak(self) -> Self {
        if !super::scope::forget_memo(super::scope::MemoKey {
            id: self.id,
            generation: self.generation,
        }) {
            tracing::debug!(
                "Memo::leak() at {}: no ambient owner held this memo; it already had \
                 app lifetime",
                std::panic::Location::caller()
            );
        }
        // `try_borrow_mut` rather than `borrow_mut`: `try_get` clones the owner
        // out before pushing it precisely so this cannot conflict, and that
        // pairing is easy to break from a distance. Degrading to "owner left in
        // place" beats taking the app down over a `leak`.
        let inner_any = MEMO_STORE.with(|store| store.borrow().get_inner(self.id, self.generation));
        if let Some(inner_any) = inner_any
            && let Ok(inner) = inner_any.downcast::<MemoInner<T>>()
            && let Ok(mut owner) = inner.owner.try_borrow_mut()
        {
            *owner = super::Owner::none();
        }
        self
    }
}

#[cfg(test)]
impl<T: 'static> Memo<T> {
    /// Free this memo — slot and marker effect — standing in for the scope
    /// disposal that #141's dispose fixpoint (PR4) will perform. Test-only.
    pub(crate) fn free_for_tests(&self) {
        super::free_memo(self.id, self.generation);
    }

    /// How many observers are subscribed to this memo, or 0 if it is freed.
    /// Test-only counterpart of `Signal::subscriber_count_for_tests`.
    pub(crate) fn subscriber_count_for_tests(&self) -> usize {
        let Some(inner_any) = MEMO_STORE.with(|s| s.borrow().get_inner(self.id, self.generation))
        else {
            return 0;
        };
        inner_any
            .downcast::<MemoInner<T>>()
            .map_or(0, |inner| inner.subscribers.borrow().len())
    }
}

impl<T: fmt::Debug + Clone + 'static> fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner_any = MEMO_STORE.with(|store| store.borrow().get_inner(self.id, self.generation));
        if let Some(inner_any) = inner_any
            && let Ok(inner) = inner_any.downcast::<MemoInner<T>>()
        {
            return f
                .debug_struct("Memo")
                .field("value", &*inner.value.borrow())
                .field("dirty", &inner.dirty.get())
                .finish();
        }
        f.debug_struct("Memo").field("error", &"freed").finish()
    }
}

#[cfg(test)]
mod liveness_tests {
    use super::*;
    use crate::reactive::{Effect, Signal};
    use std::cell::Cell;

    #[test]
    fn is_alive_flips_when_the_memo_is_freed() {
        let n = Signal::new(2);
        let doubled = Memo::new(move || n.get() * 2);

        assert!(doubled.is_alive());
        assert_eq!(doubled.try_get(), Some(4));

        doubled.free_for_tests();
        assert!(!doubled.is_alive());
        assert_eq!(doubled.try_get(), None);
    }

    #[test]
    #[should_panic(expected = "Memo::get() on a freed memo")]
    fn get_panics_after_free() {
        let n = Signal::new(1);
        let m = Memo::new(move || n.get());
        m.free_for_tests();
        let _ = m.get();
    }

    #[test]
    fn freeing_a_memo_releases_its_computation_closure() {
        // `Memo::new` puts TWO strong Rc<MemoInner> into two registries — one in
        // MEMO_STORE, one captured by the dirty-marker closure in EFFECTS.
        // Dropping only the store slot frees nothing.
        let captured = Rc::new(7u32);
        let c = Rc::clone(&captured);
        let n = Signal::new(1);
        let m = Memo::new(move || n.get() + *c);
        let _ = m.get();
        assert_eq!(Rc::strong_count(&captured), 2, "held by the memo closure");

        m.free_for_tests();

        assert_eq!(
            Rc::strong_count(&captured),
            1,
            "the marker effect must be cleared too, or the memo leaks"
        );
    }

    /// `leak` detaches a memo from its owner on **both** sides.
    ///
    /// Dropping the `MemoKey` from the bucket is the obvious half. The other is
    /// `MemoInner::owner`: the computation runs lazily, in the reader's frame,
    /// re-entering the creation-time owner — so without clearing it a `Signal`
    /// the computation allocates is still attributed to, and freed with, the
    /// scope the memo was leaked out of. The leak would then hold a handle to
    /// dead state, which is worse than not leaking at all.
    #[test]
    fn leak_detaches_a_memos_recompute_from_its_owner_too() {
        use crate::reactive::Scope;

        let source = Signal::new(1).leak();
        let scope = Scope::new();

        // Leaked in the same render that created it, per the documented rule.
        let inner_signal: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
        let sink = inner_signal.clone();
        let memo = scope.run(|| {
            Memo::new(move || {
                sink.set(Some(Signal::new(7)));
                source.get() * 2
            })
            .leak()
        });

        assert_eq!(memo.get(), 2, "the computation ran");
        let created = inner_signal
            .get()
            .expect("the computation created a signal");
        assert_eq!(
            scope.owned_counts().signals,
            0,
            "a leaked memo must not attribute its computation to the scope"
        );

        scope.dispose();

        assert!(
            memo.is_alive(),
            "the memo itself survives, as leak promises"
        );
        assert!(
            created.is_alive(),
            "and so does what its computation allocated"
        );
        assert_eq!(memo.get(), 2);
    }

    #[test]
    fn a_dependent_of_a_freed_memo_is_not_re_queued_by_its_sources() {
        // If the marker survives the free it stays subscribed to `n`, so writing
        // `n` re-queues the memo's dependents — whose `get()` then panics on the
        // now-empty slot.
        let n = Signal::new(1);
        let m = Memo::new(move || n.get() * 2);
        let runs = Rc::new(Cell::new(0));

        let r = Rc::clone(&runs);
        let _dependent = Effect::new(move || {
            let _ = m.get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1);

        m.free_for_tests();
        n.set(5); // must not resurrect the dependent, and must not panic

        assert_eq!(
            runs.get(),
            1,
            "a freed memo's sources must no longer drive its dependents"
        );
    }
}
