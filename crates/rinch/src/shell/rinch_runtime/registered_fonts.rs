//! The faces `rinch::font::register_font` registers while the app runs
//! (#3042): queued on the UI thread for whatever will register them with a
//! window, a desktop runtime or a `TestApp`.

use super::*;
use crate::font::AppFont;
use std::cell::Cell;

thread_local! {
    /// Faces [`queue_font`] queued, in order, for the runtime's next turn
    /// (or the runtime to come, or a `TestApp`).
    static QUEUED: RefCell<Vec<AppFont>> = const { RefCell::new(Vec::new()) };

    /// How many faces [`drop_queued`] dropped since a desktop runtime was
    /// last made, for [`warn_dropped`] to report if one is made after all.
    static DROPPED: Cell<usize> = const { Cell::new(0) };
}

/// Queue `font` for whatever will register it with a window, and wake that.
/// Called by `rinch::font::register_font`, on the UI thread.
///
/// - A `TestApp` on this thread: queued, and the first face of a run sends
///   it `RegisterFonts`, which it serves after its turn.
/// - A desktop runtime running ([`Drainer::Runtime`]): queued, and the first
///   face of a run sends `RegisterFonts`, so a run of registrations costs one
///   event and one turn registers all of them.
/// - No runtime yet ([`Drainer::Coming`], an app's `main`): queued and no
///   event sent. `RinchRuntime::with_app` takes the queue, so nothing has to
///   wake for it, and calls in `main` keep none of the queue's
///   `KEPT_BEFORE_RUN` places.
/// - An embed host (`Drainer::EmbedHost`): dropped. Nothing would ever
///   take it; the host registers faces with `RinchContext::register_app_font`.
///   Faces queued before the process turned out to be one are dropped when
///   it does ([`drop_queued`]), as the window requests kept for a runtime
///   are.
///
/// Whatever the event's fate, a runtime's every turn and a `TestApp`'s
/// every serve take the queue: the event only wakes them, so a lost one
/// leaves the faces for the next turn, never stuck behind a `first` that
/// will not come again.
///
/// A face identical to one already waiting is not queued twice
/// ([`AppFont::is_same_face`]).
pub(crate) fn queue_font(font: AppFont) {
    #[cfg(feature = "testing")]
    let test_app = crate::testing::has_test_app();
    #[cfg(not(feature = "testing"))]
    let test_app = false;
    queue_font_for(native_event_drainer(), test_app, font);
}

/// [`queue_font`] where `drainer` will drain the native event queue and
/// `test_app` says whether a `TestApp` on this thread takes its events.
pub(super) fn queue_font_for(drainer: Drainer, test_app: bool, font: AppFont) {
    let wake = match drainer {
        _ if test_app => true,
        Drainer::Runtime => true,
        Drainer::Coming => false,
        #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
        Drainer::EmbedHost => {
            tracing::debug!(
                "rinch::font::register_font in an embed host: ignored \
                 (register with RinchContext::register_app_font)"
            );
            return;
        }
    };
    let first = QUEUED.with(|queued| {
        let mut queued = queued.borrow_mut();
        if queued.iter().any(|q| q.is_same_face(&font)) {
            return false;
        }
        queued.push(font);
        queued.len() == 1
    });
    // A face queued behind another is registered by the event the first
    // one sent.
    if first && wake {
        send_native_event(RinchNativeEvent::RegisterFonts);
    }
}

/// Take the queued faces that `kept` does not already hold
/// ([`AppFont::is_same_face`]), append them to `kept`, and return them, in
/// order. `kept` is what the windows were made with: a runtime's
/// `RinchRuntime::fonts`, a `TestApp`'s fonts.
pub(crate) fn take_new_fonts(kept: &mut Vec<AppFont>) -> Vec<AppFont> {
    let queued = QUEUED.with(|queued| std::mem::take(&mut *queued.borrow_mut()));
    let mut new = Vec::new();
    for font in queued {
        if !kept.iter().any(|k| k.is_same_face(&font)) {
            kept.push(font.clone());
            new.push(font);
        }
    }
    new
}

/// Drop the faces queued on this thread: the process became an embed host
/// (#2801), whose own loop takes nothing from the queue, so they would stay
/// for the life of the thread. Called where the native event queue's
/// drainer becomes [`Drainer::EmbedHost`], on the thread that made the
/// `RinchContext` or ended the run, which is the UI thread `register_font`
/// queues on.
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
pub(super) fn drop_queued() {
    let dropped = QUEUED.with(|queued| std::mem::take(&mut *queued.borrow_mut()));
    if !dropped.is_empty() {
        DROPPED.with(|n| n.set(n.get() + dropped.len()));
        tracing::debug!(
            "an embed host and no desktop runtime: {} queued font face(s) dropped \
             (register with RinchContext::register_app_font)",
            dropped.len()
        );
    }
}

/// Warn (and return how many) if faces `register_font` queued were dropped because the process
/// looked like an embed host, now that a desktop runtime is being made: an
/// app that makes a `RinchContext` in `main` before `App::run` loses the
/// faces it registered up to then (#3042). Called by `RinchRuntime::with_app`
/// on the UI thread. A plain embed host never makes a runtime, so it never
/// reaches this and stays quiet (the drop itself is a debug log).
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
pub(super) fn warn_dropped() -> usize {
    let n = DROPPED.with(|n| n.replace(0));
    if n > 0 {
        tracing::warn!(
            "{n} font face(s) passed to rinch::font::register_font were dropped: they were \
             registered while a RinchContext existed and no desktop runtime had started yet, \
             which is an embed host. Pass them to App::fonts, or register them once the app \
             is running"
        );
    }
    n
}

impl RinchRuntime {
    /// Register the faces `rinch::font::register_font` queued with every open
    /// window's document, and keep them with the app's fonts so a window
    /// opened later is made with them. Each registration names its
    /// document's root dirty (#2749), so the next frame re-shapes and repaints
    /// text drawn in a fallback; the redraw asks for that frame.
    pub(super) fn register_queued_fonts(&mut self) {
        let fonts = take_new_fonts(&mut self.fonts);
        if fonts.is_empty() {
            return;
        }
        for role in self.roles() {
            let window = self.w(role);
            window.app.register_app_fonts(&fonts);
            if let Some(os_window) = window.window.as_ref() {
                os_window.request_redraw();
            }
        }
    }
}
