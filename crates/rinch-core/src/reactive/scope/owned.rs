//! The keys a scope records its resources by ([`SignalKey`], [`MemoKey`]),
//! and the bucket it keeps them in ([`Owned`], counted by [`OwnedCounts`]).

use crate::events::EventHandlerId;
use crate::reactive::ObserverId;

// ============================================================================
// Resource keys
// ============================================================================

/// Identifies a slot in `SIGNAL_STORE`.
///
/// A distinct newtype from [`MemoKey`] rather than a shared `(u32, u32)` alias:
/// signals and memos are released by *different* functions (`SignalStore::free`
/// versus `free_memo`), and calling the wrong one is silently catastrophic —
/// `MemoStore::free` alone frees nothing and leaves the memo's dirty-marker
/// subscribed, so the next write to a dependency panics its dependents. Making
/// the two keys unmixable is the cheapest guard available.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::reactive) struct SignalKey {
    pub id: u32,
    pub generation: u32,
}

/// Identifies a slot in `MEMO_STORE`. See [`SignalKey`] for why this is a
/// separate type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::reactive) struct MemoKey {
    pub id: u32,
    pub generation: u32,
}

// ============================================================================
// The owned-resource bucket
// ============================================================================

/// Resources created while a scope was the ambient owner.
///
/// Every field is plain data — ids and keys, never the resource itself — so
/// taking an `Owned` apart runs no user code and cannot re-enter the disposal
/// machinery. The actual release happens later, in
/// `run_dispose_fixpoint` (in `dispose.rs`), from the batches harvested here.
#[derive(Default)]
pub(in crate::reactive) struct Owned {
    pub(super) signals: Vec<SignalKey>,
    pub(super) memos: Vec<MemoKey>,
    pub(super) handlers: Vec<EventHandlerId>,
    /// Effects *created* under this owner.
    ///
    /// Deliberately distinct from [`ScopeInner::effects`](super::ScopeInner::effects), which holds effects
    /// *adopted* via [`Scope::add_effect`](super::Scope::add_effect). Disposal releases both: the
    /// difference between the two counts is the fire-and-forget effects nothing
    /// ever adopted, which is precisely the leak #141 was filed for.
    pub(super) effects: Vec<ObserverId>,
}

/// A snapshot of what a scope owns.
///
/// Introspection for tests and diagnostics. Disposal empties the bucket, so
/// after `dispose` every count reads zero.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OwnedCounts {
    pub signals: usize,
    pub memos: usize,
    pub handlers: usize,
    pub effects: usize,
}

impl Owned {
    pub(super) fn counts(&self) -> OwnedCounts {
        OwnedCounts {
            signals: self.signals.len(),
            memos: self.memos.len(),
            handlers: self.handlers.len(),
            effects: self.effects.len(),
        }
    }
}
