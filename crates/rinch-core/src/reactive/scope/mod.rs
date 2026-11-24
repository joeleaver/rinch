//! Scope: manages the lifetime of reactive primitives.
//!
//! # Ownership is ambient
//!
//! A [`Scope`] is both a disposal list (effects, cleanups, child scopes) and an
//! **owner**: while a scope is pushed as the ambient owner, every [`Signal`],
//! [`Memo`], [`Effect`] and event handler created is *attributed* to it.
//!
//! Disposing a scope **frees** everything attributed to it. Reads of a freed
//! [`Signal`]/[`Memo`] panic and writes become warn-once no-ops (see the
//! [`reactive`](super) module's "Liveness" section); a freed event handler
//! simply stops being found by dispatch.
//!
//! **No ambient owner means app lifetime.** Resources created outside any
//! render — in `main()`, in a startup routine, in a detached callback — are
//! owned by nobody and live as long as the thread. That is what makes
//! ownership non-breaking to add.
//!
//! Two independent pieces of ambient state are in play, and they are easy to
//! confuse:
//!
//! | state | suspended by | answers |
//! |---|---|---|
//! | current observer | [`untracked`](super::untracked) | *who subscribes to this read* |
//! | owner stack | [`unowned`] | *who owns this allocation* |
//!
//! [`Signal`]: super::Signal
//! [`Memo`]: super::Memo
//! [`Effect`]: super::Effect

mod ambient;
mod dispose;
mod handle;
mod owned;
mod owner;

pub(in crate::reactive) use ambient::{
    forget_memo, forget_signal, record_effect, record_memo, record_signal,
};
pub use ambient::{on_cleanup, on_cleanup_removable};
pub(crate) use ambient::{on_cleanup_for_ambient_owner, record_handler};
pub(crate) use dispose::disposal_available;
pub(in crate::reactive) use handle::ScopeInner;
pub use handle::{CleanupHandle, Scope};
pub use owned::OwnedCounts;
pub(in crate::reactive) use owned::{MemoKey, SignalKey};
pub(in crate::reactive) use owner::Footprint;
pub use owner::{Owner, OwnerGuard, current_owner, unowned};

#[cfg(test)]
mod dispose_tests;
#[cfg(test)]
mod owner_tests;
#[cfg(test)]
mod removable_cleanup_tests;
#[cfg(test)]
mod teardown_tests;
