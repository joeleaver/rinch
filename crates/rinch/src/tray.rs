//! System tray icon support.
//!
//! On Linux, uses `ksni` (StatusNotifierItem D-Bus protocol). On other
//! platforms, uses the `tray-icon` crate with push-based event delivery (no
//! polling thread).
//!
//! Uses the unified [`Menu`](crate::menu::Menu) / [`MenuItem`](crate::menu::MenuItem)
//! types shared with native window menus, checkbox and radio items included.
//! Menu callbacks are dispatched to the main thread.
//!
//! Everything about a built tray can change while it runs (issue #1520): its
//! icon ([`TrayIcon::set_icon`]), its tooltip ([`TrayIcon::set_tooltip`]) and
//! its whole menu ([`TrayIcon::set_menu`]); a checkbox or radio item's mark
//! follows the signal it was built with. What a primary click on the icon does
//! is [`TrayIconBuilder::on_activate`]'s, and shows the window by default.
//!
//! # Example
//!
//! ```ignore
//! use rinch::tray::{TrayIconBuilder, TrayIconImage};
//! use rinch::menu::{Menu, MenuItem};
//!
//! let muted = Signal::new(false);
//! let menu = Menu::new()
//!     .item(MenuItem::checkbox("Mute", muted))
//!     .item(MenuItem::new("Show Window").on_click(show_current_window))
//!     .separator()
//!     .item(MenuItem::new("Quit").on_click(close_current_window));
//!
//! let busy = TrayIconImage::from_png(include_bytes!("busy.png"))?;
//! let tray = TrayIconBuilder::new()
//!     .with_tooltip("My App")
//!     .with_icon_png(include_bytes!("icon.png"))?
//!     .with_menu(menu)
//!     .on_activate(move || muted.update(|m| *m = !*m))
//!     .build()?;
//!
//! // Later, from any main-thread code:
//! tray.set_icon(&busy)?;
//! tray.set_tooltip("Working…")?;
//! ```

use crate::menu::Menu;
use rinch_core::reactive::{Owner, current_owner};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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

/// A decoded tray icon image, for [`TrayIcon::set_icon`].
///
/// Decode each image once and keep it: an app that swaps its icon several
/// times a second (a recording indicator, a progress glyph) then pays no PNG
/// decode per swap. Cloning is cheap (the pixels are shared).
#[derive(Clone, Debug)]
pub struct TrayIconImage {
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
}

impl TrayIconImage {
    /// An image from straight (non-premultiplied) RGBA pixels, row by row.
    ///
    /// Fails when `rgba` is not `width * height * 4` bytes.
    pub fn from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> TrayResult<Self> {
        let want = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if want != Some(rgba.len()) || width == 0 || height == 0 {
            return Err(TrayError::IconLoadFailed(format!(
                "{} bytes of RGBA is not a {width}x{height} image",
                rgba.len()
            )));
        }
        Ok(Self {
            rgba: rgba.into(),
            width,
            height,
        })
    }

    /// An image decoded from PNG bytes (e.g. from `include_bytes!`).
    pub fn from_png(png_data: &[u8]) -> TrayResult<Self> {
        let (rgba, width, height) = crate::shell::rinch_runtime::decode_png_to_rgba(png_data)
            .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        Self::from_rgba(rgba, width, height)
    }

    /// An image decoded from a PNG file.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> TrayResult<Self> {
        let data =
            std::fs::read(path.as_ref()).map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        Self::from_png(&data)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels in the network-order ARGB32 a StatusNotifierItem carries.
    #[cfg(target_os = "linux")]
    fn to_ksni(&self) -> Vec<ksni::Icon> {
        let mut argb = Vec::with_capacity(self.rgba.len());
        for pixel in self.rgba.as_chunks::<4>().0 {
            argb.push(pixel[3]); // A
            argb.push(pixel[0]); // R
            argb.push(pixel[1]); // G
            argb.push(pixel[2]); // B
        }
        vec![ksni::Icon {
            width: self.width as i32,
            height: self.height as i32,
            data: argb,
        }]
    }

    #[cfg(not(target_os = "linux"))]
    fn to_tray_icon(&self) -> TrayResult<tray_icon::Icon> {
        tray_icon::Icon::from_rgba(self.rgba.to_vec(), self.width, self.height)
            .map_err(|e| TrayError::IconLoadFailed(e.to_string()))
    }
}

/// A system tray icon with optional menu.
///
/// On non-Linux: the tray icon is created on the main thread. Menu events
/// are handled by the global push-based handler — no polling thread.
///
/// On Linux: ksni runs the StatusNotifierItem D-Bus service on a background
/// thread of its own, and a second thread of rinch's carries runtime changes
/// to it (see [`set_icon`](Self::set_icon)).
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
/// # Changing it while it runs
///
/// [`set_icon`](Self::set_icon), [`set_tooltip`](Self::set_tooltip) and
/// [`set_menu`](Self::set_menu) take `&self`, so a tray kept in an `Rc` (or a
/// `thread_local!`) can be changed from a menu callback, a timer or an effect.
/// None of them blocks the calling thread on any platform. On Linux each change
/// is handed to a thread of rinch's own, which applies it to the ksni service:
/// a StatusNotifierWatcher that has stopped answering holds that thread, never
/// the caller's. Changes made while it is held are **coalesced** — the newest
/// icon, tooltip and menu win, and only the latest state of each check mark is
/// kept — so an app swapping icons several times a second neither queues up
/// work nor memory behind a hung watcher. Elsewhere `tray-icon` applies a
/// change on the spot, on the calling (main) thread.
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
    /// On Linux: the thread carrying runtime changes to the service. Closed
    /// first on drop.
    #[cfg(target_os = "linux")]
    updates: Option<Rc<KsniUpdater>>,
    /// On non-Linux: the tray-icon handle (must be kept alive).
    #[cfg(not(target_os = "linux"))]
    _tray: Option<tray_icon::TrayIcon>,
    /// The menu callbacks this tray registered.
    ///
    /// Dropping the tray releases them (issue #183). Without this the registry
    /// only grew: a tray rebuilt at runtime left its whole previous menu behind,
    /// and on Linux it could not even overwrite, because each build mints fresh
    /// `ksni-{N}` ids from a monotonic counter. [`set_menu`](Self::set_menu)
    /// replaces it, releasing the previous menu's callbacks and watchers the
    /// way a rebuild does.
    ///
    /// Released after [`TrayIcon`]'s `Drop` has shut the service down, so the
    /// icon leaves the panel before its items stop working, never the other
    /// way round. If the shutdown does not finish in time, the `Drop` parks it
    /// (see `park_until_closed`) rather than releasing it under an icon that
    /// may still be up.
    _menu: RefCell<Option<crate::menu::MenuRegistration>>,
    /// The [`on_activate`](TrayIconBuilder::on_activate) callback's
    /// registration, kept apart from the menu's so [`set_menu`](Self::set_menu)
    /// leaves it alone.
    _activate: Option<crate::menu::MenuRegistration>,
}

impl TrayIcon {
    /// Replace the tray's icon.
    ///
    /// Never blocks (see [Changing it while it runs](Self#changing-it-while-it-runs)).
    /// On Linux the change is applied asynchronously and this returns `Ok`
    /// whatever happens to it; elsewhere an error from the platform is
    /// returned.
    pub fn set_icon(&self, image: &TrayIconImage) -> TrayResult<()> {
        #[cfg(target_os = "linux")]
        {
            let icon = image.to_ksni();
            self.push_update(move |p| p.icon = Some(icon));
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let icon = image.to_tray_icon()?;
            match &self._tray {
                Some(tray) => tray
                    .set_icon(Some(icon))
                    .map_err(|e| TrayError::IconLoadFailed(e.to_string())),
                None => Ok(()),
            }
        }
    }

    /// Replace the tray's icon from PNG bytes. Decodes on every call; prefer
    /// [`TrayIconImage::from_png`] once and [`set_icon`](Self::set_icon) for an
    /// icon that changes often.
    pub fn set_icon_png(&self, png_data: &[u8]) -> TrayResult<()> {
        self.set_icon(&TrayIconImage::from_png(png_data)?)
    }

    /// Replace the tray's icon from straight RGBA pixels.
    pub fn set_icon_rgba(&self, rgba: Vec<u8>, width: u32, height: u32) -> TrayResult<()> {
        self.set_icon(&TrayIconImage::from_rgba(rgba, width, height)?)
    }

    /// Replace the tooltip shown on hover (on Linux also the item's title).
    ///
    /// Never blocks; see [`set_icon`](Self::set_icon) for when an error can be
    /// returned.
    pub fn set_tooltip(&self, tooltip: impl Into<String>) -> TrayResult<()> {
        let tooltip = tooltip.into();
        #[cfg(target_os = "linux")]
        {
            self.push_update(move |p| p.tooltip = Some(tooltip));
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            match &self._tray {
                Some(tray) => tray
                    .set_tooltip(Some(tooltip))
                    .map_err(|e| TrayError::MenuError(e.to_string())),
                None => Ok(()),
            }
        }
    }

    /// Replace the tray's context menu, or remove it with `None`.
    ///
    /// The previous menu's callbacks are released at once, exactly as a
    /// rebuilt menu bar releases its predecessor's: an item of the old menu
    /// that is still on screen until the change reaches the panel does nothing
    /// when clicked. Safe to call from one of the old menu's own callbacks
    /// ("Start" replacing itself with "Stop").
    ///
    /// To change a check mark, write the item's signal instead — no new menu
    /// is needed.
    pub fn set_menu(&self, menu: Option<Menu>) {
        #[cfg(target_os = "linux")]
        let registration = {
            let (entries, mut registration, toggles) = match menu {
                Some(menu) => convert_menu_to_ksni_entries(menu),
                None => Default::default(),
            };
            if let Some(updates) = &self.updates {
                watch_ksni_toggles(&mut registration, toggles, updates);
            }
            self.push_update(move |p| {
                p.menu = Some(entries);
                // Marks queued for the menu being replaced name keys it no
                // longer has.
                p.checks.clear();
            });
            registration
        };
        #[cfg(not(target_os = "linux"))]
        let registration = {
            let (muda_menu, registration) = match menu {
                Some(menu) => {
                    let (m, r) = crate::menu::build_muda_menu(menu);
                    (Some(m), r)
                }
                None => (None, Default::default()),
            };
            if let Some(tray) = &self._tray {
                tray.set_menu(
                    muda_menu.map(|m| Box::new(m) as Box<dyn tray_icon::menu::ContextMenu>),
                );
            }
            registration
        };
        // Released outside the borrow: dropping a registration drops user
        // closures, whose `Drop` may re-enter this tray.
        let _previous = self._menu.borrow_mut().replace(registration);
    }

    /// Hand a change to the Linux update thread.
    #[cfg(target_os = "linux")]
    fn push_update(&self, change: impl FnOnce(&mut KsniPending)) {
        if let Some(updates) = &self.updates {
            updates.push(change);
        }
    }
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
        Vec<(std::sync::mpsc::Receiver<()>, Vec<crate::menu::MenuRegistration>)>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

/// Keep the callbacks of `registration` registered until `closed` reports (or
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
    registration: Vec<crate::menu::MenuRegistration>,
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
            // No more changes: an update still waiting would only be applied
            // to an icon on its way out.
            if let Some(updates) = self.updates.take() {
                updates.close();
            }
            if let Some(service) = self.service.take() {
                if let Shutdown::Pending(closed) = service.shutdown() {
                    let parked: Vec<_> = self
                        ._menu
                        .get_mut()
                        .take()
                        .into_iter()
                        .chain(self._activate.take())
                        .collect();
                    if !parked.is_empty() {
                        park_until_closed(closed, parked);
                    }
                }
            }
            release_closed_parked();
        }
        #[cfg(not(target_os = "linux"))]
        if let Some(tray) = &self._tray {
            forget_activate(&tray.id().0);
        }
    }
}

/// What [`TrayIconBuilder::on_activate`] was given.
struct ActivateCallback {
    cb: Rc<dyn Fn()>,
    owner: Option<Owner>,
}

/// Source of the registry ids the activate callbacks are registered under.
static ACTIVATE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Register `activate` under a fresh id, in a registration of its own.
fn register_activate(
    activate: Option<ActivateCallback>,
) -> (Option<String>, Option<crate::menu::MenuRegistration>) {
    let Some(ActivateCallback { cb, owner }) = activate else {
        return (None, None);
    };
    let id = format!(
        "tray-activate-{}",
        ACTIVATE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let mut registration = crate::menu::MenuRegistration::default();
    registration.register_callback(&id, cb, owner);
    (Some(id), Some(registration))
}

/// Run what a primary click on the icon does, on the main thread: the
/// [`on_activate`](TrayIconBuilder::on_activate) callback registered under
/// `id`, or — with none — show the window, which is what a tray did before the
/// option existed.
fn run_activate(id: Option<String>) {
    match id {
        Some(id) => crate::shell::rinch_runtime::run_on_main_thread(move || {
            crate::menu::dispatch_menu_event(&id);
        }),
        None => {
            crate::shell::rinch_runtime::run_on_main_thread(crate::windows::show_current_window)
        }
    }
}

/// Builder for creating a system tray icon.
pub struct TrayIconBuilder {
    id: Option<String>,
    tooltip: Option<String>,
    icon: Option<TrayIconImage>,
    menu: Option<Menu>,
    activate: Option<ActivateCallback>,
}

impl TrayIconBuilder {
    /// Create a new tray icon builder.
    pub fn new() -> Self {
        Self {
            id: None,
            tooltip: None,
            icon: None,
            menu: None,
            activate: None,
        }
    }

    /// The identity the tray item reports to the desktop (issue #1529).
    ///
    /// On Linux this is the StatusNotifierItem's `Id`, which panels key
    /// per-item settings on (shown or hidden, position). Without it the id is
    /// the app's own: the `WindowProps::app_id` it declared, when the app is
    /// already running as the tray is built, else the executable's file name
    /// — the same derivation the Wayland `app_id` uses. A tray built in `main`
    /// before [`App::run`](crate::App::run) cannot see `WindowProps::app_id`,
    /// so an app that declares one should pass the same string here.
    ///
    /// Windows and macOS have no such string to set: Windows keys a
    /// notification-area icon by the executable and an icon number, and macOS
    /// a status item by the app, so the value is not used there.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The id this builder's tray item reports (see [`with_id`](Self::with_id)).
    #[cfg(target_os = "linux")]
    fn item_id(&self) -> String {
        tray_item_id(self.id.as_deref())
    }

    /// Set the tooltip text shown on hover.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set the tray icon from RGBA pixel data.
    ///
    /// Fails when `rgba` is not `width * height * 4` bytes.
    pub fn with_icon_rgba(mut self, rgba: Vec<u8>, width: u32, height: u32) -> TrayResult<Self> {
        self.icon = Some(TrayIconImage::from_rgba(rgba, width, height)?);
        Ok(self)
    }

    /// Set the tray icon from PNG data (e.g., from `include_bytes!`).
    pub fn with_icon_png(mut self, png_data: &[u8]) -> TrayResult<Self> {
        self.icon = Some(TrayIconImage::from_png(png_data)?);
        Ok(self)
    }

    /// Set the tray icon from a PNG file path.
    pub fn with_icon_path(self, path: impl AsRef<std::path::Path>) -> TrayResult<Self> {
        let data =
            std::fs::read(path.as_ref()).map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.with_icon_png(&data)
    }

    /// Set the tray icon from an image decoded beforehand.
    pub fn with_icon(mut self, image: TrayIconImage) -> Self {
        self.icon = Some(image);
        self
    }

    /// Set the tray context menu using the unified [`Menu`] type.
    pub fn with_menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// What a primary (left) click on the icon does (issue #1520).
    ///
    /// Without it, a click shows the app's window (`show_current_window`),
    /// which was the only behaviour before this option. With it, the click
    /// runs `cb` instead and nothing else: toggle a mode, open a popover, or
    /// pass `|| {}` for a tray whose click does nothing but the menu.
    ///
    /// On Linux this is the StatusNotifierItem's `Activate` (what the panel
    /// sends for a primary click; which click that is, and whether a panel
    /// opens the menu instead, is the panel's choice). On Windows and macOS it
    /// is a left-button release on the icon. `cb` runs on the main thread,
    /// under the same lifetime rule as a menu callback: it belongs to the
    /// component rendering where this is called, and stops running once that
    /// component unmounts (see [`crate::menu`]).
    pub fn on_activate(mut self, cb: impl Fn() + 'static) -> Self {
        self.activate = Some(ActivateCallback {
            cb: Rc::new(cb),
            owner: current_owner(),
        });
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
        use tray_icon::TrayIconBuilder as TrayIconBuilderInner;

        // The OS has no item id to set here (see `with_id`); tray-icon's own
        // `TrayIconId` is an in-process key, left to its counter so two trays
        // can never share one.
        let _no_os_id = self.id;

        let mut builder = TrayIconBuilderInner::new();

        if let Some(tip) = self.tooltip {
            builder = builder.with_tooltip(tip);
        }

        if let Some(image) = &self.icon {
            builder = builder.with_icon(image.to_tray_icon()?);
        }

        // Convert unified Menu → muda Menu (registers callbacks in thread-local registry)
        let mut registration = None;
        if let Some(menu) = self.menu {
            let (muda_menu, reg) = crate::menu::build_muda_menu(menu);
            registration = Some(reg);
            builder = builder.with_menu(Box::new(muda_menu));
        }

        // A left click is the activate action, not the menu.
        builder = builder.with_menu_on_left_click(false);

        let tray = builder
            .build()
            .map_err(|e| TrayError::CreateFailed(e.to_string()))?;

        // Menu events are handled by the global MenuEvent handler installed in
        // the runtime (it covers native and tray menus alike). Clicks on the
        // icon go through one global handler too, which looks the clicked tray
        // up by id.
        let (activate_id, activate_registration) = register_activate(self.activate);
        remember_activate(tray.id().0.clone(), activate_id);
        install_tray_click_handler();

        Ok(TrayIcon {
            _tray: Some(tray),
            _menu: RefCell::new(registration),
            _activate: activate_registration,
        })
    }

    /// Linux: use ksni (StatusNotifierItem) with a background thread.
    #[cfg(target_os = "linux")]
    fn build_ksni(self) -> TrayResult<TrayIcon> {
        use ksni::blocking::TrayMethods;

        release_closed_parked();
        let id = self.item_id();

        // Register callbacks on the main thread and collect entries for ksni.
        let (menu_entries, mut registration, toggles) = match self.menu {
            Some(menu) => {
                let (entries, reg, toggles) = convert_menu_to_ksni_entries(menu);
                (entries, Some(reg), toggles)
            }
            None => (Vec::new(), None, Vec::new()),
        };
        let (activate_id, activate_registration) = register_activate(self.activate);

        let tray = RinchKsniTray {
            id,
            tooltip: self.tooltip.unwrap_or_default(),
            icon_pixmap: self
                .icon
                .as_ref()
                .map(TrayIconImage::to_ksni)
                .unwrap_or_default(),
            menu_entries,
            activate_id,
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

        // Runtime changes go through a thread of their own: `Handle::update`
        // blocks until the service has emitted its change signal, which a hung
        // watcher can hold off indefinitely (the #1057 shape again).
        let sink_handle = handle.clone();
        let updates = match KsniUpdater::spawn(move |pending: KsniPending| {
            sink_handle.update(move |tray| pending.apply(tray));
        }) {
            Ok(updates) => Rc::new(updates),
            Err(e) => {
                // A service no one can update is not the tray asked for; take
                // it down again rather than return half of it.
                let _ = TrayService::shutdown(&handle);
                return Err(TrayError::CreateFailed(format!(
                    "failed to spawn the tray update thread: {e}"
                )));
            }
        };
        if let Some(registration) = registration.as_mut() {
            watch_ksni_toggles(registration, toggles, &updates);
        }

        Ok(TrayIcon {
            service: Some(Box::new(handle)),
            updates: Some(updates),
            _menu: RefCell::new(registration),
            _activate: activate_registration,
        })
    }
}

impl Default for TrayIconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Click handling (Windows / macOS) ────────────────────────────────────────

/// Which activate callback each live tray runs on a left click, by the
/// tray-icon id. `tray-icon`'s click handler is one global, `Send` function, so
/// it carries only the id strings across; the callback itself stays in the
/// main thread's registry. A tray with no `on_activate` maps to `None`.
#[cfg(not(target_os = "linux"))]
static ACTIVATE_IDS: std::sync::Mutex<Option<std::collections::HashMap<String, Option<String>>>> =
    std::sync::Mutex::new(None);

#[cfg(not(target_os = "linux"))]
fn remember_activate(tray_id: String, activate_id: Option<String>) {
    let mut ids = ACTIVATE_IDS.lock().unwrap_or_else(|p| p.into_inner());
    ids.get_or_insert_with(Default::default)
        .insert(tray_id, activate_id);
}

#[cfg(not(target_os = "linux"))]
fn forget_activate(tray_id: &str) {
    let mut ids = ACTIVATE_IDS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(ids) = ids.as_mut() {
        ids.remove(tray_id);
    }
}

/// The activate id for a click on `tray_id`; `None` when the tray has no
/// `on_activate`, or is not ours (then the click shows the window, as before).
#[cfg(not(target_os = "linux"))]
fn activate_for(tray_id: &str) -> Option<String> {
    let ids = ACTIVATE_IDS.lock().unwrap_or_else(|p| p.into_inner());
    ids.as_ref()
        .and_then(|ids| ids.get(tray_id).cloned().flatten())
}

#[cfg(not(target_os = "linux"))]
fn install_tray_click_handler() {
    use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
    tray_icon::TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            run_activate(activate_for(&id.0));
        }
    }));
}

// ── ksni implementation (Linux) ─────────────────────────────────────────────

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// A checkbox or radio entry's state, as the ksni side holds it.
#[derive(Clone, Debug, PartialEq)]
struct KsniToggle {
    /// Which entry this is, for the check-mark updates
    /// ([`KsniPending::checks`]). Unique across builds.
    key: u64,
    /// `Some(group)` for a radio option, `None` for a checkbox.
    radio_group: Option<(u32, u32)>,
    checked: bool,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Stored menu entry for ksni with string ID for callback dispatch.
#[derive(Debug)]
enum KsniMenuEntry {
    Item {
        label: String,
        enabled: bool,
        /// Menu ID string for dispatching via the thread-local callback registry.
        menu_id: Option<String>,
        /// A checkbox or radio item's state.
        toggle: Option<KsniToggle>,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<KsniMenuEntry>,
    },
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Counter for generating unique menu IDs on Linux (ksni doesn't use muda).
static KSNI_MENU_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// What a toggle entry's watcher reads: its key and the item's state.
type ToggleWatch = (u64, Rc<dyn Fn() -> Option<bool>>);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Convert a unified Menu into ksni entries, registering callbacks on the main thread.
///
/// Returns the [`MenuRegistration`](crate::menu::MenuRegistration) holding this
/// build's ids. Every build mints fresh ids from [`KSNI_MENU_COUNTER`], so a
/// rebuilt tray can never overwrite its own previous entries — without the token
/// the registry would grow by the item count on every rebuild, forever.
///
/// Also returns each checkbox and radio entry's state reader, which the caller
/// watches once it has somewhere to send changes ([`watch_ksni_toggles`]).
fn convert_menu_to_ksni_entries(
    menu: Menu,
) -> (
    Vec<KsniMenuEntry>,
    crate::menu::MenuRegistration,
    Vec<ToggleWatch>,
) {
    let mut registration = crate::menu::MenuRegistration::default();
    let mut toggles = Vec::new();
    let entries = convert_menu_entries_inner(menu, &mut registration, &mut toggles);
    (entries, registration, toggles)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn next_ksni_id() -> u64 {
    KSNI_MENU_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn convert_menu_entries_inner(
    menu: Menu,
    registration: &mut crate::menu::MenuRegistration,
    toggles: &mut Vec<ToggleWatch>,
) -> Vec<KsniMenuEntry> {
    menu.take_entries()
        .into_iter()
        .map(|entry| match entry {
            crate::menu::MenuEntryKind::Item {
                label,
                enabled,
                callback,
                callback_owner,
                toggle,
            } => {
                // A disabled item fires nothing: ksni never activates one. Minting
                // an id and registering its callback anyway left a registry entry
                // that could never be dispatched, holding the closure's captures
                // alive for the life of the tray (#377) — the muda side returns
                // early in `build_muda_item` for the same reason.
                let callback = callback.filter(|_| enabled);
                let menu_id = callback.map(|cb| {
                    let id = format!("ksni-{}", next_ksni_id());
                    registration.register_callback(&id, cb, callback_owner);
                    id
                });
                // A toggle shows its state whether or not it is enabled.
                let toggle = toggle.map(|t| {
                    let key = next_ksni_id();
                    let checked = rinch_core::reactive::untracked(|| (t.is_on)()).unwrap_or(false);
                    toggles.push((key, t.is_on));
                    KsniToggle {
                        key,
                        radio_group: match t.kind {
                            crate::menu::ToggleKind::Check => None,
                            crate::menu::ToggleKind::Radio { group } => Some(group),
                        },
                        checked,
                    }
                });
                KsniMenuEntry::Item {
                    label,
                    enabled,
                    menu_id,
                    toggle,
                }
            }
            crate::menu::MenuEntryKind::Separator => KsniMenuEntry::Separator,
            crate::menu::MenuEntryKind::Submenu { label, menu } => KsniMenuEntry::Submenu {
                label,
                entries: convert_menu_entries_inner(menu, registration, toggles),
            },
        })
        .collect()
}

/// Make each toggle entry's mark follow its item's signal: a watcher, owned by
/// `registration`, that queues the new state on `updates` whenever it changes.
///
/// The watcher's first run only subscribes — the entries were built with the
/// state as it was then — so building a menu queues nothing.
#[cfg(target_os = "linux")]
fn watch_ksni_toggles(
    registration: &mut crate::menu::MenuRegistration,
    toggles: Vec<ToggleWatch>,
    updates: &Rc<KsniUpdater>,
) {
    for (key, is_on) in toggles {
        let updates = Rc::downgrade(updates);
        let mut first = true;
        registration.watch(move || {
            let on = is_on();
            if std::mem::take(&mut first) {
                return;
            }
            if let (Some(on), Some(updates)) = (on, updates.upgrade()) {
                updates.push(|p| {
                    p.checks.insert(key, on);
                });
            }
        });
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// The changes waiting for the Linux update thread, coalesced: each field holds
/// only the newest value asked for.
#[derive(Default)]
struct KsniPending {
    icon: Option<Vec<ksni_icon::Icon>>,
    tooltip: Option<String>,
    menu: Option<Vec<KsniMenuEntry>>,
    /// The newest state of each check mark that changed, by entry key.
    checks: std::collections::HashMap<u64, bool>,
}

/// `ksni::Icon` on Linux; a stand-in elsewhere so the coalescing logic (which
/// has no platform in it) compiles and is tested on every host.
mod ksni_icon {
    #[cfg(target_os = "linux")]
    pub(super) use ksni::Icon;
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    pub(super) struct Icon;
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl KsniPending {
    fn is_empty(&self) -> bool {
        self.icon.is_none()
            && self.tooltip.is_none()
            && self.menu.is_none()
            && self.checks.is_empty()
    }

    /// Apply these changes to the tray the service serves.
    #[cfg(target_os = "linux")]
    fn apply(self, tray: &mut RinchKsniTray) {
        if let Some(icon) = self.icon {
            tray.icon_pixmap = icon;
        }
        if let Some(tooltip) = self.tooltip {
            tray.tooltip = tooltip;
        }
        if let Some(menu) = self.menu {
            tray.menu_entries = menu;
        }
        if !self.checks.is_empty() {
            set_checks(&mut tray.menu_entries, &self.checks);
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Set each toggle entry named in `checks` to its state there.
fn set_checks(entries: &mut [KsniMenuEntry], checks: &std::collections::HashMap<u64, bool>) {
    for entry in entries {
        match entry {
            KsniMenuEntry::Item {
                toggle: Some(toggle),
                ..
            } => {
                if let Some(&on) = checks.get(&toggle.key) {
                    toggle.checked = on;
                }
            }
            KsniMenuEntry::Submenu { entries, .. } => set_checks(entries, checks),
            _ => {}
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// The thread that carries runtime changes to a Linux tray's ksni service.
///
/// [`push`](Self::push) merges a change into the pending set and wakes the
/// thread; it never waits for the service. The thread takes the whole set and
/// applies it in one `Handle::update`, which blocks *that* thread until the
/// service has emitted its change signals — a hung StatusNotifierWatcher holds
/// it there, and every change made meanwhile is merged into the next set
/// rather than queued. So the caller never blocks, and memory held behind a
/// hung watcher is one set of changes, not one per call.
struct KsniUpdater {
    shared: Arc<KsniUpdaterShared>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Default)]
struct KsniUpdaterShared {
    state: std::sync::Mutex<(KsniPending, bool)>,
    wake: std::sync::Condvar,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl KsniUpdater {
    /// Start the thread, which hands each set of changes to `sink`.
    fn spawn(mut sink: impl FnMut(KsniPending) + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(KsniUpdaterShared::default());
        let theirs = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("rinch-tray-update".into())
            .spawn(move || {
                loop {
                    let pending = {
                        let mut state = theirs.state.lock().unwrap_or_else(|p| p.into_inner());
                        while state.0.is_empty() && !state.1 {
                            state = theirs.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                        }
                        if state.1 {
                            return;
                        }
                        std::mem::take(&mut state.0)
                    };
                    sink(pending);
                }
            })?;
        Ok(Self { shared })
    }

    /// Merge a change into the pending set and wake the thread.
    fn push(&self, change: impl FnOnce(&mut KsniPending)) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.1 {
            return;
        }
        change(&mut state.0);
        drop(state);
        self.shared.wake.notify_one();
    }

    /// Stop the thread once it is done with the set it holds; what is still
    /// pending is dropped.
    fn close(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.1 = true;
        state.0 = KsniPending::default();
        drop(state);
        self.shared.wake.notify_one();
    }
}

impl Drop for KsniUpdater {
    fn drop(&mut self) {
        self.close();
    }
}

/// The StatusNotifierItem id for a tray whose builder asked for `explicit`
/// (issue #1529): that, else the `WindowProps::app_id` the running app
/// declared, else the executable's file name (`resolved_app_id`, the Wayland
/// `app_id`'s derivation). It used to be the constant `"rinch-app"` for every
/// app, so panels that key per-item settings on the id treated all rinch apps
/// as one item.
#[cfg(target_os = "linux")]
fn tray_item_id(explicit: Option<&str>) -> String {
    let declared = crate::shell::rinch_runtime::declared_app_id();
    crate::shell::rinch_runtime::resolved_app_id(explicit.or(declared))
}

#[cfg(target_os = "linux")]
struct RinchKsniTray {
    /// The item's `Id` ([`tray_item_id`]).
    id: String,
    tooltip: String,
    icon_pixmap: Vec<ksni::Icon>,
    menu_entries: Vec<KsniMenuEntry>,
    /// The registry id of the [`TrayIconBuilder::on_activate`] callback;
    /// `None` shows the window.
    activate_id: Option<String>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for RinchKsniTray {
    fn id(&self) -> String {
        self.id.clone()
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

    /// A primary click on the tray icon: the `on_activate` callback, or show
    /// the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        run_activate(self.activate_id.clone());
    }

    /// Right-click context menu.
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        build_ksni_menu(&self.menu_entries)
    }
}

/// A closure for a ksni item that dispatches `menu_id` on the main thread.
#[cfg(target_os = "linux")]
fn ksni_dispatch(menu_id: &Option<String>) -> Box<dyn Fn(&mut RinchKsniTray) + Send> {
    let id = menu_id.clone();
    Box::new(move |_this: &mut RinchKsniTray| {
        if let Some(id) = &id {
            let id = id.clone();
            crate::shell::rinch_runtime::run_on_main_thread(move || {
                crate::menu::dispatch_menu_event(&id);
            });
        }
    })
}

#[cfg(target_os = "linux")]
fn build_ksni_menu(entries: &[KsniMenuEntry]) -> Vec<ksni::MenuItem<RinchKsniTray>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        match &entries[i] {
            // A run of adjacent options of one radio group is one ksni
            // `RadioGroup`: that is what draws them as radio buttons.
            KsniMenuEntry::Item {
                toggle:
                    Some(KsniToggle {
                        radio_group: Some(group),
                        ..
                    }),
                ..
            } => {
                let group = *group;
                let run: Vec<&KsniMenuEntry> = entries[i..]
                    .iter()
                    .take_while(|e| {
                        matches!(e, KsniMenuEntry::Item {
                            toggle: Some(KsniToggle { radio_group: Some(g), .. }),
                            ..
                        } if *g == group)
                    })
                    .collect();
                i += run.len();
                out.push(ksni_radio_group(&run));
            }
            KsniMenuEntry::Item {
                label,
                enabled,
                menu_id,
                toggle: Some(toggle),
            } => {
                out.push(ksni::MenuItem::Checkmark(ksni::menu::CheckmarkItem {
                    label: label.clone(),
                    enabled: *enabled,
                    checked: toggle.checked,
                    activate: ksni_dispatch(menu_id),
                    ..Default::default()
                }));
                i += 1;
            }
            KsniMenuEntry::Item {
                label,
                enabled,
                menu_id,
                toggle: None,
            } => {
                out.push(ksni::MenuItem::Standard(ksni::menu::StandardItem {
                    label: label.clone(),
                    enabled: *enabled,
                    activate: ksni_dispatch(menu_id),
                    ..Default::default()
                }));
                i += 1;
            }
            KsniMenuEntry::Separator => {
                out.push(ksni::MenuItem::Separator);
                i += 1;
            }
            KsniMenuEntry::Submenu { label, entries } => {
                out.push(ksni::MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: label.clone(),
                    submenu: build_ksni_menu(entries),
                    ..Default::default()
                }));
                i += 1;
            }
        }
    }
    out
}

/// One ksni `RadioGroup` from a run of adjacent radio entries of one group.
#[cfg(target_os = "linux")]
fn ksni_radio_group(run: &[&KsniMenuEntry]) -> ksni::MenuItem<RinchKsniTray> {
    let mut options = Vec::new();
    let mut ids = Vec::new();
    // No option selected (the signal holds a value none of them has) is an
    // index past the end: ksni then marks none.
    let mut selected = usize::MAX;
    for (idx, entry) in run.iter().enumerate() {
        if let KsniMenuEntry::Item {
            label,
            enabled,
            menu_id,
            toggle,
        } = entry
        {
            if toggle.as_ref().is_some_and(|t| t.checked) && selected == usize::MAX {
                selected = idx;
            }
            options.push(ksni::menu::RadioItem {
                label: label.clone(),
                enabled: *enabled,
                ..Default::default()
            });
            ids.push(menu_id.clone());
        }
    }
    ksni::MenuItem::RadioGroup(ksni::menu::RadioGroup {
        selected,
        select: Box::new(move |_this: &mut RinchKsniTray, idx: usize| {
            if let Some(Some(id)) = ids.get(idx) {
                let id = id.clone();
                crate::shell::rinch_runtime::run_on_main_thread(move || {
                    crate::menu::dispatch_menu_event(&id);
                });
            }
        }),
        options,
    })
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

        let (entries, registration, _) = convert_menu_to_ksni_entries(menu);

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
                        ..
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
        let (_entries, registration, _) = convert_menu_to_ksni_entries(
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
            updates: None,
            _menu: RefCell::new(Some(registration)),
            _activate: None,
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
            let (_entries, registration, _) = convert_menu_to_ksni_entries(menu);
            TrayIcon {
                service: Some(Box::new(Slow(RefCell::new(Some(closed))))),
                updates: None,
                _menu: RefCell::new(Some(registration)),
                _activate: None,
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

    /// Issue #1520: a change never waits for the service. While the update
    /// thread is held (a hung watcher holds `Handle::update`), pushes return at
    /// once and coalesce: when it is released it applies one set holding only
    /// the newest of each change, not one update per push.
    #[test]
    fn updates_never_block_and_coalesce_behind_a_held_service() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (got_tx, got_rx) = mpsc::channel::<(Option<String>, usize, usize)>();
        let updater = KsniUpdater::spawn(move |p: KsniPending| {
            let _ = got_tx.send((
                p.tooltip.clone(),
                p.checks.len(),
                p.icon.as_ref().map_or(0, |i| i.len()),
            ));
            // Every set waits for the gate, as a hung `Handle::update` would.
            let _ = gate_rx.recv();
        })
        .unwrap();

        updater.push(|p| p.tooltip = Some("first".into()));
        let first = got_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.0.as_deref(), Some("first"));

        // The thread is now held. A burst of changes returns promptly.
        let t0 = Instant::now();
        for n in 0..1000u64 {
            updater.push(|p| p.tooltip = Some(format!("t{n}")));
            updater.push(|p| {
                p.checks.insert(n % 3, n % 2 == 0);
            });
        }
        assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
        assert!(got_rx.try_recv().is_err(), "nothing applied while held");

        gate_tx.send(()).unwrap();
        let second = got_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(second.0.as_deref(), Some("t999"), "only the newest tooltip");
        assert_eq!(second.1, 3, "one state per check mark");
        gate_tx.send(()).unwrap();
        assert!(
            got_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "the burst was one set, not a queue"
        );

        // Closed: further pushes go nowhere.
        updater.close();
        updater.push(|p| p.tooltip = Some("late".into()));
        let _ = gate_tx.send(());
        assert!(got_rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    /// A tray whose update thread feeds a channel instead of a ksni service.
    #[cfg(target_os = "linux")]
    fn tray_with_probe() -> (TrayIcon, std::sync::mpsc::Receiver<KsniPending>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let updater = KsniUpdater::spawn(move |p| {
            let _ = tx.send(p);
        })
        .unwrap();
        let tray = TrayIcon {
            service: None,
            updates: Some(Rc::new(updater)),
            _menu: RefCell::new(None),
            _activate: None,
        };
        (tray, rx)
    }

    #[cfg(target_os = "linux")]
    fn next(rx: &std::sync::mpsc::Receiver<KsniPending>) -> KsniPending {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an update reached the service")
    }

    /// Issue #1520: `set_icon` and `set_tooltip` reach the service, the icon
    /// converted to the ARGB a StatusNotifierItem carries.
    #[cfg(target_os = "linux")]
    #[test]
    fn set_icon_and_set_tooltip_reach_the_service() {
        let (tray, rx) = tray_with_probe();
        let image = TrayIconImage::from_rgba(vec![10, 20, 30, 40, 50, 60, 70, 80], 2, 1).unwrap();
        tray.set_icon(&image).unwrap();
        let p = next(&rx);
        let icon = &p.icon.expect("an icon")[0];
        assert_eq!((icon.width, icon.height), (2, 1));
        assert_eq!(
            icon.data,
            [40, 10, 20, 30, 80, 50, 60, 70],
            "RGBA became ARGB"
        );
        tray.set_tooltip("Listening").unwrap();
        assert_eq!(next(&rx).tooltip.as_deref(), Some("Listening"));
        assert!(
            TrayIconImage::from_rgba(vec![0; 7], 2, 1).is_err(),
            "short pixels refused"
        );
    }

    /// Issue #1520: `set_menu` hands the new entries over and releases the
    /// previous menu's callbacks and watchers at once; a check item's signal
    /// written by the app (not a click) moves its mark through the update
    /// thread, and stops doing so once its menu is replaced.
    #[cfg(target_os = "linux")]
    #[test]
    fn set_menu_replaces_the_menu_and_its_watchers() {
        use rinch_core::reactive::Signal;

        let before = crate::menu::callback_count();
        let (tray, rx) = tray_with_probe();
        let on = Signal::new(false);
        tray.set_menu(Some(
            Menu::new()
                .item(MenuItem::checkbox("Mute", on))
                .item(MenuItem::new("Start").on_click(|| {})),
        ));
        let p = next(&rx);
        let entries = p.menu.expect("a menu");
        let Some(KsniMenuEntry::Item {
            toggle: Some(toggle),
            menu_id: Some(_),
            ..
        }) = entries.first()
        else {
            panic!("the checkbox entry: {entries:?}");
        };
        assert!(!toggle.checked);
        assert_eq!(crate::menu::callback_count(), before + 2);
        assert!(rx.try_recv().is_err(), "building the menu queued no mark");

        on.set(true);
        let p = next(&rx);
        assert_eq!(
            p.checks.get(&toggle.key),
            Some(&true),
            "the app write reached it"
        );

        tray.set_menu(Some(
            Menu::new().item(MenuItem::new("Stop").on_click(|| {})),
        ));
        let p = next(&rx);
        assert_eq!(p.menu.map(|m| m.len()), Some(1));
        assert_eq!(
            crate::menu::callback_count(),
            before + 1,
            "the old menu's callbacks were released"
        );
        on.set(false);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "the old menu's watcher is gone"
        );
        drop(tray);
        assert_eq!(crate::menu::callback_count(), before);
    }

    /// Issue #1520: a toggle's watcher only subscribes on its first run — the
    /// entries were built with that state — so watching a fresh menu (as
    /// `build` does, right after the service registered) sends nothing.
    #[cfg(target_os = "linux")]
    #[test]
    fn watching_a_fresh_menu_sends_nothing_until_a_write() {
        use rinch_core::reactive::Signal;
        let (tx, rx) = std::sync::mpsc::channel();
        let updates = Rc::new(
            KsniUpdater::spawn(move |p| {
                let _ = tx.send(p);
            })
            .unwrap(),
        );
        let on = Signal::new(true);
        let (_entries, mut registration, toggles) =
            convert_menu_to_ksni_entries(Menu::new().item(MenuItem::checkbox("Mute", on)));
        watch_ksni_toggles(&mut registration, toggles, &updates);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
        on.set(false);
        assert_eq!(
            next(&rx).checks.values().copied().collect::<Vec<_>>(),
            [false]
        );
    }

    /// Issue #1520: adjacent radio options of one group are one ksni
    /// `RadioGroup` with the selected index; a checkbox is a `Checkmark`; an
    /// option of another group starts a group of its own.
    #[cfg(target_os = "linux")]
    #[test]
    fn ksni_draws_checkboxes_and_radio_groups() {
        use rinch_core::reactive::Signal;
        let size = Signal::new(2u8);
        let other = Signal::new(true);
        let (entries, _registration, _) = convert_menu_to_ksni_entries(
            Menu::new()
                .item(MenuItem::checkbox("Mute", Signal::new(true)))
                .item(MenuItem::radio("S", size, 1))
                .item(MenuItem::radio("M", size, 2))
                .item(MenuItem::radio("L", size, 3))
                .item(MenuItem::radio("Yes", other, true)),
        );
        let items = build_ksni_menu(&entries);
        assert_eq!(
            items.len(),
            3,
            "checkbox, one group of three, one group of one"
        );
        match &items[0] {
            ksni::MenuItem::Checkmark(c) => assert!(c.checked),
            _ => panic!("the checkbox is a Checkmark"),
        }
        match &items[1] {
            ksni::MenuItem::RadioGroup(g) => {
                assert_eq!(g.options.len(), 3);
                assert_eq!(g.selected, 1, "M is selected");
            }
            _ => panic!("the run is a RadioGroup"),
        }
        match &items[2] {
            ksni::MenuItem::RadioGroup(g) => assert_eq!((g.options.len(), g.selected), (1, 0)),
            _ => panic!("another group's option is a RadioGroup of its own"),
        }
    }

    /// Issue #1529: the item's id is the app's own, not the constant
    /// `"rinch-app"` every rinch app used to share. A test binary is never
    /// named `rinch-app`, so this fails on the constant.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_tray_item_id_is_the_apps_own() {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .expect("the test binary has a path");
        assert_ne!(exe, "rinch-app");

        let default = TrayIconBuilder::new();
        let tray = RinchKsniTray {
            id: default.item_id(),
            tooltip: String::new(),
            icon_pixmap: Vec::new(),
            menu_entries: Vec::new(),
            activate_id: None,
        };
        assert_eq!(ksni::Tray::id(&tray), exe, "derived from the executable");

        let named = TrayIconBuilder::new().with_id("com.example.notes");
        let tray = RinchKsniTray {
            id: named.item_id(),
            ..tray
        };
        assert_eq!(
            ksni::Tray::id(&tray),
            "com.example.notes",
            "with_id wins, verbatim"
        );
    }

    /// Issue #1520: a primary click on the icon runs `on_activate`'s callback
    /// on the main thread, through the registry (so the #183 lifetime rule
    /// holds), and not `show_current_window`.
    #[cfg(target_os = "linux")]
    #[test]
    fn activate_runs_the_on_activate_callback() {
        use std::cell::Cell;
        let ran = Rc::new(Cell::new(0));
        let r = ran.clone();
        let builder = TrayIconBuilder::new().on_activate(move || r.set(r.get() + 1));
        let (activate_id, registration) = register_activate(builder.activate);
        assert!(activate_id.is_some() && registration.is_some());
        let mut tray = RinchKsniTray {
            id: String::new(),
            tooltip: String::new(),
            icon_pixmap: Vec::new(),
            menu_entries: Vec::new(),
            activate_id,
        };
        ksni::Tray::activate(&mut tray, 0, 0);
        assert_eq!(
            ran.get(),
            0,
            "it runs on the main thread's next drain, not inline"
        );
        rinch_core::drain_main_callbacks();
        assert_eq!(ran.get(), 1);
        drop(registration);
        ksni::Tray::activate(&mut tray, 0, 0);
        rinch_core::drain_main_callbacks();
        assert_eq!(ran.get(), 1, "a released callback does not run");
    }

    // ── review-1527 fixtures ────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    fn first_toggle(entries: &[KsniMenuEntry]) -> (Option<String>, KsniToggle) {
        for e in entries {
            if let KsniMenuEntry::Item {
                menu_id,
                toggle: Some(t),
                ..
            } = e
            {
                return (menu_id.clone(), t.clone());
            }
        }
        panic!("no toggle in {entries:?}");
    }

    /// A checkbox whose own on_click replaces the menu (with the same item):
    /// no borrow panic, the new menu is built with the flipped state, the old
    /// callbacks are released and nothing more is queued.
    #[cfg(target_os = "linux")]
    #[test]
    fn review_1527_set_menu_from_its_own_checkbox_callback() {
        use rinch_core::reactive::Signal;
        let before = crate::menu::callback_count();
        let (tray, rx) = tray_with_probe();
        let tray = Rc::new(tray);
        let on = Signal::new(false);
        fn menu(on: rinch_core::reactive::Signal<bool>, tray: std::rc::Weak<TrayIcon>) -> Menu {
            Menu::new().item(MenuItem::checkbox("Mute", on).on_click(move || {
                if let Some(t) = tray.upgrade() {
                    t.set_menu(Some(menu(on, Rc::downgrade(&t))));
                }
            }))
        }
        tray.set_menu(Some(menu(on, Rc::downgrade(&tray))));
        let (id, t0) = first_toggle(&next(&rx).menu.unwrap());
        assert!(!t0.checked);
        assert_eq!(crate::menu::callback_count(), before + 1);
        assert!(crate::menu::dispatch_menu_event(&id.unwrap()));
        assert!(on.get());
        let p = next(&rx);
        let (id2, t1) = first_toggle(p.menu.as_ref().expect("the replacement menu"));
        assert!(t1.checked, "rebuilt with the flipped state");
        assert!(p.checks.is_empty(), "{:?}", p.checks);
        assert_eq!(
            crate::menu::callback_count(),
            before + 1,
            "old callback released"
        );
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
        // A second round trip through the new menu.
        assert!(crate::menu::dispatch_menu_event(&id2.unwrap()));
        assert!(!on.get());
        let (_, t2) = first_toggle(&next(&rx).menu.unwrap());
        assert!(!t2.checked);
        drop(tray);
        assert_eq!(crate::menu::callback_count(), before);
    }

    /// Dropping a tray ends its update thread; set_menu spawns none.
    ///
    /// The thread is the only other holder of its updater's shared state, so
    /// the state is freed exactly when the thread has ended. (The review's
    /// version counted `rinch-tray-update` threads process-wide, which races
    /// with other tests' trays running in parallel.)
    #[cfg(target_os = "linux")]
    #[test]
    fn review_1527_update_threads_do_not_leak() {
        let mut ends = Vec::new();
        for _ in 0..20 {
            let (tray, rx) = tray_with_probe();
            let shared = Arc::downgrade(&tray.updates.as_ref().unwrap().shared);
            for n in 0..10 {
                tray.set_menu(Some(
                    Menu::new().item(MenuItem::new(format!("i{n}")).on_click(|| {})),
                ));
            }
            let _ = next(&rx);
            assert_eq!(
                shared.strong_count(),
                2,
                "one thread per tray, none per set_menu"
            );
            drop(tray);
            ends.push(shared);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while ends.iter().any(|w| w.strong_count() > 0) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            ends.iter().all(|w| w.strong_count() == 0),
            "every update thread ended"
        );
    }

    /// Radio groups are keyed by the signal's slot id **and generation**: a
    /// freed group's options next to a live group that reused its slot stay a
    /// group of their own (they merged when the key was the slot id alone).
    #[cfg(target_os = "linux")]
    #[test]
    fn review_1527_radio_group_key_survives_slot_reuse() {
        use rinch_core::reactive::{Scope, Signal};
        let scope = Scope::new();
        let stale = scope.run(|| {
            let a = Signal::new(1u8);
            (MenuItem::radio("A1", a, 1), MenuItem::radio("A2", a, 2))
        });
        scope.dispose();
        let b = Signal::new(1u8);
        let (entries, _r, _) = convert_menu_to_ksni_entries(
            Menu::new()
                .item(stale.0)
                .item(stale.1)
                .item(MenuItem::radio("B1", b, 1))
                .item(MenuItem::radio("B2", b, 2)),
        );
        let items = build_ksni_menu(&entries);
        assert_eq!(items.len(), 2, "two groups, not one");
    }

    /// Issue #1520, live: a burst of icon and tooltip swaps on a real tray
    /// returns at once and lands — the item's `Title` reads the last tooltip
    /// back over D-Bus. Needs a session bus with a StatusNotifierWatcher (a
    /// KDE session); `cargo test -p rinch --features system-tray --lib
    /// tray::tests::live_runtime -- --ignored`.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a live D-Bus session with a StatusNotifierWatcher"]
    fn live_runtime_swaps_reach_the_status_notifier_item() {
        use std::time::{Duration, Instant};
        let red = TrayIconImage::from_rgba([255, 0, 0, 255].repeat(16 * 16), 16, 16).unwrap();
        let blue = TrayIconImage::from_rgba([0, 0, 255, 255].repeat(16 * 16), 16, 16).unwrap();
        let tray = TrayIconBuilder::new()
            .with_tooltip("rinch #1520 probe")
            .with_icon(red.clone())
            .build()
            .expect("the tray builds");
        let t0 = Instant::now();
        for n in 0..200 {
            tray.set_icon(if n % 2 == 0 { &blue } else { &red })
                .unwrap();
            tray.set_tooltip(format!("swap {n}")).unwrap();
        }
        let took = t0.elapsed();
        eprintln!("200 swaps returned in {took:?}");
        assert!(took < Duration::from_millis(200), "{took:?}");
        let ours = format!("StatusNotifierItem-{}-", std::process::id());
        let title = || {
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
                .ok()?;
            let listed = String::from_utf8_lossy(&out.stdout).into_owned();
            let entry = listed.split('"').find(|e| e.contains(&ours))?.to_string();
            let (service, path) = entry.split_once('/')?;
            let out = std::process::Command::new("busctl")
                .args([
                    "--user",
                    "get-property",
                    service,
                    &format!("/{path}"),
                    "org.kde.StatusNotifierItem",
                    "Title",
                ])
                .output()
                .ok()?;
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let mut seen = None;
        for _ in 0..40 {
            seen = title();
            if seen.as_deref().is_some_and(|t| t.contains("swap 199")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            seen.as_deref().is_some_and(|t| t.contains("swap 199")),
            "the last tooltip reached the item: {seen:?}"
        );
        drop(tray);
    }
}
