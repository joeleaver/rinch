//! System tray icon support.
//!
//! On Linux, uses `ksni` (StatusNotifierItem D-Bus protocol) for proper
//! left-click → show window, right-click → context menu behavior.
//! On other platforms, uses the `tray-icon` crate with push-based event
//! delivery (no polling thread).
//!
//! Uses the unified [`Menu`](crate::menu::Menu) / [`MenuItem`](crate::menu::MenuItem)
//! types shared with native window menus. Menu callbacks are dispatched to the
//! main thread via the global muda event handler.
//!
//! # Example
//!
//! ```ignore
//! use rinch::tray::TrayIconBuilder;
//! use rinch::menu::{Menu, MenuItem};
//!
//! let menu = Menu::new()
//!     .item(MenuItem::new("Show Window").on_click(show_current_window))
//!     .separator()
//!     .item(MenuItem::new("Quit").on_click(close_current_window));
//!
//! let _tray = TrayIconBuilder::new()
//!     .with_tooltip("My App")
//!     .with_icon_png(include_bytes!("icon.png"))?
//!     .with_menu(menu)
//!     .build()?;
//! ```

use crate::menu::Menu;

/// Error type for tray operations.
#[derive(Debug)]
pub enum TrayError {
    /// Failed to create tray icon.
    CreateFailed(String),
    /// Failed to load icon.
    IconLoadFailed(String),
    /// Menu error.
    MenuError(String),
}

impl std::fmt::Display for TrayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrayError::CreateFailed(msg) => write!(f, "failed to create tray icon: {}", msg),
            TrayError::IconLoadFailed(msg) => write!(f, "failed to load icon: {}", msg),
            TrayError::MenuError(msg) => write!(f, "menu error: {}", msg),
        }
    }
}

impl std::error::Error for TrayError {}

/// Result type for tray operations.
pub type TrayResult<T> = Result<T, TrayError>;

/// A system tray icon with optional menu.
///
/// On non-Linux: the tray icon is created on the main thread. Menu events
/// are handled by the global push-based handler — no polling thread.
///
/// On Linux: ksni runs the StatusNotifierItem D-Bus service on a background
/// thread of its own.
///
/// **Dropping this struct removes the tray icon**, and releases the menu
/// callbacks it registered — so replacing a tray reclaims the previous one's ids
/// rather than leaving them in the registry forever (issue #183). Keep the
/// handle for as long as you want the tray. On Linux the drop shuts the ksni
/// service down and waits — for up to one second — for it to close its D-Bus
/// connection, which is what takes the icon off the panel. The callbacks are
/// released only once the connection has closed, so the icon's items keep
/// working for as long as it can still be on screen (issue #377). If the
/// service has not closed within the bound (a StatusNotifierWatcher that
/// stopped answering leaves ksni waiting on it, where it cannot see the
/// shutdown request), the drop returns anyway and the callbacks are kept
/// registered; a later tray build or drop on the same thread releases them once
/// the service has finished.
///
/// # Threads
///
/// A `TrayIcon` is **neither `Send` nor `Sync`**, on any platform: keep it
/// on the thread that built it (the main thread), for example in a local of
/// `main` or in a `thread_local!`, not in a `static OnceLock` or an
/// `Arc<Mutex<_>>`, and do not move it into a spawned thread. This is
/// deliberate. The menu's callbacks live in a thread-local registry (they
/// capture `Signal`s, which are `!Send`, and always run on the main thread),
/// and dropping the tray releases them from *that thread's* registry — dropped
/// anywhere else, it would reclaim nothing. The handle holds `Rc`s for exactly
/// that reason, so the compiler enforces it. On Linux this became true with
/// issue #183 (the handle had held only a `JoinHandle` before, and was `Send`);
/// on other platforms `tray-icon`'s own handle holds an `Rc` and was never
/// `Send`.
pub struct TrayIcon {
    /// On Linux: the running ksni service, shut down on drop.
    #[cfg(target_os = "linux")]
    service: Option<Box<dyn TrayService>>,
    /// On non-Linux: the tray-icon handle (must be kept alive).
    #[cfg(not(target_os = "linux"))]
    _tray: Option<tray_icon::TrayIcon>,
    /// The menu callbacks this tray registered.
    ///
    /// Dropping the tray releases them (issue #183). Without this the registry
    /// only grew: a tray rebuilt at runtime left its whole previous menu behind,
    /// and on Linux it could not even overwrite, because each build mints fresh
    /// `ksni-{N}` ids from a monotonic counter.
    ///
    /// Released after [`TrayIcon`]'s `Drop` has shut the service down, so the
    /// icon leaves the panel before its items stop working, never the other
    /// way round. If the shutdown does not finish in time, the `Drop` parks it
    /// (see `park_until_closed`) rather than releasing it under an icon that
    /// may still be up.
    _menu: Option<crate::menu::MenuRegistration>,
}

/// The running Linux tray service, as [`TrayIcon`] needs it: something to shut
/// down. A trait rather than the ksni handle itself only so the drop order can
/// be pinned without a D-Bus session.
#[cfg(target_os = "linux")]
trait TrayService {
    /// Ask the service to stop, and wait up to [`SHUTDOWN_WAIT`] for it to
    /// close its D-Bus connection — the moment the StatusNotifierWatcher drops
    /// the item and the panel removes the icon.
    fn shutdown(&self) -> Shutdown;
}

/// What [`TrayService::shutdown`] saw.
#[cfg(target_os = "linux")]
enum Shutdown {
    /// The connection closed within the bound: the icon is gone.
    Closed,
    /// It had not closed when the bound ran out. The receiver hears (or
    /// disconnects) once it has.
    Pending(std::sync::mpsc::Receiver<()>),
}

/// How long dropping a Linux tray waits for ksni to close its connection.
///
/// A healthy session answers in about 10 ms. The bound exists because ksni's
/// loop reads the shutdown request only between D-Bus calls, and a call to a
/// watcher that never replies has no timeout (zbus's default), so an unbounded
/// wait froze the dropping thread — the main thread, on quit or on a tray
/// rebuild — for good.
#[cfg(target_os = "linux")]
const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
impl TrayService for ksni::blocking::Handle<RinchKsniTray> {
    fn shutdown(&self) -> Shutdown {
        // Sending the request never blocks. Waiting for it does: ksni's
        // `wait` is a `block_on` on ksni's own runtime, which panics when the
        // calling thread is already inside a tokio runtime — and an app may well
        // have entered one on its main thread. So the wait runs on a thread of
        // its own, which is inside no runtime whatever the caller is, and is
        // left detached if it outlasts the bound: it ends when ksni's loop does.
        let awaiter = ksni::blocking::Handle::shutdown(self);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("rinch-tray-shutdown".into())
            .spawn(move || {
                awaiter.wait();
                let _ = done_tx.send(());
            });
        if spawned.is_err() {
            // No helper, so nothing will ever report back. The request is sent;
            // do not hold the callbacks hostage to a report that cannot come.
            return Shutdown::Closed;
        }
        match done_rx.recv_timeout(SHUTDOWN_WAIT) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Shutdown::Pending(done_rx),
            // Answered, or the helper ended without answering (it panicked):
            // either way nothing is left to wait for.
            _ => Shutdown::Closed,
        }
    }
}

#[cfg(target_os = "linux")]
thread_local! {
    /// Menu registrations of dropped trays whose service had not closed its
    /// connection within [`SHUTDOWN_WAIT`], with the channel that reports when
    /// it has. Thread-local because a `MenuRegistration` must be released on
    /// the thread that built it.
    static PARKED: std::cell::RefCell<
        Vec<(std::sync::mpsc::Receiver<()>, crate::menu::MenuRegistration)>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

/// Keep `registration`'s callbacks registered until `closed` reports (or
/// disconnects), instead of releasing them under an icon that may still be on
/// the panel. Released by the next [`release_closed_parked`] after that.
///
/// Keeping them is the choice between two bad outcomes of a hung watcher: kept,
/// an icon that might still be shown keeps working items, and the cost is the
/// entries' memory until the service finishes; released, it could show items
/// that silently do nothing — the #377 symptom this type exists to prevent.
/// During thread teardown the list is gone and the registration is simply
/// dropped, which then reclaims what it still can.
#[cfg(target_os = "linux")]
fn park_until_closed(
    closed: std::sync::mpsc::Receiver<()>,
    registration: crate::menu::MenuRegistration,
) {
    let mut entry = Some((closed, registration));
    let _ = PARKED.try_with(|parked| {
        if let Ok(mut parked) = parked.try_borrow_mut() {
            parked.push(entry.take().expect("taken once"));
        }
    });
    drop(entry);
}

/// Release every parked registration whose service has closed by now. Called
/// on each tray build and drop on the thread.
#[cfg(target_os = "linux")]
fn release_closed_parked() {
    use std::sync::mpsc::TryRecvError;
    let done = PARKED
        .try_with(|parked| {
            let Ok(mut parked) = parked.try_borrow_mut() else {
                return Vec::new();
            };
            let (done, still): (Vec<_>, Vec<_>) = parked
                .drain(..)
                .partition(|(rx, _)| !matches!(rx.try_recv(), Err(TryRecvError::Empty)));
            *parked = still;
            done
        })
        .unwrap_or_default();
    // Released outside the borrow: a registration's drop drops user closures.
    drop(done);
}

/// How long [`TrayIconBuilder::build`] waits on Linux for ksni to register the
/// item with the StatusNotifierWatcher before it gives up with
/// [`TrayError::CreateFailed`].
///
/// A healthy session registers in about 10 ms. Long enough for a watcher that
/// is merely slow (a desktop still starting up), short enough that one that
/// never answers costs the app's startup seconds rather than everything.
#[cfg(target_os = "linux")]
const BUILD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Why [`run_bounded`] has no result.
#[cfg(target_os = "linux")]
#[derive(Debug)]
enum BoundedError {
    /// The helper thread could not be started.
    Spawn(std::io::Error),
    /// The work panicked.
    Panicked,
    /// The work had not finished when the bound — the wait given, carried
    /// here — ran out. It goes on, detached, and hands its result to
    /// `on_late` when it does.
    TimedOut(std::time::Duration),
}

#[cfg(target_os = "linux")]
impl std::fmt::Display for BoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundedError::Spawn(e) => write!(f, "failed to spawn tray thread: {e}"),
            BoundedError::Panicked => write!(f, "tray thread panicked"),
            BoundedError::TimedOut(wait) => write!(
                f,
                "the status notifier watcher did not answer within {:.1} s",
                wait.as_secs_f64()
            ),
        }
    }
}

/// Run `work` on a thread named `name` and wait up to `wait` for its result.
///
/// If the wait runs out, the thread is left to finish on its own and its
/// result goes to `on_late`, on that thread — never dropped unseen, whichever
/// side of the deadline it lands on: the caller and the thread settle it under
/// one lock, so a result is either returned here or handed to `on_late`,
/// exactly once.
#[cfg(target_os = "linux")]
fn run_bounded<T: Send + 'static>(
    name: &str,
    wait: std::time::Duration,
    work: impl FnOnce() -> T + Send + 'static,
    on_late: impl FnOnce(T) + Send + 'static,
) -> Result<T, BoundedError> {
    use std::sync::{Arc, Condvar, Mutex};

    enum Slot<T> {
        Waiting,
        Done(std::thread::Result<T>),
        Abandoned,
    }

    let shared = Arc::new((Mutex::new(Slot::Waiting), Condvar::new()));
    let theirs = Arc::clone(&shared);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
            let (lock, cvar) = &*theirs;
            let mut slot = lock.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(*slot, Slot::Abandoned) {
                drop(slot);
                if let Ok(value) = result {
                    on_late(value);
                }
            } else {
                *slot = Slot::Done(result);
                cvar.notify_all();
            }
        })
        .map_err(BoundedError::Spawn)?;

    let (lock, cvar) = &*shared;
    let deadline = std::time::Instant::now() + wait;
    let mut slot = lock.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if matches!(*slot, Slot::Done(_)) {
            let Slot::Done(result) = std::mem::replace(&mut *slot, Slot::Abandoned) else {
                unreachable!()
            };
            return result.map_err(|_| BoundedError::Panicked);
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            *slot = Slot::Abandoned;
            return Err(BoundedError::TimedOut(wait));
        }
        slot = cvar
            .wait_timeout(slot, deadline - now)
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
}

/// Run a tray service's setup (`work`, ksni's `spawn`) on a thread of its own
/// and wait up to `wait` for it, as [`TrayIconBuilder::build`] does.
///
/// A setup that finishes after the wait gave up is shut down again at once:
/// the build has already failed and released the menu's callbacks, so the icon
/// it would put up could only show dead items (#1057). Each give-up leaves the
/// setup thread and its D-Bus connection waiting on the watcher until it
/// answers or goes away, which the setup cannot be told to stop doing.
#[cfg(target_os = "linux")]
fn spawn_bounded<S, E>(
    wait: std::time::Duration,
    work: impl FnOnce() -> Result<S, E> + Send + 'static,
) -> Result<Result<S, E>, BoundedError>
where
    S: TrayService + Send + 'static,
    E: Send + 'static,
{
    run_bounded("rinch-tray", wait, work, |late| {
        if let Ok(service) = late {
            // On the setup thread, inside no runtime; its outcome is moot.
            let _ = service.shutdown();
        }
    })
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // The icon first, the callbacks after (they are released when `_menu`
        // drops, once this returns). ksni's own `Handle` does not stop the
        // service when dropped — its loop ignores a closed request channel and
        // runs until told to shut down — so without this the icon stayed on the
        // panel for the life of the process, with every item dead (#377).
        #[cfg(target_os = "linux")]
        {
            if let Some(service) = self.service.take() {
                if let Shutdown::Pending(closed) = service.shutdown() {
                    if let Some(registration) = self._menu.take() {
                        park_until_closed(closed, registration);
                    }
                }
            }
            release_closed_parked();
        }
    }
}

/// Builder for creating a system tray icon.
pub struct TrayIconBuilder {
    tooltip: Option<String>,
    icon_data: Option<(Vec<u8>, u32, u32)>,
    menu: Option<Menu>,
}

impl TrayIconBuilder {
    /// Create a new tray icon builder.
    pub fn new() -> Self {
        Self {
            tooltip: None,
            icon_data: None,
            menu: None,
        }
    }

    /// Set the tooltip text shown on hover.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set the tray icon from RGBA pixel data.
    pub fn with_icon_rgba(mut self, rgba: Vec<u8>, width: u32, height: u32) -> TrayResult<Self> {
        self.icon_data = Some((rgba, width, height));
        Ok(self)
    }

    /// Set the tray icon from PNG data (e.g., from `include_bytes!`).
    pub fn with_icon_png(mut self, png_data: &[u8]) -> TrayResult<Self> {
        let (rgba, width, height) = crate::shell::rinch_runtime::decode_png_to_rgba(png_data)
            .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.icon_data = Some((rgba, width, height));
        Ok(self)
    }

    /// Set the tray icon from a PNG file path.
    pub fn with_icon_path(self, path: impl AsRef<std::path::Path>) -> TrayResult<Self> {
        let data =
            std::fs::read(path.as_ref()).map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.with_icon_png(&data)
    }

    /// Set the tray context menu using the unified [`Menu`] type.
    pub fn with_menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Build the tray icon.
    ///
    /// On Linux, spawns a background thread for the ksni D-Bus event loop.
    /// On other platforms, creates the tray icon on the main thread with
    /// push-based event delivery.
    ///
    /// On Linux this blocks until the item is registered with the
    /// StatusNotifierWatcher — about 10 ms on a healthy session — but for no
    /// more than five seconds: a watcher that holds its name and never answers
    /// (a frozen `kded` or `plasmashell`) makes it return
    /// [`TrayError::CreateFailed`] instead of hanging the caller (issue
    /// #1057). If the registration completes after that, the service is shut
    /// down again, so no icon with dead items is left behind.
    ///
    /// A registration that timed out cannot be cancelled: until the watcher
    /// answers or goes away, each such failed build leaves a background thread
    /// and a D-Bus connection waiting on it. Do not retry `build()` in a tight
    /// loop while it keeps failing this way.
    pub fn build(self) -> TrayResult<TrayIcon> {
        #[cfg(target_os = "linux")]
        return self.build_ksni();

        #[cfg(not(target_os = "linux"))]
        return self.build_tray_icon();
    }

    /// Non-Linux: create tray icon on main thread with push-based events.
    #[cfg(not(target_os = "linux"))]
    fn build_tray_icon(self) -> TrayResult<TrayIcon> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder as TrayIconBuilderInner,
            TrayIconEvent,
        };

        let mut builder = TrayIconBuilderInner::new();

        if let Some(tip) = self.tooltip {
            builder = builder.with_tooltip(tip);
        }

        if let Some((rgba, w, h)) = self.icon_data {
            let icon = Icon::from_rgba(rgba, w, h)
                .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
            builder = builder.with_icon(icon);
        }

        // Convert unified Menu → muda Menu (registers callbacks in thread-local registry)
        let mut registration = None;
        if let Some(menu) = self.menu {
            let (muda_menu, reg) = crate::menu::build_muda_menu(menu);
            registration = Some(reg);
            builder = builder.with_menu(Box::new(muda_menu));
        }

        // Left-click shows the window (not the menu)
        builder = builder.with_menu_on_left_click(false);

        let tray = builder
            .build()
            .map_err(|e| TrayError::CreateFailed(e.to_string()))?;

        // Push-based tray icon event handling for left-click → show window.
        // Menu events are already handled by the global MenuEvent::set_event_handler
        // installed in the runtime (covers both native and tray menus).
        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::shell::rinch_runtime::run_on_main_thread(
                    crate::windows::show_current_window,
                );
            }
        }));

        Ok(TrayIcon {
            _tray: Some(tray),
            _menu: registration,
        })
    }

    /// Linux: use ksni (StatusNotifierItem) with a background thread.
    #[cfg(target_os = "linux")]
    fn build_ksni(self) -> TrayResult<TrayIcon> {
        use ksni::blocking::TrayMethods;

        release_closed_parked();

        let tooltip = self.tooltip.unwrap_or_default();
        let icon_data = self.icon_data;

        // Convert RGBA → ARGB32 (network byte order) for ksni.
        let icon_pixmap = if let Some((rgba, width, height)) = icon_data {
            let mut argb = Vec::with_capacity(rgba.len());
            for pixel in rgba.as_chunks::<4>().0 {
                argb.push(pixel[3]); // A
                argb.push(pixel[0]); // R
                argb.push(pixel[1]); // G
                argb.push(pixel[2]); // B
            }
            vec![ksni::Icon {
                width: width as i32,
                height: height as i32,
                data: argb,
            }]
        } else {
            Vec::new()
        };

        // Register callbacks on the main thread and collect entries for ksni.
        let (menu_entries, registration) = if let Some(menu) = self.menu {
            let (entries, reg) = convert_menu_to_ksni_entries(menu);
            (entries, Some(reg))
        } else {
            (Vec::new(), None)
        };

        let tray = RinchKsniTray {
            tooltip,
            icon_pixmap,
            menu_entries,
        };

        // `spawn` runs ksni's service on a thread ksni starts itself, but it
        // first `block_on`s the D-Bus setup on ksni's runtime — which panics on
        // a thread that is already inside a tokio runtime. Doing the spawn on a
        // short-lived thread keeps the caller's runtime context out of it.
        //
        // That setup ends in `RegisterStatusNotifierItem` on the watcher, a
        // call zbus makes with no timeout; a watcher that holds its name and
        // never answers (a frozen kded or plasmashell) would hold `build()`,
        // and so the app's startup, forever (#1057). So the wait is bounded by
        // `BUILD_WAIT`. A setup that finishes after the build gave up is shut
        // down again at once (`spawn_bounded`): its menu's callbacks are
        // released with the error, so an icon it put up would show dead items.
        let handle = spawn_bounded(BUILD_WAIT, move || tray.spawn())
            .map_err(|e| TrayError::CreateFailed(e.to_string()))?
            .map_err(|e| TrayError::CreateFailed(e.to_string()))?;

        Ok(TrayIcon {
            service: Some(Box::new(handle)),
            _menu: registration,
        })
    }
}

impl Default for TrayIconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── ksni implementation (Linux) ─────────────────────────────────────────────

/// Stored menu entry for ksni with string ID for callback dispatch.
enum KsniMenuEntry {
    Item {
        label: String,
        enabled: bool,
        /// Menu ID string for dispatching via the thread-local callback registry.
        menu_id: Option<String>,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<KsniMenuEntry>,
    },
}

/// Counter for generating unique menu IDs on Linux (ksni doesn't use muda).
static KSNI_MENU_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Convert a unified Menu into ksni entries, registering callbacks on the main thread.
///
/// Returns the [`MenuRegistration`](crate::menu::MenuRegistration) holding this
/// build's ids. Every build mints fresh ids from [`KSNI_MENU_COUNTER`], so a
/// rebuilt tray can never overwrite its own previous entries — without the token
/// the registry would grow by the item count on every rebuild, forever.
fn convert_menu_to_ksni_entries(menu: Menu) -> (Vec<KsniMenuEntry>, crate::menu::MenuRegistration) {
    let mut registration = crate::menu::MenuRegistration::default();
    let entries = convert_menu_entries_inner(menu, &mut registration);
    (entries, registration)
}

fn convert_menu_entries_inner(
    menu: Menu,
    registration: &mut crate::menu::MenuRegistration,
) -> Vec<KsniMenuEntry> {
    menu.take_entries()
        .into_iter()
        .map(|entry| match entry {
            crate::menu::MenuEntryKind::Item {
                label,
                enabled,
                callback,
                callback_owner,
            } => {
                // A disabled item fires nothing: ksni never activates one. Minting
                // an id and registering its callback anyway left a registry entry
                // that could never be dispatched, holding the closure's captures
                // alive for the life of the tray (#377) — the muda side returns
                // early in `build_muda_item` for the same reason.
                let callback = callback.filter(|_| enabled);
                let menu_id = callback.map(|cb| {
                    let id = format!(
                        "ksni-{}",
                        KSNI_MENU_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    );
                    registration.register_callback(&id, cb, callback_owner);
                    id
                });
                KsniMenuEntry::Item {
                    label,
                    enabled,
                    menu_id,
                }
            }
            crate::menu::MenuEntryKind::Separator => KsniMenuEntry::Separator,
            crate::menu::MenuEntryKind::Submenu { label, menu } => KsniMenuEntry::Submenu {
                label,
                entries: convert_menu_entries_inner(menu, registration),
            },
        })
        .collect()
}

#[cfg(target_os = "linux")]
struct RinchKsniTray {
    tooltip: String,
    icon_pixmap: Vec<ksni::Icon>,
    menu_entries: Vec<KsniMenuEntry>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for RinchKsniTray {
    fn id(&self) -> String {
        "rinch-app".into()
    }

    fn title(&self) -> String {
        self.tooltip.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon_pixmap.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.tooltip.clone(),
            ..Default::default()
        }
    }

    /// Left-click on the tray icon shows the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        crate::shell::rinch_runtime::run_on_main_thread(crate::windows::show_current_window);
    }

    /// Right-click context menu.
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        build_ksni_menu(&self.menu_entries)
    }
}

#[cfg(target_os = "linux")]
fn build_ksni_menu(entries: &[KsniMenuEntry]) -> Vec<ksni::MenuItem<RinchKsniTray>> {
    entries
        .iter()
        .map(|entry| match entry {
            KsniMenuEntry::Item {
                label,
                enabled,
                menu_id,
            } => {
                let id = menu_id.clone();
                ksni::MenuItem::Standard(ksni::menu::StandardItem {
                    label: label.clone(),
                    enabled: *enabled,
                    activate: Box::new(move |_this: &mut RinchKsniTray| {
                        if let Some(id) = &id {
                            let id = id.clone();
                            crate::shell::rinch_runtime::run_on_main_thread(move || {
                                crate::menu::dispatch_menu_event(&id);
                            });
                        }
                    }),
                    ..Default::default()
                })
            }
            KsniMenuEntry::Separator => ksni::MenuItem::Separator,
            KsniMenuEntry::Submenu { label, entries } => {
                ksni::MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: label.clone(),
                    submenu: build_ksni_menu(entries),
                    ..Default::default()
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::MenuItem;

    /// Every id this build registered, flattened out of the ksni entries.
    fn registered_ids(entries: &[KsniMenuEntry], out: &mut Vec<String>) {
        for entry in entries {
            match entry {
                KsniMenuEntry::Item { menu_id, .. } => out.extend(menu_id.iter().cloned()),
                KsniMenuEntry::Separator => {}
                KsniMenuEntry::Submenu { entries, .. } => registered_ids(entries, out),
            }
        }
    }

    /// Issue #377 (3): ksni never activates a disabled item, so an id minted and
    /// a callback registered for one can never be dispatched — a permanent
    /// registry entry holding the closure's captures alive for the life of the
    /// tray. The muda side returns early for a disabled item
    /// (`build_muda_item`); the ksni side must too, at every depth.
    ///
    /// Three enabled and three disabled items, one of each in a submenu, so a
    /// fix that skips only top-level disabled items still fails.
    #[test]
    fn a_disabled_ksni_item_registers_no_callback() {
        let before = crate::menu::callback_count();
        let menu = Menu::new()
            .item(MenuItem::new("on-a").on_click(|| {}))
            .item(MenuItem::new("off-a").enabled(false).on_click(|| {}))
            .item(MenuItem::new("off-b").enabled(false).on_click(|| {}))
            .separator()
            .item(MenuItem::new("on-b").on_click(|| {}))
            .submenu(
                "sub",
                Menu::new()
                    .item(MenuItem::new("on-c").on_click(|| {}))
                    .item(MenuItem::new("off-c").enabled(false).on_click(|| {})),
            );

        let (entries, registration) = convert_menu_to_ksni_entries(menu);

        let mut ids = Vec::new();
        registered_ids(&entries, &mut ids);
        assert_eq!(
            ids.len(),
            3,
            "only the three enabled items get an id: {ids:?}"
        );
        assert_eq!(
            crate::menu::callback_count() - before,
            3,
            "only the three enabled items register a callback"
        );

        // The disabled entries are still rendered, greyed out.
        fn disabled_labels(entries: &[KsniMenuEntry], out: &mut Vec<String>) {
            for entry in entries {
                match entry {
                    KsniMenuEntry::Item {
                        label,
                        enabled: false,
                        menu_id,
                    } => {
                        assert!(menu_id.is_none(), "disabled `{label}` carries an id");
                        out.push(label.clone());
                    }
                    KsniMenuEntry::Submenu { entries, .. } => disabled_labels(entries, out),
                    _ => {}
                }
            }
        }
        let mut off = Vec::new();
        disabled_labels(&entries, &mut off);
        assert_eq!(off, ["off-a", "off-b", "off-c"]);

        drop(registration);
        assert_eq!(crate::menu::callback_count(), before);
    }

    /// Issue #377 (1), without a D-Bus session: dropping a `TrayIcon` shuts its
    /// service down — once — and does so while the menu callbacks are still
    /// registered, so the icon never outlives its items.
    ///
    /// The live fixture below is what shows `shutdown` actually removes the
    /// icon; this one pins that the drop calls it, and the order.
    #[cfg(target_os = "linux")]
    #[test]
    fn dropping_the_tray_shuts_the_service_down_before_releasing_the_callbacks() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct Probe {
            shutdowns: Rc<Cell<u32>>,
            callbacks_at_shutdown: Rc<Cell<Option<usize>>>,
        }
        impl TrayService for Probe {
            fn shutdown(&self) -> Shutdown {
                self.shutdowns.set(self.shutdowns.get() + 1);
                self.callbacks_at_shutdown
                    .set(Some(crate::menu::callback_count()));
                Shutdown::Closed
            }
        }

        let before = crate::menu::callback_count();
        let (_entries, registration) = convert_menu_to_ksni_entries(
            Menu::new()
                .item(MenuItem::new("a").on_click(|| {}))
                .item(MenuItem::new("b").on_click(|| {})),
        );
        let shutdowns = Rc::new(Cell::new(0));
        let callbacks_at_shutdown = Rc::new(Cell::new(None));
        let tray = TrayIcon {
            service: Some(Box::new(Probe {
                shutdowns: shutdowns.clone(),
                callbacks_at_shutdown: callbacks_at_shutdown.clone(),
            })),
            _menu: Some(registration),
        };
        assert_eq!(
            shutdowns.get(),
            0,
            "nothing shuts down while the handle lives"
        );

        drop(tray);

        assert_eq!(
            shutdowns.get(),
            1,
            "the drop shuts the service down exactly once"
        );
        assert_eq!(
            callbacks_at_shutdown.get(),
            Some(before + 2),
            "the service is shut down while its callbacks are still registered"
        );
        assert_eq!(
            crate::menu::callback_count(),
            before,
            "and the callbacks are released after"
        );
    }

    /// The bounded half of the drop (review of #1054, F1): a service that has
    /// not closed within the bound does not have its callbacks released under
    /// it. They stay registered until it reports, and the next tray build or
    /// drop on the thread releases them then — whether it reports by sending or
    /// by its helper going away.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_shutdown_that_outlasts_the_bound_keeps_the_callbacks_until_it_closes() {
        use std::cell::RefCell;

        struct Slow(RefCell<Option<std::sync::mpsc::Receiver<()>>>);
        impl TrayService for Slow {
            fn shutdown(&self) -> Shutdown {
                Shutdown::Pending(self.0.borrow_mut().take().expect("shut down once"))
            }
        }
        fn tray(items: usize, closed: std::sync::mpsc::Receiver<()>) -> TrayIcon {
            let mut menu = Menu::new();
            for i in 0..items {
                menu = menu.item(MenuItem::new(format!("i{i}")).on_click(|| {}));
            }
            let (_entries, registration) = convert_menu_to_ksni_entries(menu);
            TrayIcon {
                service: Some(Box::new(Slow(RefCell::new(Some(closed))))),
                _menu: Some(registration),
            }
        }

        let before = crate::menu::callback_count();
        let (tx_a, rx_a) = std::sync::mpsc::channel();
        let (tx_b, rx_b) = std::sync::mpsc::channel();
        drop(tray(2, rx_a));
        drop(tray(3, rx_b));
        assert_eq!(
            crate::menu::callback_count(),
            before + 5,
            "a pending shutdown keeps its callbacks registered"
        );

        release_closed_parked();
        assert_eq!(
            crate::menu::callback_count(),
            before + 5,
            "nothing has closed yet"
        );

        tx_a.send(()).unwrap();
        release_closed_parked();
        assert_eq!(
            crate::menu::callback_count(),
            before + 3,
            "the closed one is released, the pending one kept"
        );

        // A helper that ends without reporting (it panicked) counts as closed.
        drop(tx_b);
        release_closed_parked();
        assert_eq!(crate::menu::callback_count(), before);
    }

    /// Review of #1054, F1, live on a private bus: dropping a tray whose
    /// watcher has stopped answering returns within the bound, keeping its
    /// callbacks. At bd2719ae (an unbounded join) the drop never returned.
    ///
    /// Recipe (needs `dbus-run-session` and python3-gi; the test binary path is
    /// whatever cargo printed):
    ///
    /// ```text
    /// cargo test -p rinch --features system-tray --lib --no-run
    /// dbus-run-session -- bash -c 'python3 crates/rinch/tests/fixtures/hung_sni_watcher.py & sleep 1;
    ///   <test bin> tray::tests::live_dropping_a_tray_under_a_hung_watcher_is_bounded --include-ignored --nocapture'
    /// ```
    ///
    /// Run on the real session bus instead it also passes, trivially (a
    /// healthy watcher answers at once); the `pending` it prints says which
    /// case ran.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a private bus running tests/fixtures/hung_sni_watcher.py"]
    fn live_dropping_a_tray_under_a_hung_watcher_is_bounded() {
        use std::time::{Duration, Instant};

        let before = crate::menu::callback_count();
        let t = TrayIconBuilder::new()
            .with_menu(Menu::new().item(MenuItem::new("p").on_click(|| {})))
            .build()
            .expect("builds");
        // Let the fake watcher cycle its name, so ksni is parked in a
        // registration call that will never be answered.
        std::thread::sleep(Duration::from_millis(1500));
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(10));
            eprintln!("HUNG: the drop is still blocked after 10 s");
            std::process::exit(3);
        });
        let t0 = Instant::now();
        drop(t);
        let took = t0.elapsed();
        let pending = crate::menu::callback_count() - before;
        eprintln!("drop returned after {took:?}; callbacks kept: {pending}");
        assert!(
            took < SHUTDOWN_WAIT + Duration::from_millis(500),
            "{took:?}"
        );
        if took >= SHUTDOWN_WAIT {
            assert_eq!(
                pending, 1,
                "a timed-out drop keeps the callbacks registered"
            );
        }
    }

    /// Issue #1057: `run_bounded` returns within its bound when the work
    /// hangs, and hands the result the work produces afterwards to `on_late`
    /// instead of dropping it unseen (which, for a ksni handle, would leave a
    /// late icon on the panel with its callbacks already released).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_bounded_run_that_outlasts_its_bound_times_out_and_hands_the_result_on() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (late_tx, late_rx) = mpsc::channel::<u32>();
        let (ret_tx, ret_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let r = run_bounded(
                "bounded-test",
                Duration::from_millis(200),
                move || {
                    let _ = release_rx.recv();
                    7u32
                },
                move |late| {
                    let _ = late_tx.send(late);
                },
            );
            let _ = ret_tx.send((r, t0.elapsed()));
        });
        let (r, took) = ret_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("run_bounded returned while the work was still blocked");
        assert!(matches!(r, Err(BoundedError::TimedOut(_))), "{r:?}");
        assert!(took >= Duration::from_millis(200), "{took:?}");
        assert!(late_rx.try_recv().is_err(), "nothing is late yet");
        release_tx.send(()).unwrap();
        assert_eq!(
            late_rx.recv_timeout(Duration::from_secs(5)),
            Ok(7),
            "the late result reaches on_late"
        );
    }

    /// Issue #1057: work that finishes inside the bound is returned, not handed
    /// to `on_late`, and a panic is reported at once rather than at the bound.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_bounded_run_returns_a_prompt_result_and_a_prompt_panic() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let (late_tx, late_rx) = mpsc::channel::<u32>();
        let r = run_bounded(
            "bounded-test",
            Duration::from_secs(5),
            || {
                std::thread::sleep(Duration::from_millis(50));
                11u32
            },
            move |late| {
                let _ = late_tx.send(late);
            },
        );
        assert_eq!(r.ok(), Some(11));
        assert!(late_rx.recv_timeout(Duration::from_millis(200)).is_err());

        let t0 = Instant::now();
        let r = run_bounded(
            "bounded-test",
            Duration::from_secs(5),
            || -> u32 { panic!("expected panic in a bounded-run test") },
            |_| {},
        );
        assert!(matches!(r, Err(BoundedError::Panicked)), "{r:?}");
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    }

    /// Issue #1057, review F1: a tray service whose setup finishes after the
    /// build gave up is shut down, not merely dropped — ksni's handle does not
    /// stop the service on drop, so dropping it left the icon up with its
    /// callbacks already released (measured live on a private bus).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_tray_service_that_arrives_after_the_build_gave_up_is_shut_down() {
        use std::sync::mpsc;
        use std::time::Duration;

        struct Late(mpsc::Sender<()>);
        impl TrayService for Late {
            fn shutdown(&self) -> Shutdown {
                let _ = self.0.send(());
                Shutdown::Closed
            }
        }

        let (shut_tx, shut_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let r = spawn_bounded(Duration::from_millis(50), move || {
            let _ = release_rx.recv();
            Ok::<_, ()>(Late(shut_tx))
        });
        assert!(
            matches!(r, Err(BoundedError::TimedOut(w)) if w == Duration::from_millis(50)),
            "{:?}",
            r.as_ref().err()
        );
        assert_eq!(
            r.err().map(|e| e.to_string()).as_deref(),
            Some("the status notifier watcher did not answer within 0.1 s")
        );
        assert!(shut_rx.try_recv().is_err(), "nothing has arrived yet");
        release_tx.send(()).unwrap();
        assert_eq!(
            shut_rx.recv_timeout(Duration::from_secs(5)),
            Ok(()),
            "the late service was not shut down"
        );
    }

    /// Issue #1057, live on a private bus: building a tray while the watcher
    /// holds its name but never answers `RegisterStatusNotifierItem` returns
    /// an error within the bound, and keeps none of the menu's callbacks. At
    /// 2fa50a2f `build()` never returned (the watchdog below fired).
    ///
    /// Recipe as for the drop test above, with the watcher started as
    /// `hung_sni_watcher.py --hang-first`. On a healthy session bus it fails
    /// (the build succeeds), which is its positive control.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a private bus running tests/fixtures/hung_sni_watcher.py --hang-first"]
    fn live_building_a_tray_under_a_hung_watcher_is_bounded() {
        use std::time::{Duration, Instant};

        let before = crate::menu::callback_count();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(12));
            eprintln!("HUNG: build() is still blocked after 12 s");
            std::process::exit(3);
        });
        let t0 = Instant::now();
        let built = TrayIconBuilder::new()
            .with_menu(Menu::new().item(MenuItem::new("p").on_click(|| {})))
            .build();
        let took = t0.elapsed();
        eprintln!("build returned after {took:?}: {:?}", built.as_ref().err());
        assert!(
            built.is_err(),
            "the build cannot succeed under a hung watcher"
        );
        assert!(took < BUILD_WAIT + Duration::from_secs(1), "{took:?}");
        assert_eq!(
            crate::menu::callback_count(),
            before,
            "a failed build keeps no callbacks"
        );
    }

    /// Issue #377 (1), live: a dropped `TrayIcon` must take its icon with it.
    ///
    /// Needs a session bus with a StatusNotifierWatcher and a registered host
    /// (a KDE session does), and `busctl`, so it is `#[ignore]`d; run it with
    /// `cargo test -p rinch --features system-tray --lib tray::tests::live -- --ignored`.
    /// It puts an icon in the real tray for well under a second.
    ///
    /// ksni registers the item under its well-known name
    /// `org.freedesktop.StatusNotifierItem-{pid}-{n}`, which the watcher lists in
    /// `RegisteredStatusNotifierItems` until that name's owner goes away.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a live D-Bus session with a StatusNotifierWatcher"]
    fn live_a_dropped_tray_leaves_the_status_notifier_watcher() {
        fn ours() -> usize {
            let out = std::process::Command::new("busctl")
                .args([
                    "--user",
                    "get-property",
                    "org.kde.StatusNotifierWatcher",
                    "/StatusNotifierWatcher",
                    "org.kde.StatusNotifierWatcher",
                    "RegisteredStatusNotifierItems",
                ])
                .output()
                .expect("busctl runs");
            assert!(out.status.success(), "busctl: {out:?}");
            let needle = format!("StatusNotifierItem-{}-", std::process::id());
            String::from_utf8_lossy(&out.stdout)
                .matches(&needle)
                .count()
        }
        fn wait_for(want: usize) -> usize {
            let mut seen = ours();
            for _ in 0..60 {
                if seen == want {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                seen = ours();
            }
            seen
        }

        assert_eq!(ours(), 0, "no tray of ours before the build");
        let tray = TrayIconBuilder::new()
            .with_tooltip("rinch #377 probe")
            .with_menu(Menu::new().item(MenuItem::new("probe").on_click(|| {})))
            .build()
            .expect("the tray builds");
        // Positive control: the instrument sees our item while the handle lives.
        assert_eq!(wait_for(1), 1, "the watcher lists the live tray");

        drop(tray);
        assert_eq!(wait_for(0), 0, "the dropped tray is still registered");
    }
}
