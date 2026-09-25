//! Work that needs `&mut RinchApp` and arrives on a later turn (issue #328).
//!
//! The main-thread callback queue (`rinch_core::run_on_main_thread`,
//! `park_main_callback`) runs closures with no access to the app: that is
//! enough for anything that writes signals or holds an `Rc` handle (the
//! editor's asynchronous paste re-enters through its `EditorHandle`), and not
//! enough for a completion that has to drive the runtime itself — the plain
//! `<input>`/`<textarea>` paste goes through `handle_input_edit_command`, which
//! is the focus arbiter's, the undo grouping's and the `oninput` dispatch's.
//!
//! So each app owns an inbox. An [`AppWorkSender`] (cheap to clone, `Send`)
//! pushes a `Send` closure into it from any thread and wakes the host through
//! `run_on_main_thread`; the host calls [`RinchApp::run_deferred_work`] after
//! it drains the main-thread queue, which is the turn that wake produces. The
//! closure receives the app as an argument, so it needs to carry only `Send`
//! data (the pasted text, the node it was aimed at) and never the app itself.
//!
//! The sender holds the inbox weakly: work sent after its app was dropped is
//! discarded, as a parked callback is when its component unmounts.

use std::sync::{Arc, Mutex, Weak};

use super::RinchApp;

/// One unit of deferred work: runs once, on the main thread, with the app.
pub(crate) type AppWork = Box<dyn FnOnce(&mut RinchApp) + Send>;

/// The app's inbox. Behind an `Arc` so a sender can reach it from the thread
/// that produced the result.
#[derive(Default)]
pub(crate) struct DeferredWork {
    queue: Mutex<Vec<AppWork>>,
}

impl DeferredWork {
    /// Whether anything is waiting.
    pub(crate) fn is_pending(&self) -> bool {
        !self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Everything queued so far, in the order it was sent.
    pub(crate) fn take(&self) -> Vec<AppWork> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Sends work to one app's inbox from any thread.
///
/// Its one producer today is the plain-control paste, which exists only with
/// the `clipboard` feature; the inbox and its drain are there regardless, so
/// a host's call to [`RinchApp::run_deferred_work`] does not depend on it.
#[derive(Clone)]
#[cfg_attr(not(feature = "clipboard"), allow(dead_code))]
pub(crate) struct AppWorkSender(Weak<DeferredWork>);

#[cfg_attr(not(feature = "clipboard"), allow(dead_code))]
impl AppWorkSender {
    pub(crate) fn new(inbox: &Arc<DeferredWork>) -> Self {
        Self(Arc::downgrade(inbox))
    }

    /// Queue `work` for the app's next [`RinchApp::run_deferred_work`] and
    /// wake the host so that turn comes. A no-op if the app has been dropped.
    ///
    /// The push happens before the wake, so the turn the wake produces always
    /// finds the work; a host that happens to run its deferred work earlier
    /// finds it sooner, and the wake's turn then finds nothing.
    pub(crate) fn send(&self, work: impl FnOnce(&mut RinchApp) + Send + 'static) {
        let Some(inbox) = self.0.upgrade() else {
            return;
        };
        inbox
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(work));
        drop(inbox);
        // The wake: an empty main-thread callback. On a background thread it
        // is queued through the host's dispatcher, which wakes the event loop
        // (desktop, Android) or is drained at the next `update` (embed); on the
        // main thread — or before any host registered one — it runs at once
        // and does nothing, and the work waits for the host's next turn.
        rinch_core::run_on_main_thread(|| {});
    }
}

impl RinchApp {
    /// A sender for work that must run with this app on a later turn.
    #[cfg_attr(not(feature = "clipboard"), allow(dead_code))]
    pub(crate) fn app_work_sender(&self) -> AppWorkSender {
        AppWorkSender::new(&self.deferred_work)
    }

    /// Whether other threads have sent work that [`Self::run_deferred_work`]
    /// has not run yet (an embed host's `needs_update` asks it).
    pub fn has_deferred_work(&self) -> bool {
        self.deferred_work.is_pending()
    }

    /// Run the work other threads have sent this app since the last call —
    /// today, the completion of a plain `<input>`/`<textarea>` paste whose
    /// clipboard read ran off the UI thread (issue #328). Answers how many
    /// items ran; a non-zero answer means the document may have changed, so a
    /// host should repaint.
    ///
    /// **A host calls this right after `rinch_core::drain_main_callbacks`**,
    /// on the main thread: the desktop shell on each wake, the Android loop
    /// each iteration, an embedded `RinchContext` at the top of each
    /// `update`. A host that drives a `RinchApp` itself and never calls it
    /// gets no plain-control paste — the chord starts the read and the result
    /// waits here.
    ///
    /// Items run in the order they were sent. Work sent while one runs waits
    /// for the next call, so a completion that sends more cannot spin here.
    pub fn run_deferred_work(&mut self) -> usize {
        let work = self.deferred_work.take();
        let ran = work.len();
        for item in work {
            item(self);
        }
        if ran > 0 {
            self.scene_dirty = true;
        }
        ran
    }
}
