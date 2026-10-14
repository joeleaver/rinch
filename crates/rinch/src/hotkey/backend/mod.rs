//! The per-platform hotkey backends behind [`super::register`].
//!
//! A backend lives on the main thread (in `super::BACKEND`) and owns whatever
//! threads it needs. Every press, release and binding change goes to the
//! main thread through [`super::report`].

use super::matcher::BindingKey;
use super::{Backend, Binding, HotkeyError, Meta, Permission, Shortcut};

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(test)]
pub(crate) mod fake;

pub(crate) trait BackendImpl {
    fn kind(&self) -> Backend;

    /// Bind `key` to `shortcut` (`None`: the system chooses). On an error
    /// nothing is left registered under `key`.
    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        meta: &Meta,
    ) -> Result<Binding, HotkeyError>;

    /// Unbind `key`. A key that is not bound is ignored.
    fn unregister(&mut self, key: BindingKey);

    fn permission(&self) -> Permission {
        Permission::NotNeeded
    }

    fn request_permission(&mut self) -> Permission {
        self.permission()
    }
}

/// The backend this session would use, without starting it.
pub(crate) fn choose() -> Backend {
    #[cfg(test)]
    if fake::installed() {
        return Backend::Unsupported;
    }
    #[cfg(target_os = "windows")]
    {
        Backend::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Backend::MacOs
    }
    #[cfg(target_os = "linux")]
    {
        linux_choice(
            std::env::var("RINCH_HOTKEY_BACKEND").ok().as_deref(),
            std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland"),
            std::env::var_os("DISPLAY").is_some(),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Backend::Unsupported
    }
}

/// Linux: the portal in a Wayland session — an X11 grab there sees only keys
/// typed into XWayland windows — and X11 otherwise. `RINCH_HOTKEY_BACKEND`
/// (`x11` or `portal`) overrides.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn linux_choice(forced: Option<&str>, wayland: bool, x11: bool) -> Backend {
    match forced.map(str::to_ascii_lowercase).as_deref() {
        Some("x11") => return Backend::X11,
        Some("portal" | "wayland") => return Backend::WaylandPortal,
        _ => {}
    }
    if wayland {
        Backend::WaylandPortal
    } else if x11 {
        Backend::X11
    } else {
        Backend::Unsupported
    }
}

/// Start the backend [`choose`] names.
pub(crate) fn create() -> Result<Box<dyn BackendImpl>, HotkeyError> {
    #[cfg(test)]
    if let Some(b) = fake::take() {
        return Ok(b);
    }
    match choose() {
        #[cfg(target_os = "windows")]
        Backend::Windows => Ok(Box::new(windows::WinBackend::start()?)),
        #[cfg(target_os = "macos")]
        Backend::MacOs => Ok(Box::new(macos::MacBackend::new())),
        #[cfg(target_os = "linux")]
        Backend::X11 => Ok(Box::new(x11::X11Backend::start()?)),
        #[cfg(target_os = "linux")]
        Backend::WaylandPortal => Ok(Box::new(portal::PortalBackend::start()?)),
        other => Err(HotkeyError::Unsupported(format!(
            "no global-hotkey backend for this platform or session ({other:?})"
        ))),
    }
}
