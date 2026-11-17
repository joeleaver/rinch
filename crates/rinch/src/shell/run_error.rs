//! How a desktop app's event loop ended other than normally (#2531), or
//! could not start (#2761): [`RunError`], answered by
//! [`App::try_run`](crate::App::try_run).

use winit::error::EventLoopError;

/// Why [`App::try_run`](crate::App::try_run) ended with an error instead of
/// the app closing.
///
/// Either way the app's state has been dropped by then, its windows closed
/// and its documents disposed, as at an ordinary exit; nothing of the event
/// loop is left to clean up. An error found while the event loop is being
/// built comes before any of that exists.
///
/// The rule between the first two variants is whether a connection to a
/// display server was ever made: [`NoDisplay`](RunError::NoDisplay) when none
/// was (there was nothing to connect to), [`DisplayLost`](RunError::DisplayLost)
/// when one was made and then broke, at startup or later. A desktop
/// component restarts after the second; the first is no proof that no
/// display will come (see [`NoDisplay`](RunError::NoDisplay)), so it retries
/// that more slowly or a bounded number of times.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// The connection to the display server failed: the Wayland compositor
    /// went away (it exited, crashed or ended the session) or dropped the
    /// app's connection. Ordinary for a desktop component at logout or a
    /// compositor restart, rather than a fault of the app. The text is what
    /// the windowing system said.
    ///
    /// This covers the whole run, startup included: the compositor going away
    /// while the app runs, and also after the app connected to it but before
    /// the event loop ran (the connection reset or its pipe broken while the
    /// loop was being built, #2761).
    ///
    /// An X server going away does not end up here: Xlib's I/O error handler
    /// exits the process (status 1) before winit returns. winit's X11 backend
    /// returns this only if its own poll of the connection fails.
    DisplayLost(String),
    /// There was no display server to connect to when the app started, so no
    /// event loop was made (#2761): no `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` or
    /// `DISPLAY` set; a `WAYLAND_DISPLAY` (or `WAYLAND_SOCKET`) that names no
    /// compositor (a stale variable, a compositor not started yet, or one that
    /// had already finished exiting before rinch connected: wayland-client
    /// reports every failed connect as "no compositor", so a compositor
    /// restart can answer this as well as [`DisplayLost`](RunError::DisplayLost));
    /// a `DISPLAY` that names no X server that would take the connection. With [`App::prefer_x11`](crate::App::prefer_x11) it means
    /// neither backend had one. Only on Linux; elsewhere a loop that cannot be
    /// built is [`EventLoop`](RunError::EventLoop). The text is what the
    /// windowing system said.
    NoDisplay(String),
    /// Any other error the windowing system's event loop ended with, or was
    /// built with: a library that would not load, a compositor without a
    /// protocol winit needs, a second event loop in one process.
    EventLoop(String),
}

impl RunError {
    /// Whether the display server went away ([`RunError::DisplayLost`]).
    pub fn is_display_lost(&self) -> bool {
        matches!(self, Self::DisplayLost(_))
    }

    /// Whether there was no display server to connect to when the app
    /// started ([`RunError::NoDisplay`]).
    pub fn is_no_display(&self) -> bool {
        matches!(self, Self::NoDisplay(_))
    }

    /// The error building the event loop failed with (#2761), as rinch reads
    /// it. `display_named` is whether the environment names a display at all
    /// (`WAYLAND_DISPLAY`, `WAYLAND_SOCKET` or `DISPLAY`, not empty), read by
    /// [`display_named`] where the loop is built.
    ///
    /// On Linux, walking the error's sources:
    /// - winit's "not supported" with no display named: winit found nothing
    ///   to connect to ([`NoDisplay`](Self::NoDisplay)). With one named it is
    ///   something else (an X server without an extension winit needs).
    /// - Wayland's connect error, no compositor at the socket or a
    ///   `WAYLAND_SOCKET` that is not one: [`NoDisplay`](Self::NoDisplay).
    /// - Xlib's `XOpenDisplay` failing (no X server takes the connection),
    ///   read from its text, as winit keeps its error type private:
    ///   [`NoDisplay`](Self::NoDisplay).
    /// - An I/O error of a connection that broke (reset, broken pipe,
    ///   aborted, end of file), bare or inside Wayland's error, so after a
    ///   connection was made: [`DisplayLost`](Self::DisplayLost).
    /// - Anything else: [`EventLoop`](Self::EventLoop).
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    pub(crate) fn from_event_loop_start(error: EventLoopError, display_named: bool) -> Self {
        #[cfg(target_os = "linux")]
        if let Some(classified) = linux_start::classify(&error, display_named) {
            return classified;
        }
        Self::EventLoop(error.to_string())
    }

    /// The error the event loop ended with, as rinch reads it. winit's
    /// Wayland and X11 backends end the loop with an exit failure in one
    /// case only: the connection's flush or its dispatch failed (a broken
    /// pipe once the compositor is gone, a protocol error), so on Linux that
    /// is the display going away. Elsewhere rinch sets no failure code of its
    /// own, so one is reported as it is.
    pub(crate) fn from_event_loop(error: EventLoopError) -> Self {
        match error {
            #[cfg(target_os = "linux")]
            EventLoopError::ExitFailure(code) => Self::DisplayLost(format!(
                "the event loop's connection failed (exit code {code})"
            )),
            other => Self::EventLoop(other.to_string()),
        }
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisplayLost(detail) => write!(
                f,
                "the display server went away (the compositor exited or dropped the \
                 connection): {detail}"
            ),
            Self::NoDisplay(detail) => write!(
                f,
                "no display server to connect to (no Wayland compositor or X server \
                 took the connection): {detail}"
            ),
            Self::EventLoop(detail) => write!(f, "the event loop failed: {detail}"),
        }
    }
}

impl std::error::Error for RunError {}

/// The end of [`App::run`](crate::App::run) and the deprecated `run_*`
/// functions, which return nothing: an error is logged in one line, with no
/// backtrace, and the process exits with status 1. The app's state is already
/// dropped (the event loop dropped it before returning), so this is the
/// shutdown an ordinary exit has, with a status that says it failed.
/// `std::process::exit` does not unwind: `main`'s locals are not dropped
/// (`App::run`'s docs say so, and point at `try_run`).
///
/// The line goes through `tracing`: rinch's own subscriber drops a failed
/// write (`log_internal_errors(false)`, `logging.rs`), so a closed stderr
/// (a session that ended) cannot turn it into a panic, as `eprintln!` would.
pub(crate) fn exit_on_run_error(result: Result<(), RunError>) {
    if let Err(error) = result {
        tracing::error!("rinch: {error}; exiting");
        std::process::exit(1);
    }
}

/// What the event loop's result becomes: `Ok` stays `Ok`, an error becomes a
/// [`RunError`]. Never a panic: it used to be `.expect("Event loop error")`,
/// so a compositor going away panicked every app (#2531).
pub(crate) fn run_result(result: Result<(), EventLoopError>) -> Result<(), RunError> {
    result.map_err(RunError::from_event_loop)
}

/// Whether the environment names a display for winit to connect to: a
/// `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` or `DISPLAY` that is set and not
/// empty, the variables winit reads to choose a backend.
pub(crate) fn display_named() -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// [`RunError::from_event_loop_start`]'s reading of winit's Linux backends.
#[cfg(target_os = "linux")]
mod linux_start {
    use super::RunError;
    use std::error::Error;
    use std::io::ErrorKind;
    use wayland_client::ConnectError;
    use wayland_client::backend::WaylandError;
    use winit::error::EventLoopError;

    /// What winit-x11's private `XNotSupported::XOpenDisplayFailed` prints.
    /// The `stale DISPLAY` case of `tests/start_without_display_2761.rs`
    /// builds a real loop against it, so a winit that words it differently
    /// fails there.
    const X_OPEN_DISPLAY_FAILED: &str = "Failed to open connection to X server";

    pub(super) fn classify(error: &EventLoopError, display_named: bool) -> Option<RunError> {
        let detail = || error.to_string();
        match error {
            EventLoopError::NotSupported(_) if !display_named => {
                return Some(RunError::NoDisplay(detail()));
            }
            EventLoopError::Os(os) => {
                // `WaylandError` has no `source` (only the deprecated
                // `cause`), so it is read where it is met.
                let mut next: Option<&(dyn Error + 'static)> = os.source();
                while let Some(e) = next {
                    if let Some(c) = e.downcast_ref::<ConnectError>() {
                        if matches!(c, ConnectError::NoCompositor | ConnectError::InvalidFd) {
                            return Some(RunError::NoDisplay(detail()));
                        }
                    }
                    if e.to_string() == X_OPEN_DISPLAY_FAILED {
                        return Some(RunError::NoDisplay(detail()));
                    }
                    let io = e.downcast_ref::<std::io::Error>().or(
                        match e.downcast_ref::<WaylandError>() {
                            Some(WaylandError::Io(io)) => Some(io),
                            _ => None,
                        },
                    );
                    if io.is_some_and(|io| connection_broke(io.kind())) {
                        return Some(RunError::DisplayLost(detail()));
                    }
                    next = e.source();
                }
            }
            _ => {}
        }
        None
    }

    /// An I/O error that says a connection was there and broke.
    fn connection_broke(kind: ErrorKind) -> bool {
        matches!(
            kind,
            ErrorKind::ConnectionReset
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionAborted
                | ErrorKind::UnexpectedEof
                | ErrorKind::NotConnected
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What winit's Wayland backend returns when the compositor goes away:
    /// `ExitFailure(1)` from the failed flush, or the dispatch error's errno
    /// (`EPIPE`, 32). Both are the display going away, and neither panics.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_connection_is_the_display_going_away() {
        for code in [1, 32] {
            let error = run_result(Err(EventLoopError::ExitFailure(code))).unwrap_err();
            assert!(error.is_display_lost(), "{error:?}");
            assert!(error.to_string().contains("display server went away"));
            assert!(error.to_string().contains(&format!("exit code {code}")));
        }
    }

    #[test]
    fn another_error_is_the_event_loop_failing() {
        let error = run_result(Err(EventLoopError::RecreationAttempt)).unwrap_err();
        assert!(!error.is_display_lost());
        assert!(matches!(error, RunError::EventLoop(_)), "{error:?}");
    }

    #[test]
    fn a_normal_end_is_ok() {
        assert_eq!(run_result(Ok(())), Ok(()));
    }

    fn os(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> EventLoopError {
        EventLoopError::Os(winit::error::OsError::new(1, "winit", error))
    }

    fn not_supported() -> EventLoopError {
        EventLoopError::NotSupported(winit::error::NotSupportedError::new(
            "neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set.",
        ))
    }

    /// Nothing named, or a compositor or X server that is not there: there
    /// was no display (#2761).
    #[cfg(target_os = "linux")]
    #[test]
    fn nothing_to_connect_to_is_no_display() {
        for (error, why) in [
            (not_supported(), "no display variables"),
            (
                os(wayland_client::ConnectError::NoCompositor),
                "no compositor",
            ),
            (
                os(wayland_client::ConnectError::InvalidFd),
                "garbage WAYLAND_SOCKET",
            ),
            (os("Failed to open connection to X server"), "no X server"),
        ] {
            let error = RunError::from_event_loop_start(error, false);
            assert!(error.is_no_display(), "{why}: {error:?}");
            assert!(!error.is_display_lost(), "{why}");
            assert!(
                error.to_string().contains("no display server"),
                "{why}: {error}"
            );
        }
    }

    /// A connection that was made and broke while the loop was built is the
    /// display going away, as it is once the loop runs: winit's registry
    /// roundtrip failing with `ECONNRESET` (what the issue saw) or a broken
    /// pipe, inside Wayland's error or bare.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_connection_that_broke_at_startup_is_display_lost() {
        use std::io::{Error, ErrorKind};
        use wayland_client::backend::WaylandError;
        for kind in [ErrorKind::ConnectionReset, ErrorKind::BrokenPipe] {
            let wrapped = os(wayland_client::globals::GlobalError::Backend(
                WaylandError::Io(Error::from(kind)),
            ));
            let error = RunError::from_event_loop_start(wrapped, true);
            assert!(error.is_display_lost(), "{kind:?}: {error:?}");
            let bare = RunError::from_event_loop_start(os(Error::from(kind)), true);
            assert!(bare.is_display_lost(), "{kind:?} bare: {bare:?}");
        }
    }

    /// Not a display question: the event loop failing.
    #[test]
    fn another_start_error_is_the_event_loop_failing() {
        let others = [
            // A display named, and still "not supported": an X server without
            // an extension winit needs.
            RunError::from_event_loop_start(not_supported(), true),
            RunError::from_event_loop_start(EventLoopError::RecreationAttempt, false),
            RunError::from_event_loop_start(
                os(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                true,
            ),
            RunError::from_event_loop_start(
                os("Failed to load one of xlib's shared libraries"),
                true,
            ),
        ];
        for error in others {
            assert!(matches!(error, RunError::EventLoop(_)), "{error:?}");
            assert!(!error.is_no_display() && !error.is_display_lost());
        }
    }
}
