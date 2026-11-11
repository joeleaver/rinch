//! The xdg-activation token requests the compositor has not answered yet
//! (#2501), kept by winit's request serial: see [`crate::activation`].
//!
//! Not by window: winit names the answer with the id of the window that
//! asked, even after that window was dropped, and a window made later can
//! have the same id (a Wayland window's id is its surface object's address).
//! The serial is unique for the life of the event loop. Each request also
//! remembers its window (`facts_key`: `None` the main window, `Some(doc)`
//! another), so that hiding or closing the window ends its requests.

use std::cell::RefCell;

use rinch_core::MainCallbackId;
use winit::event_loop::AsyncRequestSerial;

use crate::activation::{ActivationToken, ActivationTokenError, answer};

/// One unanswered request.
struct Pending {
    serial: AsyncRequestSerial,
    /// The window that asked, by its `facts_key`.
    window: Option<u64>,
    /// The parked callback to answer.
    request: MainCallbackId,
}

thread_local! {
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
}

/// Note that winit asked the compositor for `request`'s token under
/// `serial`, for the window `window`.
pub(crate) fn requested(serial: AsyncRequestSerial, window: Option<u64>, request: MainCallbackId) {
    PENDING.with(|p| {
        p.borrow_mut().push(Pending {
            serial,
            window,
            request,
        });
    });
}

/// The compositor's answer for `serial`: the request is answered with the
/// token. An answer for no pending request (one already ended because its
/// window went, or a request this app never made) is dropped.
pub(crate) fn done(serial: AsyncRequestSerial, token: String) {
    match take(|pending| pending.serial == serial).pop() {
        Some(request) => answer(request, Ok(ActivationToken::new(token))),
        None => tracing::debug!("activation token for no pending request ({serial:?}) dropped"),
    }
}

/// The window `window` was hidden or closed: its requests end with
/// [`ActivationTokenError::WindowClosed`], since the compositor need not
/// answer for a surface it no longer has.
pub(crate) fn window_gone(window: Option<u64>) {
    for request in take(|pending| pending.window == window) {
        answer(request, Err(ActivationTokenError::WindowClosed));
    }
}

/// Remove the pending requests `which` picks, in the order they were made.
fn take(which: impl Fn(&Pending) -> bool) -> Vec<MainCallbackId> {
    PENDING.with(|p| {
        let mut taken = Vec::new();
        p.borrow_mut().retain(|pending| {
            let take = which(pending);
            if take {
                taken.push(pending.request);
            }
            !take
        });
        taken
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::ActivationTokenResult;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Answers = Rc<RefCell<Vec<(u32, ActivationTokenResult)>>>;

    /// A request numbered `n` whose answers go to `answers`.
    fn park(answers: &Answers, n: u32) -> MainCallbackId {
        let answers = Rc::clone(answers);
        rinch_core::park_main_callback(move |result: ActivationTokenResult| {
            answers.borrow_mut().push((n, result));
        })
    }

    fn token(text: &str) -> ActivationTokenResult {
        Ok(ActivationToken::new(text.into()))
    }

    #[test]
    fn an_answer_finds_its_request_by_serial() {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let answers = Answers::default();
        let (first, second) = (AsyncRequestSerial::get(), AsyncRequestSerial::get());
        requested(first, None, park(&answers, 1));
        requested(second, None, park(&answers, 2));
        done(second, "two".into());
        done(first, "one".into());
        rinch_core::drain_main_callbacks();
        assert_eq!(*answers.borrow(), [(2, token("two")), (1, token("one"))]);
        // Answered once: the same serial again, or one never asked for, is
        // dropped.
        done(first, "again".into());
        done(AsyncRequestSerial::get(), "stranger".into());
        rinch_core::drain_main_callbacks();
        assert_eq!(answers.borrow().len(), 2);
    }

    #[test]
    fn a_window_that_goes_ends_its_own_requests_and_drops_their_late_answers() {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let answers = Answers::default();
        let (main, other) = (AsyncRequestSerial::get(), AsyncRequestSerial::get());
        requested(main, None, park(&answers, 1));
        requested(other, Some(7), park(&answers, 2));
        window_gone(None);
        rinch_core::drain_main_callbacks();
        assert_eq!(
            *answers.borrow(),
            [(1, Err(ActivationTokenError::WindowClosed))]
        );
        // The compositor answers after all: too late, dropped.
        done(main, "late".into());
        // The other window's request is untouched.
        done(other, "two".into());
        rinch_core::drain_main_callbacks();
        assert_eq!(
            *answers.borrow(),
            [
                (1, Err(ActivationTokenError::WindowClosed)),
                (2, token("two")),
            ]
        );
    }
}
