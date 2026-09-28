//! The one Drop path for the crate's thread-local save/restore guards
//! (issue #234, item 3).
//!
//! rinch-core keeps its ambient state in thread-locals — the reactive
//! [`Runtime`](super::Runtime) (observer stack, owner stack, batching flag),
//! the context root and dispatching document, the effect depth, the
//! child-observer dispatch flag — and every piece of code that changes one of
//! them for the length of a call restores it with an RAII guard, so that a
//! panic caught upstream cannot leave it changed (#141, #232). Those guards
//! used to hand-roll their `Drop`s, and the hand-rolls drifted: some used
//! `with` where others used `try_with`, and a borrowed runtime was skipped
//! silently by one guard and logged by the next. The #233 review found an
//! unwind hole in `untracked()` that a shared path would have closed by
//! construction.
//!
//! Two shapes cover them:
//!
//! - [`RestoreCell`] — a thread-local [`Cell`] (or a `Cell` field of one) set
//!   to a value now and put back to the value it had on drop.
//! - [`with_runtime_on_drop`] — what a guard over a [`Runtime`](super::Runtime)
//!   field does in its `Drop`: nothing if the thread's TLS is already gone (a
//!   guard dropped at thread exit — legitimate, silent), an error log if the
//!   runtime is borrowed (unreachable in a correct program, and a silent skip
//!   there would strand the state the guard exists to restore), the restore
//!   otherwise.
//!
//! Counting guards (`ReactiveDepthGuard`, `EffectFlushSuppressed`) increment
//! on entry and decrement on drop rather than save and restore, so that an
//! out-of-order drop still balances; they are not this module's shape.

use std::cell::Cell;
use std::marker::PhantomData;
use std::thread::LocalKey;

use super::{RUNTIME, Runtime};

/// Sets a thread-local [`Cell`] now and restores its previous value on drop —
/// including while unwinding, and silently not at all once the thread's TLS
/// has been torn down.
///
/// `S` is the thread-local's type and `cell` picks the `Cell` inside it, so a
/// guard can restore one field of a struct thread-local (`AMBIENT.root`) as
/// well as a bare `thread_local! { static X: Cell<T> }` ([`Self::replace`]).
///
/// `!Send`: the value it restores belongs to the thread that made it.
#[must_use = "the previous value is restored the instant this guard drops"]
pub(crate) struct RestoreCell<S: 'static, T: Copy + 'static> {
    key: &'static LocalKey<S>,
    cell: fn(&S) -> &Cell<T>,
    prev: T,
    _not_send: PhantomData<*const ()>,
}

impl<S: 'static, T: Copy + 'static> RestoreCell<S, T> {
    /// Set the `Cell` `cell` picks out of `key` to `value` until the guard
    /// drops.
    pub(crate) fn replace_in(
        key: &'static LocalKey<S>,
        cell: fn(&S) -> &Cell<T>,
        value: T,
    ) -> Self {
        let prev = key.with(|s| cell(s).replace(value));
        RestoreCell {
            key,
            cell,
            prev,
            _not_send: PhantomData,
        }
    }
}

impl<T: Copy + 'static> RestoreCell<Cell<T>, T> {
    /// Set the thread-local `key` to `value` until the guard drops.
    pub(crate) fn replace(key: &'static LocalKey<Cell<T>>, value: T) -> Self {
        Self::replace_in(key, |c| c, value)
    }
}

impl<S: 'static, T: Copy + 'static> Drop for RestoreCell<S, T> {
    fn drop(&mut self) {
        let (cell, prev) = (self.cell, self.prev);
        // `try_with`: TLS may already be torn down at thread exit.
        let _ = self.key.try_with(|s| cell(s).set(prev));
    }
}

/// Why [`with_runtime_on_drop`] could not run its restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unrestored {
    /// The thread's TLS is already torn down. Silent.
    TlsGone,
    /// The runtime was borrowed. Already logged.
    Borrowed,
}

/// Run `restore` against the thread's reactive runtime from a guard's `Drop`,
/// and answer what it returned — or why it could not run.
///
/// It cannot run in two cases, and they are deliberately treated differently:
///
/// - **The TLS is gone** (a guard dropped at thread exit, after the runtime's
///   destructor ran): silent. There is nothing left to restore.
/// - **The runtime is borrowed**: logged as an error naming `what` the guard
///   restores. No correct program reaches it — a caller holding the borrow
///   would have made the guard's own constructor panic, and unwinding releases
///   an inner `RefMut` before the guard's frame drops — but skipping it
///   silently would leave the state changed for the rest of the thread's life,
///   which is the failure every one of these guards exists to prevent.
pub(crate) fn with_runtime_on_drop<R>(
    what: &'static str,
    restore: impl FnOnce(&mut Runtime) -> R,
) -> Result<R, Unrestored> {
    RUNTIME
        .try_with(|rt| match rt.try_borrow_mut() {
            Ok(mut rt) => Ok(restore(&mut rt)),
            Err(_) => {
                tracing::error!(
                    "could not restore {what} (reactive runtime already borrowed); \
                     reactive state may be left changed"
                );
                Err(Unrestored::Borrowed)
            }
        })
        .unwrap_or(Err(Unrestored::TlsGone))
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static PLAIN: Cell<u32> = const { Cell::new(0) };
    }

    struct Pair {
        a: Cell<u32>,
        b: Cell<u32>,
    }

    thread_local! {
        static PAIR: Pair = const { Pair { a: Cell::new(0), b: Cell::new(0) } };
    }

    /// Nested guards restore in LIFO order, each to the value *it* displaced —
    /// not to the cell's initial value, which a guard that "resets" rather
    /// than restores would write. Sampled off zero for that reason.
    #[test]
    fn nested_guards_restore_the_value_each_displaced() {
        PLAIN.with(|c| c.set(3));
        {
            let _outer = RestoreCell::replace(&PLAIN, 5);
            {
                let _inner = RestoreCell::replace(&PLAIN, 9);
                assert_eq!(PLAIN.with(|c| c.get()), 9);
            }
            assert_eq!(PLAIN.with(|c| c.get()), 5, "the inner guard put back 5");
        }
        assert_eq!(PLAIN.with(|c| c.get()), 3, "the outer guard put back 3");
    }

    /// A panic caught upstream still restores the cell — the #232 shape.
    #[test]
    fn a_guard_restores_while_unwinding() {
        PLAIN.with(|c| c.set(4));
        let caught = std::panic::catch_unwind(|| {
            let _g = RestoreCell::replace(&PLAIN, 8);
            panic!("boom");
        });
        assert!(caught.is_err());
        assert_eq!(PLAIN.with(|c| c.get()), 4);
    }

    /// A guard over one field of a struct thread-local touches only that field.
    #[test]
    fn a_field_guard_restores_only_its_field() {
        PAIR.with(|p| {
            p.a.set(1);
            p.b.set(2);
        });
        {
            let _g = RestoreCell::replace_in(&PAIR, |p| &p.a, 7);
            PAIR.with(|p| p.b.set(6));
            assert_eq!(PAIR.with(|p| p.a.get()), 7);
        }
        assert_eq!(PAIR.with(|p| (p.a.get(), p.b.get())), (1, 6));
    }

    /// A borrowed runtime is reported (`Err(Borrowed)`), not panicked on and not
    /// silently treated as restored.
    #[test]
    fn a_borrowed_runtime_is_refused_not_panicked_on() {
        let ran = RUNTIME.with(|rt| {
            let _held = rt.borrow();
            with_runtime_on_drop("a test value", |_| ())
        });
        assert_eq!(
            ran,
            Err(Unrestored::Borrowed),
            "the restore could not run under a borrow"
        );

        // Positive control: with the borrow released it runs and answers.
        let depth = with_runtime_on_drop("a test value", |rt| rt.observer_stack.len());
        assert!(depth.is_ok(), "an unborrowed runtime runs the restore");
    }
}
