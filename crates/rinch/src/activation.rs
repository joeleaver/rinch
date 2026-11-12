//! xdg-activation tokens (#2501): what an app that starts another program
//! hands it, so that the program's first window gets the focus. Re-exported
//! through [`crate::windows`](mod@crate::windows).
//!
//! On Wayland a compositor keeps a new window from taking the focus unless
//! it brings a token (KWin's focus-stealing prevention; wlroots compositors
//! mark it urgent instead). The starting app asks the compositor for a token
//! for its own window, tied to the user action that asked (a key press, or
//! a pointer button's press or release)
//! (`request_activation_token`), and passes it to the child: in
//! `XDG_ACTIVATION_TOKEN` in its environment, or as the `activation-token`
//! platform data of a D-Bus `Activate` or `Open`. The child's toolkit
//! activates its first window with it.
//!
//! # How a request travels
//!
//! 1. `request_activation_token` parks the callback on the UI thread
//!    ([`rinch_core::park_main_callback`]), under the ambient owner and the
//!    asking document, and queues a native event that carries only the
//!    callback's id and the document. A request outside a user action is
//!    refused here.
//! 2. The shell gives the event to the asking document's window, which asks
//!    winit for a token. winit names the window's surface and the seat and
//!    serial of the latest input the window received, and answers with an
//!    `AsyncRequestSerial`. The pending request is kept by that serial
//!    (`shell::activation_tokens`), not by the window: hiding a window
//!    re-creates it with a new id.
//! 3. winit's `WindowEvent::ActivationTokenDone` comes back with the serial
//!    and the token, for whatever window id the request had, even one
//!    already dropped. The shell finds the request by serial and resumes the
//!    callback.

use rinch_core::{MainCallbackId, park_main_callback, resume_main_callback};

/// A token from [`request_activation_token`]: give it to the program being
/// started, in `XDG_ACTIVATION_TOKEN` or as the `activation-token` platform
/// data of a D-Bus activation. One token is for one start.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivationToken(String);

impl ActivationToken {
    /// The token, as the child is given it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The token as an owned string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn new(token: String) -> Self {
        Self(token)
    }
}

/// Why [`request_activation_token`] has no token to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActivationTokenError {
    /// There is no xdg-activation here: not Wayland (X11, Windows, macOS,
    /// Android), or a compositor without `xdg_activation_v1`.
    Unsupported,
    /// The request was not made from a user action, a key press or a pointer
    /// button's press or release: a token stands for the user's asking, and
    /// the compositor checks the input it names. Ask from inside the handler
    /// of the action that starts the program (`onclick`, `onkeydown`,
    /// `onmouseup`), not from a timer, an effect, a move, a later turn or
    /// another thread.
    NotFromInput,
    /// The window that asked was hidden or closed before the compositor
    /// answered, or was already hidden when it asked: its surface is gone,
    /// and a compositor need not answer for a surface it no longer has.
    /// Hide after the callback has run.
    WindowClosed,
}

impl std::fmt::Display for ActivationTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "xdg-activation is not available here",
            Self::NotFromInput => {
                "an activation token was not requested from a key press or a pointer button"
            }
            Self::WindowClosed => "the window was hidden or closed before the compositor answered",
        })
    }
}

impl std::error::Error for ActivationTokenError {}

/// What a request's callback is given.
pub type ActivationTokenResult = Result<ActivationToken, ActivationTokenError>;

/// Ask the compositor for an xdg-activation token for the current window,
/// for a program this app is about to start.
///
/// Call it from the handler of the user action that starts the program: a
/// key press (`onkeydown` on Enter), or a pointer button's press or release
/// (`onclick`, which rinch fires on the release for an element with a
/// pending drag; `onmouseup`). Call it on the UI thread. The token is tied to
/// that action: the compositor is told the window's surface and the seat
/// and serial of the latest input the window received, which is the key
/// press, or the button's press (winit notes a button's press, not its
/// release, since some compositors accept only the press's serial). The
/// current window is the window of the document whose code calls this.
///
/// `callback` runs on the UI thread, in a later turn of the event loop, with
/// the token or the reason there is none ([`ActivationTokenError`]). It is
/// called exactly once, with one exception: like a timer, a callback
/// registered during a component's render or handler is never called if
/// that component is unmounted first. It runs under the document that asked
/// and with the asking component as its owner, untracked, as other app
/// callbacks do.
///
/// # Hide after the callback
///
/// An app that hides once it has started something (a launcher) hides in
/// the callback, after the start, not when it asks:
///
/// - **winit still delivers the answer after the window is dropped**, under
///   the id the window had, which a newer window can have by then. The shell
///   keeps the request by winit's request serial, so the id is never trusted.
/// - **Whether the compositor answers at all once the window's surface is
///   gone is the compositor's choice**: it may send a token that grants no
///   focus, or never answer. So a request whose window is hidden or closed
///   before the answer ends then, with
///   [`ActivationTokenError::WindowClosed`], and an answer that comes later
///   is dropped. (Hiding re-creates the window, and moving a layer surface
///   to another output hides and shows it, so either ends the request.)
///
/// # Errors
///
/// Through `callback`:
///
/// - [`Unsupported`](ActivationTokenError::Unsupported) off Wayland or on a
///   compositor without `xdg_activation_v1`;
/// - [`NotFromInput`](ActivationTokenError::NotFromInput) outside a user
///   action (or off the UI thread, where the callback is called at
///   once, on that thread);
/// - [`WindowClosed`](ActivationTokenError::WindowClosed) when the window
///   is hidden or closed before the answer.
///
/// No error says the compositor will refuse the token: the protocol always
/// answers with a token, and what it is worth is decided when the child
/// uses it.
///
/// # Example
///
/// ```ignore
/// use rinch::windows::request_activation_token;
///
/// div { onkeydown: move |k| {
///     if k.key != "Enter" { return false; }
///     request_activation_token(move |token| {
///         let mut child = std::process::Command::new("foot");
///         if let Ok(token) = &token {
///             child.env("XDG_ACTIVATION_TOKEN", token.as_str());
///         }
///         let _ = child.spawn();
///         hide_current_window();
///     });
///     true
/// } }
/// ```
pub fn request_activation_token(callback: impl FnOnce(ActivationTokenResult) + 'static) {
    if !rinch_core::reactive::is_main_thread() {
        callback(Err(ActivationTokenError::NotFromInput));
        return;
    }
    let request = park_main_callback(move |result: ActivationTokenResult| {
        rinch_core::untracked_handler(|| callback(result));
    });
    if !crate::app::user_input::is_active() {
        answer(request, Err(ActivationTokenError::NotFromInput));
        return;
    }
    ask(request);
}

/// Send `request` to the shell, which asks the current window's compositor.
#[cfg(feature = "desktop")]
fn ask(request: MainCallbackId) {
    use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};
    send_native_event(RinchNativeEvent::RequestActivationToken(request));
}

/// No windowing system here has xdg-activation (Android, a native embed).
#[cfg(not(feature = "desktop"))]
fn ask(request: MainCallbackId) {
    answer(request, Err(ActivationTokenError::Unsupported));
}

/// Answer `request` with `result`: its callback runs in a later turn of the
/// event loop, never inside the code that answers (a native event's
/// handling, a window's hiding).
pub(crate) fn answer(request: MainCallbackId, result: ActivationTokenResult) {
    let resume = move || resume_main_callback(request, result);
    #[cfg(feature = "desktop")]
    crate::shell::rinch_runtime::run_on_main_thread(resume);
    #[cfg(not(feature = "desktop"))]
    rinch_core::queue_main_callback(Box::new(resume));
}
