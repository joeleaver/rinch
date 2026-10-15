//! The per-platform backends, started once per process on the first `show`.

use std::sync::OnceLock;

use super::{Backend, NotificationError};

#[cfg(target_os = "linux")]
pub(crate) mod freedesktop;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, test))]
pub(crate) mod toast_xml;
#[cfg(windows)]
mod windows;

static BACKEND: OnceLock<Result<Box<dyn Backend>, NotificationError>> = OnceLock::new();

fn start() -> Result<Box<dyn Backend>, NotificationError> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(freedesktop::FreedesktopBackend::start(
            None,
            super::main_thread_sink(),
        )?))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(windows::ToastBackend::start(
            super::main_thread_sink(),
        )?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::UserNotificationsBackend::start(
            super::main_thread_sink(),
        )?))
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        Err(NotificationError::Unsupported(format!(
            "no notification backend for {}",
            std::env::consts::OS
        )))
    }
}

/// The process's backend, started on first use.
pub(crate) fn current() -> Result<&'static dyn Backend, NotificationError> {
    match BACKEND.get_or_init(start) {
        Ok(b) => Ok(b.as_ref()),
        Err(e) => Err(e.clone()),
    }
}

/// The backend if a `show` has started one (a `close` starts nothing).
pub(crate) fn current_if_started() -> Option<&'static dyn Backend> {
    match BACKEND.get() {
        Some(Ok(b)) => Some(b.as_ref()),
        _ => None,
    }
}
