//! How a desktop app's event loop ended other than normally (#2531):
//! [`RunError`], answered by [`App::try_run`](crate::App::try_run).

use winit::error::EventLoopError;

/// Why [`App::try_run`](crate::App::try_run) ended with an error instead of
/// the app closing.
///
/// Either way the app's state has been dropped by then, its windows closed
/// and its documents disposed, as at an ordinary exit; nothing of the event
/// loop is left to clean up.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// The connection to the display server failed while the app ran: the
    /// Wayland compositor went away (it exited, crashed or ended the session)
    /// or dropped the app's connection. Ordinary for a desktop component at
    /// logout or a compositor restart, rather than a fault of the app. The
    /// text is what the windowing system said.
    ///
    /// An X server going away does not end up here: Xlib's I/O error handler
    /// exits the process (status 1) before winit returns. winit's X11 backend
    /// returns this only if its own poll of the connection fails.
    DisplayLost(String),
    /// Any other error the windowing system's event loop ended with.
    EventLoop(String),
}

impl RunError {
    /// Whether the display server went away ([`RunError::DisplayLost`]).
    pub fn is_display_lost(&self) -> bool {
        matches!(self, Self::DisplayLost(_))
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
}
