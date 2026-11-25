//! Signal and memo storage, and memo staleness.

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use super::main_thread::{
    QUEUE_THREAD, SLOT_MASK, TAG_SHIFT, claim_thread_tag, release_thread_tag,
};
use super::{ObserverId, effect, memo};

// ============================================================================
// Signal Storage
// ============================================================================

// Thread-local storage for all signal values.
// Signals are stored as type-erased `Box<dyn Any>` in a generation-counted
// slot vec. This allows `Signal<T>` to be `Copy` — it only contains an
// index and generation counter.
thread_local! {
    pub(super) static SIGNAL_STORE: RefCell<SignalStore> = RefCell::new(SignalStore::new());
}

/// A signal's value and subscriber set, each in its **own** `RefCell` —
/// separate from the `RefCell<SignalStore>` that the slot vec itself lives in
/// (issue #546).
///
/// Every read/write first clones this `Rc` out of `SIGNAL_STORE` under a
/// brief, shared `borrow()`, then drops the store borrow and operates on
/// `value`/`subscribers` alone. That is what lets a nested read of a
/// *different* signal — `a.with(|av| b.get())` — work both in and out of an
/// effect: `b`'s `track()` no longer needs `SIGNAL_STORE.borrow_mut()` while
/// `a`'s closure is running, because `a`'s own borrow was never taken on the
/// store in the first place; it is taken on `a`'s own `SignalCell`, which is a
/// different `RefCell` from `b`'s. Only a signal reading or writing *itself*
/// from inside its own `with`/`update` closure still panics — genuine
/// reentrancy on one `RefCell`, not an artifact of sharing the whole store.
///
/// This mirrors `Memo`'s existing shape (`MemoInner` is reached the same way,
/// through an `Rc<dyn Any>` cloned out of `MEMO_STORE`), which is why memos
/// never had this bug.
pub(crate) struct SignalCell {
    pub(crate) value: RefCell<Box<dyn Any>>,
    /// Observers to notify, ordered. `BTreeSet` (not `HashSet`) so iteration
    /// yields ascending `ObserverId` = registration order; see the module-level
    /// "Execution order" docs.
    pub(crate) subscribers: RefCell<BTreeSet<ObserverId>>,
}

/// One entry in [`SignalStore`]'s slot vec: the generation that names it, plus
/// the `Rc<SignalCell>` callers clone out to reach the value/subscribers
/// without holding the store borrow.
struct SignalSlot {
    inner: Rc<SignalCell>,
    generation: u32,
}

pub(crate) struct SignalStore {
    slots: Vec<Option<SignalSlot>>,
    /// Indices of freed slots available for reuse.
    free_list: Vec<u32>,
    /// Generation counter, incremented on each allocation, never 0.
    next_gen: u32,
    /// This thread's tag, in an id's high bits (issue #2778): every id is
    /// `tag_bits | slot`, so a signal names its thread. An id with other tag
    /// bits names no signal here.
    tag_bits: u32,
    /// Whether this store has taken a thread tag yet (at its first signal).
    tagged: bool,
}

impl SignalStore {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            next_gen: 1, // Start at 1 so generation=0 is never valid
            tag_bits: 0,
            tagged: false,
        }
    }

    /// Take this thread's tag, once: at the first signal, so a thread that
    /// only asks [`Self::has_allocated`] takes none.
    pub(crate) fn claim_tag(&mut self) {
        if self.tagged {
            return;
        }
        self.tagged = true;
        let (tag, counter) = claim_thread_tag();
        self.tag_bits = tag << TAG_SHIFT;
        self.next_gen = counter;
        let _ = QUEUE_THREAD.try_with(|q| q.tag.set(tag));
    }

    pub(crate) fn alloc<T: 'static>(&mut self, value: T) -> (u32, u32) {
        self.claim_tag();
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        if self.next_gen == 0 {
            self.next_gen = 1; // Skip 0
        }

        let slot = SignalSlot {
            inner: Rc::new(SignalCell {
                value: RefCell::new(Box::new(value)),
                subscribers: RefCell::new(BTreeSet::new()),
            }),
            generation,
        };

        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(slot);
            (self.tag_bits | idx, generation)
        } else {
            let idx = self.slots.len() as u32;
            assert!(
                idx <= SLOT_MASK,
                "more than {} signals alive at once on one thread",
                SLOT_MASK as u64 + 1
            );
            self.slots.push(Some(slot));
            (self.tag_bits | idx, generation)
        }
    }

    /// The slot index `id` names in this store, or `None` when its tag is
    /// another thread's (issue #2778): ids of two threads' stores never alias.
    fn slot_of(&self, id: u32) -> Option<usize> {
        (id & !SLOT_MASK == self.tag_bits).then_some((id & SLOT_MASK) as usize)
    }

    /// Whether `id` was made on another thread: its tag is not this store's,
    /// so no slot here can hold it (issue #2889 tells this drop from a freed
    /// slot's).
    pub(crate) fn is_foreign(&self, id: u32) -> bool {
        self.slot_of(id).is_none()
    }

    /// Whether this store has ever allocated a signal (slots are never
    /// removed, only emptied, so a freed signal still counts).
    pub(super) fn has_allocated(&self) -> bool {
        !self.slots.is_empty()
    }

    /// The slot's `SignalCell`, cloned out under a shared borrow of the store
    /// alone — the store borrow is gone by the time the caller touches
    /// `value` or `subscribers`.
    pub(crate) fn get_inner(&self, id: u32, generation: u32) -> Option<Rc<SignalCell>> {
        self.slots
            .get(self.slot_of(id)?)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| Rc::clone(&s.inner))
    }

    /// Free a slot, **returning** its value rather than dropping it in place.
    ///
    /// The caller must drop the returned box after releasing the store borrow:
    /// a value whose `Drop` touches a signal would otherwise `BorrowMutError`
    /// (issue #141, SD4). Returns `None` if the slot was already freed or the
    /// generation does not match.
    ///
    /// The per-signal `Rc<SignalCell>` is `try_unwrap`'d here rather than
    /// dropped as-is: nothing but the store itself normally holds a clone at
    /// dispose time, so this succeeds in the overwhelming common case and the
    /// caller gets the same `Box<dyn Any>` contract as before #546. If some
    /// in-flight borrow elsewhere still holds a clone — reentrant disposal
    /// from inside this very signal's own `with`/`update` closure, which is
    /// not a shape any caller is expected to construct — the value is simply
    /// not parked for the deterministic end-of-fixpoint drop; it drops
    /// whenever that other clone's last reference goes away instead, same as
    /// any other `Rc`.
    fn free(&mut self, id: u32, generation: u32) -> Option<Box<dyn Any>> {
        let index = self.slot_of(id)?;
        let slot = self.slots.get_mut(index)?;
        if !slot.as_ref().is_some_and(|s| s.generation == generation) {
            return None;
        }
        let taken = slot.take()?;
        self.free_list.push(index as u32);
        match Rc::try_unwrap(taken.inner) {
            Ok(cell) => Some(cell.value.into_inner()),
            Err(_still_shared) => None,
        }
    }
}

impl Drop for SignalStore {
    /// The thread is ending: its tag is free for another thread, which
    /// continues its generation counter (see [`TagPool`](super::main_thread::TagPool)).
    fn drop(&mut self) {
        if self.tag_bits != 0 {
            let _ = QUEUE_THREAD.try_with(|q| q.tag.set(0));
            release_thread_tag(self.tag_bits >> TAG_SHIFT, self.next_gen);
        }
    }
}

/// Free a signal slot, **returning** its value for the caller to drop once every
/// borrow is released. See [`SignalStore::free`].
///
/// The disposal fixpoint parks the returned value and drops it at the very end,
/// so that a value's `Drop` sees a deterministic world rather than an arbitrary
/// prefix of its siblings.
pub(crate) fn free_signal(id: u32, generation: u32) -> Option<Box<dyn Any>> {
    SIGNAL_STORE.with(|store| store.borrow_mut().free(id, generation))
}

/// Free a signal slot and drop its value, standing in for scope disposal.
/// Test-only convenience over [`free_signal`].
#[cfg(test)]
pub(crate) fn free_signal_for_tests(id: u32, generation: u32) {
    // Dropped out here, after the store borrow is released.
    let _value = free_signal(id, generation);
}

// ============================================================================
// Memo Storage
// ============================================================================

thread_local! {
    pub(super) static MEMO_STORE: RefCell<MemoStore> = RefCell::new(MemoStore::new());
}

struct MemoSlot {
    inner: Rc<dyn Any>, // Type-erased Rc<MemoInner<T>>
    /// The same allocation as `inner`, as the runtime's type-erased memo
    /// interface: the version check in `flush_effects` starts from a
    /// [`DepKey`](super::DepKey) and has to refresh the memo without knowing its `T`.
    node: Rc<dyn memo::MemoNode>,
    /// The memo's dirty-marker effect, which holds the *second* strong
    /// reference to the same `MemoInner`. Recorded here because the slot is
    /// type-erased: freeing a memo has to remove the marker's `EFFECTS` entry
    /// too, and a type-erased caller cannot downcast to reach `MemoInner::id`.
    observer: ObserverId,
    /// The memo's subscriber set, shared with its `MemoInner`.
    ///
    /// Shared rather than owned for the same type-erasure reason as `observer`:
    /// unsubscribing a disposed observer (issue #171) starts from a [`DepKey`](super::DepKey),
    /// which names a slot, and a type-erased caller cannot downcast an
    /// `Rc<dyn Any>` to `MemoInner<T>` to reach the set inside it.
    subscribers: Rc<RefCell<BTreeSet<ObserverId>>>,
    generation: u32,
}

pub(crate) struct MemoStore {
    slots: Vec<Option<MemoSlot>>,
    free_list: Vec<u32>,
    next_gen: u32,
}

impl MemoStore {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            next_gen: 1,
        }
    }

    pub(crate) fn alloc(
        &mut self,
        inner: Rc<dyn Any>,
        node: Rc<dyn memo::MemoNode>,
        observer: ObserverId,
        subscribers: Rc<RefCell<BTreeSet<ObserverId>>>,
    ) -> (u32, u32) {
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        if self.next_gen == 0 {
            self.next_gen = 1;
        }

        let slot = MemoSlot {
            inner,
            node,
            observer,
            subscribers,
            generation,
        };

        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(slot);
            (idx, generation)
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(slot));
            (idx, generation)
        }
    }

    pub(crate) fn get_inner(&self, id: u32, generation: u32) -> Option<Rc<dyn Any>> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| Rc::clone(&s.inner))
    }

    /// The memo in this slot as a [`MemoNode`](memo::MemoNode), if the slot
    /// still holds the memo the caller recorded.
    pub(crate) fn get_node(&self, id: u32, generation: u32) -> Option<Rc<dyn memo::MemoNode>> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| Rc::clone(&s.node))
    }

    /// Remove an observer from a memo's subscriber set, if this slot still
    /// holds the memo the caller recorded.
    ///
    /// Type-erased on purpose: the caller holds a [`DepKey`](super::DepKey), not a `Memo<T>`,
    /// and an `Rc<dyn Any>` cannot be downcast without `T`. Hence the set being
    /// shared with the slot rather than living only inside `MemoInner`.
    pub(crate) fn unsubscribe(&self, id: u32, generation: u32, observer: ObserverId) {
        if let Some(slot) = self
            .slots
            .get(id as usize)
            .and_then(|s| s.as_ref())
            .filter(|s| s.generation == generation)
        {
            slot.subscribers.borrow_mut().remove(&observer);
        }
    }

    /// Free a slot, **returning** its `Rc` and the marker's [`ObserverId`]
    /// rather than dropping in place — same rule and same reason as
    /// [`SignalStore::free`]: a `MemoInner`'s cached value can own arbitrary
    /// user data whose `Drop` may touch the reactive stores.
    ///
    /// Freeing the slot alone does **not** release the memo. Use
    /// [`free_memo`], which also removes the marker effect's registry entry.
    pub(crate) fn free(&mut self, id: u32, generation: u32) -> Option<(Rc<dyn Any>, ObserverId)> {
        let slot = self.slots.get_mut(id as usize)?;
        if !slot.as_ref().is_some_and(|s| s.generation == generation) {
            return None;
        }
        let taken = slot.take();
        self.free_list.push(id);
        taken.map(|s| (s.inner, s.observer))
    }
}

/// Release a memo completely: its store slot **and** its dirty-marker effect.
///
/// `Memo::new` puts two strong `Rc<MemoInner>` references into two different
/// registries — one in `MEMO_STORE`, one captured by the marker closure in
/// `EFFECTS`. Dropping only the store slot frees nothing (the marker keeps the
/// cached value and the computation closure alive) and is actively harmful: the
/// marker stays subscribed to the memo's sources, so the next write to any of
/// them re-queues the memo's dependents, whose `Memo::get()` then panics on the
/// now-empty slot.
///
/// Both `Rc`s are dropped after every borrow is released, since the cached value
/// is arbitrary user data whose `Drop` may touch the reactive stores.
pub(crate) fn free_memo(id: u32, generation: u32) {
    let Some((inner, observer)) = MEMO_STORE.with(|store| store.borrow_mut().free(id, generation))
    else {
        return;
    };
    let marker = effect::EFFECTS.with(|effects| effects.borrow_mut().remove(&observer));
    if let Some(marker) = &marker {
        // Same reason `dispose_effect` sets it: removal alone only stops *future*
        // lookups, and an `Rc` handed out by an earlier one can still be in
        // flight. This is the flag that tells such a run it has been retired.
        marker.disposed.set(true);
        // And release what the memo's *computation* subscribed to: the marker is
        // the id a memo's sources hold, so leaving its deps recorded would strand
        // a dead `ObserverId` in every signal the computation read (issue #171).
        effect::unsubscribe_deps_of(marker, observer);
    }
    drop(marker);
    drop(inner);
}

// ============================================================================
// Memo staleness
// ============================================================================

/// The memo a registered observer stands for, if it is a memo's marker.
fn memo_node_of(observer: ObserverId) -> Option<Rc<dyn memo::MemoNode>> {
    effect::EFFECTS.with(|effects| {
        effects
            .borrow()
            .get(&observer)
            .and_then(|inner| inner.memo.clone())
    })
}

/// Mark stale, synchronously, every memo among a written signal's
/// `subscribers` (`Dirty`), and every memo downstream of those (`Check`).
///
/// This is the *push* half of the memo model: it is what lets a read made
/// straight after a write — inside a [`batch`](super::batch), inside an event handler that
/// has not returned yet — see the new value, where a memo invalidated only
/// when its queued marker ran would answer from its cache until the flush.
/// It computes nothing. The walk stops at a memo that was already stale,
/// because a stale memo's dependents were marked when it became stale.
pub(crate) fn mark_memos_stale(subscribers: &[ObserverId]) {
    let mut to_check: Vec<ObserverId> = Vec::new();
    for &observer in subscribers {
        if let Some(node) = memo_node_of(observer)
            && node.mark_dirty()
        {
            to_check.extend(node.subscriber_snapshot());
        }
    }
    while let Some(observer) = to_check.pop() {
        if let Some(node) = memo_node_of(observer)
            && node.mark_check()
        {
            to_check.extend(node.subscriber_snapshot());
        }
    }
}

#[cfg(test)]
mod generation_tests;
