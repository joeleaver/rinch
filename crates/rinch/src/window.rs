//! Window-level facts and acts that work the same on the desktop and in the
//! browser (issue #1770): opening a URL in the system browser, the window's
//! title, its display scale, its size and whether it has focus.
//!
//! The three facts are **reactive**: [`scale_factor`], [`window_size`] and
//! [`window_focused`] read a signal, so a `{|| …}` expression, a [`Memo`] or an
//! [`Effect`] that reads one runs again when it changes — that is how an app
//! subscribes to a resize, a blur or a move to a display with another scale.
//! A value equal to the last one wakes nothing.
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! #[component]
//! fn status() -> NodeHandle {
//!     rsx! {
//!         p { {|| format!("{:?} at {}x", window_size(), scale_factor())} }
//!         p { {|| if window_focused() { "focused" } else { "in the background" }} }
//!         button { onclick: || { let _ = open_url("https://example.com"); }, "Docs" }
//!     }
//! }
//! ```
//!
//! Where the values come from:
//!
//! - **Desktop**: the main window — published by the runtime at mount and on
//!   every resize, scale change and focus change (`RinchApp`'s
//!   [`set_publishes_window_state`](crate::app::RinchApp::set_publishes_window_state);
//!   the DevTools window does not publish). A window hidden to the tray reads
//!   as not focused.
//! - **Web**: the page — `innerWidth`/`innerHeight`, `devicePixelRatio` and the
//!   window's own `focus`/`blur`, read by listeners installed on the first read.
//! - **Android**: published by the Android runtime as on the desktop.
//! - **Native embed** (`RinchContext`): nothing publishes — the host owns the
//!   window — so the getters answer their defaults (`1.0`, `(0.0, 0.0)`,
//!   `false`) unless the host's `RinchApp` turns publishing on.
//!
//! The getters are for the UI thread (they read signals, which belong to the
//! thread): read on another thread they answer that thread's defaults, and
//! say so once per thread in a `tracing` warning. [`set_window_title`] may be
//! called from any thread on the desktop.
//!
//! [`Memo`]: rinch_core::Memo
//! [`Effect`]: rinch_core::Effect

use std::cell::Cell;

use rinch_core::Signal;

/// Why [`open_url`] did not open the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenUrlError {
    /// The string is not a URL rinch will hand on: it has no scheme
    /// (`https:`, `mailto:`, `file:` …), its "scheme" is one letter (a Windows
    /// path, `C:\…`, which the system would open as a program), it starts with
    /// `-` (it would read as an option to the desktop launcher), or it holds a
    /// control character.
    Invalid,
    /// The URL's scheme runs code rather than naming something to open:
    /// `javascript:`, `vbscript:` or `data:`. In a browser such a URL would run
    /// in (or with the reach of) the app's own page — `window.open` gives a
    /// `javascript:` URL the opener's origin — so it is refused everywhere.
    ScriptScheme,
    /// This platform has no way to open a URL from rinch (Android, for now).
    Unsupported,
    /// The browser refused to open a new tab — a popup blocker, usually
    /// because the call was not made from a user gesture (a click handler).
    Blocked,
    /// The desktop launcher (`xdg-open`, `open`, `ShellExecuteW`) could not be
    /// started; the text says why.
    Launch(String),
}

impl std::fmt::Display for OpenUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenUrlError::Invalid => f.write_str("not a URL with a scheme"),
            OpenUrlError::ScriptScheme => {
                f.write_str("javascript:, vbscript: and data: URLs are not opened")
            }
            OpenUrlError::Unsupported => f.write_str("opening a URL is not supported here"),
            OpenUrlError::Blocked => f.write_str("the browser blocked the new tab"),
            OpenUrlError::Launch(why) => write!(f, "could not start the URL launcher: {why}"),
        }
    }
}

impl std::error::Error for OpenUrlError {}

/// Whether [`open_url`] hands `url` on: a scheme of two or more characters
/// (RFC 3986: a letter, then letters, digits, `+`, `-` or `.`, then `:`; a
/// one-letter one is a Windows drive), no leading `-`, no control character,
/// and not a scheme that runs code.
fn check_url(url: &str) -> Result<(), OpenUrlError> {
    if url.starts_with('-') || url.chars().any(char::is_control) {
        return Err(OpenUrlError::Invalid);
    }
    let Some((scheme, _)) = url.split_once(':') else {
        return Err(OpenUrlError::Invalid);
    };
    let mut chars = scheme.chars();
    let well_formed = scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !well_formed {
        return Err(OpenUrlError::Invalid);
    }
    if ["javascript", "vbscript", "data"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
    {
        return Err(OpenUrlError::ScriptScheme);
    }
    Ok(())
}

/// Open `url` in the system's default handler: the default browser for
/// `http(s):`, the mail client for `mailto:`, and so on.
///
/// - **Desktop**: `xdg-open` (Linux and the BSDs), `open` (macOS) or
///   `ShellExecuteW` (Windows), started without waiting for it. `Ok` means the
///   launcher started; whether a handler then opened the URL is not reported.
/// - **Web**: a new tab (`window.open(url, "_blank")`, with `opener` cleared so
///   the page cannot reach back). Browsers allow this only during a user
///   gesture; outside one it is usually [`OpenUrlError::Blocked`].
/// - **Android**: [`OpenUrlError::Unsupported`].
///
/// Refused on every target: what is not a URL ([`OpenUrlError::Invalid`],
/// a one-letter "scheme" — a Windows path — included) and `javascript:`,
/// `vbscript:` and `data:` ([`OpenUrlError::ScriptScheme`]).
///
/// **Everything else is handed to the system as given, and the system may run
/// a program with it.** A `file:` URL opens that file with its default
/// application — an executable or a script runs; a custom scheme
/// (`steam:`, `ms-settings:`, an app's own) goes to whatever program
/// registered it, with the rest of the URL as its input. So never pass a URL
/// an untrusted party chose (a link in a document, a message, a web response)
/// without checking that its scheme is one you mean to open — usually just
/// `https:` (and `http:`, `mailto:`).
pub fn open_url(url: &str) -> Result<(), OpenUrlError> {
    check_url(url)?;
    platform::open_url(url)
}

/// Set the title of the current window: the main window's title bar (and
/// taskbar entry) on the desktop, `document.title` on the web. Replaces the
/// title given at startup (`App::title`), and is kept when a window hidden to
/// the tray is shown again.
///
/// On the desktop it may be called from any thread and is applied on the event
/// loop's next turn (before the loop starts: when the window is created). The
/// title is stored, not queued: with no desktop window to apply it to (an embed
/// host, which owns its window and its title) it only replaces the stored
/// one. A no-op on Android.
pub fn set_window_title(title: &str) {
    platform::set_window_title(title);
}

/// The current window's display scale — physical pixels per logical (CSS)
/// pixel: `2.0` on a typical HiDPI display, the page's `devicePixelRatio` on
/// the web (browser zoom included). Reactive: a reader re-runs when the window
/// moves to a display with another scale or the scale is changed.
///
/// `1.0` until something publishes it (see the [module docs](self)).
pub fn scale_factor() -> f64 {
    signals().scale.get()
}

/// The current window's content-area size in logical (CSS) pixels, `(width,
/// height)` — the viewport the document is laid out in. Reactive: a reader
/// re-runs when the window is resized.
///
/// `(0.0, 0.0)` until something publishes it (see the [module docs](self)).
/// On the desktop, [`current_window_size`](crate::windows::current_window_size)
/// answers the same size from any thread, without subscribing.
pub fn window_size() -> (f64, f64) {
    signals().size.get()
}

/// Whether the current window has the keyboard focus of the system — `false`
/// once the user switches to another window or tab. Reactive: a reader re-runs
/// on every focus change, which is how an app hears that the window lost focus
/// (to pause, to save).
///
/// This is the window's focus, not an element's: moving focus between fields
/// inside the window does not change it.
pub fn window_focused() -> bool {
    signals().focused.get()
}

#[derive(Clone, Copy)]
struct WindowSignals {
    scale: Signal<f64>,
    size: Signal<(f64, f64)>,
    focused: Signal<bool>,
}

thread_local! {
    static SIGNALS: Cell<Option<WindowSignals>> = const { Cell::new(None) };
}

/// The thread's window signals, made on first use. They belong to no scope
/// (`unowned`): they live as long as the thread, whichever component first
/// asked.
fn signals() -> WindowSignals {
    if let Some(s) = SIGNALS.with(Cell::get) {
        return s;
    }
    if !rinch_core::is_main_thread() {
        tracing::warn!(
            "rinch::window was read off the UI thread: the window facts belong to the \
             UI thread, so this thread sees its own defaults"
        );
    }
    let initial = platform::initial_state();
    let s = rinch_core::unowned(|| WindowSignals {
        scale: Signal::new(initial.scale),
        size: Signal::new(initial.size),
        focused: Signal::new(initial.focused),
    });
    SIGNALS.with(|c| c.set(Some(s)));
    platform::watch();
    s
}

// Unused only in a native build with no runtime to publish (no `desktop`,
// `android` or `embed`): nothing there has a window.
#[cfg_attr(
    not(any(
        feature = "desktop",
        feature = "android",
        feature = "embed",
        target_arch = "wasm32"
    )),
    allow(dead_code)
)]
/// A window state to publish; `None` fields are left as they are.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct WindowFacts {
    pub scale: Option<f64>,
    pub size: Option<(f64, f64)>,
    pub focused: Option<bool>,
}

// Unused only in a native build with no runtime to publish (no `desktop`,
// `android` or `embed`): nothing there has a window.
#[cfg_attr(
    not(any(
        feature = "desktop",
        feature = "android",
        feature = "embed",
        target_arch = "wasm32"
    )),
    allow(dead_code)
)]
/// Write `state` into the window signals, as one batch: readers of several of
/// them run once. Equal values wake nothing.
pub(crate) fn publish(state: WindowFacts) {
    let s = signals();
    rinch_core::batch(|| {
        if let Some(scale) = state.scale {
            s.scale.set_if_changed(scale);
        }
        if let Some(size) = state.size {
            s.size.set_if_changed(size);
        }
        if let Some(focused) = state.focused {
            s.focused.set_if_changed(focused);
        }
    });
}

struct Initial {
    scale: f64,
    size: (f64, f64),
    focused: bool,
}

/// The title [`set_window_title`] stored on the desktop (#1770): taken by the
/// shell when it creates its window and on each `ApplyWindowTitle`.
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
static STORED_TITLE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Take the title [`set_window_title`] stored, if any.
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub(crate) fn take_stored_title() -> Option<String> {
    STORED_TITLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use super::{Initial, OpenUrlError};

    pub(super) fn initial_state() -> Initial {
        Initial {
            scale: 1.0,
            size: (0.0, 0.0),
            focused: false,
        }
    }

    /// Nothing to watch: the runtime publishes.
    pub(super) fn watch() {}

    pub(super) fn set_window_title(title: &str) {
        #[cfg(feature = "desktop")]
        {
            *super::STORED_TITLE
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(title.to_string());
            // Wake a running shell to apply it; before the loop starts (or
            // with no shell at all) nothing is queued: the window takes it
            // when it is created.
            if crate::shell::rinch_runtime::GLOBAL_PROXY.get().is_some() {
                crate::shell::rinch_runtime::send_native_event(
                    crate::shell::rinch_runtime::RinchNativeEvent::ApplyWindowTitle,
                );
            }
        }
        #[cfg(not(feature = "desktop"))]
        let _ = title;
    }

    #[cfg(target_os = "android")]
    pub(super) fn open_url(_url: &str) -> Result<(), OpenUrlError> {
        Err(OpenUrlError::Unsupported)
    }

    #[cfg(windows)]
    pub(super) fn open_url(url: &str) -> Result<(), OpenUrlError> {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let verb = wide("open");
        let file = wide(url);
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
        // every other argument may be null.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW answers a value above 32 on success.
        if result as isize > 32 {
            Ok(())
        } else {
            Err(OpenUrlError::Launch(format!(
                "ShellExecuteW failed ({})",
                result as isize
            )))
        }
    }

    #[cfg(all(not(target_os = "android"), not(windows)))]
    pub(super) fn open_url(url: &str) -> Result<(), OpenUrlError> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        launch(std::ffi::OsStr::new(program), url)
    }

    /// Start `program url` with no stdio and reap it on a thread of its own,
    /// so the caller does not wait and no zombie is left behind.
    #[cfg(all(not(target_os = "android"), not(windows)))]
    pub(super) fn launch(program: &std::ffi::OsStr, url: &str) -> Result<(), OpenUrlError> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| OpenUrlError::Launch(format!("{}: {e}", program.to_string_lossy())))?;
        std::thread::Builder::new()
            .name("rinch-open-url".into())
            .spawn(move || {
                let _ = child.wait();
            })
            .map_err(|e| OpenUrlError::Launch(e.to_string()))?;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use super::{Initial, OpenUrlError, WindowFacts, publish};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::Closure;

    fn size(window: &web_sys::Window) -> (f64, f64) {
        let read = |v: Result<wasm_bindgen::JsValue, _>| v.ok().and_then(|v| v.as_f64());
        (
            read(window.inner_width()).unwrap_or(0.0),
            read(window.inner_height()).unwrap_or(0.0),
        )
    }

    pub(super) fn initial_state() -> Initial {
        let Some(window) = web_sys::window() else {
            return Initial {
                scale: 1.0,
                size: (0.0, 0.0),
                focused: false,
            };
        };
        let focused = window
            .document()
            .and_then(|d| d.has_focus().ok())
            .unwrap_or(false);
        Initial {
            scale: window.device_pixel_ratio(),
            size: size(&window),
            focused,
        }
    }

    /// Install the page listeners, once per thread (the signals are made
    /// once). The closures live as long as the page.
    pub(super) fn watch() {
        let Some(window) = web_sys::window() else {
            return;
        };
        let on_resize = Closure::<dyn Fn()>::new(|| {
            if let Some(window) = web_sys::window() {
                // A zoom changes `devicePixelRatio` and fires `resize` too.
                publish(WindowFacts {
                    scale: Some(window.device_pixel_ratio()),
                    size: Some(size(&window)),
                    focused: None,
                });
            }
        });
        let _ =
            window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        on_resize.forget();
        for (event, focused) in [("focus", true), ("blur", false)] {
            // Bubble phase on `window`: an element's focus and blur do not
            // bubble, so only the window's own reach here.
            let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
                let at_window = e
                    .target()
                    .is_some_and(|t| t.dyn_ref::<web_sys::Window>().is_some());
                if at_window {
                    publish(WindowFacts {
                        focused: Some(focused),
                        ..WindowFacts::default()
                    });
                }
            });
            let _ =
                window.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
            listener.forget();
        }
        watch_resolution(&window);
    }

    /// `devicePixelRatio` changes without a `resize` when the window moves to
    /// a display with another scale: watch a media query for the current
    /// value, and arm a new one at the new value each time it stops matching.
    fn watch_resolution(window: &web_sys::Window) {
        let dpr = window.device_pixel_ratio();
        let Ok(Some(query)) = window.match_media(&format!("(resolution: {dpr}dppx)")) else {
            return;
        };
        let options = web_sys::AddEventListenerOptions::new();
        options.set_once(true);
        let on_change = Closure::once_into_js(move || {
            if let Some(window) = web_sys::window() {
                publish(WindowFacts {
                    scale: Some(window.device_pixel_ratio()),
                    ..WindowFacts::default()
                });
                watch_resolution(&window);
            }
        });
        let _ = query.add_event_listener_with_callback_and_add_event_listener_options(
            "change",
            on_change.unchecked_ref(),
            &options,
        );
    }

    pub(super) fn set_window_title(title: &str) {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            document.set_title(title);
        }
    }

    pub(super) fn open_url(url: &str) -> Result<(), OpenUrlError> {
        let window = web_sys::window().ok_or(OpenUrlError::Unsupported)?;
        match window.open_with_url_and_target(url, "_blank") {
            Ok(Some(opened)) => {
                // The new page cannot reach back through `window.opener`
                // (what `rel="noopener"` does for a link; the `noopener`
                // feature would make `open` answer null, hiding a block).
                let _ = opened.set_opener(&wasm_bindgen::JsValue::NULL);
                Ok(())
            }
            Ok(None) => Err(OpenUrlError::Blocked),
            Err(e) => Err(OpenUrlError::Launch(format!("{e:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn a_url_needs_a_scheme_and_no_leading_dash() {
        for ok in [
            "https://example.com/a?b=c&d",
            "mailto:a@b.c",
            "file:///tmp/x",
            "x-app+v1.2:thing",
            "js:x",
            "datax:y",
        ] {
            assert_eq!(check_url(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "example.com",
            "-https://x",
            "--help",
            "1http://x",
            ":nothing",
            "ht tp://x",
            "https://x\ny",
            "https://x\0",
            "C:\\Windows\\System32\\calc.exe",
            "c:/Windows/notepad.exe",
            "z:thing",
        ] {
            assert_eq!(open_url(bad), Err(OpenUrlError::Invalid), "{bad:?}");
        }
    }

    #[test]
    fn script_schemes_are_refused_in_any_case() {
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(localStorage.x)",
            "vbscript:msgbox(1)",
            "VBSCRIPT:x",
            "data:text/html,<script>alert(1)</script>",
            "DATA:text/html;base64,PHNjcmlwdD4=",
        ] {
            assert_eq!(open_url(bad), Err(OpenUrlError::ScriptScheme), "{bad:?}");
        }
    }

    /// The launcher is started with the URL as its one argument, unquoted and
    /// unsplit, and `open_url` returns before it exits.
    #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
    #[test]
    fn the_launcher_gets_the_url_as_its_one_argument() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rinch-open-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("args");
        let script = dir.join("fake-xdg-open");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep 1\nprintf '%s|' \"$#\" \"$@\" > {}.tmp && mv {0}.tmp {0}\n",
                out.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let url = "https://example.com/a b?c=1&d='x'";
        let started = std::time::Instant::now();
        platform::launch(script.as_os_str(), url).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(900),
            "open_url waited for the launcher"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !out.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let args = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(args, format!("1|{url}|"));
    }

    #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
    #[test]
    fn a_launcher_that_cannot_start_is_an_error() {
        let missing = std::ffi::OsStr::new("/nonexistent/rinch-no-such-launcher");
        assert!(matches!(
            platform::launch(missing, "https://example.com"),
            Err(OpenUrlError::Launch(_))
        ));
    }

    /// A reader runs again for a published change, once per batch, and not
    /// for an equal value.
    #[test]
    fn readers_rerun_for_a_change_and_not_for_an_equal_value() {
        let runs = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new((0.0, (0.0, 0.0), true)));
        let _effect = {
            let (runs, seen) = (runs.clone(), seen.clone());
            rinch_core::Effect::new(move || {
                runs.set(runs.get() + 1);
                seen.set((scale_factor(), window_size(), window_focused()));
            })
        };
        assert_eq!(runs.get(), 1);
        assert_eq!(seen.get(), (1.0, (0.0, 0.0), false));

        publish(WindowFacts {
            scale: Some(1.5),
            size: Some((640.0, 480.0)),
            focused: Some(true),
        });
        assert_eq!(runs.get(), 2, "one run for three changes in one batch");
        assert_eq!(seen.get(), (1.5, (640.0, 480.0), true));

        publish(WindowFacts {
            scale: Some(1.5),
            size: Some((640.0, 480.0)),
            focused: Some(true),
        });
        assert_eq!(runs.get(), 2, "equal values wake nothing");

        publish(WindowFacts {
            focused: Some(false),
            ..WindowFacts::default()
        });
        assert_eq!(runs.get(), 3);
        assert_eq!(seen.get(), (1.5, (640.0, 480.0), false));
    }
}
