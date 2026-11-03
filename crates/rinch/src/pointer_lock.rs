//! Pointer lock and raw pointer motion (issue #2092), re-exported through
//! [`crate::window`].
//!
//! A game's mouse look or an editor's fly camera needs two things a window
//! does not give by default: the pointer kept in the window and out of sight,
//! and the mouse's motion as deltas that do not stop where the cursor would
//! meet the edge of the screen. [`request_pointer_lock`] (the window) and
//! [`request_pointer_lock_on`] (an element; a `RenderSurface` has
//! [`RenderSurfaceHandle::request_pointer_lock`](crate::render_surface::RenderSurfaceHandle::request_pointer_lock))
//! ask for the first, [`on_pointer_motion`] and
//! [`SurfaceEvent::PointerMotion`](crate::render_surface::SurfaceEvent::PointerMotion)
//! deliver the second, and [`pointer_lock`] says, reactively, what was granted.
//!
//! # Who does the grabbing
//!
//! The state here is per thread (the UI thread); the grab itself is a
//! [`PointerLockHost`]'s, installed with [`set_pointer_lock_host`]:
//!
//! - **Desktop**: the shell installs one for the main window when it creates
//!   it (winit's `set_cursor_grab`, `Locked` and else `Confined`, and
//!   `set_cursor_visible(false)`), and removes it when the window is hidden.
//! - **Web**: `rinch-web` installs one at mount (`requestPointerLock`,
//!   `exitPointerLock`, `pointerlockchange`), and forwards `movementX`/`Y`.
//! - **Native embed** (`RinchContext`) and anything else: none, so a request
//!   is [`PointerLockError::Unsupported`] until the host installs its own.
//!
//! # Release
//!
//! A lock ends — and the host is told to give the pointer back — on
//! [`exit_pointer_lock`]; when the window loses focus; on Escape, which is
//! consumed (it closes no modal and reaches no handler: the browser's rule);
//! when the element that holds it leaves the document or the `RenderSurface`
//! that holds it unmounts; and when the desktop window is hidden.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::Signal;
use rinch_core::dom::NodeHandle;

/// What [`request_pointer_lock`] got: read it with [`pointer_lock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerLockMode {
    /// The pointer is held where it was, and hidden. Motion arrives only as
    /// deltas ([`on_pointer_motion`]); the cursor position the document sees
    /// does not move. Wayland, Windows, macOS and the browser.
    Locked,
    /// The pointer is kept inside the window, and hidden, but it still moves
    /// there; the platform could not hold it still. X11, whose winit backend
    /// has no lock. Raw motion still arrives unclamped at the window's edge,
    /// and the document's cursor position is held where it was while the
    /// lock lasts, as under `Locked`.
    Confined,
}

/// Why a pointer-lock request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PointerLockError {
    /// Nothing on this thread can grab the pointer: no window (before the
    /// event loop made one, or while it is hidden), Android, a native embed
    /// host that installed no [`PointerLockHost`], a WASI guest, or a call
    /// made off the UI thread.
    Unsupported,
    /// The element is not in a document (not mounted yet, or removed).
    NotConnected,
    /// The platform refused both the lock and the confinement; the text says
    /// why.
    Refused(String),
}

impl std::fmt::Display for PointerLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PointerLockError::Unsupported => f.write_str("pointer lock is not available here"),
            PointerLockError::NotConnected => f.write_str("the element is not in a document"),
            PointerLockError::Refused(why) => write!(f, "the platform refused the lock: {why}"),
        }
    }
}

impl std::error::Error for PointerLockError {}

/// One step of relative pointer motion: see [`on_pointer_motion`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PointerMotion {
    /// Horizontal motion, positive to the right.
    pub dx: f64,
    /// Vertical motion, positive downwards.
    pub dy: f64,
}

/// What a [`PointerLockHost`] is asked to lock the pointer to.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PointerLockTarget {
    /// The window ([`request_pointer_lock`]). A browser host locks the page's
    /// root element.
    Window,
    /// An element ([`request_pointer_lock_on`], or a `RenderSurface`'s own).
    /// A desktop host grabs for the whole window either way; a browser host
    /// calls the element's `requestPointerLock`.
    Element(NodeHandle),
}

/// What grabs and releases the pointer for this thread's window: see the
/// [module docs](self). The desktop shell and `rinch-web` install their own;
/// a native embed host that wants pointer lock installs one with
/// [`set_pointer_lock_host`].
pub trait PointerLockHost {
    /// Grab the pointer for `target` and hide the cursor.
    ///
    /// `Ok(Some(mode))`: granted now, as `mode` (prefer
    /// [`Locked`](PointerLockMode::Locked); fall back to
    /// [`Confined`](PointerLockMode::Confined) where the platform cannot hold
    /// the pointer still). `Ok(None)`: asked for, and the platform will answer
    /// later (the browser) — report it with [`pointer_lock_changed`].
    /// `Err`: refused.
    fn lock(
        &mut self,
        target: &PointerLockTarget,
    ) -> Result<Option<PointerLockMode>, PointerLockError>;

    /// Give the pointer back and show the cursor. Called only while a lock is
    /// held or pending.
    fn unlock(&mut self);
}

/// A lock that is held, or asked for and pending.
struct Held {
    target: PointerLockTarget,
    /// The `RenderSurface` that asked, which hears the motion as
    /// `SurfaceEvent::PointerMotion`.
    surface: Option<usize>,
}

type MotionHandler = Rc<RefCell<dyn FnMut(PointerMotion)>>;

thread_local! {
    static HOST: RefCell<Option<Box<dyn PointerLockHost>>> = const { RefCell::new(None) };
    static HELD: RefCell<Option<Held>> = const { RefCell::new(None) };
    /// The granted mode; [`MODE_SIGNAL`] follows it once something reads it.
    static MODE: Cell<Option<PointerLockMode>> = const { Cell::new(None) };
    static MODE_SIGNAL: Cell<Option<Signal<Option<PointerLockMode>>>> = const { Cell::new(None) };
    static HANDLERS: RefCell<Vec<(u64, MotionHandler)>> = const { RefCell::new(Vec::new()) };
    static NEXT_HANDLER: Cell<u64> = const { Cell::new(1) };
}

/// Install (or, with `None`, remove) this thread's [`PointerLockHost`].
///
/// A lock held through the old host is released through it first: the new
/// one knows nothing of it.
pub fn set_pointer_lock_host(host: Option<Box<dyn PointerLockHost>>) {
    release();
    HOST.with(|h| *h.borrow_mut() = host);
}

/// Run `f` on the installed host, out of its cell so the host may call back
/// into this module.
fn with_host<R>(f: impl FnOnce(&mut dyn PointerLockHost) -> R) -> Option<R> {
    let mut host = HOST.with(|h| h.borrow_mut().take())?;
    let out = f(&mut *host);
    HOST.with(|h| {
        let mut slot = h.borrow_mut();
        // A host installed while this one ran (from inside it) wins.
        if slot.is_none() {
            *slot = Some(host);
        }
    });
    Some(out)
}

fn set_mode(mode: Option<PointerLockMode>) {
    MODE.with(|m| m.set(mode));
    if let Some(signal) = MODE_SIGNAL.with(Cell::get) {
        signal.set_if_changed(mode);
    }
}

/// Lock the pointer to the window: hide it, keep it from leaving, and deliver
/// the mouse's motion as deltas ([`on_pointer_motion`]).
///
/// - **Desktop**: asks winit for `CursorGrabMode::Locked`, and for `Confined`
///   where that is refused (X11), and hides the cursor. The answer is
///   immediate: after `Ok`, [`pointer_lock`] already says which was granted.
///   While the lock lasts the document's pointer position does not move —
///   hover stays where it was, and a press lands where the pointer was when
///   the lock began.
/// - **Web**: calls `requestPointerLock` on the page's root element. The
///   browser grants it only during a user gesture (a click or key handler;
///   Firefox also asks for an engagement gesture on some pages) and answers
///   later: after `Ok`, [`pointer_lock`] turns `Some` when the browser has
///   locked, and stays `None` if it refused.
/// - **Android, WASI, native embed** without a [`PointerLockHost`]:
///   [`PointerLockError::Unsupported`].
///
/// The lock ends on [`exit_pointer_lock`], when the window loses focus, and
/// on Escape (see the [module docs](self)). A second request moves the lock to
/// the new target.
pub fn request_pointer_lock() -> Result<(), PointerLockError> {
    request(PointerLockTarget::Window, None)
}

/// Lock the pointer as [`request_pointer_lock`] does, held by `element`: the
/// lock ends when the element leaves the document (unmounted, removed), as
/// well as on everything that ends a window's lock. In the browser the
/// element itself is locked (`element.requestPointerLock()`), so its own
/// pointer listeners keep hearing the motion; on the desktop the whole window
/// is grabbed either way.
pub fn request_pointer_lock_on(element: &NodeHandle) -> Result<(), PointerLockError> {
    request(PointerLockTarget::Element(element.clone()), None)
}

/// The shared request: `surface` is the `RenderSurface` asking, if one is.
pub(crate) fn request(
    target: PointerLockTarget,
    surface: Option<usize>,
) -> Result<(), PointerLockError> {
    if let PointerLockTarget::Element(element) = &target
        && !element.is_connected()
    {
        return Err(PointerLockError::NotConnected);
    }
    let granted = with_host(|host| host.lock(&target)).ok_or(PointerLockError::Unsupported)??;
    HELD.with(|h| *h.borrow_mut() = Some(Held { target, surface }));
    if let Some(mode) = granted {
        set_mode(Some(mode));
    }
    Ok(())
}

/// Release the pointer lock, if one is held or pending: the cursor shows
/// again and is free to leave the window. Nothing happens without one.
pub fn exit_pointer_lock() {
    release();
}

/// Release the lock through the host. Answers whether one was held or
/// pending.
pub(crate) fn release() -> bool {
    if HELD.with(|h| h.borrow_mut().take()).is_none() {
        return false;
    }
    with_host(|host| host.unlock());
    set_mode(None);
    true
}

/// What the pointer lock is: `Some(mode)` while it is held, `None` otherwise
/// (and while a browser's answer is pending). Reactive: a `{|| …}`, a memo or
/// an effect that reads it runs again when the lock is granted or lost, which
/// is how an app hears that Escape or a focus loss ended it.
pub fn pointer_lock() -> Option<PointerLockMode> {
    let signal = match MODE_SIGNAL.with(Cell::get) {
        Some(signal) => signal,
        None => {
            let signal = rinch_core::unowned(|| Signal::new(MODE.with(Cell::get)));
            MODE_SIGNAL.with(|s| s.set(Some(signal)));
            signal
        }
    };
    signal.get()
}

/// For a [`PointerLockHost`] whose platform granted or ended a lock on its
/// own: the browser's `pointerlockchange` (a granted request, Escape, a tab
/// switch), or an embed host's window losing its grab. `None` ends the lock
/// without asking the host to unlock; `Some` marks it granted (a lock the
/// host reports with no request on record is taken as the window's).
pub fn pointer_lock_changed(mode: Option<PointerLockMode>) {
    match mode {
        None => {
            HELD.with(|h| h.borrow_mut().take());
            set_mode(None);
        }
        Some(mode) => {
            HELD.with(|h| {
                h.borrow_mut().get_or_insert(Held {
                    target: PointerLockTarget::Window,
                    surface: None,
                });
            });
            set_mode(Some(mode));
        }
    }
}

/// Whether a lock is held (granted, not pending). Not reactive: for the shell.
// Unused in a build with no runtime to read it.
#[cfg_attr(
    not(any(feature = "desktop", feature = "android", feature = "embed")),
    allow(dead_code)
)]
pub(crate) fn is_locked() -> bool {
    MODE.with(Cell::get).is_some()
}

/// Whether raw motion has anywhere to go: a lock is held, or an
/// [`on_pointer_motion`] handler is registered. The desktop shell skips the
/// device's motion events otherwise.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub(crate) fn wants_motion() -> bool {
    is_locked() || HANDLERS.with(|h| !h.borrow().is_empty())
}

/// An Escape was pressed: a held lock ends, and the key is consumed (the
/// answer is `true`), as a browser consumes the Escape that exits its pointer
/// lock. Without a lock the key goes on as usual.
#[cfg_attr(
    not(any(feature = "desktop", feature = "android", feature = "embed")),
    allow(dead_code)
)]
pub(crate) fn escape() -> bool {
    is_locked() && release()
}

/// End a lock whose element left the document. Called on every event the
/// app handles (`AboutToWait` included), so a lock does not outlive its
/// element by more than one turn of the event loop.
#[cfg_attr(
    not(any(feature = "desktop", feature = "android", feature = "embed")),
    allow(dead_code)
)]
pub(crate) fn prune() {
    let element = HELD.with(|h| match h.borrow().as_ref() {
        Some(Held {
            target: PointerLockTarget::Element(element),
            ..
        }) => Some(element.clone()),
        _ => None,
    });
    if element.is_some_and(|e| !e.is_connected()) {
        release();
    }
}

/// `RenderSurface` `id` unmounted: a lock it holds ends with it.
pub(crate) fn surface_gone(id: usize) {
    let holds = HELD.with(|h| {
        h.borrow()
            .as_ref()
            .is_some_and(|held| held.surface == Some(id))
    });
    if holds {
        release();
    }
}

/// Whether `RenderSurface` `id` holds the lock (granted or pending).
pub(crate) fn held_by_surface(id: usize) -> bool {
    HELD.with(|h| {
        h.borrow()
            .as_ref()
            .is_some_and(|held| held.surface == Some(id))
    })
}

/// Deliver one step of raw pointer motion: to the `RenderSurface` holding a
/// granted lock (as `SurfaceEvent::PointerMotion`), then to every
/// [`on_pointer_motion`] handler, in one batch and untracked. Non-finite and
/// zero motion is dropped.
///
/// The desktop shell, `RinchApp` (for `PlatformEvent::PointerMotion`, which a
/// native embed host forwards) and `rinch-web` call this; a host with motion
/// of its own may too.
pub fn dispatch_pointer_motion(dx: f64, dy: f64) {
    if !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
        return;
    }
    let surface = if is_locked() {
        HELD.with(|h| h.borrow().as_ref().and_then(|held| held.surface))
    } else {
        None
    };
    let handlers: Vec<MotionHandler> =
        HANDLERS.with(|h| h.borrow().iter().map(|(_, f)| f.clone()).collect());
    if surface.is_none() && handlers.is_empty() {
        return;
    }
    let motion = PointerMotion { dx, dy };
    rinch_core::batch(|| {
        rinch_core::untracked_handler(|| {
            if let Some(id) = surface {
                crate::render_surface::dispatch_surface_event(
                    id,
                    crate::render_surface::SurfaceEvent::PointerMotion {
                        dx: dx as f32,
                        dy: dy as f32,
                    },
                );
            }
            for handler in handlers {
                // A handler that (somehow) dispatches motion from inside
                // itself is not run a second time.
                if let Ok(mut handler) = handler.try_borrow_mut() {
                    handler(motion);
                }
            }
        });
    });
}

/// A registration made by [`on_pointer_motion`].
#[derive(Debug)]
pub struct PointerMotionSubscription {
    id: u64,
}

impl PointerMotionSubscription {
    /// Stop delivering motion to the handler now, before its component goes.
    pub fn cancel(self) {
        remove_handler(self.id);
    }
}

fn remove_handler(id: u64) {
    let _ = HANDLERS.try_with(|h| h.borrow_mut().retain(|(i, _)| *i != id));
}

/// Hear the mouse's raw relative motion: `handler` gets each step's
/// [`PointerMotion`] (`dx`, `dy`).
///
/// - **Units.** The platform's own, not logical pixels: unaccelerated device
///   counts on X11 and Windows, unaccelerated surface units on Wayland,
///   points on macOS, `movementX`/`Y` in the browser (CSS pixels in Firefox;
///   Chrome's have been physical pixels on some platforms). Multiply by a
///   sensitivity the app chooses.
/// - **When.** While a pointer lock is held, every motion. Without one, on the
///   desktop, every motion while the window has focus (wherever the cursor
///   is, and unclamped at the screen's edge) — enough for a drag-to-orbit
///   camera; in the browser, while the pointer moves over the page. A burst
///   of motion within one turn of the desktop event loop is summed into one
///   step, as a browser sends one `mousemove` per frame.
/// - **Lifetime.** Registered with the component being rendered: the handler
///   is removed when that component unmounts. Outside a component it lives
///   until [`PointerMotionSubscription::cancel`].
///
/// The handler runs untracked and in a batch, like an event handler.
pub fn on_pointer_motion(
    handler: impl FnMut(PointerMotion) + 'static,
) -> PointerMotionSubscription {
    let id = NEXT_HANDLER.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let handler: MotionHandler = Rc::new(RefCell::new(handler));
    HANDLERS.with(|h| h.borrow_mut().push((id, handler)));
    rinch_core::reactive::on_cleanup(move || remove_handler(id));
    PointerMotionSubscription { id }
}

#[cfg(test)]
pub(crate) mod test_host {
    //! A [`PointerLockHost`] that records what it was asked, for tests.
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct Log {
        pub locks: Vec<&'static str>,
        pub unlocks: usize,
    }

    pub(crate) struct FakeHost {
        pub log: Rc<RefCell<Log>>,
        pub grant: Result<Option<PointerLockMode>, PointerLockError>,
    }

    impl PointerLockHost for FakeHost {
        fn lock(
            &mut self,
            target: &PointerLockTarget,
        ) -> Result<Option<PointerLockMode>, PointerLockError> {
            self.log.borrow_mut().locks.push(match target {
                PointerLockTarget::Window => "window",
                PointerLockTarget::Element(_) => "element",
            });
            self.grant.clone()
        }
        fn unlock(&mut self) {
            self.log.borrow_mut().unlocks += 1;
        }
    }

    /// Install a host that grants `grant`; answers its log.
    pub(crate) fn install(
        grant: Result<Option<PointerLockMode>, PointerLockError>,
    ) -> Rc<RefCell<Log>> {
        let log = Rc::new(RefCell::new(Log::default()));
        set_pointer_lock_host(Some(Box::new(FakeHost {
            log: log.clone(),
            grant,
        })));
        log
    }
}

#[cfg(test)]
mod tests {
    use super::test_host::install;
    use super::*;

    #[test]
    fn no_host_is_unsupported() {
        set_pointer_lock_host(None);
        assert_eq!(request_pointer_lock(), Err(PointerLockError::Unsupported));
        assert_eq!(pointer_lock(), None);
    }

    #[test]
    fn a_grant_is_reported_and_a_release_unlocks_once() {
        let log = install(Ok(Some(PointerLockMode::Confined)));
        let seen = Rc::new(Cell::new(None));
        let runs = Rc::new(Cell::new(0));
        let _effect = {
            let (seen, runs) = (seen.clone(), runs.clone());
            rinch_core::Effect::new(move || {
                runs.set(runs.get() + 1);
                seen.set(pointer_lock());
            })
        };
        assert_eq!(request_pointer_lock(), Ok(()));
        assert_eq!(seen.get(), Some(PointerLockMode::Confined));
        assert_eq!(log.borrow().locks, ["window"]);
        exit_pointer_lock();
        exit_pointer_lock();
        assert_eq!(seen.get(), None);
        assert_eq!(log.borrow().unlocks, 1, "a second exit has nothing to undo");
        assert_eq!(runs.get(), 3);
        set_pointer_lock_host(None);
    }

    #[test]
    fn a_refusal_holds_nothing() {
        let log = install(Err(PointerLockError::Refused("no".into())));
        assert_eq!(
            request_pointer_lock(),
            Err(PointerLockError::Refused("no".into()))
        );
        assert_eq!(pointer_lock(), None);
        assert!(!release(), "nothing is held");
        assert_eq!(log.borrow().unlocks, 0);
        set_pointer_lock_host(None);
    }

    #[test]
    fn a_pending_lock_is_granted_and_lost_by_the_host() {
        let log = install(Ok(None));
        assert_eq!(request_pointer_lock(), Ok(()));
        assert_eq!(pointer_lock(), None, "pending");
        assert!(!escape(), "Escape ends no pending lock");
        pointer_lock_changed(Some(PointerLockMode::Locked));
        assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
        pointer_lock_changed(None);
        assert_eq!(pointer_lock(), None);
        assert_eq!(log.borrow().unlocks, 0, "the host lost it; nothing to undo");
        set_pointer_lock_host(None);
    }

    #[test]
    fn motion_reaches_handlers_until_they_are_cancelled() {
        set_pointer_lock_host(None);
        let got = Rc::new(RefCell::new(Vec::new()));
        let sub = {
            let got = got.clone();
            on_pointer_motion(move |m| got.borrow_mut().push((m.dx, m.dy)))
        };
        assert!(wants_motion());
        dispatch_pointer_motion(3.5, -2.0);
        dispatch_pointer_motion(0.0, 0.0);
        dispatch_pointer_motion(f64::NAN, 1.0);
        sub.cancel();
        dispatch_pointer_motion(1.0, 1.0);
        assert_eq!(*got.borrow(), [(3.5, -2.0)]);
        assert!(!wants_motion());
    }

    #[test]
    fn a_handler_goes_with_its_scope() {
        let got = Rc::new(Cell::new(0));
        let scope = rinch_core::Scope::new();
        {
            let got = got.clone();
            scope.run(|| {
                let _ = on_pointer_motion(move |_| got.set(got.get() + 1));
            });
        }
        dispatch_pointer_motion(1.0, 0.0);
        scope.dispose();
        dispatch_pointer_motion(1.0, 0.0);
        assert_eq!(got.get(), 1);
    }
}
