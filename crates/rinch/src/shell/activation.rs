//! The macOS activation policy (issue #1528): whether the app has a Dock icon
//! and a menu bar of its own.
//!
//! macOS decides that per *application*, not per window, through
//! `NSApplicationActivationPolicy`. A tray-resident app (a status item and a
//! window it shows on request) wants [`ActivationPolicy::Accessory`]: no Dock
//! icon and no app menu, while its windows can still be shown and take the
//! keyboard. Without it, an app started with `WindowProps::visible: false`
//! still puts an icon in the Dock.
//!
//! The choice comes from [`App::activation_policy`](crate::App::activation_policy)
//! and is handed to winit (`EventLoopBuilderExtMacOS::with_activation_policy`)
//! when the event loop is built. It has no effect on any other platform.

use std::sync::OnceLock;

/// How the app presents itself on macOS: with a Dock icon and menu bar, or as
/// an accessory without them.
///
/// Corresponds to `NSApplicationActivationPolicy`. Only macOS has the concept;
/// on Windows and Linux every value does nothing (a Windows or Linux app has a
/// taskbar entry exactly while it has a visible window).
///
/// Not setting one (the default) leaves the decision to winit: an
/// unbundled executable is [`Regular`](ActivationPolicy::Regular), and a
/// bundled `.app` follows its `Info.plist` (`LSUIElement` = accessory).
/// Setting one overrides the `Info.plist`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationPolicy {
    /// An ordinary app: a Dock icon, a menu bar while it is frontmost, and an
    /// entry in the Cmd+Tab switcher.
    Regular,
    /// No Dock icon and no menu bar of its own, but its windows can be shown,
    /// focused and typed into. What a tray-only app (a status item plus a
    /// window it opens on request) wants; the same as `LSUIElement` in a
    /// bundle's `Info.plist`.
    Accessory,
    /// No Dock icon, and the app may not create windows or be activated. For a
    /// background helper; a rinch app that ever shows its window should use
    /// [`Accessory`](ActivationPolicy::Accessory).
    Prohibited,
}

static REQUESTED: OnceLock<ActivationPolicy> = OnceLock::new();

/// Install the app's choice. Called once by [`App::run`](crate::App::run)
/// before the event loop exists; a second call is ignored, as winit permits
/// one event loop per process.
pub(crate) fn set_requested(policy: ActivationPolicy) {
    let _ = REQUESTED.set(policy);
}

/// Build the desktop event loop with the requested activation policy applied
/// (on macOS; elsewhere the policy is not read).
pub(crate) fn new_event_loop() -> Result<winit::event_loop::EventLoop, winit::error::EventLoopError>
{
    #[allow(unused_mut)]
    let mut builder = winit::event_loop::EventLoop::builder();
    #[cfg(target_os = "macos")]
    if let Some(policy) = REQUESTED.get().copied() {
        use winit::platform::macos::{ActivationPolicy as Mac, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(match policy {
            ActivationPolicy::Regular => Mac::Regular,
            ActivationPolicy::Accessory => Mac::Accessory,
            ActivationPolicy::Prohibited => Mac::Prohibited,
        });
    }
    builder.build()
}
