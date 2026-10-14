//! Overlay window behaviour: non-activating, click-through and skip-taskbar
//! windows (issue #1519).
//!
//! An overlay — a status pill, a HUD, a dictation indicator — sits on top of
//! another application while the user keeps typing into that application. It
//! must never take keyboard focus or activation: not when it is created, not
//! when it is shown, and not when it is clicked. [`WindowProps::non_activating`],
//! [`WindowProps::click_through`] and [`WindowProps::skip_taskbar`] ask for
//! that, and this module turns them into what each windowing system offers.
//!
//! The work is split in two so that the decision is testable without a
//! display: [`WindowSteps::plan`] is pure and says what to do on a backend,
//! and [`apply_to_attributes`] / [`after_create`] do it.
//!
//! | | non-activating | click-through | skip taskbar |
//! |---|---|---|---|
//! | Windows | `WS_EX_NOACTIVATE`, `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`, shown with `SW_SHOWNOACTIVATE` | `set_cursor_hittest(false)` (`WS_EX_TRANSPARENT \| WS_EX_LAYERED`) | `WS_EX_TOOLWINDOW` (no taskbar button, no Alt+Tab) and winit's `with_skip_taskbar` |
//! | macOS | non-activating `NSPanel`, `becomesKeyOnlyIfNeeded`, `hidesOnDeactivate = NO`, ordered front without becoming key | `ignoresMouseEvents` (through `set_cursor_hittest`) | nothing: a window has no Dock entry of its own |
//! | X11 | `WM_HINTS.input = False` (ICCCM "no input" model) | an empty input shape (through `set_cursor_hittest`) | `_NET_WM_STATE_SKIP_TASKBAR`, `_NET_WM_STATE_SKIP_PAGER` |
//! | Wayland | **not available** (xdg-toplevel cannot refuse focus) | an empty input region (through `set_cursor_hittest`) | **not available** |
//!
//! On every backend but Wayland a window that needs a tweak winit cannot make
//! at creation is created hidden, adjusted, and then shown, so it is never on
//! screen in a state that could take focus.

use std::sync::atomic::{AtomicBool, Ordering};

use rinch_core::element::WindowProps;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes};

/// The windowing system a window is created on. Each target constructs only
/// its own variants outside the tests.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Windows,
    MacOs,
    X11,
    Wayland,
    Other,
}

impl Backend {
    /// The backend `event_loop` runs on.
    pub(crate) fn of(event_loop: &dyn ActiveEventLoop) -> Self {
        let _ = event_loop;
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
            use winit::platform::wayland::ActiveEventLoopExtWayland;
            if event_loop.is_wayland() {
                Backend::Wayland
            } else {
                Backend::X11
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Backend::Other
        }
    }
}

/// What to do for one window on one backend. Pure: built by
/// [`WindowSteps::plan`] from the props, read by the functions that act.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WindowSteps {
    /// Pass `with_active(false)`: shown without activation (Windows'
    /// `SW_SHOWNOACTIVATE`, macOS `orderFront` rather than
    /// `makeKeyAndOrderFront`).
    pub(crate) inactive: bool,
    /// Create the window hidden, adjust it, then show it.
    pub(crate) create_hidden: bool,
    /// Windows: subclass for `WS_EX_NOACTIVATE` and `MA_NOACTIVATE`.
    pub(crate) win_no_activate: bool,
    /// Windows: `WS_EX_TOOLWINDOW` (no taskbar button, no Alt+Tab) and
    /// winit's `with_skip_taskbar`.
    pub(crate) win_tool_window: bool,
    /// macOS: a non-activating `NSPanel` that becomes key only when needed.
    pub(crate) mac_panel: bool,
    /// X11: `WM_HINTS.input = False`.
    pub(crate) x11_no_input: bool,
    /// X11: `_NET_WM_STATE_SKIP_TASKBAR` + `_NET_WM_STATE_SKIP_PAGER`.
    pub(crate) x11_skip_taskbar: bool,
    /// Wayland was asked for something it cannot do: warn once.
    pub(crate) wayland_unsupported: bool,
    /// Turn the pointer hit test off after creation.
    pub(crate) click_through: bool,
}

impl WindowSteps {
    /// Decide the steps for `props` on `backend`. `click_through` is the
    /// runtime's current click-through state, which starts at
    /// [`WindowProps::click_through`] and is changed by
    /// [`set_click_through_current_window`](crate::windows::set_click_through_current_window).
    pub(crate) fn plan(props: &WindowProps, click_through: bool, backend: Backend) -> Self {
        let non_activating = props.non_activating;
        let skip_taskbar = props.skip_taskbar;
        let mut steps = WindowSteps {
            inactive: non_activating,
            click_through,
            ..Default::default()
        };
        match backend {
            Backend::Windows => {
                steps.win_no_activate = non_activating;
                steps.win_tool_window = skip_taskbar;
                steps.create_hidden = non_activating || skip_taskbar;
            }
            Backend::MacOs => {
                steps.mac_panel = non_activating;
                steps.create_hidden = non_activating;
            }
            Backend::X11 => {
                steps.x11_no_input = non_activating;
                steps.x11_skip_taskbar = skip_taskbar;
                steps.create_hidden = non_activating || skip_taskbar;
            }
            Backend::Wayland => {
                steps.wayland_unsupported = non_activating || skip_taskbar;
            }
            Backend::Other => {}
        }
        steps
    }
}

/// Whether [`App::prefer_x11`](crate::App::prefer_x11) asked for X11.
static PREFER_X11: AtomicBool = AtomicBool::new(false);

/// Record [`App::prefer_x11`](crate::App::prefer_x11) for the event loop
/// [`new_event_loop`] is about to build.
pub(crate) fn set_prefer_x11(prefer: bool) {
    PREFER_X11.store(prefer, Ordering::Relaxed);
}

/// Whether the event loop is to be forced onto X11: asked for, and an X
/// server to connect to (`DISPLAY` set). Pure for the test.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn forces_x11(prefer: bool, display: Option<&std::ffi::OsStr>) -> bool {
    prefer && display.is_some_and(|d| !d.is_empty())
}

/// Build the desktop event loop, on X11 when the app prefers it and an X
/// server is reachable (XWayland under a Wayland session).
pub(crate) fn new_event_loop() -> EventLoop {
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var_os("DISPLAY");
        if forces_x11(PREFER_X11.load(Ordering::Relaxed), display.as_deref()) {
            use winit::platform::x11::EventLoopBuilderExtX11;
            return EventLoop::builder()
                .with_x11()
                .build()
                .expect("Failed to create event loop");
        }
    }
    EventLoop::new().expect("Failed to create event loop")
}

/// Fold the overlay steps, and the platform attributes rinch always sets
/// (`app_id` / `WM_CLASS` on Linux), into `attrs`.
pub(crate) fn apply_to_attributes(
    mut attrs: WindowAttributes,
    steps: WindowSteps,
    backend: Backend,
    app_id: Option<&str>,
) -> WindowAttributes {
    let _ = (backend, app_id);
    if steps.inactive {
        attrs = attrs.with_active(false);
    }
    if steps.create_hidden {
        attrs = attrs.with_visible(false);
    }
    #[cfg(target_os = "linux")]
    if let Some(app_id) = app_id {
        // One platform-attributes box per window, read by whichever backend
        // the event loop runs on — a Wayland box is discarded by X11.
        if backend == Backend::X11 {
            use winit::platform::x11::WindowAttributesX11;
            let x11 = WindowAttributesX11::default().with_name(app_id, app_id);
            attrs = attrs.with_platform_attributes(Box::new(x11));
        } else {
            use winit::platform::wayland::WindowAttributesWayland;
            let wayland = WindowAttributesWayland::default().with_name(app_id, app_id);
            attrs = attrs.with_platform_attributes(Box::new(wayland));
        }
    }
    #[cfg(target_os = "windows")]
    if steps.win_tool_window {
        use winit::platform::windows::WindowAttributesWindows;
        let win = WindowAttributesWindows::default().with_skip_taskbar(true);
        attrs = attrs.with_platform_attributes(Box::new(win));
    }
    #[cfg(target_os = "macos")]
    if steps.mac_panel {
        use winit::platform::macos::WindowAttributesMacOS;
        let mac = WindowAttributesMacOS::default()
            .with_panel(true)
            .with_accepts_first_mouse(true);
        attrs = attrs.with_platform_attributes(Box::new(mac));
    }
    attrs
}

/// Warned once per process: Wayland cannot keep a window from taking focus.
static WAYLAND_WARNED: AtomicBool = AtomicBool::new(false);

/// Adjust a freshly created window, then show it if it was created hidden.
pub(crate) fn after_create(window: &dyn Window, steps: WindowSteps, always_on_top: bool) {
    if steps.wayland_unsupported && !WAYLAND_WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "WindowProps::non_activating / skip_taskbar have no effect on Wayland: \
             xdg-toplevel cannot refuse keyboard focus or a taskbar entry. \
             Use App::prefer_x11(true) to run under XWayland, where they work."
        );
    }

    #[cfg(target_os = "windows")]
    if steps.win_no_activate || steps.win_tool_window {
        win32::install(window, steps.win_no_activate, steps.win_tool_window);
    }
    #[cfg(target_os = "macos")]
    if steps.mac_panel {
        macos::configure_panel(window);
    }
    #[cfg(target_os = "linux")]
    if steps.x11_no_input || steps.x11_skip_taskbar {
        if let Err(e) = x11::set_hints(window, steps.x11_no_input, steps.x11_skip_taskbar) {
            tracing::warn!("Could not set the X11 overlay hints: {e}");
        }
    }

    set_click_through(window, steps.click_through);

    if steps.create_hidden {
        window.set_visible(true);
        // A window level set on an unmapped X11 window is a client message
        // the window manager ignores (EWMH asks for the property instead);
        // say it again now the window is mapped.
        if always_on_top {
            window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
        }
    }
}

/// Let pointer events through to whatever is under the window, or stop doing so.
pub(crate) fn set_click_through(window: &dyn Window, click_through: bool) {
    if let Err(e) = window.set_cursor_hittest(!click_through) {
        if click_through {
            tracing::warn!("Click-through is not supported here: {e}");
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;
    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::properties::WmHints;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    /// Set `WM_HINTS.input = False` and the skip-taskbar states on an X11
    /// window, before it is mapped. A separate connection: the properties
    /// live on the server, and the round trip at the end makes sure the
    /// server has them before winit's own connection maps the window.
    pub(super) fn set_hints(
        window: &dyn Window,
        no_input: bool,
        skip_taskbar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let xid = match window.window_handle()?.as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => return Ok(()),
        };
        let (conn, _) = x11rb::connect(None)?;
        set_hints_on(&conn, xid, no_input, skip_taskbar)
    }

    /// [`set_hints`] on a given connection and window id.
    pub(crate) fn set_hints_on<C: Connection + RequestConnection>(
        conn: &C,
        xid: u32,
        no_input: bool,
        skip_taskbar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if no_input {
            let mut hints = WmHints::get(conn, xid)?.reply()?.unwrap_or_default();
            hints.input = Some(false);
            hints.set(conn, xid)?;
        }
        if skip_taskbar {
            let state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
            let taskbar = conn
                .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")?
                .reply()?
                .atom;
            let pager = conn
                .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")?
                .reply()?
                .atom;
            // APPEND, not REPLACE: winit may already have written a state.
            conn.change_property32(
                PropMode::APPEND,
                xid,
                state,
                AtomEnum::ATOM,
                &[taskbar, pager],
            )?;
        }
        // A round trip: the server has processed everything above.
        conn.get_input_focus()?.reply()?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    /// winit made a non-activating `NSPanel`; keep it from becoming the key
    /// window on a click (`becomesKeyOnlyIfNeeded`: only a view that asks for
    /// the keyboard makes it key, and winit's view does not), and keep it on
    /// screen while its application is inactive, which for an overlay is
    /// always (`hidesOnDeactivate` is `YES` by default on a panel).
    pub(super) fn configure_panel(window: &dyn Window) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        let view = handle.ns_view.as_ptr() as *mut AnyObject;
        // SAFETY: `ns_view` is winit's live `NSView`, on the main thread
        // (window creation runs there); `window` answers its `NSWindow`, which
        // winit created as an `NSPanel` (`with_panel(true)`), so both setters
        // exist on it.
        unsafe {
            let ns_window: *mut AnyObject = msg_send![view, window];
            if ns_window.is_null() {
                return;
            }
            let _: () = msg_send![ns_window, setBecomesKeyOnlyIfNeeded: true];
            let _: () = msg_send![ns_window, setHidesOnDeactivate: false];
        }
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use std::cell::RefCell;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GWL_EXSTYLE, GWLP_WNDPROC, MA_NOACTIVATE, STYLESTRUCT, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_MOUSEACTIVATE,
        WM_NCDESTROY, WM_STYLECHANGING, WNDPROC, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };
    #[cfg(target_pointer_width = "64")]
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW};
    #[cfg(target_pointer_width = "32")]
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW as GetWindowLongPtrW, SetWindowLongW as SetWindowLongPtrW,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    /// A subclassed window: the procedure it had, and what it asked for.
    #[derive(Clone, Copy)]
    struct Entry {
        hwnd: isize,
        old_proc: WNDPROC,
        no_activate: bool,
        tool_window: bool,
    }

    thread_local! {
        // Window procedures run on the window's thread, which is the one that
        // created it — rinch's main thread.
        static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    }

    fn entry(hwnd: HWND) -> Option<Entry> {
        ENTRIES.with(|e| e.borrow().iter().find(|e| e.hwnd == hwnd as isize).copied())
    }

    /// The extended style with what the overlay asked for laid over it.
    /// winit rewrites `GWL_EXSTYLE` wholesale on every flag change
    /// (`set_cursor_hittest`, `set_visible`, …), so this runs on every
    /// `WM_STYLECHANGING`, not once.
    fn adjust(ex_style: u32, entry: &Entry) -> u32 {
        let mut ex = ex_style;
        if entry.no_activate {
            ex |= WS_EX_NOACTIVATE;
        }
        if entry.tool_window {
            ex |= WS_EX_TOOLWINDOW;
            ex &= !WS_EX_APPWINDOW;
        }
        ex
    }

    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let Some(e) = entry(hwnd) else {
            return 0;
        };
        match msg {
            // A click must not activate the window.
            WM_MOUSEACTIVATE if e.no_activate => return MA_NOACTIVATE as LRESULT,
            WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE => {
                let style = lparam as *mut STYLESTRUCT;
                if !style.is_null() {
                    // SAFETY: for WM_STYLECHANGING, lParam is a STYLESTRUCT the
                    // system lets the procedure change.
                    unsafe { (*style).styleNew = adjust((*style).styleNew, &e) };
                }
            }
            WM_NCDESTROY => {
                ENTRIES.with(|v| v.borrow_mut().retain(|x| x.hwnd != hwnd as isize));
            }
            _ => {}
        }
        // SAFETY: the procedure the window had before ours.
        unsafe { CallWindowProcW(e.old_proc, hwnd, msg, wparam, lparam) }
    }

    /// Subclass the window and apply the extended styles once.
    pub(super) fn install(window: &dyn Window, no_activate: bool, tool_window: bool) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get() as HWND;
        if entry(hwnd).is_some() {
            return;
        }
        // SAFETY: `hwnd` is winit's live window, on its own thread.
        unsafe {
            let old = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as *const () as _);
            // SAFETY: a window procedure pointer, or 0 (`None`).
            let old_proc: WNDPROC = std::mem::transmute::<isize, WNDPROC>(old as isize);
            ENTRIES.with(|v| {
                v.borrow_mut().push(Entry {
                    hwnd: hwnd as isize,
                    old_proc,
                    no_activate,
                    tool_window,
                })
            });
            // Rewrite the extended style through our WM_STYLECHANGING.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(non_activating: bool, skip_taskbar: bool) -> WindowProps {
        WindowProps {
            non_activating,
            skip_taskbar,
            ..Default::default()
        }
    }

    #[test]
    fn an_ordinary_window_takes_no_step_on_any_backend() {
        for b in [
            Backend::Windows,
            Backend::MacOs,
            Backend::X11,
            Backend::Wayland,
            Backend::Other,
        ] {
            assert_eq!(
                WindowSteps::plan(&props(false, false), false, b),
                WindowSteps::default(),
                "{b:?}"
            );
        }
    }

    #[test]
    fn a_non_activating_window_is_shown_inactive_and_created_hidden() {
        let w = WindowSteps::plan(&props(true, false), false, Backend::Windows);
        assert!(w.inactive && w.create_hidden && w.win_no_activate && !w.win_tool_window);
        let m = WindowSteps::plan(&props(true, false), false, Backend::MacOs);
        assert!(m.inactive && m.create_hidden && m.mac_panel);
        let x = WindowSteps::plan(&props(true, false), false, Backend::X11);
        assert!(x.inactive && x.create_hidden && x.x11_no_input && !x.x11_skip_taskbar);
    }

    #[test]
    fn skip_taskbar_alone_does_not_make_a_window_non_activating() {
        let w = WindowSteps::plan(&props(false, true), false, Backend::Windows);
        assert!(w.win_tool_window && w.create_hidden && !w.win_no_activate && !w.inactive);
        let x = WindowSteps::plan(&props(false, true), false, Backend::X11);
        assert!(x.x11_skip_taskbar && !x.x11_no_input && !x.inactive);
        // macOS has no per-window Dock entry to skip.
        let m = WindowSteps::plan(&props(false, true), false, Backend::MacOs);
        assert!(!m.mac_panel && !m.create_hidden);
    }

    #[test]
    fn wayland_warns_instead_of_pretending() {
        let w = WindowSteps::plan(&props(true, true), true, Backend::Wayland);
        assert!(w.wayland_unsupported);
        assert!(!w.create_hidden, "nothing to adjust before showing");
        assert!(w.click_through, "an empty input region works on Wayland");
        assert!(
            !WindowSteps::plan(&props(false, false), true, Backend::Wayland).wayland_unsupported
        );
    }

    #[test]
    fn click_through_follows_the_runtime_state_not_the_props() {
        let p = WindowProps {
            click_through: true,
            ..Default::default()
        };
        assert!(!WindowSteps::plan(&p, false, Backend::X11).click_through);
        assert!(WindowSteps::plan(&props(false, false), true, Backend::X11).click_through);
    }

    #[test]
    fn x11_is_forced_only_when_asked_and_reachable() {
        use std::ffi::OsStr;
        assert!(forces_x11(true, Some(OsStr::new(":0"))));
        assert!(!forces_x11(true, None));
        assert!(!forces_x11(true, Some(OsStr::new(""))));
        assert!(!forces_x11(false, Some(OsStr::new(":0"))));
    }

    #[test]
    fn the_attributes_carry_inactive_and_hidden() {
        let steps = WindowSteps::plan(&props(true, false), false, Backend::X11);
        let attrs = apply_to_attributes(WindowAttributes::default(), steps, Backend::X11, None);
        assert!(!attrs.active);
        assert!(!attrs.visible);
        let plain = apply_to_attributes(
            WindowAttributes::default(),
            WindowSteps::default(),
            Backend::X11,
            None,
        );
        assert!(plain.active && plain.visible);
    }

    /// The X11 hints on a real X server: needs `DISPLAY` (Xvfb or XWayland).
    /// `cargo test -p rinch --lib overlay_window -- --ignored`.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs an X server (DISPLAY)"]
    fn x11_hints_land_on_the_window() {
        use x11rb::connection::Connection;
        use x11rb::properties::WmHints;
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, WindowClass};

        let (conn, screen) = x11rb::connect(None).expect("an X server");
        let root = conn.setup().roots[screen].root;
        let xid = conn.generate_id().unwrap();
        conn.create_window(
            0,
            xid,
            root,
            0,
            0,
            10,
            10,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap();
        // A hint the window already had survives (the property is merged).
        let mut pre = WmHints::new();
        pre.urgent = true;
        pre.set(&conn, xid).unwrap();
        // So does a state already written (APPEND, not REPLACE).
        let state = conn
            .intern_atom(false, b"_NET_WM_STATE")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let above = conn
            .intern_atom(false, b"_NET_WM_STATE_ABOVE")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        {
            use x11rb::protocol::xproto::PropMode;
            use x11rb::wrapper::ConnectionExt as _;
            conn.change_property32(PropMode::REPLACE, xid, state, AtomEnum::ATOM, &[above])
                .unwrap();
        }

        x11::set_hints_on(&conn, xid, true, true).unwrap();

        let hints = WmHints::get(&conn, xid).unwrap().reply().unwrap().unwrap();
        assert_eq!(hints.input, Some(false));
        assert!(hints.urgent);
        let skip = conn
            .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let value = conn
            .get_property(false, xid, state, AtomEnum::ATOM, 0, 16)
            .unwrap()
            .reply()
            .unwrap();
        let atoms: Vec<u32> = value.value32().unwrap().collect();
        assert!(atoms.contains(&skip), "{atoms:?}");
        assert!(atoms.contains(&above), "{atoms:?}");
    }
}
