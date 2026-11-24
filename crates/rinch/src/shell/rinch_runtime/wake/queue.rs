//! The native event queue: what is waiting for the desktop runtime, and
//! whether anything will ever take it (#2801, #3015).

use std::collections::VecDeque;

use super::RinchNativeEvent;

/// How many events a process keeps for a desktop runtime that has not been
/// made yet (#3015). An app's `main` sends a handful before
/// [`App::run`](crate::App::run); a process that never runs one (a CLI that
/// shares code with the app, a test binary's worker thread) must not grow
/// the queue without bound. Past it a request is dropped, with one warning.
/// [`RinchNativeEvent::Exit`] is never dropped for it, and is kept once.
pub(crate) const KEPT_BEFORE_RUN: usize = 1024;

/// What will drain the queue, which decides what a send does with its event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drainer {
    /// No desktop runtime yet, and nothing says none will come: an app
    /// before [`App::run`](crate::App::run). Events are kept for it, up to
    /// [`KEPT_BEFORE_RUN`], and the runtime it makes delivers them; an
    /// [`RinchNativeEvent::Exit`] among them makes the run return before it
    /// opens a window ([`NativeEventQueue::take_exit`]).
    Coming,
    /// A desktop runtime was made (and possibly has ended since): it drains
    /// the queue at each wake. Never left: a request sent after the runtime
    /// is gone waits for the next one, as it always has.
    Runtime,
    /// None was made and a [`RinchContext`](crate::embed::RinchContext) was:
    /// an embed host, whose own loop drains nothing here. Its contexts'
    /// requests are their host's (#2801), and one from outside any context is
    /// dropped. A runtime made later still takes over.
    #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
    EmbedHost,
}

/// The native events waiting for the next `proxy_wake_up`, with at most one
/// [`RinchNativeEvent::ReRender`] among them.
///
/// `ReRender` means "signals changed since the last resolve", and one resolve
/// answers any number of changes — so a second one queued behind the first is
/// pure cost: a mutex, an event-loop wake and a `resolve_and_repaint` preamble
/// that finds nothing to do. It used to be queued once per signal flush, which
/// a handler writing *n* signals outside a batch turned into *n* events.
///
/// `rerender_pending` is cleared by [`Self::drain`] **under the same lock** that
/// empties the queue: a `ReRender` sent after the drain (from an effect the
/// drained events run, or from another thread) finds the flag down and queues
/// afresh, and one sent before it is already in the drained batch. Clearing the
/// flag outside the lock would leave a window in which a send saw it up,
/// queued nothing, and then had its wake-up discarded.
///
/// Each event carries the window it is for ([`EventFor`], #2416): the document
/// whose code sent it, or a window a [`WindowHandle`](crate::WindowHandle)
/// names. The runtime resolves it to one of its windows
/// (`RinchRuntime::role_for`), which is how `hide_current_window` from the
/// DevTools window's code is DevTools'.
///
/// What will drain it ([`Drainer`]) lives under the same lock as the events,
/// so a send racing the runtime's construction on another thread (an exit
/// from a signal-watching thread, #3015) lands on one side of it: kept for
/// the runtime, or queued to it, never lost between the two.
pub(crate) struct NativeEventQueue {
    pub(super) events: VecDeque<(EventFor, RinchNativeEvent)>,
    rerender_pending: bool,
    drainer: Drainer,
    /// Whether a request past [`KEPT_BEFORE_RUN`] was dropped and warned of.
    overflowed: bool,
}

impl NativeEventQueue {
    /// An empty queue a runtime drains, for the unit tests of what it does
    /// with the events.
    #[cfg(test)]
    pub(crate) const fn new() -> Self {
        Self::with_drainer(Drainer::Runtime)
    }

    pub(super) const fn with_drainer(drainer: Drainer) -> Self {
        Self {
            events: VecDeque::new(),
            rerender_pending: false,
            drainer,
            overflowed: false,
        }
    }

    /// Queue `event` for the window `to` names. Returns whether it was
    /// added, and so whether a wake is owed: `false` for a `ReRender` that
    /// coalesced into one already queued, and for an event nothing will
    /// drain (`Drainer::EmbedHost`, or past [`KEPT_BEFORE_RUN`] before a
    /// runtime, or a second exit kept before one).
    pub(crate) fn push(&mut self, to: EventFor, event: RinchNativeEvent) -> bool {
        match self.drainer {
            Drainer::Runtime => {}
            #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
            Drainer::EmbedHost => {
                tracing::debug!("no desktop runtime to take {event:?}: dropped");
                return false;
            }
            Drainer::Coming => {
                if matches!(event, RinchNativeEvent::Exit) {
                    if self.holds_exit() {
                        return false;
                    }
                } else if self.events.len() >= KEPT_BEFORE_RUN {
                    if !self.overflowed {
                        self.overflowed = true;
                        tracing::warn!(
                            "{KEPT_BEFORE_RUN} window requests are waiting for a desktop \
                             runtime that has not been made (App::run): later ones are dropped"
                        );
                    }
                    return false;
                }
            }
        }
        if matches!(event, RinchNativeEvent::ReRender) {
            if self.rerender_pending {
                return false;
            }
            self.rerender_pending = true;
        }
        self.events.push_back((to, event));
        true
    }

    /// Take every queued event with the window it is for, and let the next
    /// `ReRender` queue again.
    pub(crate) fn drain(&mut self) -> Vec<(EventFor, RinchNativeEvent)> {
        self.rerender_pending = false;
        self.events.drain(..).collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn holds_exit(&self) -> bool {
        self.events
            .iter()
            .any(|(_, e)| matches!(e, RinchNativeEvent::Exit))
    }

    fn clear(&mut self) {
        self.events.clear();
        self.rerender_pending = false;
    }

    /// A desktop runtime exists: from now on everything is queued for it,
    /// and what was kept before it waits for its first wake.
    pub(super) fn note_runtime(&mut self) {
        self.drainer = Drainer::Runtime;
    }

    /// A `RinchContext` was made. With no runtime made, the process is an
    /// embed host: what was kept for a runtime is dropped, and so is every
    /// later request from outside a context.
    #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
    pub(super) fn note_embed_host(&mut self) {
        if self.drainer == Drainer::Coming {
            if !self.events.is_empty() {
                tracing::debug!(
                    "an embedded RinchContext and no desktop runtime: {} kept request(s) dropped",
                    self.events.len()
                );
            }
            self.drainer = Drainer::EmbedHost;
            self.clear();
        }
    }

    /// Whether an exit is waiting for a run that has not begun. When one is,
    /// the queue is emptied: the run it was for ends before it starts, and
    /// nothing else queued for it has a window to reach.
    pub(super) fn take_exit(&mut self) -> bool {
        let exit = self.holds_exit();
        if exit {
            self.clear();
        }
        exit
    }

    #[cfg(test)]
    pub(super) fn drainer(&self) -> Drainer {
        self.drainer
    }
}

/// Which window a queued native event is for (#2416).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventFor {
    /// The window of the document whose code sent it, `None` from outside any
    /// document (`main`, a worker thread, a menu callback): the main window.
    Sender(Option<u64>),
    /// The window a [`WindowHandle`](crate::WindowHandle) names, by its id.
    /// For nothing once that window has closed.
    Window(u64),
}

#[cfg(test)]
mod tests;
