//! Operating-system notifications (feature `notifications`, issue #1522).
//!
//! ```ignore
//! use rinch::notification::Notification;
//!
//! Notification::new("Hotkey unavailable")
//!     .body("Ctrl+Alt+Space is taken by another app.")
//!     .icon_png(include_bytes!("icon.png"))
//!     .on_click(|| show_current_window())
//!     .show()?;
//! ```
//!
//! | Platform | Mechanism |
//! |---|---|
//! | Linux / BSD desktops | `org.freedesktop.Notifications` over D-Bus (KDE, GNOME, dunst, mako, …) |
//! | Windows | WinRT toast notifications (`Windows.UI.Notifications`) |
//! | macOS | `UNUserNotificationCenter` (UserNotifications); needs an app bundle |
//! | anything else (Android) | [`NotificationError::Unsupported`] from [`Notification::show`] |
//!
//! The feature does not build for `wasm32`: it brings in the workspace's
//! tokio, whose multi-threaded runtime has no wasm support.
//!
//! # Threads and callbacks
//!
//! [`Notification::show`] makes no IPC call on the calling thread: the D-Bus
//! and WinRT calls run on a worker thread of their own (started on the first
//! `show`), and macOS's calls are asynchronous already. The only work it does
//! in place is writing an [`icon_png`](Notification::icon_png) image to a
//! file, once per distinct image. Call `show` on the main (UI) thread — the
//! callbacks are `!Send` and are kept there.
//!
//! [`on_click`](Notification::on_click), [`on_dismiss`](Notification::on_dismiss)
//! and [`on_error`](Notification::on_error) run on the main thread, each in one
//! [`batch`](rinch_core::reactive::batch), delivered through
//! [`rinch_core::dispatch_main_callback`]. They follow the menu and tray
//! callbacks' lifetime rule (#183): a callback set while a component renders
//! belongs to that component and is not called once it has unmounted; one set
//! outside any render (from `main`, a timer) keeps app lifetime.
//!
//! # Errors are never silent
//!
//! What can be known without talking to the platform is the `Err` of
//! [`Notification::show`]: an unsupported platform, an unbundled macOS app.
//! What only the platform can answer — no notification daemon on the session
//! bus, the user refused permission (macOS), a toast Windows would not show —
//! arrives later on the main thread, through [`Notification::on_error`]; a
//! notification with no `on_error` logs it with `tracing::warn!`.
//!
//! # App identity
//!
//! The desktop files a notification under the application that sent it. rinch
//! names the app with the same identity its window carries:
//! [`WindowProps::app_id`](rinch_core::element::WindowProps) when the window
//! sets one, else the executable's file name (without `.exe` on Windows) — the
//! Wayland `app_id` derivation. [`set_app_id`] overrides it.
//!
//! - **Linux:** sent as the `desktop-entry` hint, so the server can show the
//!   app's icon and name from `<app id>.desktop`, and as `app_name` (the
//!   [`set_app_name`] display name, else the app id). No file is written.
//! - **Windows:** the AppUserModelID the toasts are created with. An unpackaged
//!   app's AUMID must be registered for Windows to show its toasts with a name;
//!   on the first `show` rinch writes
//!   `HKCU\Software\Classes\AppUserModelId\<AUMID>` with a `DisplayName`
//!   (never replacing a key that already exists — an installer's own
//!   registration wins). [`set_register_app`]`(false)` turns that off. No
//!   Start-menu shortcut is needed or created. A toast clicked in the Action
//!   Center **after the app has exited** does nothing: relaunching the app on
//!   click needs a COM activator, which rinch does not register.
//!   An AUMID may not contain spaces, so an executable named `My App.exe`
//!   needs [`set_app_id`] (`"com.example.MyApp"` is the usual shape); that
//!   also keeps two unpackaged apps with the same file name apart, since
//!   rinch never rewrites the other one's registration.
//! - **macOS:** the bundle identifier. An executable run outside an `.app`
//!   bundle has none, and the UserNotifications framework refuses it, so
//!   `show` answers [`NotificationError::Unsupported`] there.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use rinch_core::reactive::{Owner, current_owner, unowned};

mod backend;

#[cfg(test)]
mod tests;

/// Why a notification went away without being clicked
/// ([`Notification::on_dismiss`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DismissReason {
    /// It timed out.
    Expired,
    /// The user dismissed it.
    Dismissed,
    /// The app closed it ([`NotificationHandle::close`]).
    Closed,
    /// The platform gave no reason.
    Unknown,
}

/// What went wrong with a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationError {
    /// This platform, or this build of the app, cannot show notifications
    /// (no backend on the target, an unbundled macOS executable).
    Unsupported(String),
    /// Nothing is there to show it: no D-Bus session bus, or no notification
    /// server (`org.freedesktop.Notifications`) on it.
    NoService(String),
    /// The user has refused this app notifications (macOS).
    PermissionDenied,
    /// The platform refused it for another reason.
    Platform(String),
}

impl std::fmt::Display for NotificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotificationError::Unsupported(m) => write!(f, "notifications are unsupported: {m}"),
            NotificationError::NoService(m) => write!(f, "no notification service: {m}"),
            NotificationError::PermissionDenied => {
                write!(f, "the user has not allowed this app to show notifications")
            }
            NotificationError::Platform(m) => write!(f, "the notification failed: {m}"),
        }
    }
}

impl std::error::Error for NotificationError {}

/// A notification to show. Build it, then call [`show`](Notification::show).
pub struct Notification {
    title: String,
    body: String,
    icon_png: Option<Vec<u8>>,
    on_click: Option<Owned<dyn Fn()>>,
    on_dismiss: Option<Owned<dyn Fn(DismissReason)>>,
    on_error: Option<OnError>,
}

impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification")
            .field("title", &self.title)
            .field("body", &self.body)
            .field("icon_png", &self.icon_png.as_ref().map(Vec::len))
            .field("on_click", &self.on_click.is_some())
            .field("on_dismiss", &self.on_dismiss.is_some())
            .field("on_error", &self.on_error.is_some())
            .finish()
    }
}

/// An `on_error` callback.
type OnError = Owned<dyn Fn(&NotificationError)>;

/// A callback with the owner that was rendering when it was set.
struct Owned<F: ?Sized> {
    cb: Rc<F>,
    owner: Option<Owner>,
}

impl<F: ?Sized> Clone for Owned<F> {
    fn clone(&self) -> Self {
        Owned {
            cb: self.cb.clone(),
            owner: self.owner.clone(),
        }
    }
}

impl<F: ?Sized> Owned<F> {
    fn new(cb: Rc<F>) -> Self {
        Owned {
            cb,
            owner: current_owner(),
        }
    }

    fn is_dead(&self) -> bool {
        self.owner.as_ref().is_some_and(|o| !o.is_alive())
    }

    /// Run `call` with the callback inside its owner, in one batch, as a menu
    /// callback runs (`menu::invoke_menu_callback`). A dead owner runs
    /// nothing.
    fn invoke(&self, call: impl FnOnce(&F)) -> bool {
        match &self.owner {
            Some(owner) if !owner.is_alive() => false,
            Some(owner) => {
                rinch_core::reactive::batch(|| owner.run(|| call(&self.cb)));
                true
            }
            None => {
                rinch_core::reactive::batch(|| unowned(|| call(&self.cb)));
                true
            }
        }
    }
}

impl Notification {
    /// A notification with this title (the "summary" on Linux).
    pub fn new(title: impl Into<String>) -> Self {
        Notification {
            title: title.into(),
            body: String::new(),
            icon_png: None,
            on_click: None,
            on_dismiss: None,
            on_error: None,
        }
    }

    /// The text under the title. Plain text: no markup is interpreted.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// An image shown with the notification, as PNG bytes. rinch writes it to
    /// a file in a directory only this user can write (`$XDG_RUNTIME_DIR` on
    /// Linux, else the user's cache directory; the per-user temporary
    /// directory on macOS and Windows) and hands the platform its path
    /// (the `image-path` hint on Linux, the app-logo override on Windows, an
    /// attachment on macOS). Without one, the platform shows the app's icon.
    pub fn icon_png(mut self, png: &[u8]) -> Self {
        self.icon_png = Some(png.to_vec());
        self
    }

    /// Run `f` on the main thread when the user clicks the notification.
    ///
    /// On Linux this needs a server that supports actions (KDE and GNOME do).
    /// On Windows a click is reported while the app is running and the toast
    /// is up; once it has timed out into the Action Center
    /// ([`DismissReason::Expired`] has run), a click there is not reported.
    /// On macOS a click also brings the app forward; a click that launches
    /// the app is not reported.
    pub fn on_click(mut self, f: impl Fn() + 'static) -> Self {
        self.on_click = Some(Owned::new(Rc::new(f)));
        self
    }

    /// Run `f` on the main thread when the notification goes away **without**
    /// being clicked: it expired, the user dismissed it, or the app closed it.
    ///
    /// Linux and Windows report this; macOS does not report a dismissal (a
    /// [`NotificationHandle::close`] is reported as [`DismissReason::Closed`]
    /// on every platform).
    pub fn on_dismiss(mut self, f: impl Fn(DismissReason) + 'static) -> Self {
        self.on_dismiss = Some(Owned::new(Rc::new(f)));
        self
    }

    /// Run `f` on the main thread if the platform refuses the notification
    /// after [`show`](Notification::show) returned: no notification server,
    /// permission denied, a toast Windows would not show. Without it, such an
    /// error is logged with `tracing::warn!`.
    pub fn on_error(mut self, f: impl Fn(&NotificationError) + 'static) -> Self {
        self.on_error = Some(Owned::new(Rc::new(f)));
        self
    }

    /// Show it. Returns as soon as the request is queued; no IPC call runs on
    /// the calling thread.
    ///
    /// `Err` is what is known at once (see the module docs); a refusal only
    /// the platform can give arrives through [`on_error`](Self::on_error).
    pub fn show(self) -> Result<NotificationHandle, NotificationError> {
        show_with(backend::current(), self)
    }
}

/// A shown notification. Dropping it leaves the notification up and its
/// callbacks registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationHandle {
    id: u64,
}

impl NotificationHandle {
    /// Take the notification down. Its `on_dismiss` runs with
    /// [`DismissReason::Closed`] (unless it was clicked already, which ends
    /// its callbacks).
    pub fn close(&self) {
        if let Some(backend) = backend::current_if_started() {
            backend.close(self.id);
        }
    }

    /// An id unique within this process.
    pub fn id(&self) -> u64 {
        self.id
    }
}

// ── What a backend is handed ────────────────────────────────────────────────

/// A notification as a backend sees it: everything `Send`.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) icon: Option<std::path::PathBuf>,
    /// Whether anyone listens for a click (Linux asks for the default action
    /// only then).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) wants_click: bool,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) app_id: String,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) app_name: String,
}

/// What a backend reports about a notification, from any thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Clicked,
    Dismissed(DismissReason),
    Failed(NotificationError),
}

/// A backend: shows and closes, and reports [`Event`]s through the sink it
/// was made with.
pub(crate) trait Backend: Send + Sync {
    /// Queue `req`. `Err` only for what is known without asking the platform.
    fn show(&self, req: Request) -> Result<(), NotificationError>;
    fn close(&self, id: u64);
}

/// Where a backend reports: the production sink hops to the main thread.
pub(crate) type Sink = std::sync::Arc<dyn Fn(u64, Event) + Send + Sync>;

pub(crate) fn main_thread_sink() -> Sink {
    std::sync::Arc::new(|id, ev| {
        rinch_core::dispatch_main_callback(Box::new(move || deliver(id, ev)));
    })
}

// ── The main-thread half: callbacks by id ───────────────────────────────────

struct Entry {
    on_click: Option<Owned<dyn Fn()>>,
    on_dismiss: Option<Owned<dyn Fn(DismissReason)>>,
    on_error: Option<OnError>,
}

impl Entry {
    fn is_dead(&self) -> bool {
        [
            self.on_click.as_ref().map(Owned::is_dead),
            self.on_dismiss.as_ref().map(Owned::is_dead),
            self.on_error.as_ref().map(Owned::is_dead),
        ]
        .into_iter()
        .flatten()
        .all(|dead| dead)
    }
}

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
}

/// Process-wide, so an id names one notification whichever thread showed it
/// (a backend's maps are keyed on it).
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn show_with(
    backend: Result<&'static dyn Backend, NotificationError>,
    n: Notification,
) -> Result<NotificationHandle, NotificationError> {
    let backend = backend?;
    let id = next_id();
    let icon = match &n.icon_png {
        Some(png) => Some(write_icon(png).map_err(NotificationError::Platform)?),
        None => None,
    };
    let req = Request {
        id,
        title: n.title,
        body: n.body,
        icon,
        wants_click: n.on_click.is_some(),
        app_id: app_id(),
        app_name: app_name(),
    };
    // Prune entries whose components are gone (a server that never reports
    // a close would otherwise keep them for good).
    CALLBACKS.with(|c| c.borrow_mut().retain(|_, e| !e.is_dead()));
    if n.on_click.is_some() || n.on_dismiss.is_some() || n.on_error.is_some() {
        CALLBACKS.with(|c| {
            c.borrow_mut().insert(
                id,
                Entry {
                    on_click: n.on_click,
                    on_dismiss: n.on_dismiss,
                    on_error: n.on_error,
                },
            )
        });
    }
    if let Err(e) = backend.show(req) {
        CALLBACKS.with(|c| c.borrow_mut().remove(&id));
        return Err(e);
    }
    Ok(NotificationHandle { id })
}

/// Run what `ev` calls for, on the main thread. Every event ends the entry:
/// a click is not followed by a dismissal (the close a Linux server reports
/// after an action finds nothing), and nothing follows a failure.
pub(crate) fn deliver(id: u64, ev: Event) {
    // Clone the callback out before calling it: it may show another
    // notification, which borrows the map.
    enum Call {
        Click(Owned<dyn Fn()>),
        Dismiss(Owned<dyn Fn(DismissReason)>, DismissReason),
        Error(OnError, NotificationError),
        Warn(NotificationError),
    }
    let call = CALLBACKS.with(|c| {
        let mut map = c.borrow_mut();
        match ev {
            Event::Clicked => map.remove(&id)?.on_click.map(Call::Click),
            Event::Dismissed(reason) => map
                .remove(&id)?
                .on_dismiss
                .map(|cb| Call::Dismiss(cb, reason)),
            Event::Failed(err) => match map.remove(&id) {
                Some(Entry {
                    on_error: Some(cb), ..
                }) => Some(Call::Error(cb, err)),
                _ => Some(Call::Warn(err)),
            },
        }
    });
    match call {
        Some(Call::Click(cb)) => {
            cb.invoke(|f| f());
        }
        Some(Call::Dismiss(cb, reason)) => {
            cb.invoke(|f| f(reason));
        }
        Some(Call::Error(cb, err)) => {
            if !cb.invoke(|f| f(&err)) {
                warn_unhandled(id, &err);
            }
        }
        Some(Call::Warn(err)) => warn_unhandled(id, &err),
        None => {}
    }
}

/// An error nobody handles is logged, never dropped.
fn warn_unhandled(id: u64, err: &NotificationError) {
    tracing::warn!("notification {id}: {err}");
    #[cfg(test)]
    UNHANDLED.with(|u| u.set(u.get() + 1));
}

#[cfg(test)]
thread_local! {
    /// How many errors [`warn_unhandled`] logged on this thread.
    pub(crate) static UNHANDLED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn registered_count() -> usize {
    CALLBACKS.with(|c| c.borrow().len())
}

// ── App identity ────────────────────────────────────────────────────────────

static APP_ID: Mutex<Option<String>> = Mutex::new(None);
static APP_NAME: Mutex<Option<String>> = Mutex::new(None);
static REGISTER_APP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Name the application to the notification system: the freedesktop
/// `desktop-entry` hint, the Windows AppUserModelID. It defaults to the id
/// the window carries (`WindowProps::app_id`, else the executable's name), so
/// set it only when a notification can be shown before the window exists
/// and the window gets an explicit id, or to match an installer's AUMID.
/// Ignored on macOS, where the bundle identifier is the identity.
pub fn set_app_id(id: impl Into<String>) {
    *APP_ID.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.into());
}

/// The name shown as the notification's source: the freedesktop `app_name`,
/// the `DisplayName` rinch registers for the Windows AUMID. Defaults to the
/// app id.
pub fn set_app_name(name: impl Into<String>) {
    *APP_NAME.lock().unwrap_or_else(|p| p.into_inner()) = Some(name.into());
}

/// Whether rinch may register the app's AppUserModelID under
/// `HKCU\Software\Classes\AppUserModelId` on Windows (on by default). It is
/// written once, on the first notification, and never over a key that
/// exists. Turn it off when an installer registers the AUMID (or a Start-menu
/// shortcut carries it); Windows may then show no toast for an unregistered
/// id. No effect elsewhere.
pub fn set_register_app(allowed: bool) {
    REGISTER_APP.store(allowed, std::sync::atomic::Ordering::Relaxed);
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn may_register_app() -> bool {
    REGISTER_APP.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn app_id() -> String {
    if let Some(id) = APP_ID.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        return id;
    }
    #[cfg(all(target_os = "linux", feature = "desktop"))]
    {
        crate::shell::rinch_runtime::current_app_id()
    }
    #[cfg(not(all(target_os = "linux", feature = "desktop")))]
    {
        std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(app_id_from_exe_path)
            .unwrap_or_else(|| "rinch-app".to_string())
    }
}

fn app_name() -> String {
    APP_NAME
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(app_id)
}

/// The id an executable path implies: its whole file name (a reverse-DNS
/// name keeps its dots, as `resolved_app_id` keeps them), less a Windows
/// `.exe`.
#[cfg_attr(all(target_os = "linux", feature = "desktop"), allow(dead_code))]
pub(crate) fn app_id_from_exe_path(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let name = match name.len().checked_sub(4) {
        Some(cut) if name[cut..].eq_ignore_ascii_case(".exe") => name[..cut].to_string(),
        _ => name,
    };
    (!name.is_empty()).then_some(name)
}

/// Write `png` to a directory only this user can write, and answer its path.
/// One file per distinct image (named by a hash of its bytes), written once;
/// macOS gets a fresh copy per notification, since the system moves an
/// attachment into its own store.
fn write_icon(png: &[u8]) -> Result<std::path::PathBuf, String> {
    write_icon_in(&icon_dir(), png)
}

/// Where icons go. Not a shared `/tmp` directory: another local user could
/// predict the names there and plant a file or a symlink. On Unix it is
/// `$XDG_RUNTIME_DIR` (per user, 0700 by specification), else the user's
/// cache directory; macOS's and Windows's temporary directories are per user
/// already. [`write_icon_in`] checks the directory either way.
fn icon_dir() -> std::path::PathBuf {
    const NAME: &str = "rinch-notifications";
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let nonempty = |v: Option<std::ffi::OsString>| v.filter(|v| !v.is_empty());
        if let Some(run) = nonempty(std::env::var_os("XDG_RUNTIME_DIR")) {
            return std::path::PathBuf::from(run).join(NAME);
        }
        if let Some(cache) = nonempty(std::env::var_os("XDG_CACHE_HOME")) {
            return std::path::PathBuf::from(cache).join(NAME);
        }
        if let Some(home) = nonempty(std::env::var_os("HOME")) {
            return std::path::PathBuf::from(home).join(".cache").join(NAME);
        }
    }
    std::env::temp_dir().join(NAME)
}

/// [`write_icon`] into `dir`, which is made private (0700 on Unix) and
/// refused unless it is a real directory with no group or other access —
/// one this process could then not write into unless it owns it. The file is
/// written under a fresh name opened with `create_new` (which neither follows
/// a symlink nor reuses a file) and renamed into place.
pub(crate) fn write_icon_in(
    dir: &std::path::Path,
    png: &[u8],
) -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    use std::io::Write;
    let err = |p: &std::path::Path, e: std::io::Error| format!("{}: {e}", p.display());

    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
    }
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(err(dir, e)),
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| err(dir, e))?;
    if !meta.file_type().is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "{} is open to other users (mode {:o}); refusing to write the icon there",
                dir.display(),
                meta.permissions().mode() & 0o777
            ));
        }
    }

    let mut h = std::collections::hash_map::DefaultHasher::new();
    png.hash(&mut h);
    let hash = h.finish();
    let path = dir.join(format!("{hash:016x}.png"));
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
        return Ok(path);
    }
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        "{hash:016x}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp).map_err(|e| err(&tmp, e))?;
    let written = file.write_all(png).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err(&path, e));
    }
    Ok(path)
}
