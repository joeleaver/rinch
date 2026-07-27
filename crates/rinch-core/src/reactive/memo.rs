//! Memo: a cached computed value that only recomputes when dependencies change.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use super::effect::EffectInner;
use super::{MEMO_STORE, ObserverId, RUNTIME};

// Re-use the EFFECTS storage from the effect module
use super::effect::EFFECTS;

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
    /// Observers to notify, ordered — same contract (and same reason) as
    /// `SignalSlot::subscribers`: a memo's dependents run in registration order.
    subscribers: RefCell<BTreeSet<ObserverId>>,
}

impl<T: Clone + 'static> Memo<T> {
    /// Create a new memo with the given computation function.
    pub fn new<F: Fn() -> T + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        let inner = Rc::new(MemoInner {
            id,
            value: RefCell::new(None),
            f: RefCell::new(Box::new(f)),
            dirty: Cell::new(true),
            root: crate::context::current_context_root(),
            subscribers: RefCell::new(BTreeSet::new()),
        });

        // Store memo as an effect so it can be notified
        let inner_clone = Rc::clone(&inner);
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let idx = id.0;
            if idx >= effects.len() {
                effects.resize(idx + 1, None);
            }
            // We store a "marker" effect that marks the memo as dirty
            let memo_inner = inner_clone;
            effects[idx] = Some(Rc::new(EffectInner {
                id,
                f: RefCell::new(Box::new(move || {
                    memo_inner.dirty.set(true);
                    // Notify memo's subscribers
                    let subscribers: Vec<_> =
                        memo_inner.subscribers.borrow().iter().copied().collect();
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
            }));
        });

        // Store in MEMO_STORE and return Copy handle
        let (store_id, generation) =
            MEMO_STORE.with(|store| store.borrow_mut().alloc(inner as Rc<dyn Any>));

        Self {
            id: store_id,
            generation,
            _phantom: PhantomData,
        }
    }

    /// Get the current value, recomputing if necessary.
    pub fn get(&self) -> T {
        // Clone Rc out of store, releasing the borrow immediately
        let inner_any = MEMO_STORE.with(|store| {
            store
                .borrow()
                .get_inner(self.id, self.generation)
                .expect("Memo::get() on freed memo")
        });

        let inner = inner_any
            .downcast::<MemoInner<T>>()
            .expect("Memo type mismatch (internal error)");

        // Subscribe current observer to this memo
        RUNTIME.with(|rt| {
            let rt = rt.borrow();
            if let Some(&observer) = rt.observer_stack.last() {
                inner.subscribers.borrow_mut().insert(observer);
            }
        });

        // Recompute if dirty.
        //
        // The computation runs HERE, at the first read after invalidation — not
        // in the dirty-marker effect. So the marker's root is irrelevant to it,
        // and the memo must re-enter its own creation root, or a memo created in
        // one context but first read from another resolves the wrong stores
        // (issue #136 follow-up). Both guards are RAII so a panic in the user
        // computation cannot strand state (issue #141).
        if inner.dirty.get() {
            let _root_guard = crate::context::push_context_root(inner.root);
            let _observer_guard = super::effect::ObserverGuard::push(inner.id);

            let value = (inner.f.borrow())();
            *inner.value.borrow_mut() = Some(value);
            inner.dirty.set(false);
        }

        inner
            .value
            .borrow()
            .clone()
            .expect("memo should have value after get")
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
