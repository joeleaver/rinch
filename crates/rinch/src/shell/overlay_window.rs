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
//! | macOS | non-activating `NSPanel`, `becomesKeyOnlyIfNeeded`, `hidesOnDeactivate = NO`, ordered front at creation without becoming key | `ignoresMouseEvents` (through `set_cursor_hittest`) | nothing: a window has no Dock entry of its own |
//! | X11 | `WM_HINTS.input = False` (ICCCM "no input" model) | an empty `ShapeInput` region set by rinch over its own x11rb connection (winit's `set_cursor_hittest` never negotiates XFixes and is refused) | `_NET_WM_STATE_SKIP_TASKBAR`, `_NET_WM_STATE_SKIP_PAGER` |
//! | Wayland | **not available** (xdg-toplevel cannot refuse focus) | an empty input region (through `set_cursor_hittest`) | **not available** |
//!
//! On Windows and X11 a window that needs a tweak winit cannot make at
//! creation is created hidden, adjusted, and then shown, so it is never on
//! screen in a state that could take focus. Not on macOS, where showing later
//! (`set_visible`) is `makeKeyAndOrderFront`: the panel is shown at creation
//! with `orderFront` and adjusted right after. A window that asks for none of
//! this takes no step at all.

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
                // NOT created hidden: in winit-appkit `set_visible(true)` is
                // `makeKeyAndOrderFront`, whatever `active` says, and the
                // panel would take the keyboard. Only the creation-time show
                // honours `with_active(false)` (`orderFront`), and
                // `configure_panel` runs right after it, before any event.
                steps.mac_panel = non_activating;
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

/// Whether to try X11 first: asked for, and `DISPLAY` set. Whether the
/// server then accepts the connection is [`build_event_loop`]'s question.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn forces_x11(prefer: bool, display: Option<&std::ffi::OsStr>) -> bool {
    prefer && display.is_some_and(|d| !d.is_empty())
}

/// Build the desktop event loop, on X11 when the app prefers it and an X
/// server accepts the connection (XWayland under a Wayland session).
pub(crate) fn new_event_loop() -> EventLoop {
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var_os("DISPLAY");
        let try_x11 = forces_x11(PREFER_X11.load(Ordering::Relaxed), display.as_deref());
        return build_event_loop(
            try_x11,
            || {
                use winit::platform::x11::EventLoopBuilderExtX11;
                EventLoop::builder().with_x11().build()
            },
            || EventLoop::new().expect("Failed to create event loop"),
        );
    }
    // Elsewhere the backend is fixed; macOS takes the activation policy
    // (#1528) when the loop is built.
    #[allow(unreachable_code)]
    super::activation::new_event_loop().expect("Failed to create event loop")
}

/// X11 when `try_x11` and it builds; otherwise the default backend, with a
/// warning when X11 was asked for and refused (a stale `DISPLAY`: an ssh or
/// tmux leftover, XWayland turned off).
#[cfg(any(target_os = "linux", test))]
pub(crate) fn build_event_loop<T, E: std::fmt::Display>(
    try_x11: bool,
    x11: impl FnOnce() -> Result<T, E>,
    default: impl FnOnce() -> T,
) -> T {
    if try_x11 {
        match x11() {
            Ok(event_loop) => return event_loop,
            Err(e) => tracing::warn!(
                "App::prefer_x11: the X server would not take the connection ({e}); \
                 using the default backend"
            ),
        }
    }
    default()
}

/// Fold the overlay steps, and the platform attributes rinch always sets on
/// Linux, into `attrs`: the Wayland `app_id` (`wayland_app_id`, always
/// resolved) and the X11 `WM_CLASS` (`x11_class`: only an explicit
/// `WindowProps::app_id`; without one winit's default — `argv[0]`'s file
/// name, `RESOURCE_NAME` for the instance — is kept, as it always was).
pub(crate) fn apply_to_attributes(
    mut attrs: WindowAttributes,
    steps: WindowSteps,
    backend: Backend,
    wayland_app_id: Option<&str>,
    x11_class: Option<&str>,
) -> WindowAttributes {
    let _ = (backend, wayland_app_id, x11_class);
    if steps.inactive {
        attrs = attrs.with_active(false);
    }
    if steps.create_hidden {
        attrs = attrs.with_visible(false);
    }
    // One platform-attributes box per window, read by whichever backend the
    // event loop runs on — X11 discards a Wayland box, which is how an
    // explicit app_id used to miss WM_CLASS.
    #[cfg(target_os = "linux")]
    if backend == Backend::X11 {
        if let Some(class) = x11_class {
            use winit::platform::x11::WindowAttributesX11;
            let x11 = WindowAttributesX11::default().with_name(class, class);
            attrs = attrs.with_platform_attributes(Box::new(x11));
        }
    } else if let Some(app_id) = wayland_app_id {
        use winit::platform::wayland::WindowAttributesWayland;
        let wayland = WindowAttributesWayland::default().with_name(app_id, app_id);
        attrs = attrs.with_platform_attributes(Box::new(wayland));
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

/// One thing [`after_create`] does to a new window, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterCreate {
    /// Log once that Wayland cannot do what was asked.
    WarnWayland,
    /// Windows: subclass for no-activate / tool-window.
    Win32Install {
        no_activate: bool,
        tool_window: bool,
    },
    /// macOS: `becomesKeyOnlyIfNeeded`, `hidesOnDeactivate = NO`.
    MacPanel,
    /// X11: input hint and skip-taskbar states, before the map.
    X11Hints { no_input: bool, skip_taskbar: bool },
    /// Let the pointer through. Only when asked for: an ordinary window
    /// takes no hit-test call at all, exactly as before #1519.
    ClickThrough,
    /// Show the window created hidden.
    Show,
    /// Ask for always-on-top again, now mapped.
    ReassertOnTop,
}

/// What [`after_create`] does for `steps`. Pure, so the order and the
/// conditions are tested without a window.
pub(crate) fn after_create_actions(steps: WindowSteps, always_on_top: bool) -> Vec<AfterCreate> {
    let mut actions = Vec::new();
    if steps.wayland_unsupported {
        actions.push(AfterCreate::WarnWayland);
    }
    if steps.win_no_activate || steps.win_tool_window {
        actions.push(AfterCreate::Win32Install {
            no_activate: steps.win_no_activate,
            tool_window: steps.win_tool_window,
        });
    }
    if steps.mac_panel {
        actions.push(AfterCreate::MacPanel);
    }
    if steps.x11_no_input || steps.x11_skip_taskbar {
        actions.push(AfterCreate::X11Hints {
            no_input: steps.x11_no_input,
            skip_taskbar: steps.x11_skip_taskbar,
        });
    }
    if steps.click_through {
        actions.push(AfterCreate::ClickThrough);
    }
    if steps.create_hidden {
        actions.push(AfterCreate::Show);
        // A window level set on an unmapped X11 window is a client message
        // the window manager ignores (EWMH asks for the property instead).
        if always_on_top {
            actions.push(AfterCreate::ReassertOnTop);
        }
    }
    actions
}

/// Adjust a freshly created window, then show it if it was created hidden.
pub(crate) fn after_create(window: &dyn Window, steps: WindowSteps, always_on_top: bool) {
    for action in after_create_actions(steps, always_on_top) {
        match action {
            AfterCreate::WarnWayland => {
                if !WAYLAND_WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "WindowProps::non_activating / skip_taskbar have no effect on Wayland: \
                         xdg-toplevel cannot refuse keyboard focus or a taskbar entry. \
                         Use App::prefer_x11(true) to run under XWayland, where they work."
                    );
                }
            }
            AfterCreate::Win32Install {
                no_activate,
                tool_window,
            } => {
                #[cfg(target_os = "windows")]
                win32::install(window, no_activate, tool_window);
                let _ = (no_activate, tool_window);
            }
            AfterCreate::MacPanel => {
                #[cfg(target_os = "macos")]
                macos::configure_panel(window);
            }
            AfterCreate::X11Hints {
                no_input,
                skip_taskbar,
            } => {
                #[cfg(target_os = "linux")]
                if let Err(e) = x11::set_hints(window, no_input, skip_taskbar) {
                    tracing::warn!("Could not set the X11 overlay hints: {e}");
                }
                let _ = (no_input, skip_taskbar);
            }
            AfterCreate::ClickThrough => set_click_through(window, true),
            AfterCreate::Show => window.set_visible(true),
            AfterCreate::ReassertOnTop => {
                window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
        }
    }
}

/// Let pointer events through to whatever is under the window, or stop doing so.
///
/// On X11 rinch sets the input shape itself (see `x11::set_input_passthrough`):
/// winit's `set_cursor_hittest` sends XFixes requests without negotiating the
/// XFixes version, which the server refuses, so it did nothing there.
pub(crate) fn set_click_through(window: &dyn Window, click_through: bool) {
    #[cfg(target_os = "linux")]
    match x11::set_input_passthrough(window, click_through) {
        Ok(true) => return,
        Ok(false) => {} // not an X11 window
        Err(e) => {
            tracing::warn!("Could not set the X11 input shape: {e}");
            return;
        }
    }
    if let Err(e) = window.set_cursor_hittest(!click_through) {
        if click_through {
            tracing::warn!("Click-through is not supported here: {e}");
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::cell::RefCell;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::properties::WmHints;
    use x11rb::protocol::shape::SK;
    use x11rb::protocol::xfixes::{ConnectionExt as _, RegionWrapper};
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// The X11 window id behind `window`, or `None` for another backend.
    fn xid_of(window: &dyn Window) -> Result<Option<u32>, Box<dyn std::error::Error>> {
        Ok(match window.window_handle()?.as_raw() {
            RawWindowHandle::Xlib(h) => Some(h.window as u32),
            RawWindowHandle::Xcb(h) => Some(h.window.get()),
            _ => None,
        })
    }

    thread_local! {
        /// rinch's own connection for input shapes, XFixes version negotiated.
        /// Kept: a toggle should not open a connection each time.
        static SHAPE_CONN: RefCell<Option<RustConnection>> = const { RefCell::new(None) };
    }

    /// Give `window` an empty input shape (pointer passes through) or none
    /// (the default: the whole window takes the pointer). `Ok(false)` when
    /// `window` is not an X11 window.
    pub(super) fn set_input_passthrough(
        window: &dyn Window,
        pass_through: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let Some(xid) = xid_of(window)? else {
            return Ok(false);
        };
        SHAPE_CONN.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let (conn, _) = x11rb::connect(None)?;
                *slot = Some(conn);
            }
            let conn = slot.as_ref().expect("just connected");
            let result = set_input_passthrough_on(conn, xid, pass_through);
            if result.is_err() {
                // A dead connection is not kept for the next toggle.
                *slot = None;
            }
            result.map(|()| true)
        })
    }

    /// [`set_input_passthrough`] on a given connection and window id.
    pub(crate) fn set_input_passthrough_on<C: Connection + RequestConnection>(
        conn: &C,
        xid: u32,
        pass_through: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // The server refuses an XFixes request above the version the client
        // negotiated (none, until this): BadRequest, silently.
        conn.xfixes_query_version(5, 0)?.reply()?;
        if pass_through {
            let region = RegionWrapper::create_region(conn, &[])?;
            conn.xfixes_set_window_shape_region(xid, SK::INPUT, 0, 0, region.region())?
                .check()?;
        } else {
            // No input shape: the window's bounding shape, which follows the
            // window as it resizes.
            conn.xfixes_set_window_shape_region(xid, SK::INPUT, 0, 0, x11rb::NONE)?
                .check()?;
        }
        Ok(())
    }

    /// Set `WM_HINTS.input = False` and the skip-taskbar states on an X11
    /// window, before it is mapped. A separate connection: the properties
    /// live on the server, and the round trip at the end makes sure the
    /// server has them before winit's own connection maps the window.
    pub(super) fn set_hints(
        window: &dyn Window,
        no_input: bool,
        skip_taskbar: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(xid) = xid_of(window)? else {
            return Ok(());
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
        CallWindowProcW, DefWindowProcW, GWL_EXSTYLE, GWLP_WNDPROC, MA_NOACTIVATE, STYLESTRUCT,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        WM_MOUSEACTIVATE, WM_NCDESTROY, WM_STYLECHANGING, WNDPROC, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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
            // Not ours (cannot happen while installed; be a valid procedure
            // anyway rather than answer 0 to everything).
            // SAFETY: the system's default procedure for a live window.
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
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
        // An entry left by a window whose WM_NCDESTROY never reached us would
        // name a recycled HWND: replace it rather than skip this window.
        let previous = entry(hwnd);
        ENTRIES.with(|v| v.borrow_mut().retain(|x| x.hwnd != hwnd as isize));
        // SAFETY: `hwnd` is winit's live window, on its own thread.
        unsafe {
            let ours = proc as *const () as isize;
            let old = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, ours as _) as isize;
            let old_proc: WNDPROC = match previous {
                // Still subclassed by us (installed twice): chaining to
                // ourselves would recurse, so keep the procedure we wrapped.
                Some(prev) if old == ours => prev.old_proc,
                // SAFETY: a window procedure pointer, or 0 (`None`).
                _ => std::mem::transmute::<isize, WNDPROC>(old),
            };
            let entry = Entry {
                hwnd: hwnd as isize,
                old_proc,
                no_activate,
                tool_window,
            };
            ENTRIES.with(|v| v.borrow_mut().push(entry));
            // Write the adjusted style directly: Windows need not send
            // WM_STYLECHANGING for an unchanged value.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, adjust(ex, &entry) as _);
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
    fn a_non_activating_window_is_shown_inactive() {
        let w = WindowSteps::plan(&props(true, false), false, Backend::Windows);
        assert!(w.inactive && w.create_hidden && w.win_no_activate && !w.win_tool_window);
        let m = WindowSteps::plan(&props(true, false), false, Backend::MacOs);
        // Not created hidden: winit-appkit's `set_visible(true)` is
        // `makeKeyAndOrderFront`, which would make the panel key (review F1).
        assert!(m.inactive && !m.create_hidden && m.mac_panel);
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
        let attrs =
            apply_to_attributes(WindowAttributes::default(), steps, Backend::X11, None, None);
        assert!(!attrs.active);
        assert!(!attrs.visible);
        let plain = apply_to_attributes(
            WindowAttributes::default(),
            WindowSteps::default(),
            Backend::X11,
            None,
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

    /// The X11 window id behind a winit window, for the X tests.
    #[cfg(target_os = "linux")]
    fn test_xid(w: &dyn Window) -> u32 {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match w.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            other => panic!("not an X11 window: {other:?}"),
        }
    }

    /// Real winit X11 windows (never mapped: this may run on someone's
    /// desktop), created by an `ApplicationHandler` that hands each one to
    /// `make`. Built from the review of PR #1525.
    #[cfg(target_os = "linux")]
    fn with_x11_windows(
        make: impl FnMut(&dyn ActiveEventLoop) -> Vec<std::sync::Arc<dyn Window>> + 'static,
    ) -> Vec<std::sync::Arc<dyn Window>> {
        use std::time::Duration;
        use winit::application::ApplicationHandler;
        use winit::event::WindowEvent;
        use winit::event_loop::pump_events::EventLoopExtPumpEvents;
        use winit::platform::x11::EventLoopBuilderExtX11;
        use winit::window::WindowId;

        struct P<F> {
            make: F,
            wins: Vec<std::sync::Arc<dyn Window>>,
            done: bool,
        }
        impl<F: FnMut(&dyn ActiveEventLoop) -> Vec<std::sync::Arc<dyn Window>>> ApplicationHandler
            for P<F>
        {
            fn can_create_surfaces(&mut self, el: &dyn ActiveEventLoop) {
                if !self.done {
                    assert_eq!(Backend::of(el), Backend::X11);
                    self.wins = (self.make)(el);
                    self.done = true;
                }
            }
            fn window_event(&mut self, _: &dyn ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let mut el = EventLoop::builder()
            .with_x11()
            .with_any_thread(true)
            .build()
            .expect("an X server");
        let mut p = P {
            make,
            wins: Vec::new(),
            done: false,
        };
        for _ in 0..20 {
            el.pump_app_events(Some(Duration::from_millis(20)), &mut p);
            if p.done {
                break;
            }
        }
        // Let winit flush its own connection's requests.
        for _ in 0..3 {
            el.pump_app_events(Some(Duration::from_millis(20)), &mut p);
        }
        assert!(p.done, "the event loop never offered to create surfaces");
        std::mem::take(&mut p.wins)
    }

    /// Real winit X11 windows, as the runtime creates them (one event loop:
    /// winit allows one per process, so this is one test).
    ///
    /// - F2 of the review of PR #1525: click-through must give the window an
    ///   empty input shape with rinch's software renderer attached. winit's
    ///   own `set_cursor_hittest` sends XFixes requests without negotiating
    ///   the version and is refused (the window kept 1 rectangle). Turning it
    ///   off restores the default (one rectangle, the window).
    /// - An overlay carries `WM_HINTS.input = False` and its explicit
    ///   `app_id` as `WM_CLASS`; an ordinary window's `WM_CLASS` is left to
    ///   winit (`argv[0]`), as before #1519.
    ///
    /// `cargo test -p rinch --lib overlay_window -- --ignored`.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs an X server (DISPLAY)"]
    fn x11_overlay_windows_on_a_real_server() {
        use std::sync::Arc;
        use x11rb::properties::WmHints;
        use x11rb::protocol::shape::{ConnectionExt as _, SK};
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

        let mut renderers = Vec::new();
        let wins = with_x11_windows(move |el| {
            let size = winit::dpi::PhysicalSize::new(100u32, 100u32);
            let a = WindowAttributes::default()
                .with_visible(false)
                .with_surface_size(size);
            let soft: Arc<dyn Window> = Arc::from(el.create_window(a).unwrap());
            renderers.push(crate::shell::softbuffer_renderer::SoftbufferRenderer::new(
                soft.clone(),
                100,
                100,
                false,
            ));

            let steps = WindowSteps::plan(&props(true, true), false, Backend::X11);
            let a = apply_to_attributes(
                WindowAttributes::default().with_surface_size(size),
                steps,
                Backend::X11,
                Some("ignored-on-x11"),
                Some("probe-app-id"),
            );
            assert!(!a.visible, "created hidden");
            let overlay: Arc<dyn Window> = Arc::from(el.create_window(a).unwrap());
            x11::set_hints(&*overlay, steps.x11_no_input, steps.x11_skip_taskbar).unwrap();

            let plain = apply_to_attributes(
                WindowAttributes::default()
                    .with_surface_size(size)
                    .with_visible(false),
                WindowSteps::default(),
                Backend::X11,
                Some("resolved-app-id"),
                None,
            );
            let plain: Arc<dyn Window> = Arc::from(el.create_window(plain).unwrap());
            vec![soft, overlay, plain]
        });
        let (conn, _) = x11rb::connect(None).unwrap();

        let w = &*wins[0];
        let rects = || {
            conn.get_input_focus().unwrap().reply().unwrap();
            conn.shape_get_rectangles(test_xid(w), SK::INPUT)
                .unwrap()
                .reply()
                .unwrap()
                .rectangles
                .len()
        };
        assert_eq!(rects(), 1, "a new window takes the pointer everywhere");
        set_click_through(w, true);
        assert_eq!(rects(), 0, "click-through: an empty input shape");
        set_click_through(w, false);
        assert_eq!(rects(), 1, "off again: the window's own shape");
        set_click_through(w, true);
        assert_eq!(rects(), 0, "and on again");

        let wm_class = |id| {
            let c = conn
                .get_property(false, id, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                .unwrap()
                .reply()
                .unwrap();
            String::from_utf8_lossy(&c.value).into_owned()
        };
        let overlay = test_xid(&*wins[1]);
        assert_eq!(wm_class(overlay), "probe-app-id\0probe-app-id\0");
        let plain = wm_class(test_xid(&*wins[2]));
        assert!(
            !plain.is_empty() && !plain.contains("resolved-app-id"),
            "{plain:?}"
        );
        let hints = WmHints::get(&conn, overlay)
            .unwrap()
            .reply()
            .unwrap()
            .unwrap();
        assert_eq!(hints.input, Some(false));
    }

    /// F4 of the review: a `DISPLAY` that names no server falls back to the
    /// default backend instead of aborting.
    #[test]
    fn a_refused_x11_connection_falls_back_to_the_default_backend() {
        let refused = build_event_loop(true, || Err::<&str, _>("XOpenDisplayFailed"), || "default");
        assert_eq!(refused, "default");
        assert_eq!(
            build_event_loop(true, || Ok::<_, &str>("x11"), || "default"),
            "x11"
        );
        assert_eq!(
            build_event_loop(false, || Ok::<_, &str>("x11"), || "default"),
            "default",
            "not asked: X11 is not even tried"
        );
    }

    #[test]
    fn after_create_does_what_the_steps_say_in_order() {
        // An ordinary window: nothing at all (no hit-test call either).
        assert!(after_create_actions(WindowSteps::default(), true).is_empty());
        let x = WindowSteps::plan(&props(true, true), true, Backend::X11);
        assert_eq!(
            after_create_actions(x, true),
            vec![
                AfterCreate::X11Hints {
                    no_input: true,
                    skip_taskbar: true
                },
                AfterCreate::ClickThrough,
                AfterCreate::Show,
                AfterCreate::ReassertOnTop,
            ]
        );
        let w = WindowSteps::plan(&props(true, false), false, Backend::Windows);
        assert_eq!(
            after_create_actions(w, false),
            vec![
                AfterCreate::Win32Install {
                    no_activate: true,
                    tool_window: false
                },
                AfterCreate::Show,
            ]
        );
        // macOS is never re-shown through set_visible (makeKeyAndOrderFront).
        let m = WindowSteps::plan(&props(true, true), false, Backend::MacOs);
        assert_eq!(after_create_actions(m, true), vec![AfterCreate::MacPanel]);
        let wl = WindowSteps::plan(&props(true, false), true, Backend::Wayland);
        assert_eq!(
            after_create_actions(wl, true),
            vec![AfterCreate::WarnWayland, AfterCreate::ClickThrough]
        );
    }
}
