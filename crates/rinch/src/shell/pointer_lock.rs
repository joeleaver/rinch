//! The desktop window's pointer lock (#2092): the [`PointerLockHost`] the
//! shell installs for its main window, and the sum of the device's raw motion
//! over a turn of the event loop.
//!
//! The grab is winit's: `CursorGrabMode::Locked` first (Wayland's
//! pointer-constraints lock, Windows, macOS), `Confined` where that is refused
//! (X11, whose winit backend has no lock), and the cursor hidden for either.
//! The motion is winit's `DeviceEvent::PointerMotion` (XInput2 raw motion,
//! Wayland's relative-pointer, Windows raw input, macOS deltas), which the
//! cursor's position does not clamp.

use std::sync::Weak;

use winit::window::{CursorGrabMode, Window};

use crate::window::{PointerLockError, PointerLockHost, PointerLockMode, PointerLockTarget};

/// The two calls a pointer lock makes on a window, apart so a test can stand
/// in for winit.
pub(crate) trait CursorGrab {
    fn focused(&self) -> bool;
    fn grab(&self, mode: CursorGrabMode) -> Result<(), String>;
    fn show_cursor(&self, visible: bool);
}

impl CursorGrab for dyn Window {
    fn focused(&self) -> bool {
        self.has_focus()
    }
    fn grab(&self, mode: CursorGrabMode) -> Result<(), String> {
        self.set_cursor_grab(mode).map_err(|e| e.to_string())
    }
    fn show_cursor(&self, visible: bool) {
        self.set_cursor_visible(visible);
    }
}

/// Lock `window`'s pointer: `Locked`, else `Confined`, and hide the cursor.
/// Answers what was granted, or why neither was.
///
/// Refused while the window does not have the keyboard focus, as a browser
/// refuses: X11's confinement is an active pointer grab, which the server
/// grants an unfocused window too — the pointer would be caught and hidden
/// over a window a click cannot focus, Escape would go to another
/// application, and no focus loss would ever come to end it.
pub(crate) fn grab<W: CursorGrab + ?Sized>(
    window: &W,
) -> Result<PointerLockMode, PointerLockError> {
    if !window.focused() {
        return Err(PointerLockError::NotFocused);
    }
    let mode = match window.grab(CursorGrabMode::Locked) {
        Ok(()) => PointerLockMode::Locked,
        Err(locked) => match window.grab(CursorGrabMode::Confined) {
            Ok(()) => PointerLockMode::Confined,
            Err(confined) => {
                return Err(PointerLockError::Refused(format!(
                    "lock: {locked}; confine: {confined}"
                )));
            }
        },
    };
    window.show_cursor(false);
    Ok(mode)
}

/// Give `window`'s pointer back and show the cursor.
pub(crate) fn ungrab<W: CursorGrab + ?Sized>(window: &W) {
    if let Err(e) = window.grab(CursorGrabMode::None) {
        tracing::warn!("releasing the pointer lock failed: {e}");
    }
    window.show_cursor(true);
}

/// The main window's host. Holds the window weakly: the shell owns it, and
/// replaces this host (or removes it) whenever it makes or drops one.
pub(crate) struct WindowPointerLock {
    pub(crate) window: Weak<dyn Window>,
}

impl PointerLockHost for WindowPointerLock {
    fn lock(
        &mut self,
        _target: &PointerLockTarget,
    ) -> Result<Option<PointerLockMode>, PointerLockError> {
        // An element's lock grabs for the whole window: the OS has no
        // narrower grab, and the element only decides how long it lasts.
        let window = self.window.upgrade().ok_or(PointerLockError::Unsupported)?;
        grab(&*window).map(Some)
    }

    fn unlock(&mut self) {
        if let Some(window) = self.window.upgrade() {
            ungrab(&*window);
        }
    }
}

/// Raw motion held back until the turn of the event loop ends, summed: a
/// 1000 Hz mouse reports a motion every millisecond, and each one is not
/// worth a trip through the app. The sum is what a camera turns by.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct MotionSum {
    pending: Option<(f64, f64)>,
}

impl MotionSum {
    /// Add one device motion; non-finite steps are dropped.
    pub(crate) fn add(&mut self, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let (x, y) = self.pending.unwrap_or((0.0, 0.0));
        self.pending = Some((x + dx, y + dy));
    }

    /// The summed motion since the last take, if there was any.
    pub(crate) fn take(&mut self) -> Option<(f64, f64)> {
        self.pending.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A window that refuses the grab modes it is told to, and logs the calls.
    struct FakeWindow {
        focused: bool,
        refuse: Vec<CursorGrabMode>,
        calls: RefCell<Vec<String>>,
    }

    impl CursorGrab for FakeWindow {
        fn focused(&self) -> bool {
            self.focused
        }
        fn grab(&self, mode: CursorGrabMode) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("grab {mode:?}"));
            if self.refuse.contains(&mode) {
                Err(format!("{mode:?} not supported"))
            } else {
                Ok(())
            }
        }
        fn show_cursor(&self, visible: bool) {
            self.calls.borrow_mut().push(format!("visible {visible}"));
        }
    }

    fn window(refuse: &[CursorGrabMode]) -> FakeWindow {
        FakeWindow {
            focused: true,
            refuse: refuse.to_vec(),
            calls: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_lock_is_preferred_and_hides_the_cursor() {
        let w = window(&[]);
        assert_eq!(grab(&w), Ok(PointerLockMode::Locked));
        assert_eq!(*w.calls.borrow(), ["grab Locked", "visible false"]);
    }

    /// X11: winit has no lock there, so the window is confined, and says so.
    #[test]
    fn a_refused_lock_falls_back_to_confine() {
        let w = window(&[CursorGrabMode::Locked]);
        assert_eq!(grab(&w), Ok(PointerLockMode::Confined));
        assert_eq!(
            *w.calls.borrow(),
            ["grab Locked", "grab Confined", "visible false"]
        );
    }

    #[test]
    fn refusing_both_grabs_nothing_and_keeps_the_cursor() {
        let w = window(&[CursorGrabMode::Locked, CursorGrabMode::Confined]);
        let err = grab(&w).unwrap_err();
        assert!(matches!(&err, PointerLockError::Refused(why)
            if why.contains("Locked not supported") && why.contains("Confined not supported")));
        assert_eq!(*w.calls.borrow(), ["grab Locked", "grab Confined"]);
    }

    /// An unfocused window grabs nothing (X11 would trap the pointer over
    /// it), and says why.
    #[test]
    fn an_unfocused_window_is_refused_without_a_grab() {
        let mut w = window(&[]);
        w.focused = false;
        assert_eq!(grab(&w), Err(PointerLockError::NotFocused));
        assert!(w.calls.borrow().is_empty(), "{:?}", w.calls.borrow());
    }

    #[test]
    fn a_release_ungrabs_and_shows_the_cursor() {
        let w = window(&[]);
        ungrab(&w);
        assert_eq!(*w.calls.borrow(), ["grab None", "visible true"]);
    }

    #[test]
    fn motion_is_summed_until_taken() {
        let mut sum = MotionSum::default();
        assert_eq!(sum.take(), None);
        sum.add(1.5, -2.0);
        sum.add(f64::INFINITY, 1.0);
        sum.add(0.25, 4.0);
        assert_eq!(sum.take(), Some((1.75, 2.0)));
        assert_eq!(sum.take(), None);
    }
}
