//! The user's zoom (`rinch::window::set_zoom`): the display's scale times a
//! factor the app sets, applied by the desktop shell wherever it turns the
//! display's scale into the document's (`apply`).

#[cfg(feature = "windowing")]
use std::cell::Cell;

/// The smallest zoom [`set_zoom`] takes: a quarter size, as browsers allow.
pub const ZOOM_MIN: f64 = 0.25;
/// The largest zoom [`set_zoom`] takes: five times, as browsers allow.
pub const ZOOM_MAX: f64 = 5.0;

#[cfg(feature = "windowing")]
thread_local! {
    /// The UI thread's zoom, read by the shell wherever it turns the
    /// display's scale into the document's (`apply`).
    static ZOOM: Cell<f64> = const { Cell::new(1.0) };
    /// The reactive copy [`zoom`] reads, made on first use.
    static SIGNAL: Cell<Option<rinch_core::Signal<f64>>> = const { Cell::new(None) };
}

/// The app's zoom: `1.0` at actual size, `1.5` drawn half as large again.
/// Reactive: a reader runs again when [`set_zoom`] changes it.
///
/// Always `1.0` where rinch does not apply a zoom (the web, Android; see
/// [`set_zoom`]).
pub fn zoom() -> f64 {
    #[cfg(feature = "windowing")]
    {
        signal().get()
    }
    #[cfg(not(feature = "windowing"))]
    {
        1.0
    }
}

/// Zoom every window of the app to `zoom` (`1.0` is actual size), clamped
/// to [`ZOOM_MIN`]`..=`[`ZOOM_MAX`]. Answers the zoom now in effect: the
/// clamped value, the current zoom for a value that is not a positive
/// finite number, and `1.0` where rinch applies no zoom (the web, Android).
///
/// On the desktop it takes effect on the event loop's next turn: each open
/// window is restyled, laid out and painted again at its display's scale
/// times the zoom, keeping its size on screen. Set before the loop starts,
/// the windows are created at it. Call it on the UI thread.
///
/// A zoom multiplies the display scale. A window on a display at scale `s`
/// zoomed to `z` is laid out and painted at `s × z` physical pixels per CSS
/// pixel, so text, borders, images, SVG, `RenderSurface`s and every length
/// the app wrote in CSS grow together and stay sharp: nothing is a scaled
/// bitmap. The window keeps its size on screen; its layout viewport (what
/// [`window_size`](crate::window::window_size) answers, in CSS pixels)
/// shrinks by `z`, exactly as a browser's `innerWidth` does when the page is
/// zoomed in. Pointer positions handed to the app, hit testing, the caret's
/// IME area and the size a content-sized window asks for follow the same
/// scale, so a click lands on what is drawn under it at any zoom.
///
/// [`scale_factor`](crate::window::scale_factor) answers the scale the
/// document is drawn at, zoom included (the browser's `devicePixelRatio`
/// includes its zoom too). It changes when the windows apply the zoom, on the
/// event loop's next turn, not when `set_zoom` returns (`zoom()` changes at
/// once). A display change keeps the zoom: a window dragged
/// from a 1x display to a 2x one at zoom 1.5 is drawn at 3x there.
///
/// The zoom is the app's: every window it opens follows it (DevTools too).
/// It is not remembered between runs; an app that wants that saves the value
/// and sets it again at start, before the first window is made when it can,
/// so the first frame is drawn at it.
///
/// Window geometry stays in the system's units: the size set with
/// `rinch::windows::set_current_window_size` and the
/// window's position are in the display's logical pixels whatever the zoom,
/// as a browser window's outer size is.
///
/// Where it applies:
///
/// - **Desktop** (`windowing`): as above.
/// - **Web**: not applied. The browser zooms the page itself (its own
///   `Ctrl` `+`, which rinch-web leaves to it), and that zoom already reaches
///   the app through `devicePixelRatio`. [`set_zoom`] changes nothing there
///   and [`zoom`] answers `1.0`, so an app can tell (and say so).
/// - **Android**: not applied (the system's font and display size settings
///   are the platform's zoom); [`zoom`] answers `1.0`.
/// - **Native embed** (`RinchContext`): the host owns the scale, and
///   `set_zoom` does not change it. A host that wants the zoom passes
///   `scale × rinch::window::zoom()` to `RinchContext::set_scale_factor`.
///
/// The zoom belongs to the UI thread: set it and read it there. Set from
/// another thread it is ignored, with a `tracing` warning.
pub fn set_zoom(zoom: f64) -> f64 {
    #[cfg(feature = "windowing")]
    {
        if !rinch_core::is_main_thread() {
            tracing::warn!("rinch::window::set_zoom was called off the UI thread; ignored");
            return current();
        }
        if !(zoom.is_finite() && zoom > 0.0) {
            return current();
        }
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if zoom == current() {
            return zoom;
        }
        ZOOM.with(|z| z.set(zoom));
        signal().set(zoom);
        // Wake a running shell to apply it; before the loop starts the
        // windows take it when they are created.
        if crate::shell::rinch_runtime::GLOBAL_PROXY.get().is_some() {
            crate::shell::rinch_runtime::send_native_event(
                crate::shell::rinch_runtime::RinchNativeEvent::ApplyZoom,
            );
        }
        zoom
    }
    #[cfg(not(feature = "windowing"))]
    {
        let _ = zoom;
        1.0
    }
}

/// The UI thread's zoom, without subscribing.
#[cfg(feature = "windowing")]
pub(crate) fn current() -> f64 {
    ZOOM.with(Cell::get)
}

/// The scale a document on a display at `os_scale` is drawn at: the
/// display's scale times the zoom. The one place the two are combined.
#[cfg(feature = "windowing")]
pub(crate) fn apply(os_scale: f64) -> f64 {
    os_scale * current()
}

#[cfg(feature = "windowing")]
fn signal() -> rinch_core::Signal<f64> {
    if let Some(s) = SIGNAL.with(Cell::get) {
        return s;
    }
    let s = rinch_core::unowned(|| rinch_core::Signal::new(current()));
    SIGNAL.with(|c| c.set(Some(s)));
    s
}

#[cfg(all(test, feature = "windowing"))]
mod tests {
    use super::*;

    /// Each test thread has its own zoom; put it back for the next test the
    /// thread runs.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ZOOM.with(|z| z.set(1.0));
            if let Some(s) = SIGNAL.with(Cell::get) {
                s.set(1.0);
            }
        }
    }

    #[test]
    fn clamps_and_refuses_nonsense() {
        let _reset = Reset;
        assert_eq!(zoom(), 1.0);
        assert_eq!(set_zoom(1.5), 1.5);
        assert_eq!(zoom(), 1.5);
        assert_eq!(set_zoom(f64::NAN), 1.5, "not a number: unchanged");
        assert_eq!(set_zoom(0.0), 1.5, "not positive: unchanged");
        assert_eq!(set_zoom(-2.0), 1.5);
        assert_eq!(set_zoom(f64::INFINITY), 1.5);
        assert_eq!(set_zoom(100.0), ZOOM_MAX);
        assert_eq!(set_zoom(0.01), ZOOM_MIN);
        assert_eq!(apply(2.0), 2.0 * ZOOM_MIN);
    }

    #[test]
    fn a_reader_runs_again_on_a_change_only() {
        let _reset = Reset;
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let _effect = {
            let seen = seen.clone();
            rinch_core::Effect::new(move || seen.borrow_mut().push(zoom()))
        };
        set_zoom(1.25);
        set_zoom(1.25);
        set_zoom(0.8);
        assert_eq!(*seen.borrow(), [1.0, 1.25, 0.8]);
    }
}
