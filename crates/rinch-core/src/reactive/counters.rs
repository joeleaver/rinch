//! Reactive-runtime performance counters.

// ============================================================================
// Performance counters
// ============================================================================

/// Cumulative reactive-runtime counters, since the thread started (or since a
/// document began to be counted). Monotonic: a consumer that wants per-frame
/// numbers keeps the last snapshot and subtracts (the desktop shell folds the
/// deltas into the document's `rinch_dom::perf` counters at the end of each
/// frame).
///
/// [`reactive_counters`] reads the thread's: every document's work and
/// everybody else's. A document [counted on its own](count_doc_reactive_work)
/// is read with [`doc_reactive_counters`], and the work of no counted document
/// with [`unattributed_reactive_counters`] (#2798). An effect's run is the
/// document it was created in (the one its reactive frame re-enters), and a
/// signal write the document current where it is written (an effect's, or
/// the one dispatching).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReactiveCounters {
    /// Effect bodies run (an effect skipped as disposed or re-entered is not
    /// counted).
    pub effect_runs: u64,
    /// `Signal` change notifications (`set`, `update`, and the other writers).
    pub signal_notifies: u64,
}

thread_local! {
    static EFFECT_RUNS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Not only a counter: [`batch`] compares it across an outermost batch to
    /// tell whether the batch wrote anything (issue #234), so every signal
    /// write must keep bumping it.
    pub(super) static SIGNAL_NOTIFIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A snapshot of this thread's [`ReactiveCounters`]: all of its work,
/// whichever document it was for.
pub fn reactive_counters() -> ReactiveCounters {
    ReactiveCounters {
        effect_runs: EFFECT_RUNS.with(|c| c.get()),
        signal_notifies: SIGNAL_NOTIFIES.with(|c| c.get()),
    }
}

/// The reactive work of each document counted on its own, and of nobody
/// else (#2798).
#[derive(Debug)]
pub(crate) struct DocReactiveCounters {
    /// Each counted document's counters. A thread has a few documents (a
    /// window each, DevTools), so a scan beats a map on the path every
    /// effect run and signal write takes.
    docs: Vec<(u64, ReactiveCounters)>,
    /// Work under no document, or under one nobody counts.
    unattributed: ReactiveCounters,
}

impl DocReactiveCounters {
    const fn new() -> Self {
        Self {
            docs: Vec::new(),
            unattributed: ReactiveCounters {
                effect_runs: 0,
                signal_notifies: 0,
            },
        }
    }

    #[inline]
    fn of(&mut self, doc: u64) -> &mut ReactiveCounters {
        if doc != 0
            && let Some(i) = self.docs.iter().position(|(d, _)| *d == doc)
        {
            return &mut self.docs[i].1;
        }
        &mut self.unattributed
    }
}

impl crate::doc_keyed::ForgetDoc for DocReactiveCounters {
    fn take_doc(&mut self, doc: u64) -> crate::doc_keyed::Removed {
        self.docs.retain(|(d, _)| *d != doc);
        Vec::new()
    }

    fn entries_for_doc(&self, doc: u64) -> usize {
        self.docs.iter().filter(|(d, _)| *d == doc).count()
    }
}

crate::doc_keyed! {
    /// Each counted document's reactive work; its sweep stops counting it.
    static DOC_COUNTERS: DocReactiveCounters = DocReactiveCounters::new();
}

/// Count document `doc_key`'s reactive work on its own from now on, from
/// zero, until the document is forgotten ([`forget_doc`](crate::forget_doc)).
/// What a performance consumer (the shell's window, an embedded context)
/// calls when it mounts the document, so the document's frames can carry
/// its own work rather than the thread's (#2798). `0` (no document) and a
/// document already counted change nothing.
pub fn count_doc_reactive_work(doc_key: u64) {
    if doc_key == 0 {
        return;
    }
    DOC_COUNTERS.with_mut(|c| {
        if !c.docs.iter().any(|(d, _)| *d == doc_key) {
            c.docs.push((doc_key, ReactiveCounters::default()));
        }
    });
}

/// Document `doc_key`'s reactive work since it began to be counted
/// ([`count_doc_reactive_work`]); zero for a document not counted.
pub fn doc_reactive_counters(doc_key: u64) -> ReactiveCounters {
    DOC_COUNTERS
        .try_with(|c| c.docs.iter().find(|(d, _)| *d == doc_key).map(|(_, r)| *r))
        .flatten()
        .unwrap_or_default()
}

/// The thread's reactive work under no counted document: under no document
/// at all (a write from `main`, a worker's queued closure, a test), or under
/// one nobody counts, since the thread started.
pub fn unattributed_reactive_counters() -> ReactiveCounters {
    DOC_COUNTERS
        .try_with(|c| c.unattributed)
        .unwrap_or_default()
}

/// How many documents this thread counts on its own.
pub fn counted_doc_count() -> usize {
    DOC_COUNTERS.try_with(|c| c.docs.len()).unwrap_or(0)
}

/// One effect body run, for document `doc` (the raw key its frame re-entered;
/// `0` for none).
#[inline]
pub(crate) fn count_effect_run(doc: u64) {
    EFFECT_RUNS.with(|c| c.set(c.get().wrapping_add(1)));
    let _ = DOC_COUNTERS.try_with_mut(|c| {
        let r = c.of(doc);
        r.effect_runs = r.effect_runs.wrapping_add(1);
    });
}

/// One signal write, for the document current where it is written.
#[inline]
pub(crate) fn count_signal_notify() {
    SIGNAL_NOTIFIES.with(|c| c.set(c.get().wrapping_add(1)));
    let doc = crate::context::current_doc_raw();
    let _ = DOC_COUNTERS.try_with_mut(|c| {
        let r = c.of(doc);
        r.signal_notifies = r.signal_notifies.wrapping_add(1);
    });
}
