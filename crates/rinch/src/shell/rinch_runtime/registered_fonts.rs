//! The faces `rinch::font::register_font` registers while the app runs
//! (#3042): queued on the UI thread for whatever will register them with a
//! window, a desktop runtime or a `TestApp`.

use super::*;
use crate::font::AppFont;

thread_local! {
    /// Faces [`queue_font`] queued, in order, for the runtime's next turn
    /// (or the runtime to come, or a `TestApp`).
    static QUEUED: RefCell<Vec<AppFont>> = const { RefCell::new(Vec::new()) };
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
