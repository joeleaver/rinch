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

    #[cfg(feature = "windowing")]
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

/// The environment variable a launcher hands a Wayland program its
/// xdg-activation token in.
const LAUNCH_TOKEN_VAR: &str = "XDG_ACTIVATION_TOKEN";

/// The xdg-activation token this process was started with, taken out of its
/// environment (#2817).
///
/// A launcher (a desktop's menu, a browser opening a link, a file manager)
/// starts a Wayland program with a token in `XDG_ACTIVATION_TOKEN`, tied to
/// the user action that started it. A single-instance app's second launch
/// takes it here and hands it to the instance that is already running, over
/// the app's own IPC, and that instance raises its window with it
/// ([`activate_current_window_with_token`](crate::windows::activate_current_window_with_token),
/// [`WindowHandle::activate_with_token`](crate::WindowHandle::activate_with_token)).
///
/// The variable is removed from the environment, as the xdg-activation
/// protocol asks, so that a program this one starts does not inherit a
/// token meant for this one. So it answers `Some` once per process; an
/// empty or non-UTF-8 value is `None` (and removed as well). It reads only
/// `XDG_ACTIVATION_TOKEN`: X11's startup notification (`DESKTOP_STARTUP_ID`)
/// is another mechanism, left where it is.
///
/// An app's first launch, which opens its own window, needs none of this:
/// its window is new, and a compositor gives a new window the focus when it
/// maps.
///
/// # Safety
///
/// It changes the process's environment ([`std::env::remove_var`]), so no
/// other thread may read or write the environment while it runs (in Rust
/// or in C: `getenv` in a library counts). Call it first thing in `main`,
/// before the app or any library starts a thread.
///
/// # Example
///
/// ```ignore
/// fn main() {
///     // SAFETY: nothing else runs yet.
///     let token = unsafe { rinch::windows::take_launch_activation_token() };
///     if let Some(running) = my_ipc::connect_to_running_instance() {
///         running.send_open(link, token); // it calls activate_current_window_with_token
///         return;
///     }
///     App::new(app).run();
/// }
/// ```
#[must_use]
pub unsafe fn take_launch_activation_token() -> Option<String> {
    let token = launch_token(std::env::var_os(LAUNCH_TOKEN_VAR));
    // SAFETY: the caller's: no other thread touches the environment now.
    unsafe { std::env::remove_var(LAUNCH_TOKEN_VAR) };
    token
}

/// The token a launch's `XDG_ACTIVATION_TOKEN` value stands for.
fn launch_token(value: Option<std::ffi::OsString>) -> Option<String> {
    value?.into_string().ok().filter(|token| !token.is_empty())
}

/// Send `request` to the shell, which asks the current window's compositor.
#[cfg(feature = "windowing")]
fn ask(request: MainCallbackId) {
    use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};
    send_native_event(RinchNativeEvent::RequestActivationToken(request));
}

/// No windowing system here has xdg-activation (Android, a native embed).
#[cfg(not(feature = "windowing"))]
fn ask(request: MainCallbackId) {
    answer(request, Err(ActivationTokenError::Unsupported));
}

/// Answer `request` with `result`: its callback runs in a later turn of the
/// event loop, never inside the code that answers (a native event's
/// handling, a window's hiding).
pub(crate) fn answer(request: MainCallbackId, result: ActivationTokenResult) {
    let resume = move || resume_main_callback(request, result);
    #[cfg(feature = "windowing")]
    crate::shell::rinch_runtime::run_on_main_thread(resume);
    #[cfg(not(feature = "windowing"))]
    rinch_core::queue_main_callback(Box::new(resume));
}

#[cfg(test)]
mod launch_token_tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_launch_token_is_the_variables_value_and_nothing_else_2817() {
        assert_eq!(
            launch_token(Some(OsString::from("kwin-4271"))).as_deref(),
            Some("kwin-4271")
        );
        assert_eq!(launch_token(None), None);
        assert_eq!(
            launch_token(Some(OsString::new())),
            None,
            "empty is no token"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let not_utf8 = OsString::from_vec(vec![0x66, 0xff, 0x6f]);
            assert_eq!(launch_token(Some(not_utf8)), None);
        }
    }
}
