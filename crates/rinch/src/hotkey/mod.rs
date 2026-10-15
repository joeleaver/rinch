//! System-wide hotkeys: bindings that fire while another application has the
//! keyboard focus, with **press and release** events (enable with the
//! `global-hotkeys` feature).
//!
//! ```ignore
//! use rinch::hotkey::{self, HotkeyState};
//!
//! let hk = hotkey::register("Ctrl+Alt+Space", move |ev| match ev.state {
//!     HotkeyState::Pressed => listening.set(true),
//!     HotkeyState::Released => listening.set(false),
//! })?;
//! hk.rebind("RightCtrl")?;   // a modifier-only binding: hold Right Ctrl
//! drop(hk);                  // unregisters
//! ```
//!
//! - **Callbacks run on the main thread**, one `batch` each, like every event
//!   handler, so they may set signals directly. Register on the main thread
//!   (a [`Hotkey`] is `!Send`).
//! - **Lifetime** follows the menu-callback rule (#183): a hotkey registered
//!   while a component renders belongs to that component and is unregistered
//!   when it unmounts; one registered outside any render (from `main`) lives
//!   until its [`Hotkey`] is dropped. Dropping the handle always unregisters.
//! - **Conflicts are errors.** A chord another application holds is
//!   [`HotkeyError::Conflict`], where the platform can tell (see below).
//! - **Shortcut strings** name physical keys — see [`Shortcut`] for the
//!   grammar (sided modifiers, modifier-only bindings).
//! - [`HotkeyCapture`] turns the keys pressed in a settings field into a
//!   shortcut string.
//!
//! # Backends
//!
//! | Platform | Plain chord (`Ctrl+Alt+Space`) | Modifier-only / sided (`RightCtrl`) |
//! |---|---|---|
//! | Windows | `RegisterHotKey` reserves it (conflicts reported, the keystroke is swallowed); a `WH_KEYBOARD_LL` hook reports press and release | the hook alone: not swallowed, no conflict check |
//! | macOS | Carbon `RegisterEventHotKey`: press/release, conflicts, no permission | a `CGEventTap`: needs Input Monitoring ([`permission`]) |
//! | X11 | `XGrabKey` reserves it (conflicts reported, swallowed) | XInput2 raw key events: not swallowed, no conflict check |
//! | Wayland | the `org.freedesktop.portal.GlobalShortcuts` portal: the user approves, and may change, the binding in a system dialog — [`Binding::SystemChosen`] | same |
//!
//! On X11 and Windows press/release of every binding comes from one stream
//! of every key on the system (the hook, XInput2 raw events); the OS hot-key
//! API is used only to reserve a chord.

mod backend;
mod capture;
mod matcher;
mod shortcut;

pub use capture::{CaptureStep, HotkeyCapture};
pub use shortcut::{Modifier, Shortcut, Side, Trigger};

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use rinch_core::reactive::{Owner, current_owner, on_cleanup, unowned, untracked_handler};

use matcher::BindingKey;

/// Whether a hotkey went down or came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyState {
    /// The hotkey was pressed. Auto-repeat does not press it again.
    Pressed,
    /// The hotkey was released.
    Released,
}

/// A hotkey event, delivered on the main thread.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HotkeyEvent {
    /// Pressed or released.
    pub state: HotkeyState,
    /// The binding's id ([`HotkeySpec::id`]).
    pub id: String,
    /// On a `Released`: another key went down while the hotkey was held —
    /// `Right Ctrl` then `C` for a `"RightCtrl"` binding. A modifier-only
    /// binding is pressed the moment its modifier goes down, before anyone
    /// can know a chord will follow; a hold-to-talk app uses the press as is,
    /// and a toggle app ignores a release with `chorded` set (the user was
    /// typing Ctrl+C, not asking for the hotkey). Always `false` on a
    /// `Pressed`, and on the backends that cannot see other keys (the Wayland
    /// portal, macOS Carbon chords).
    pub chorded: bool,
}

/// What a hotkey is bound to now.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Binding {
    /// Bound to exactly this shortcut.
    Exact(Shortcut),
    /// The system chose the binding (the Wayland portal: the user approves or
    /// changes it in a system dialog). `description` is the system's own text
    /// for it (`"Ctrl+Alt+Space"`), when it gave one.
    SystemChosen {
        /// The system's description of the trigger, if any.
        description: Option<String>,
    },
    /// Waiting for the system (the portal dialog is open). A
    /// [`Hotkey::on_binding_change`] callback hears the outcome.
    Pending,
    /// The system declined it (the user dismissed the portal dialog). The
    /// hotkey fires nothing; [`Hotkey::rebind`] asks again.
    Declined,
}

/// Which mechanism delivers hotkeys in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Backend {
    /// `RegisterHotKey` + a `WH_KEYBOARD_LL` hook.
    Windows,
    /// Carbon hot keys, plus a `CGEventTap` for modifier-only / sided bindings.
    MacOs,
    /// `XGrabKey` + XInput2 raw key events.
    X11,
    /// The `org.freedesktop.portal.GlobalShortcuts` portal.
    WaylandPortal,
    /// None: [`register`] answers [`HotkeyError::Unsupported`].
    Unsupported,
}

/// Whether the bindings registered now need a permission the user has not
/// granted. Only the macOS event tap (modifier-only and sided bindings)
/// needs one: Input Monitoring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    /// Nothing registered needs a permission.
    NotNeeded,
    /// A registered binding needs one, and it is granted.
    Granted,
    /// A registered binding needs one, and it is missing: those bindings fire
    /// nothing until the user grants it ([`request_permission`]) and the app
    /// registers them again.
    Missing,
}

/// Why a hotkey could not be registered.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HotkeyError {
    /// The shortcut string does not parse.
    Parse(String),
    /// Another application — or this one — holds the shortcut.
    Conflict(String),
    /// Another hotkey of this app already has this id.
    DuplicateId(String),
    /// The binding needs a permission the user has not granted (macOS Input
    /// Monitoring).
    PermissionMissing,
    /// No backend on this platform or session, or this backend cannot express
    /// the binding.
    Unsupported(String),
    /// The platform refused for another reason.
    Platform(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotkeyError::Parse(s) => write!(f, "invalid hotkey {s}"),
            HotkeyError::Conflict(s) => write!(f, "hotkey {s} is already taken"),
            HotkeyError::DuplicateId(s) => {
                write!(f, "a hotkey with id {s:?} is already registered")
            }
            HotkeyError::PermissionMissing => {
                f.write_str("global hotkeys need the Input Monitoring permission")
            }
            HotkeyError::Unsupported(s) => write!(f, "global hotkeys unsupported: {s}"),
            HotkeyError::Platform(s) => write!(f, "hotkey registration failed: {s}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// What to register: a shortcut, plus the id and description a system
/// dialog shows (the Wayland portal lists them, and persists the user's
/// binding per app and id).
#[derive(Clone, Debug)]
pub struct HotkeySpec {
    shortcut: Option<String>,
    id: Option<String>,
    description: Option<String>,
}

impl HotkeySpec {
    /// A spec for `shortcut`. On the portal it is the *preferred* trigger.
    pub fn new(shortcut: impl Into<String>) -> Self {
        HotkeySpec {
            shortcut: Some(shortcut.into()),
            id: None,
            description: None,
        }
    }

    /// A spec with no shortcut: the system chooses (the portal asks the user).
    /// Elsewhere it registers nothing and answers [`HotkeyError::Unsupported`].
    pub fn system_chosen(id: impl Into<String>) -> Self {
        HotkeySpec {
            shortcut: None,
            id: Some(id.into()),
            description: None,
        }
    }

    /// A stable id for the binding. Defaults to one derived from the
    /// shortcut. Give one explicitly when the shortcut can be rebound, so the
    /// portal keeps the user's choice across runs.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// What the hotkey does, in words, for a system dialog.
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
}

impl From<&str> for HotkeySpec {
    fn from(s: &str) -> Self {
        HotkeySpec::new(s)
    }
}

impl From<String> for HotkeySpec {
    fn from(s: String) -> Self {
        HotkeySpec::new(s)
    }
}

/// What a backend is told about a binding besides its shortcut.
#[derive(Clone, Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // the portal reads it
pub(crate) struct Meta {
    pub(crate) id: String,
    pub(crate) description: String,
}

/// A registered hotkey. Dropping it unregisters.
///
/// `!Send`: it names an entry in the main thread's registry.
pub struct Hotkey {
    key: BindingKey,
    _not_send: PhantomData<Rc<()>>,
}

impl fmt::Debug for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hotkey")
            .field("id", &self.id())
            .field("binding", &self.binding())
            .finish()
    }
}

type Callback = Rc<dyn Fn(&HotkeyEvent)>;
type BindingCallback = Rc<dyn Fn(&Binding)>;

struct Entry {
    id: String,
    description: String,
    shortcut: Option<Shortcut>,
    binding: Binding,
    cb: Callback,
    on_binding: Option<BindingCallback>,
    owner: Option<Owner>,
    /// Whether the last state delivered was `Pressed`, so a rebind that takes
    /// the binding away mid-hold can end the hold with a `Released`.
    pressed: bool,
    /// Bumped by every rebind; part of the key the backend holds
    /// ([`backend_key`]), so a report the backend queued for the old binding
    /// is dropped instead of pressing the new one with no release to follow.
    generation: u64,
}

/// The bits of a backend key that name the registry entry; the rest are its
/// generation.
const GENERATION_SHIFT: u32 = 40;
const HANDLE_MASK: BindingKey = (1 << GENERATION_SHIFT) - 1;

/// The key a backend holds for `handle` at `generation`. Generation 0 is the
/// handle itself.
fn backend_key(handle: BindingKey, generation: u64) -> BindingKey {
    handle | (generation << GENERATION_SHIFT)
}

thread_local! {
    static REGISTRY: RefCell<BTreeMap<BindingKey, Entry>> = const { RefCell::new(BTreeMap::new()) };
    static BACKEND: RefCell<Option<Box<dyn backend::BackendImpl>>> = const { RefCell::new(None) };
    static NEXT_KEY: Cell<BindingKey> = const { Cell::new(1) };
}

/// Register a system-wide hotkey. `spec` is a shortcut string or a
/// [`HotkeySpec`]. See the module docs.
pub fn register(
    spec: impl Into<HotkeySpec>,
    cb: impl Fn(&HotkeyEvent) + 'static,
) -> Result<Hotkey, HotkeyError> {
    let spec = spec.into();
    let shortcut = spec.shortcut.as_deref().map(Shortcut::parse).transpose()?;
    let id = match (&spec.id, &shortcut) {
        (Some(id), _) => id.clone(),
        (None, Some(sc)) => format!("hotkey-{sc}"),
        (None, None) => {
            return Err(HotkeyError::Parse(
                "a spec with no shortcut needs an id".into(),
            ));
        }
    };
    let description = spec.description.clone().unwrap_or_else(|| match &shortcut {
        Some(sc) => sc.to_string(),
        None => id.clone(),
    });
    let taken = REGISTRY.with(|r| {
        let r = r.borrow();
        if let Some(sc) = &shortcut
            && r.values().any(|e| e.shortcut.as_ref() == Some(sc))
        {
            return Some(HotkeyError::Conflict(format!(
                "{sc} (already registered by this app)"
            )));
        }
        if r.values().any(|e| e.id == id) {
            return Some(HotkeyError::DuplicateId(id.clone()));
        }
        None
    });
    if let Some(e) = taken {
        return Err(e);
    }
    let key = NEXT_KEY.with(|n| {
        let k = n.get();
        n.set(k + 1);
        k
    });
    let meta = Meta {
        id: id.clone(),
        description: description.clone(),
    };
    let binding = with_backend(|b| b.register(key, shortcut.as_ref(), &meta))?;
    let owner = current_owner();
    REGISTRY.with(|r| {
        r.borrow_mut().insert(
            key,
            Entry {
                id,
                description,
                shortcut,
                binding,
                cb: Rc::new(cb),
                on_binding: None,
                owner: owner.clone(),
                pressed: false,
                generation: 0,
            },
        )
    });
    if owner.is_some() {
        on_cleanup(move || unregister_key(key));
    }
    Ok(Hotkey {
        key,
        _not_send: PhantomData,
    })
}

impl Hotkey {
    /// The binding's id.
    pub fn id(&self) -> String {
        REGISTRY.with(|r| {
            r.borrow()
                .get(&self.key)
                .map(|e| e.id.clone())
                .unwrap_or_default()
        })
    }

    /// The shortcut asked for (`None` for a [`HotkeySpec::system_chosen`]).
    pub fn shortcut(&self) -> Option<Shortcut> {
        REGISTRY.with(|r| r.borrow().get(&self.key).and_then(|e| e.shortcut))
    }

    /// What it is bound to now. [`Binding::Exact`] everywhere but the Wayland
    /// portal.
    pub fn binding(&self) -> Binding {
        REGISTRY.with(|r| {
            r.borrow()
                .get(&self.key)
                .map(|e| e.binding.clone())
                .unwrap_or(Binding::Declined)
        })
    }

    /// Hear every change of [`Self::binding`] (the portal dialog's outcome,
    /// the user changing it in the system settings). Replaces an earlier one.
    pub fn on_binding_change(&self, cb: impl Fn(&Binding) + 'static) {
        REGISTRY.with(|r| {
            if let Some(e) = r.borrow_mut().get_mut(&self.key) {
                e.on_binding = Some(Rc::new(cb));
            }
        });
    }

    /// Bind this hotkey to another shortcut. On failure the old binding
    /// stays and the error says why (a [`HotkeyError::Conflict`] most often).
    pub fn rebind(&self, shortcut: &str) -> Result<(), HotkeyError> {
        let new = Shortcut::parse(shortcut)?;
        let (old, meta) = REGISTRY.with(|r| {
            let r = r.borrow();
            let e = r
                .get(&self.key)
                .ok_or_else(|| HotkeyError::Platform("unregistered".into()))?;
            if r.iter()
                .any(|(k, o)| *k != self.key && o.shortcut == Some(new))
            {
                return Err(HotkeyError::Conflict(format!(
                    "{new} (already registered by this app)"
                )));
            }
            Ok((
                e.shortcut,
                Meta {
                    id: e.id.clone(),
                    description: e.description.clone(),
                },
            ))
        })?;
        if old == Some(new) {
            return Ok(());
        }
        let generation = REGISTRY.with(|r| r.borrow().get(&self.key).map_or(0, |e| e.generation));
        with_backend(|b| {
            b.unregister(backend_key(self.key, generation));
            Ok(())
        })?;
        // A binding taken away while held gets its release now: the backend
        // forgot it, and a hold-to-talk app must not stay "talking".
        let held = REGISTRY.with(|r| r.borrow().get(&self.key).is_some_and(|e| e.pressed));
        if held {
            deliver(
                backend_key(self.key, generation),
                Report::State(HotkeyState::Released),
            );
        }
        // From here on, reports for the old generation are dropped.
        let next = generation + 1;
        REGISTRY.with(|r| {
            if let Some(e) = r.borrow_mut().get_mut(&self.key) {
                e.generation = next;
                e.pressed = false;
            }
        });
        let bk = backend_key(self.key, next);
        match with_backend(|b| b.register(bk, Some(&new), &meta)) {
            Ok(binding) => {
                REGISTRY.with(|r| {
                    if let Some(e) = r.borrow_mut().get_mut(&self.key) {
                        e.shortcut = Some(new);
                        e.binding = binding;
                    }
                });
                // The portal keeps the user's own choice per id; let them
                // change it there (a no-op elsewhere).
                let _ = with_backend(|b| {
                    b.configure(bk);
                    Ok(())
                });
                Ok(())
            }
            Err(err) => {
                // Put the old binding back; if even that fails, the hotkey is
                // bound to nothing and says so.
                let restored = with_backend(|b| b.register(bk, old.as_ref(), &meta));
                if restored.is_err() {
                    REGISTRY.with(|r| {
                        if let Some(e) = r.borrow_mut().get_mut(&self.key) {
                            e.binding = Binding::Declined;
                        }
                    });
                }
                Err(err)
            }
        }
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        unregister_key(self.key);
    }
}

fn unregister_key(key: BindingKey) {
    let removed = REGISTRY
        .try_with(|r| r.try_borrow_mut().ok().and_then(|mut r| r.remove(&key)))
        .ok()
        .flatten();
    if let Some(entry) = removed {
        let _ = BACKEND.try_with(|b| {
            if let Ok(mut b) = b.try_borrow_mut()
                && let Some(b) = b.as_mut()
            {
                b.unregister(backend_key(key, entry.generation));
            }
        });
    }
}

fn with_backend<R>(
    f: impl FnOnce(&mut dyn backend::BackendImpl) -> Result<R, HotkeyError>,
) -> Result<R, HotkeyError> {
    BACKEND.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(backend::create()?);
        }
        f(slot.as_mut().expect("just created").as_mut())
    })
}

/// The backend this process uses (or would use: none is started until the
/// first [`register`]).
pub fn backend() -> Backend {
    BACKEND.with(|b| match b.borrow().as_ref() {
        Some(b) => b.kind(),
        None => backend::choose(),
    })
}

/// Whether the hotkeys registered now need a permission, and whether it is
/// granted. See [`Permission`].
pub fn permission() -> Permission {
    BACKEND.with(|b| {
        b.borrow()
            .as_ref()
            .map_or(Permission::NotNeeded, |b| b.permission())
    })
}

/// Ask the user for the permission the registered hotkeys need (macOS: the
/// system shows its Input Monitoring prompt or settings pane), and answer
/// the state now. A grant reaches the event tap only after the app registers
/// those hotkeys again (macOS applies it to taps created afterwards).
pub fn request_permission() -> Permission {
    BACKEND.with(|b| {
        b.borrow_mut()
            .as_mut()
            .map_or(Permission::NotNeeded, |b| b.request_permission())
    })
}

static APP_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Name the application to the desktop for its hotkeys. Only the Wayland
/// portal uses it: the desktop files the user's bindings under this id
/// (KDE refuses an app with none). It defaults to the id the window carries —
/// `WindowProps::app_id`, else the executable's name — so set it only when
/// hotkeys are registered before any window exists and the window will get
/// an explicit id. Call it before the first [`register`].
pub fn set_app_id(id: impl Into<String>) {
    *APP_ID.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.into());
}

static WRITE_DESKTOP_ENTRY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Whether the Wayland portal backend may write a desktop entry for the app
/// (on by default). The portal accepts hotkeys only from an application with
/// an installed `.desktop` entry; when registering finds none — in no system
/// directory and not in the user's — rinch writes a minimal one with
/// `NoDisplay=true` to `$XDG_DATA_HOME/applications/<app id>.desktop`
/// (`~/.local/share/applications`), never replacing an existing file. Turn it
/// off when the app installs its own entry; registration then fails with
/// [`HotkeyError::Unsupported`] naming the missing file.
pub fn set_write_desktop_entry(allowed: bool) {
    WRITE_DESKTOP_ENTRY.store(allowed, std::sync::atomic::Ordering::Relaxed);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn may_write_desktop_entry() -> bool {
    WRITE_DESKTOP_ENTRY.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn app_id() -> String {
    if let Some(id) = APP_ID.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        return id;
    }
    #[cfg(target_os = "linux")]
    {
        crate::shell::rinch_runtime::current_app_id()
    }
    #[cfg(not(target_os = "linux"))]
    {
        String::new()
    }
}

/// What a backend reports, from any thread.
#[derive(Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // `Binding`: the portal
pub(crate) enum Report {
    State(HotkeyState),
    /// A `Released` after another key went down during the hold.
    ChordedRelease,
    Binding(Binding),
}

/// Report what a [`matcher::Matcher`] pressed and released, with
/// [`HotkeyEvent::chorded`] read from it.
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
pub(crate) fn report_matched(matcher: &matcher::Matcher, out: &mut Vec<(BindingKey, HotkeyState)>) {
    for (k, s) in out.drain(..) {
        let r = if s == HotkeyState::Released && matcher.chorded(k) {
            Report::ChordedRelease
        } else {
            Report::State(s)
        };
        report(k, r);
    }
}

/// Deliver a backend report to the main thread. Safe from any thread and from
/// a `Drop`; never runs inline (the backend may hold a lock).
pub(crate) fn report(key: BindingKey, r: Report) {
    #[cfg(test)]
    let Some(r) = test_queue::capture(key, r) else {
        return;
    };
    rinch_core::reactive::dispatch_main_callback(Box::new(move || deliver(key, r)));
}

/// A queue of this test thread's own, standing in for the main-thread queue.
/// That queue is process-wide, and other lib tests drain it from their
/// threads, where this thread's registry does not exist: a test that reports
/// and then drains it races them (#1530's second review measured the
/// generation-filter fixture passing with the filter removed).
#[cfg(test)]
pub(crate) mod test_queue {
    use super::{BindingKey, Report, deliver};
    use std::cell::RefCell;

    thread_local! {
        static QUEUE: RefCell<Option<Vec<(BindingKey, Report)>>> = const { RefCell::new(None) };
    }

    /// Reports made on this thread queue here, not on the main queue.
    pub(crate) fn install() {
        QUEUE.with(|q| *q.borrow_mut() = Some(Vec::new()));
    }

    /// The report back if no queue is installed on this thread.
    pub(super) fn capture(key: BindingKey, r: Report) -> Option<Report> {
        QUEUE.with(|q| match q.borrow_mut().as_mut() {
            Some(v) => {
                v.push((key, r));
                None
            }
            None => Some(r),
        })
    }

    /// Deliver what this thread queued, in order, as the main loop would.
    pub(crate) fn drain() {
        let items = QUEUE.with(|q| {
            q.borrow_mut()
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default()
        });
        for (k, r) in items {
            deliver(k, r);
        }
    }
}

/// Run on the main thread: hand a report to its hotkey's callbacks.
pub(crate) fn deliver(bk: BindingKey, r: Report) {
    let key = bk & HANDLE_MASK;
    let generation = bk >> GENERATION_SHIFT;
    let found = REGISTRY.with(|reg| {
        let reg = reg.borrow();
        reg.get(&key)
            .filter(|e| e.generation == generation)
            .map(|e| {
                (
                    e.id.clone(),
                    e.cb.clone(),
                    e.on_binding.clone(),
                    e.owner.clone(),
                )
            })
    });
    let Some((id, cb, on_binding, owner)) = found else {
        return; // unregistered or rebound since the backend saw it
    };
    if let Some(o) = &owner
        && !o.is_alive()
    {
        unregister_key(key);
        return;
    }
    let run = |f: &dyn Fn()| {
        rinch_core::reactive::batch(|| match &owner {
            Some(o) => o.run(|| untracked_handler(f)),
            None => unowned(|| untracked_handler(f)),
        })
    };
    let (r, chorded) = match r {
        Report::ChordedRelease => (Report::State(HotkeyState::Released), true),
        other => (other, false),
    };
    match r {
        Report::State(state) => {
            let was = REGISTRY.with(|reg| {
                reg.borrow_mut()
                    .get_mut(&key)
                    .map(|e| std::mem::replace(&mut e.pressed, state == HotkeyState::Pressed))
            });
            if was == Some(state == HotkeyState::Pressed) {
                return; // a repeat of the state already delivered
            }
            let ev = HotkeyEvent { state, id, chorded };
            run(&|| cb(&ev));
        }
        Report::ChordedRelease => unreachable!("mapped above"),
        Report::Binding(b) => {
            REGISTRY.with(|reg| {
                if let Some(e) = reg.borrow_mut().get_mut(&key) {
                    e.binding = b.clone();
                }
            });
            if let Some(on) = on_binding {
                run(&|| on(&b));
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod review_1530_tests;
