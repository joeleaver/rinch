//! Which window roles the display offers, known before any window is opened
//! (issue #2659): [`window_roles`], re-exported through `rinch::window`.

// Only the desktop shell publishes; without it the decision and the
// encoding are reached from the tests alone.
#![cfg_attr(not(feature = "desktop"), allow(dead_code))]

use std::sync::atomic::{AtomicU8, Ordering};

/// The window roles the display offers ([`window_roles`]). More fields join
/// as rinch learns more roles (a popup's, #2552), so it is
/// `#[non_exhaustive]`: read its fields, do not build it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct WindowRoles {
    /// A window asked for a layer surface
    /// ([`WindowProps::wayland_layer`](rinch_core::element::WindowProps::wayland_layer),
    /// an overlay shorthand) gets one: the Wayland compositor offers
    /// `zwlr_layer_shell_v1`. Where this is `false` such a window is an
    /// ordinary toplevel (Wayland), or the platform's ordinary window.
    pub layer_shell: bool,
}

impl WindowRoles {
    /// No role beyond an ordinary window: X11, Windows, macOS, Android, the
    /// web.
    pub(crate) const NONE: WindowRoles = WindowRoles { layer_shell: false };

    /// What a Wayland compositor offers, from the interfaces its registry
    /// advertises.
    pub(crate) fn of_wayland_interfaces<'a>(
        interfaces: impl IntoIterator<Item = &'a str>,
    ) -> WindowRoles {
        let mut roles = WindowRoles::NONE;
        for interface in interfaces {
            if interface == "zwlr_layer_shell_v1" {
                roles.layer_shell = true;
            }
        }
        roles
    }

    fn encode(self) -> u8 {
        KNOWN | if self.layer_shell { LAYER_SHELL } else { 0 }
    }

    fn decode(bits: u8) -> Option<WindowRoles> {
        (bits & KNOWN != 0).then_some(WindowRoles {
            layer_shell: bits & LAYER_SHELL != 0,
        })
    }
}

const KNOWN: u8 = 1 << 7;
const LAYER_SHELL: u8 = 1;

/// What the shell read when its event loop started; `0` before.
static PUBLISHED: AtomicU8 = AtomicU8::new(0);

/// The window roles this display offers, or `None` while that is not known
/// yet. Callable before any window is opened, from `App::windowless`'s root
/// or the main window's first render on.
///
/// A window asked for a Wayland layer surface
/// ([`WindowProps::wayland_layer`], an overlay shorthand) is an ordinary
/// toplevel where the compositor has no `wlr-layer-shell` (GNOME/Mutter).
/// For a panel that is a fair fallback; a wallpaper, a screen-edge overlay or
/// an OSD made an ordinary window covers or litters the desktop instead. Such
/// an app asks [`window_roles`] (or [`layer_shell_available`]) first and
/// opens nothing when the role is missing:
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[component]
/// fn wallpapers() -> NodeHandle {
///     if window_roles().is_none_or(|roles| !roles.layer_shell) {
///         eprintln!("this compositor has no layer shell; no wallpaper");
///         exit();
///     }
///     rsx! { div { /* a `Window` with a `Background` layer surface per output */ } }
/// }
///
/// fn main() {
///     App::windowless(wallpapers).run();
/// }
/// ```
///
/// # Where the answer comes from
///
/// - **Wayland**: the compositor's registry. `layer_shell` is whether it
///   advertises `zwlr_layer_shell_v1`, read when the event loop starts, before
///   the main window is made and before `App::windowless`'s root runs, so both
///   can read it.
/// - **X11** (Xwayland included), **Windows**, **macOS**, **Android**, **the
///   web**: none of these roles exists, so every field is `false`. On
///   Windows, macOS, Android and the web that is known at once; on Linux,
///   which of Wayland and X11 the app runs on is known when the event loop
///   starts.
/// - **Unknown**: `None` on Linux before the event
///   loop starts (before `App::run`) and in a native embed host (whose
///   display is the host's).
///
/// Callable from any thread. Not reactive: a compositor that offers a role
/// offers it for the session.
///
/// [`WindowProps::wayland_layer`]: rinch_core::element::WindowProps::wayland_layer
pub fn window_roles() -> Option<WindowRoles> {
    if cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
        all(target_arch = "wasm32", target_os = "unknown"),
    )) {
        return Some(WindowRoles::NONE);
    }
    WindowRoles::decode(PUBLISHED.load(Ordering::Acquire))
}

/// Whether a window asked for a layer surface gets one:
/// [`window_roles`]'s [`layer_shell`](WindowRoles::layer_shell), and `false`
/// while that is not known yet (before the event loop starts on Linux). An
/// app that must tell "no" from "not yet" reads [`window_roles`].
pub fn layer_shell_available() -> bool {
    window_roles().is_some_and(|roles| roles.layer_shell)
}

/// The shell's answer, published when its event loop starts.
pub(crate) fn publish(roles: WindowRoles) {
    PUBLISHED.store(roles.encode(), Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decision from the registry's interfaces: a compositor with
    /// `zwlr_layer_shell_v1` (KWin, sway, Hyprland) offers layer surfaces,
    /// one without it (Mutter, Weston) does not.
    #[test]
    fn the_layer_shell_is_offered_when_the_registry_advertises_it() {
        // What `wayland-info` lists on a KWin 6.6 session, abridged.
        let kwin = [
            "wl_compositor",
            "wl_shm",
            "xdg_wm_base",
            "wl_output",
            "zxdg_output_manager_v1",
            "zwlr_layer_shell_v1",
            "xdg_activation_v1",
        ];
        assert!(WindowRoles::of_wayland_interfaces(kwin).layer_shell);
        // Mutter: everything above but the layer shell.
        let mutter = [
            "wl_compositor",
            "wl_shm",
            "xdg_wm_base",
            "wl_output",
            "zxdg_output_manager_v1",
            "xdg_activation_v1",
        ];
        assert_eq!(
            WindowRoles::of_wayland_interfaces(mutter),
            WindowRoles::NONE
        );
        // A similar name is not the protocol.
        assert!(
            !WindowRoles::of_wayland_interfaces(["zwlr_layer_shell_v2", "layer_shell"]).layer_shell
        );
        assert!(!WindowRoles::of_wayland_interfaces([]).layer_shell);
    }

    /// Unknown, then exactly what was published; both answers survive the
    /// round trip through the atomic.
    #[test]
    fn the_answer_is_unknown_until_published() {
        assert_eq!(WindowRoles::decode(0), None);
        for roles in [WindowRoles::NONE, WindowRoles { layer_shell: true }] {
            assert_eq!(WindowRoles::decode(roles.encode()), Some(roles));
        }
    }
}
