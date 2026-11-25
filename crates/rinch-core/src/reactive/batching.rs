//! Batching and effect flushing.

use super::counters::SIGNAL_NOTIFIES;
use super::{RUNTIME, notify_signal_change, restore};

// The effect-queue drain; both flush sites reach it via `flush_effects_and_notify`.
use super::effect::flush_effects;

// ============================================================================
// Batching
// ============================================================================

/// RAII guard for the batching flag.
///
/// Saves the previous flag value on construction and restores it on drop —
/// including while unwinding. A bare set/clear pair has two failure modes:
/// a panic between them leaks `batching = true`, after which every later
/// write queues observers that nothing ever flushes (a silent UI freeze,
/// issue #232); and a nested `batch()` clearing to `false` ends the *outer*
/// batch's transaction early. Restoring the saved value fixes both — only
/// the outermost guard restores `false`.
struct BatchGuard {
    prev: bool,
}

impl BatchGuard {
    fn raise() -> Self {
        let prev = RUNTIME.with(|rt| std::mem::replace(&mut rt.borrow_mut().batching, true));
        BatchGuard { prev }
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        // A borrowed runtime is logged, not skipped silently: that would latch
        // `batching = true` for the rest of the thread's life — the #232
        // freeze this guard exists to prevent.
        let prev = self.prev;
        let _ = restore::with_runtime_on_drop("the batching flag", |rt| rt.batching = prev);
    }
}

/// Batch multiple signal updates to avoid redundant effect runs.
///
/// Effects will only run once after the batch completes, even if multiple
/// signals they depend on are updated.
///
/// Batches nest: a `batch()` inside another batch's *closure* joins the outer
/// transaction, and effects flush once, when the outermost batch exits. A
/// `batch()` entered during a *flush* — from inside an effect the outer batch
/// woke — is instead a fresh outermost batch (the outer flag has already been
/// restored by then), so it keeps the top-level contract: the flush completes
/// synchronously before `batch()` returns.
///
/// That flush-time rule is about flushes, not effect bodies in general: an
/// effect body that runs while a batch is still *open* — e.g. `Effect::new`'s
/// immediate first run inside a batch closure — is inside that batch, and a
/// `batch()` opened there joins the outer transaction. Code that pairs a
/// batch with an RAII window (raise flag → batch writes → drop flag) and
/// needs observers to run inside the window must account for this case.
///
/// Until the flush, no effect has run: inside the closure — including after a
/// *nested* `batch()` returns — effects have not executed. [`Memo::get`](super::Memo::get) is
/// current all the same: a write marks the memos that read it stale
/// synchronously, and the read recomputes (see `memo`'s module docs).
///
/// Event-handler dispatch runs inside one (`events::dispatch_event` and its
/// siblings, the keyboard and paste interceptors, the dismiss stack, `Drag`
/// callbacks, each drained main-thread callback, each timer — the guide's
/// "Event handlers run as batches" has the exact list, and what is left out),
/// so a handler that writes several signals flushes once. A `NodeHandle`
/// operation inside the batch runs the effects queued so far first
/// ([`flush_pending_effects`]), so the DOM a handler touches is never behind the
/// writes it already made.
///
/// An outermost batch that wrote no signal, finds no [host
/// change](note_host_change) owed and no effect queued at its exit does
/// nothing there: no flush, and no signal-change callback — so a handler that
/// wrote nothing requests no re-render. A batch that wrote runs
/// the callbacks once at its exit even when its effects already ran mid-batch,
/// and a batch that wrote nothing still flushes (and notifies for) effects an
/// earlier batch left queued.
///
/// # Panics
///
/// A panic inside the closure propagates, and the batching flag is restored on
/// the way out. Nothing is flushed while unwinding — running arbitrary effect
/// code mid-panic would be worse than the panic itself — so the effects the
/// aborted batch queued stay pending and run at the next flush (the next
/// signal write outside a batch, or the next outermost batch exit). No such
/// flush is *scheduled*: if nothing ever writes again, the queued effects stay
/// pending and no re-render is requested for the writes that did land before
/// the panic.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(0);
/// let name = Signal::new("".to_string());
///
/// batch(|| {
///     count.set(1);
///     name.set("Alice".to_string());
///     // Effects only run once, after this batch
/// });
/// ```
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    let guard = BatchGuard::raise();
    let outermost = !guard.prev;

    // An outermost batch is its own flush context, even when it is opened from
    // inside an effect body (an effect that dispatches a handler, a web
    // `focus()` that fires one synchronously). The effect depth that makes
    // `flush_pending_effects` a no-op inside effect bodies is set aside for its
    // duration, so a DOM call in the batch still sees the batch's own writes.
    // Library suppression (`suppress_effect_flush`) is deliberately NOT set
    // aside: a borrow the library holds is still held.
    let _depth = outermost.then(DepthSetAside::enter);
    // What suppression the outermost batch was opened under (library code can
    // legitimately open one while holding a guard), so its exit can check that
    // nothing taken inside it outlived it.
    let suppressed_at_entry = outermost.then(|| FLUSH_SUPPRESSED.with(|s| s.get()));
    // How many signal writes this thread had made when the outermost batch
    // opened, so its exit can tell a batch that wrote nothing (issue #234).
    let writes_at_entry = outermost.then(|| SIGNAL_NOTIFIES.with(|c| c.get()));

    let result = f();

    // A leaked `suppress_effect_flush` guard (`mem::forget`, a guard stashed
    // in a struct that outlives the handler) would silently switch the
    // mid-batch flush off on this thread for good, and nothing else would ever
    // notice: every handler would just lose program order again.
    if let Some(at_entry) = suppressed_at_entry {
        debug_assert_eq!(
            FLUSH_SUPPRESSED.with(|s| s.get()),
            at_entry,
            "a suppress_effect_flush() guard taken inside this batch outlived it; \
             mid-batch flushing would stay off on this thread"
        );
    }

    // Restore the flag *before* flushing: `Signal::set` inside a flushed
    // effect must see `batching = false` again, and a `batch()` opened there
    // must count as a fresh outermost batch.
    drop(guard);

    // An outermost batch that wrote no signal and left nothing queued has
    // nothing to run and nothing to tell the host (issue #234): every event
    // dispatch is a batch, so a handler that wrote nothing used to post a
    // desktop `ReRender` and dirty every embedded context for a no-op. Both
    // halves of the test matter. A write whose effects already ran mid-batch
    // (`flush_pending_effects`) leaves the queue empty and still owes the host
    // its notification; and effects an earlier batch left queued — one whose
    // closure panicked and was caught, see `# Panics` — are run, and notified
    // for, by the next batch, written to or not.
    //
    // A host change ([`note_host_change`] — the thread-global theme slot is
    // one) that no notification has delivered yet is owed too, whether it was
    // made inside this batch or outside any: the hosts pick it up only from
    // this notification.
    if let Some(at_entry) = writes_at_entry {
        let wrote = SIGNAL_NOTIFIES.with(|c| c.get()) != at_entry;
        if wrote || host_change_owed() || has_pending_effects() {
            flush_effects_and_notify();
        }
    }

    result
}

thread_local! {
    /// Set by [`note_host_change`], cleared by every signal-change notification
    /// (each one tells every host), and read by [`batch`]'s exit.
    pub(super) static HOST_CHANGE_OWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn host_change_owed() -> bool {
    HOST_CHANGE_OWED.with(|c| c.get())
}

/// Record a change to state the **hosts** read outside the signal graph, so
/// the batch it is made in still calls the signal-change callbacks.
///
/// An outermost [`batch`] that wrote no signal calls no signal-change callback
/// (issue #234), and those callbacks are how a host learns something changed:
/// the desktop shell's `ReRender`, an embedded context's dirty flag, the web
/// backend's theme `<style>` refresh. A piece of state that is not a
/// [`Signal`](super::Signal), that code may change from a handler, and that a host reads
/// only when told — `set_current_theme_css` (`rinch::update_theme`) is the one
/// in rinch today — calls this when it changes, and the enclosing batch's exit
/// notifies as if a signal had been written. Made outside any batch, it
/// notifies nothing at once and is owed to the next notification: the next
/// outermost batch notifies for it whether or not that batch writes anything
/// (so the user's next click of any kind delivers it, as before #234), and any
/// other notification — an unbatched signal write — pays it along the way.
///
/// State a host polls on its own (a dirty DOM node, a pending focus request,
/// the editor's owed overlay pass, a finished image decode, a queued
/// `NodeHandle::scroll_into_view` / `scroll_to_fraction` request — #1145)
/// does not need it.
pub fn note_host_change() {
    HOST_CHANGE_OWED.with(|c| c.set(true));
}

/// Whether any effect is queued on this thread. Answers `true` when the
/// runtime is borrowed — unreachable at a batch's exit (see `BatchGuard::drop`),
/// and `true` keeps the unconditional flush that was there before #234.
fn has_pending_effects() -> bool {
    RUNTIME.with(|rt| {
        rt.try_borrow()
            .map(|rt| !rt.pending_effects.is_empty())
            .unwrap_or(true)
    })
}

thread_local! {
    /// How many effect bodies and memo computations are running on this thread
    /// right now (nested runs count once each). See [`flush_pending_effects`].
    static REACTIVE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Saves [`REACTIVE_DEPTH`], zeroes it, and puts it back on drop — for an
/// outermost [`batch`], which is a flush context of its own.
struct DepthSetAside {
    _restore: restore::RestoreCell<ReactiveDepthSlot>,
}

restore::restore_slot!(ReactiveDepthSlot: u32 = REACTIVE_DEPTH);

impl DepthSetAside {
    fn enter() -> Self {
        DepthSetAside {
            _restore: restore::RestoreCell::replace(0),
        }
    }
}

thread_local! {
    /// Open [`suppress_effect_flush`] guards on this thread.
    static FLUSH_SUPPRESSED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// While the returned guard lives, [`flush_pending_effects`] — and therefore
/// every `NodeHandle` operation — runs no effects.
///
/// For **library code that touches the DOM while it holds a borrow of its own
/// state**. A `NodeHandle` operation made inside a batch runs the effects the
/// batch has queued so far, and a user effect is free to call back into the
/// library — which, under a held `RefCell` borrow, is a `BorrowMutError`. The
/// editor is the shape: `EditorHandle::command` holds its core mutably while
/// the view patches the DOM, and a toolbar effect reading
/// `is_mark_active` would run in the middle of it. Take the guard *with* the
/// borrow and drop it after; the pending effects run at the next DOM access
/// outside it, or when the batch ends.
///
/// Library code should also call [`flush_pending_effects`] **before** taking
/// such a borrow, so that the caller's earlier writes have reached the DOM the
/// library is about to work on (program order, as for any DOM access).
///
/// Unlike the effect depth, suppression is not set aside by a [`batch`]
/// opened inside it: a batch does not release the borrow.
#[must_use = "the suppression ends when the guard is dropped"]
pub fn suppress_effect_flush() -> EffectFlushSuppressed {
    FLUSH_SUPPRESSED.with(|s| s.set(s.get() + 1));
    EffectFlushSuppressed {
        _not_send: std::marker::PhantomData,
    }
}

/// The guard [`suppress_effect_flush`] returns.
pub struct EffectFlushSuppressed {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for EffectFlushSuppressed {
    fn drop(&mut self) {
        let _ = FLUSH_SUPPRESSED.try_with(|s| s.set(s.get().saturating_sub(1)));
    }
}

/// Marks an effect body or memo computation as running, for
/// [`flush_pending_effects`]. RAII, so a panicking body cannot strand it.
pub(crate) struct ReactiveDepthGuard;

impl ReactiveDepthGuard {
    pub(crate) fn enter() -> Self {
        REACTIVE_DEPTH.with(|d| d.set(d.get() + 1));
        ReactiveDepthGuard
    }
}

impl Drop for ReactiveDepthGuard {
    fn drop(&mut self) {
        let _ = REACTIVE_DEPTH.try_with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Run, now, the effects the current [`batch`] has queued so far — the flush it
/// would otherwise run when it exits — and leave the batch open.
///
/// This is what keeps an event handler's **program order** intact although
/// the handler is a batch: every `NodeHandle` operation calls it first, so
/// `open.set(true); field.focus()` focuses *after* the dialog's effects have
/// run (and after the dialog's own focus request, which the handler's then
/// overrides), and `rows.update(..); list.scroll_to_bottom()` scrolls to the
/// new last row. It is the analogue of a browser flushing pending style when
/// script asks for layout. Call it yourself before handing control to code
/// that reads the DOM some other way.
///
/// The flush is an ordinary one: the batching flag is lowered while it runs,
/// so the effects see exactly what the end-of-batch flush would show them
/// (a write inside one flushes synchronously; a `batch()` inside one flushes
/// before returning), and raised again afterwards so the handler's later writes
/// are batched as before. Order is the queue's, so the #154 contract holds.
///
/// A no-op outside a batch (writes there have already flushed), when nothing
/// is queued, while an effect body or memo computation is running — those
/// run *inside* a flush, and draining the queue from one would run effects
/// queued behind it ahead of their turn (an outermost `batch` opened inside
/// one is its own context again) — and under [`suppress_effect_flush`]. The signal-change callbacks (the
/// host's re-render request) are not called here; the batch's own exit calls
/// them once.
pub fn flush_pending_effects() {
    if REACTIVE_DEPTH.with(|d| d.get()) > 0 || FLUSH_SUPPRESSED.with(|s| s.get()) > 0 {
        return;
    }
    let pending = RUNTIME.with(|rt| {
        rt.try_borrow()
            .map(|rt| rt.batching && !rt.pending_effects.is_empty())
            .unwrap_or(false)
    });
    if !pending {
        return;
    }
    /// Lowers the flag for the flush and raises it again — on unwind too.
    struct Lowered;
    impl Drop for Lowered {
        fn drop(&mut self) {
            let _ = restore::with_runtime_on_drop("the batching flag", |rt| rt.batching = true);
        }
    }
    RUNTIME.with(|rt| rt.borrow_mut().batching = false);
    let _raise = Lowered;
    flush_effects();
}

/// Run every pending effect, then the UI re-render callbacks.
///
/// The order is a contract: effects run BEFORE the callbacks, so fine-grained
/// updates are queued by the time a callback decides whether a full re-render
/// is needed. Both flush sites — `Signal::notify` (unbatched writes) and the
/// outermost [`batch`] exit — go through here, so the policy cannot drift
/// between them.
pub(crate) fn flush_effects_and_notify() {
    flush_effects();
    notify_signal_change();
}
