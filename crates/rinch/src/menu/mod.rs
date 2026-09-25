//! Unified menu system for native window menus, the in-app DOM menu bar, and
//! system tray context menus.
//!
//! Provides [`Menu`] and [`MenuItem`] builder types that work with native menu
//! bars (via `muda`), the DOM menu bar ([`render_with_menu_bar`]) and tray
//! context menus (via `tray-icon`). Callbacks are `Rc<dyn Fn()>` — no
//! `Send`/`Sync` burden on users. The runtime wires push-based event delivery
//! so callbacks always run on the main thread.
//!
//! # What is platform-independent, and why
//!
//! Everything an app *declares* — [`Menu`], [`MenuItem`], shortcuts as plain
//! strings, `on_click`, `enabled`, separators, submenus — and everything that
//! *renders* a menu out of DOM nodes ([`render_with_menu_bar`],
//! [`render_menu_bar_standalone`]) builds with `default-features = false`: no
//! `muda`, no `winit`, no windowing at all. That is what lets a `rinch-web` app
//! declare the same menus as its desktop build and get the same bar in the
//! browser (`rinch_web::mount_with_menu_bar`).
//!
//! Behind `#[cfg(feature = "desktop")]` sit only the pieces that need a native
//! toolkit: the `muda` builders (`build_native_menu_bar`, `build_muda_menu`),
//! the accelerator conversion, the muda event handler, window attachment, and
//! `match_shortcut`, which takes a `winit::keyboard::KeyCode`. Chord matching
//! itself is not desktop-only — it lives in [`match_shortcut_code`], keyed by
//! the W3C `KeyboardEvent.code` name that both winit's `KeyCode` variants and a
//! browser keydown are named after.
//!
//! # Lifetime
//!
//! A menu callback belongs to the component that *created* it — the scope that
//! was rendering when [`MenuItem::on_click`] was called, which is where the
//! closure captured its `Signal`s. Once that component unmounts its signals are
//! freed, and a read of a freed signal panics (issue #183, #141 PR4), so the
//! callback stops being dispatched. It also runs *inside* that owner, so a
//! `Signal` it creates belongs to the menu's component rather than to whatever
//! the event loop happened to be doing.
//!
//! Every path that can fire an item applies that rule through one function,
//! `invoke_menu_callback`: the registry (`dispatch_menu_event`, which muda, ksni
//! and `match_shortcut_code` all route through) and the DOM menu bar, which
//! renders items straight out of the [`Menu`] and holds the `Rc<dyn Fn()>`
//! itself rather than a registry id.
//!
//! Ownership is recorded per **item**, not per menu build. One `Menu` may be
//! assembled from items contributed by several components — and the build
//! itself commonly happens somewhere else entirely (`main`, a tray builder) — so
//! a per-build owner would both silence live items and, worse, keep an unmounted
//! component's item armed.
//!
//! Neither direction of that error is the cheap one, so it is worth being exact.
//! Under-attributing restores the #141 panic. Over-attributing is *not* merely
//! "drops a click": both prune paths **remove** the entry from the map — and the
//! chord path deletes its chords — so an item wrongly judged dead is disabled
//! permanently, until something rebuilds the menu. The owner therefore has to be
//! the item's own, not an approximation in either direction.
//!
//! Registering with **no ambient owner** — from `main`, from startup code,
//! before the event loop — records no owner and keeps app lifetime, unchanged.
//! That is how every in-tree menu is built.
//!
//! **Removal is a separate question from invocation, and it is deliberately not
//! tied to any scope.** The owner check above already makes a dead callback
//! inert the moment its component unmounts; what is left is reclaiming the
//! memory, and that is `MenuRegistration`'s job — an RAII token holding the
//! ids one build registered, released when whoever owns that build replaces or
//! drops it. The registry was otherwise append-only: nothing removed an entry,
//! ever, so a menu rebuilt at runtime accumulated — and the ksni tray path mints
//! a fresh `ksni-{N}` id for every item on every build, so it could never even
//! overwrite. A dead entry lingers until the next rebuild or until the token
//! drops, and fires nothing in the meantime.
//!
//! Hanging removal on `on_cleanup` as well — reclaiming an id when the scope
//! that *built* the menu is disposed — was tried, and is wrong twice over. It
//! reclaims by the **building** scope while the menu is still on screen and its
//! token still alive, so a component that assembles a menu out of items
//! contributed by others silences all of them, permanently, when it unmounts
//! (`a_builder_unmounting_does_not_kill_another_components_live_item`). And it
//! is unbounded on exactly the shape this module advertises: a callback runs
//! inside its owner, so a callback that rebuilds its own menu registers with
//! that owner ambient, appending one boxed cleanup plus one pinning `Weak` per
//! item per rebuild for the life of the component. `rinch_core::reactive`'s
//! scoped-registry docs say so directly: a registry written repeatedly from a
//! live component "must instead carry an `Owner` beside the callback and check
//! `is_alive` at dispatch" — which is what this module does.
//!
//! # Example
//!
//! ```ignore
//! use rinch::menu::{Menu, MenuItem};
//!
//! let file_menu = Menu::new()
//!     .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| println!("New!")))
//!     .separator()
//!     .item(MenuItem::new("Quit").on_click(|| std::process::exit(0)));
//!
//! // For native menu bar:
//! App::new(app)
//!     .title("My App")
//!     .size(800, 600)
//!     .menu(vec![("File", file_menu)])
//!     .run();
//!
//! // For the browser, from the same `Menu` values:
//! rinch_web::mount_with_menu_bar(theme, vec![("File", file_menu)], app);
//!
//! // For tray context menu:
//! TrayIconBuilder::new().with_menu(menu).build()?;
//! ```

// Compiled on every target, not just Linux: the DOM menu bar it renders is the
// menu bar a `rinch-web` app gets in the browser, where there is no native one
// to fall back to either. See [`render_with_menu_bar`].
pub(crate) mod app_menu_bar;

pub use app_menu_bar::{MENU_BAR_HEIGHT, render_menu_bar_standalone, render_with_menu_bar};

#[cfg(feature = "desktop")]
use muda::accelerator::Accelerator;
use rinch_core::reactive::{Owner, current_owner, unowned};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::Rc;
#[cfg(feature = "desktop")]
use std::str::FromStr;
#[cfg(feature = "desktop")]
use winit::keyboard::KeyCode;

// ── Public API ──────────────────────────────────────────────────────────────

/// A menu containing items, separators, and submenus.
///
/// Used for both native window menu bars and tray context menus.
#[derive(Clone)]
pub struct Menu {
    entries: Vec<MenuEntryInner>,
}

/// A single menu item with optional shortcut and callback.
#[derive(Clone)]
pub struct MenuItem {
    label: String,
    shortcut: Option<String>,
    enabled: bool,
    callback: Option<Rc<dyn Fn()>>,
    /// The scope that was rendering when [`on_click`](MenuItem::on_click) was
    /// called, if any.
    ///
    /// Captured *there* rather than where the menu is built, because that is
    /// where the closure captured its `Signal`s — see the [module docs](self).
    /// `None` means the item was created outside any render and has app
    /// lifetime. `Owner` is a `Weak`, so this keeps nothing alive.
    callback_owner: Option<Owner>,
}

#[derive(Clone)]
enum MenuEntryInner {
    Item(MenuItem),
    Separator,
    Submenu { label: String, menu: Menu },
}

/// Read-only view of a menu entry.
pub enum MenuEntryRef<'a> {
    Item {
        label: &'a str,
        shortcut: Option<&'a str>,
        enabled: bool,
        callback: Option<&'a Rc<dyn Fn()>>,
    },
    Separator,
    Submenu {
        label: &'a str,
        menu: &'a Menu,
    },
}

impl MenuItem {
    /// Create a new menu item with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            enabled: true,
            callback: None,
            callback_owner: None,
        }
    }

    /// Set the keyboard shortcut (e.g., `"Ctrl+N"`, `"Cmd+Shift+S"`).
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }

    /// Set whether this item is enabled.
    pub fn enabled(mut self, e: bool) -> Self {
        self.enabled = e;
        self
    }

    /// Set the callback invoked when this item is activated.
    ///
    /// The callback belongs to the component that is rendering *here*, where the
    /// closure captured whatever it captured; once that component unmounts the
    /// item stops firing. Called outside any render — from `main`, as every
    /// in-tree menu does — it keeps app lifetime. See the [module docs](self).
    pub fn on_click(mut self, cb: impl Fn() + 'static) -> Self {
        self.callback = Some(Rc::new(cb));
        self.callback_owner = current_owner();
        self
    }
}

impl Menu {
    /// Create a new empty menu.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a menu item.
    pub fn item(mut self, item: MenuItem) -> Self {
        self.entries.push(MenuEntryInner::Item(item));
        self
    }

    /// Add a separator line.
    pub fn separator(mut self) -> Self {
        self.entries.push(MenuEntryInner::Separator);
        self
    }

    /// Add a submenu.
    pub fn submenu(mut self, label: impl Into<String>, menu: Menu) -> Self {
        self.entries.push(MenuEntryInner::Submenu {
            label: label.into(),
            menu,
        });
        self
    }
}

impl Menu {
    /// Iterate over entries as read-only references.
    pub fn iter_entries(&self) -> impl Iterator<Item = MenuEntryRef<'_>> {
        self.entries.iter().map(|entry| match entry {
            MenuEntryInner::Item(item) => MenuEntryRef::Item {
                label: &item.label,
                shortcut: item.shortcut.as_deref(),
                enabled: item.enabled,
                callback: item.callback.as_ref(),
            },
            MenuEntryInner::Separator => MenuEntryRef::Separator,
            MenuEntryInner::Submenu { label, menu } => MenuEntryRef::Submenu { label, menu },
        })
    }
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

/// Decomposed menu entry for consumption by platform backends (e.g., ksni on Linux).
#[cfg(feature = "system-tray")]
pub(crate) enum MenuEntryKind {
    Item {
        label: String,
        enabled: bool,
        callback: Option<Rc<dyn Fn()>>,
        /// Carried through so the tray backend registers the item under the
        /// scope that created its callback, not the one that built the tray.
        callback_owner: Option<Owner>,
    },
    Separator,
    Submenu {
        label: String,
        menu: Menu,
    },
}

#[cfg(feature = "system-tray")]
impl Menu {
    /// Consume the menu and return its entries for platform-specific conversion.
    pub(crate) fn take_entries(self) -> Vec<MenuEntryKind> {
        self.entries
            .into_iter()
            .map(|entry| match entry {
                MenuEntryInner::Item(item) => MenuEntryKind::Item {
                    label: item.label,
                    enabled: item.enabled,
                    callback: item.callback,
                    callback_owner: item.callback_owner,
                },
                MenuEntryInner::Separator => MenuEntryKind::Separator,
                MenuEntryInner::Submenu { label, menu } => MenuEntryKind::Submenu { label, menu },
            })
            .collect()
    }
}

// ── Thread-local callback registry ──────────────────────────────────────────

/// One registered menu callback plus the scope that created it.
///
/// Held as an `Rc` so the registry's "still mine" checks can be made by
/// [`Rc::ptr_eq`]. That is the only reliable discriminator here: the ids come
/// from two different sources with two different shapes — muda's
/// `MenuItem::id().0` and the ksni backend's monotonic `ksni-{N}` counter — and
/// they share one map, so nothing about the key itself can be assumed.
struct MenuCallback {
    /// See [`MenuItem::callback_owner`]. `None` means app lifetime.
    owner: Option<Owner>,
    cb: Rc<dyn Fn()>,
}

impl MenuCallback {
    /// Whether the component that created this callback is gone.
    ///
    /// `false` for an ownerless registration, which has app lifetime.
    fn is_dead(&self) -> bool {
        self.owner.as_ref().is_some_and(|owner| !owner.is_alive())
    }
}

/// One registered shortcut.
///
/// The `serial` is what a cleanup or a [`MenuRegistration`] removes by. The
/// entry carries no callback — only the id to dispatch — so there is no `Rc`
/// identity to compare, and removing "the entry for this menu id" would take
/// out a later build's entry too.
struct ShortcutEntry {
    serial: u64,
    shortcut: ParsedShortcut,
    menu_id: String,
}

thread_local! {
    /// Map from menu id string → callback. Thread-local because callbacks
    /// capture `Signal` (which is `!Send`) and must run on the main thread.
    static MENU_CALLBACKS: RefCell<HashMap<String, Rc<MenuCallback>>> = RefCell::new(HashMap::new());

    /// Registered keyboard shortcuts mapped to their menu ID strings.
    static MENU_SHORTCUTS: RefCell<Vec<ShortcutEntry>> = const { RefCell::new(Vec::new()) };

    /// Source of [`ShortcutEntry::serial`].
    static NEXT_SHORTCUT_SERIAL: Cell<u64> = const { Cell::new(0) };

    /// Source of the ids [`register_menu_shortcuts`] registers under.
    static NEXT_MENU_ID: Cell<u64> = const { Cell::new(0) };

    /// Source of [`MenuRegistration::build`].
    ///
    /// Starts at **1**, so the `0` a `MenuRegistration::default()` carries — the
    /// native bar's build, a tray's — is a number no [`MenuBarChords`] can ever
    /// name. A token releases the slot only when the slot is still its own
    /// build, and a build nobody holds a token for must never be mistaken for
    /// one.
    static NEXT_MENU_BAR_BUILD: Cell<u64> = const { Cell::new(1) };

    /// The registration held by the most recently built menu bar, native
    /// (`build_native_menu_bar`) or DOM ([`register_menu_shortcuts`]).
    ///
    /// The bar is a process-wide singleton owned by the runtime rather than by
    /// any caller, so its token lives here: building a new bar replaces this,
    /// dropping the previous build's ids. This is what keeps either builder from
    /// growing the registry on every rebuild. One slot serves both because no
    /// app has two menu bars: a desktop build arms its chords through muda, a
    /// web build through `register_menu_shortcuts`, and neither runs twice over
    /// one window.
    ///
    /// *Replacing* is the whole of the story on the desktop, where the bar has
    /// the app's lifetime. A web island does not: it can be **unmounted**, and
    /// then nothing would ever replace its build — so the chords would go on
    /// firing and go on calling `preventDefault` on somebody else's page. That
    /// is what [`MenuBarChords`] closes, and why the slot has to be able to say
    /// *which* build it is holding.
    static MENU_BAR_REGISTRATION: RefCell<Option<MenuRegistration>> = const { RefCell::new(None) };
}

/// A build number no previous [`register_menu_shortcuts`] call has used.
fn next_menu_bar_build() -> u64 {
    NEXT_MENU_BAR_BUILD.with(|next| {
        let build = next.get();
        next.set(build + 1);
        build
    })
}

/// Register `cb` under `menu_id` as belonging to `owner`, returning the entry so
/// a [`MenuRegistration`] can hold it for the `Rc::ptr_eq` check on release.
///
/// Nothing here is tied to the scope that is currently rendering. `owner` is the
/// scope that *created* the callback and it gates **invocation** (see
/// [`dispatch_menu_event`]); **removal** belongs to the returned entry's
/// [`MenuRegistration`]. The two questions are answered separately because a
/// component routinely contributes an item to a menu somebody else assembles —
/// see the [module docs](self).
fn register_callback_owned(
    menu_id: &str,
    cb: Rc<dyn Fn()>,
    owner: Option<Owner>,
) -> Rc<MenuCallback> {
    let entry = Rc::new(MenuCallback { owner, cb });
    // Bound outside the borrow: a displaced entry is user code whose `Drop` may
    // re-enter the registry.
    let _previous =
        MENU_CALLBACKS.with(|map| map.borrow_mut().insert(menu_id.to_string(), entry.clone()));
    entry
}

/// Register `shortcut_str` as dispatching `menu_id`, returning the serial that
/// identifies this registration (`None` if the string does not parse).
///
/// Removal belongs to the [`MenuRegistration`] that recorded the serial, not to
/// any scope. A chord outliving its callback does not swallow the key in the
/// meantime: [`match_shortcut`] answers `true` only when a callback actually
/// ran, and [`dispatch_menu_event`] takes a dead item's chords out with it.
fn register_shortcut(shortcut_str: &str, menu_id: &str) -> Option<u64> {
    let parsed = parse_shortcut_for_matching(shortcut_str)?;
    let serial = NEXT_SHORTCUT_SERIAL.with(|next| {
        let serial = next.get();
        next.set(serial + 1);
        serial
    });
    MENU_SHORTCUTS.with(|shortcuts| {
        shortcuts.borrow_mut().push(ShortcutEntry {
            serial,
            shortcut: parsed,
            menu_id: menu_id.to_string(),
        });
    });
    Some(serial)
}

/// Invoke `cb` on behalf of the scope that created it, returning whether it ran.
///
/// The one place the lifetime rule is applied, so every path that can fire a
/// menu item obeys it: the registry ([`dispatch_menu_event`]) *and* the DOM
/// menu bar, which renders items straight from the [`Menu`] and holds the
/// `Rc<dyn Fn()>` itself rather than a registry id.
///
/// A live callback runs inside its owner, so a `Signal` it creates belongs to
/// the menu's component. An ownerless one runs [`unowned`] for the mirror-image
/// reason: it has app lifetime, and what it allocates must not be handed to
/// whatever scope the dispatch happened to be nested inside. A callback whose
/// owner is gone is not run at all — its captured signals are freed, and reading
/// one panics.
pub(crate) fn invoke_menu_callback(cb: &Rc<dyn Fn()>, owner: Option<&Owner>) -> bool {
    match owner {
        Some(owner) if !owner.is_alive() => false,
        // One transaction per callback, like every event handler
        // (`rinch_core::reactive::batch`).
        Some(owner) => {
            rinch_core::reactive::batch(|| owner.run(|| cb()));
            true
        }
        None => {
            rinch_core::reactive::batch(|| unowned(|| cb()));
            true
        }
    }
}

/// Dispatch a menu event by looking up and invoking the callback.
///
/// Returns whether a callback actually ran, which is what tells
/// [`match_shortcut`] whether the keystroke was really consumed.
///
/// The callback is cloned **out** of the registry before it is called, so a
/// callback may rebuild the menu it was dispatched from — registering, and so
/// mutably borrowing, the very map being read — without a double-borrow panic.
///
/// A callback whose component has since unmounted is not called, and the entry
/// is pruned. This check is the *only* thing that makes a dead callback inert —
/// no scope cleanup removes one — so it has to happen here, on every dispatch,
/// and must not be traded for a removal hook.
pub(crate) fn dispatch_menu_event(menu_id: &str) -> bool {
    let Some(entry) = MENU_CALLBACKS.with(|map| map.borrow().get(menu_id).cloned()) else {
        return false;
    };

    if entry.is_dead() {
        prune_callback(menu_id, &entry);
        return false;
    }

    invoke_menu_callback(&entry.cb, entry.owner.as_ref())
}

/// Take `menu_id` out of `map` if it is still `entry`.
///
/// The one copy of "only reclaim what is still yours", shared by
/// [`prune_callback`] and [`MenuRegistration::drop`]. Menu ids come from two
/// sources with two different shapes (muda's counter and the ksni backend's
/// `ksni-{N}`) into one map, so identity — not the key — is the discriminator:
/// without it an earlier release would clobber a later registration.
fn take_callback_if_ours(
    map: &mut HashMap<String, Rc<MenuCallback>>,
    menu_id: &str,
    entry: &Rc<MenuCallback>,
) -> Option<Rc<MenuCallback>> {
    if map
        .get(menu_id)
        .is_some_and(|installed| Rc::ptr_eq(installed, entry))
    {
        map.remove(menu_id)
    } else {
        None
    }
}

/// Remove `menu_id` if it still holds `entry`, along with any chords that would
/// dispatch it.
fn prune_callback(menu_id: &str, entry: &Rc<MenuCallback>) {
    // Bound outside the borrow: the callback is user code whose `Drop` may
    // re-enter the registry.
    let dead =
        MENU_CALLBACKS.with(|map| take_callback_if_ours(&mut map.borrow_mut(), menu_id, entry));
    // Only if the entry really was ours: a later registration at this id owns
    // both the callback and any chord that reaches it.
    if dead.is_some() {
        MENU_SHORTCUTS.with(|shortcuts| {
            shortcuts.borrow_mut().retain(|e| e.menu_id != menu_id);
        });
    }
}

/// Check if a keyboard event matches a registered menu shortcut.
/// If so, dispatch the callback and return `true`.
///
/// The matched ids are taken out of the registry before dispatching, so a
/// shortcut's callback may register a shortcut of its own.
///
/// `true` means a callback actually **ran**, because the caller uses it to
/// swallow the keystroke. Every chord that matches is tried, in registration
/// order, until one fires: a chord whose creating component has since unmounted
/// (pruned by [`dispatch_menu_event`]) must not shadow a live duplicate, and a
/// chord registered for an item with no callback at all must fall through to the
/// app instead of eating that key combination forever.
#[cfg(feature = "desktop")]
pub(crate) fn match_shortcut(ctrl: bool, meta: bool, alt: bool, shift: bool, key: KeyCode) -> bool {
    let Some(code) = key_code_name(key) else {
        return false;
    };
    match_shortcut_code(ctrl, meta, alt, shift, code)
}

/// Check whether a keyboard event matches a registered menu shortcut, keyed by
/// the W3C UI Events `code` name of the key (`"KeyK"`, `"Digit1"`, `"F5"`). If
/// so, dispatch the callback and return `true`.
///
/// The platform-independent half of the desktop's `match_shortcut`, and the
/// entry point a browser keydown uses directly: `KeyboardEvent.code` already
/// *is* this string.
/// `meta` is folded into `ctrl` exactly as it is on the desktop, so an app
/// declares `"Ctrl+K"` once and Cmd+K works on macOS.
///
/// `true` means a callback actually **ran**, which is the caller's cue to
/// swallow the keystroke (`preventDefault` in a browser). Every chord that
/// matches is tried, in registration order, until one fires: a chord whose
/// creating component has since unmounted must not shadow a live duplicate, and
/// a chord registered for an item with no callback at all must fall through to
/// the app instead of eating that key combination forever.
pub fn match_shortcut_code(ctrl: bool, meta: bool, alt: bool, shift: bool, code: &str) -> bool {
    let ctrl_or_cmd = ctrl || meta;

    // `collect` on an empty iterator does not allocate, so the overwhelmingly
    // common "no chord matches" keystroke stays allocation-free.
    let matched: Vec<String> = MENU_SHORTCUTS.with(|shortcuts| {
        shortcuts
            .borrow()
            .iter()
            .filter(|entry| {
                entry.shortcut.ctrl_or_cmd == ctrl_or_cmd
                    && entry.shortcut.alt == alt
                    && entry.shortcut.shift == shift
                    && entry.shortcut.code == code
            })
            .map(|entry| entry.menu_id.clone())
            .collect()
    });

    matched.iter().any(|menu_id| dispatch_menu_event(menu_id))
}

// ── Registration token ──────────────────────────────────────────────────────

/// The ids one menu build registered, released when the build is replaced.
///
/// Nothing used to remove a menu id, ever: the registry only grew, so a menu
/// rebuilt at runtime accumulated a full set of stale callbacks each time, and
/// the ksni tray path — which mints a fresh `ksni-{N}` id for every item on
/// every build — could not even overwrite its own previous entries.
///
/// A scope cleanup cannot close it, which is why there is none. It would reclaim
/// an id when the *scope that built the menu* is disposed — and the in-tree
/// menus are built from `main`, before the event loop, with no scope at all,
/// while a menu built during a render would take the still-live items other
/// components contributed down with it. Something has to own the **build**; this
/// is that thing. Whoever holds a build's token — the
/// [`MENU_BAR_REGISTRATION`] slot for the window menu bar, the `TrayIcon` for a
/// tray — releases the build's ids by replacing or dropping it.
///
/// Release touches thread-local state, so it has to happen on the thread that
/// built the menu — anywhere else it would reclaim nothing and leave that
/// thread's entries stranded. Holding `Rc`s makes the token `!Send`, so the
/// compiler enforces that rather than the docs asking for it.
#[derive(Default)]
pub(crate) struct MenuRegistration {
    /// The callbacks this build installed, held so removal can check
    /// [`Rc::ptr_eq`] rather than trusting the key.
    callbacks: Vec<(String, Rc<MenuCallback>)>,
    shortcuts: Vec<u64>,
    /// Which build this is, for the one holder that can outlive its own
    /// replacement: see [`MenuBarChords`]. `0` — the `Default` — means "no token
    /// names this build", which is every registration but a
    /// [`register_menu_shortcuts`] one.
    build: u64,
}

impl MenuRegistration {
    /// Register `cb` under `menu_id` on behalf of `owner`, and record the id so
    /// dropping this token takes it back out.
    pub(crate) fn register_callback(
        &mut self,
        menu_id: &str,
        cb: Rc<dyn Fn()>,
        owner: Option<Owner>,
    ) {
        let entry = register_callback_owned(menu_id, cb, owner);
        self.callbacks.push((menu_id.to_string(), entry));
    }

    /// Register a chord dispatching `menu_id`, recording it for release.
    fn register_shortcut(&mut self, shortcut_str: &str, menu_id: &str) {
        if let Some(serial) = register_shortcut(shortcut_str, menu_id) {
            self.shortcuts.push(serial);
        }
    }
}

impl Drop for MenuRegistration {
    fn drop(&mut self) {
        // `try_with`/`try_borrow_mut`: a token can be dropped from a TLS
        // destructor at thread exit, when the registry may already be gone, or
        // while unwinding, so release degrades to "not reclaimed" rather than
        // panicking. The removed callbacks stay in `self.callbacks` and are
        // dropped when that `Vec` is, after this function returns — they are
        // user code whose `Drop` may re-enter the registry.
        let _ = MENU_CALLBACKS.try_with(|map| {
            let Ok(mut map) = map.try_borrow_mut() else {
                return;
            };
            for (menu_id, entry) in &self.callbacks {
                // Only reclaim what is still ours: a later build may have
                // registered its own callback at this id.
                take_callback_if_ours(&mut map, menu_id, entry);
            }
        });
        // One pass over the chord list, not one per serial: releasing a menu bar
        // drops every chord it registered at once.
        if !self.shortcuts.is_empty() {
            let doomed: HashSet<u64> = self.shortcuts.iter().copied().collect();
            let _ = MENU_SHORTCUTS.try_with(|shortcuts| {
                if let Ok(mut shortcuts) = shortcuts.try_borrow_mut() {
                    shortcuts.retain(|entry| !doomed.contains(&entry.serial));
                }
            });
        }
    }
}

/// The chords one [`register_menu_shortcuts`] call armed, released when this is
/// dropped.
///
/// [`MENU_BAR_REGISTRATION`] holds the build itself; this names it. Dropping the
/// token releases that build **only while the slot still holds it** — the
/// [`take_callback_if_ours`] discipline one level up — so:
///
/// * the island that armed the chords gives them back when it unmounts, instead
///   of leaving a removed widget eating a key combination from its host page for
///   the rest of the session;
/// * an island that armed them and was then *replaced* by a later bar releases
///   nothing, because the chords live are no longer its own. "One page, one set
///   of chords" is a last-writer-wins rule, and un-arming has to obey it or the
///   first island's unmount would silently disarm the second island.
///
/// The desktop needs no token: its bar has the app's lifetime and is only ever
/// replaced, never taken away, so `build_native_menu_bar` leaves the build at
/// `0` and nothing can name it.
///
/// `!Send` like [`MenuRegistration`], and for the same reason: release touches
/// thread-local state, so it has to happen on the thread that armed the chords.
/// The token holds no `Rc` of its own, so the marker says it explicitly.
#[must_use = "dropping the token immediately releases the chords it just armed; \
              hold it for as long as the menu bar is mounted"]
pub struct MenuBarChords {
    build: u64,
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for MenuBarChords {
    fn drop(&mut self) {
        let build = self.build;
        // Bound outside the borrow, like every other release here: the reclaimed
        // registration holds user callbacks whose `Drop` may re-enter the
        // registry. `try_with`/`try_borrow_mut` for the thread-exit and
        // unwinding cases — release degrades to "not reclaimed" rather than
        // panicking.
        let _reclaimed = MENU_BAR_REGISTRATION.try_with(|slot| {
            let mut slot = slot.try_borrow_mut().ok()?;
            if slot.as_ref().is_some_and(|held| held.build == build) {
                slot.take()
            } else {
                None
            }
        });
    }
}

// ── Build functions (Menu → the registry, Menu → muda types) ────────────────

/// Arm the keyboard shortcuts every item in `menus` declares, and release
/// whatever the previous call armed.
///
/// The DOM menu bar ([`render_with_menu_bar`]) invokes an item straight from its
/// `Rc<dyn Fn()>`, so a *click* needs nothing registered. A **chord** does: it
/// arrives as a bare keystroke with no menu in sight, and the registry is what
/// turns it back into a callback. On the desktop `build_native_menu_bar`
/// happens to do both jobs at once — Linux builds the native bar it never
/// attaches purely for this side effect — but there is no muda on the web, so
/// this is the same registration with the toolkit half left out.
///
/// Only items with **both** a shortcut and a callback are registered: an item
/// with a chord and no `on_click` would otherwise swallow that key combination
/// (see [`match_shortcut_code`]), and an item with neither has nothing a chord
/// could reach.
///
/// The build's ids are held in [`MENU_BAR_REGISTRATION`], the same slot the
/// native bar uses, so re-arming a rebuilt menu bar releases the previous
/// build's ids instead of growing the registry forever.
///
/// The returned [`MenuBarChords`] is what takes the chords back **down** again:
/// hold it for as long as the bar is mounted and drop it when it is not. A web
/// island can be unmounted, and nothing else would ever reclaim its build.
pub fn register_menu_shortcuts(menus: &[(&str, &Menu)]) -> MenuBarChords {
    let mut registration = MenuRegistration::default();
    let build = next_menu_bar_build();
    registration.build = build;
    for (_, menu) in menus {
        register_menu_shortcuts_into(menu, &mut registration);
    }
    // Bound outside the borrow: dropping the displaced token drops user
    // callbacks, whose `Drop` may re-enter the registry.
    let _previous = MENU_BAR_REGISTRATION.with(|slot| slot.borrow_mut().replace(registration));
    MenuBarChords {
        build,
        _not_send: PhantomData,
    }
}

/// Walk one menu (and its submenus), registering each chord-bearing item.
fn register_menu_shortcuts_into(menu: &Menu, registration: &mut MenuRegistration) {
    for entry in &menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                // A disabled item fires nothing, so its chord must not either —
                // the same rule `build_muda_item` applies, for the same reason.
                if !item.enabled {
                    continue;
                }
                let (Some(cb), Some(shortcut)) = (item.callback.as_ref(), item.shortcut.as_deref())
                else {
                    continue;
                };
                let menu_id = next_menu_id();
                registration.register_callback(
                    &menu_id,
                    Rc::clone(cb),
                    item.callback_owner.clone(),
                );
                registration.register_shortcut(shortcut, &menu_id);
            }
            MenuEntryInner::Separator => {}
            MenuEntryInner::Submenu { menu, .. } => {
                register_menu_shortcuts_into(menu, registration);
            }
        }
    }
}

/// A registry id no other build has used.
///
/// Monotonic rather than derived from the item, the way the ksni backend's
/// `ksni-{N}` is: two items may carry the same label in the same menu, and an id
/// collision between builds would let an earlier release reclaim a later
/// registration (see [`take_callback_if_ours`]).
fn next_menu_id() -> String {
    NEXT_MENU_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        format!("menu-bar-{id}")
    })
}

/// Build a native menu bar from a list of `(label, Menu)` pairs.
///
/// Each pair becomes a top-level submenu in the menu bar. Callbacks are
/// registered in the thread-local registry, and the ids are recorded in
/// [`MENU_BAR_REGISTRATION`] — so building a new bar releases the previous
/// build's, instead of leaving it in the registry forever.
#[cfg(feature = "desktop")]
pub(crate) fn build_native_menu_bar(menus: Vec<(&str, Menu)>) -> muda::Menu {
    let mut registration = MenuRegistration::default();
    let menu_bar = muda::Menu::new();
    for (label, menu) in menus {
        let submenu = muda::Submenu::new(label, true);
        build_muda_entries(&submenu, menu, &mut registration);
        let _ = menu_bar.append(&submenu);
    }
    // Bound outside the borrow: dropping the displaced token drops user
    // callbacks, whose `Drop` may re-enter the registry.
    let _previous = MENU_BAR_REGISTRATION.with(|slot| slot.borrow_mut().replace(registration));
    menu_bar
}

/// Build a muda `Menu` from a unified `Menu` (for tray context menus).
///
/// Returns the token holding this build's ids alongside the menu; the caller
/// keeps it for as long as the menu is live (see [`MenuRegistration`]).
#[cfg(feature = "desktop")]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn build_muda_menu(menu: Menu) -> (muda::Menu, MenuRegistration) {
    let mut registration = MenuRegistration::default();
    let muda_menu = muda::Menu::new();
    for entry in menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                let muda_item = build_muda_item(&item, &mut registration);
                let _ = muda_menu.append(&muda_item);
            }
            MenuEntryInner::Separator => {
                let _ = muda_menu.append(&muda::PredefinedMenuItem::separator());
            }
            MenuEntryInner::Submenu { label, menu } => {
                let submenu = muda::Submenu::new(&label, true);
                build_muda_entries(&submenu, menu, &mut registration);
                let _ = muda_menu.append(&submenu);
            }
        }
    }
    (muda_menu, registration)
}

/// Recursively populate a muda Submenu from a unified Menu.
#[cfg(feature = "desktop")]
fn build_muda_entries(submenu: &muda::Submenu, menu: Menu, registration: &mut MenuRegistration) {
    for entry in menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                let muda_item = build_muda_item(&item, registration);
                let _ = submenu.append(&muda_item);
            }
            MenuEntryInner::Separator => {
                let _ = submenu.append(&muda::PredefinedMenuItem::separator());
            }
            MenuEntryInner::Submenu { label, menu } => {
                let nested = muda::Submenu::new(&label, true);
                build_muda_entries(&nested, menu, registration);
                let _ = submenu.append(&nested);
            }
        }
    }
}

/// Build a single muda MenuItem, register its callback and shortcut.
#[cfg(feature = "desktop")]
fn build_muda_item(item: &MenuItem, registration: &mut MenuRegistration) -> muda::MenuItem {
    let accelerator = item.shortcut.as_ref().and_then(|s| parse_shortcut(s));
    let muda_item = muda::MenuItem::new(&item.label, item.enabled, accelerator);

    // A disabled item fires nothing. muda will not emit a `MenuEvent` for one,
    // and the in-app menu bar already skips its click handler — but registering
    // its chord anyway let `match_shortcut` run the callback the greyed-out item
    // refuses to run, *and* swallow the keystroke on the way.
    if !item.enabled {
        return muda_item;
    }

    // Register callback, owned by the scope that created it rather than by
    // whoever is building this menu.
    if let Some(cb) = &item.callback {
        registration.register_callback(&muda_item.id().0, cb.clone(), item.callback_owner.clone());
    }

    // Register shortcut for keyboard matching
    if let Some(shortcut_str) = &item.shortcut {
        registration.register_shortcut(shortcut_str, &muda_item.id().0);
    }

    muda_item
}

/// Set up the global muda event handler. Call once during app init.
///
/// This single handler covers both native menu events and tray context
/// menu events (same muda static after tray-icon 0.19 + muda 0.15).
#[cfg(feature = "desktop")]
pub(crate) fn install_menu_event_handler() {
    muda::MenuEvent::set_event_handler(Some(|event: muda::MenuEvent| {
        let id = event.id().0.clone();
        crate::shell::rinch_runtime::run_on_main_thread(move || {
            dispatch_menu_event(&id);
        });
    }));
}

// ── Platform-specific menu attachment ───────────────────────────────────────

/// Attach a native menu bar to a window (Windows).
#[cfg(all(feature = "desktop", target_os = "windows"))]
pub(crate) fn attach_menu_to_window(menu: &muda::Menu, window: &dyn winit::window::Window) {
    use winit::raw_window_handle::HasWindowHandle;
    if let Ok(handle) = window.window_handle() {
        if let winit::raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() {
            let hwnd = win32.hwnd.get() as isize;
            // Safety: hwnd is a valid window handle from winit, and we're on the main thread.
            unsafe {
                let _ = menu.init_for_hwnd(hwnd);
            }
        }
    }
}

/// Attach a native menu bar to the application (macOS).
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub(crate) fn attach_menu_to_window(menu: &muda::Menu, _window: &winit::window::Window) {
    menu.init_for_nsapp();
}

/// Attach a native menu bar to a window (Linux — not yet supported).
#[cfg(all(feature = "desktop", target_os = "linux"))]
pub(crate) fn attach_menu_to_window(_menu: &muda::Menu, _window: &dyn winit::window::Window) {
    // Linux GTK menu integration not yet implemented.
}

// ── Shortcut parsing ────────────────────────────────────────────────────────

/// A parsed keyboard shortcut for matching against keyboard events.
#[derive(Debug, Clone)]
pub(crate) struct ParsedShortcut {
    pub ctrl_or_cmd: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key's W3C UI Events `code` name — `"KeyK"`, `"Digit1"`, `"Enter"`,
    /// `"ArrowUp"`, `"F5"`.
    ///
    /// A string rather than a `winit::keyboard::KeyCode` so this module — and
    /// with it the DOM menu bar — builds with no windowing stack at all. It
    /// costs nothing on either side: winit's `KeyCode` variants are named after
    /// exactly this table (`key_code_name` is the one-to-one map), and a browser
    /// `KeyboardEvent.code` *is* this string, so the web path needs no
    /// translation whatsoever.
    pub code: &'static str,
}

/// Parse a shortcut string like "Cmd+N" or "Ctrl+Shift+S" into a muda Accelerator.
#[cfg(feature = "desktop")]
fn parse_shortcut(shortcut: &str) -> Option<Accelerator> {
    let normalized = shortcut
        .replace("Cmd+", "CmdOrCtrl+")
        .replace("Ctrl+", "CmdOrCtrl+")
        .replace("Meta+", "CmdOrCtrl+");

    Accelerator::from_str(&normalized).ok()
}

/// Parse a shortcut string into a ParsedShortcut for keyboard event matching.
fn parse_shortcut_for_matching(shortcut: &str) -> Option<ParsedShortcut> {
    let parts: Vec<&str> = shortcut.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut ctrl_or_cmd = false;
    let mut alt = false;
    let mut shift = false;
    let mut key_str = "";

    for part in &parts {
        let part_lower = part.to_lowercase();
        match part_lower.as_str() {
            "cmd" | "ctrl" | "control" | "meta" | "cmdorctrl" => ctrl_or_cmd = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            _ => key_str = part,
        }
    }

    let code = match key_str.to_uppercase().as_str() {
        "A" => "KeyA",
        "B" => "KeyB",
        "C" => "KeyC",
        "D" => "KeyD",
        "E" => "KeyE",
        "F" => "KeyF",
        "G" => "KeyG",
        "H" => "KeyH",
        "I" => "KeyI",
        "J" => "KeyJ",
        "K" => "KeyK",
        "L" => "KeyL",
        "M" => "KeyM",
        "N" => "KeyN",
        "O" => "KeyO",
        "P" => "KeyP",
        "Q" => "KeyQ",
        "R" => "KeyR",
        "S" => "KeyS",
        "T" => "KeyT",
        "U" => "KeyU",
        "V" => "KeyV",
        "W" => "KeyW",
        "X" => "KeyX",
        "Y" => "KeyY",
        "Z" => "KeyZ",
        "0" => "Digit0",
        "1" => "Digit1",
        "2" => "Digit2",
        "3" => "Digit3",
        "4" => "Digit4",
        "5" => "Digit5",
        "6" => "Digit6",
        "7" => "Digit7",
        "8" => "Digit8",
        "9" => "Digit9",
        "=" | "EQUAL" | "PLUS" => "Equal",
        "-" | "MINUS" => "Minus",
        "F1" => "F1",
        "F2" => "F2",
        "F3" => "F3",
        "F4" => "F4",
        "F5" => "F5",
        "F6" => "F6",
        "F7" => "F7",
        "F8" => "F8",
        "F9" => "F9",
        "F10" => "F10",
        "F11" => "F11",
        "F12" => "F12",
        "ENTER" | "RETURN" => "Enter",
        "ESCAPE" | "ESC" => "Escape",
        "BACKSPACE" => "Backspace",
        "TAB" => "Tab",
        "SPACE" => "Space",
        "DELETE" | "DEL" => "Delete",
        "HOME" => "Home",
        "END" => "End",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "UP" | "ARROWUP" => "ArrowUp",
        "DOWN" | "ARROWDOWN" => "ArrowDown",
        "LEFT" | "ARROWLEFT" => "ArrowLeft",
        "RIGHT" | "ARROWRIGHT" => "ArrowRight",
        _ => return None,
    };

    Some(ParsedShortcut {
        ctrl_or_cmd,
        alt,
        shift,
        code,
    })
}

/// The W3C `code` name of a winit key, or `None` for a key no shortcut string
/// can name.
///
/// The inverse of the table in [`parse_shortcut_for_matching`], and the only
/// place a `winit` type meets [`ParsedShortcut`]. Spelled out rather than
/// derived from `Debug`: the two happen to agree today, and a rename upstream
/// would silently unbind every shortcut in every app.
#[cfg(feature = "desktop")]
fn key_code_name(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::KeyA => "KeyA",
        KeyCode::KeyB => "KeyB",
        KeyCode::KeyC => "KeyC",
        KeyCode::KeyD => "KeyD",
        KeyCode::KeyE => "KeyE",
        KeyCode::KeyF => "KeyF",
        KeyCode::KeyG => "KeyG",
        KeyCode::KeyH => "KeyH",
        KeyCode::KeyI => "KeyI",
        KeyCode::KeyJ => "KeyJ",
        KeyCode::KeyK => "KeyK",
        KeyCode::KeyL => "KeyL",
        KeyCode::KeyM => "KeyM",
        KeyCode::KeyN => "KeyN",
        KeyCode::KeyO => "KeyO",
        KeyCode::KeyP => "KeyP",
        KeyCode::KeyQ => "KeyQ",
        KeyCode::KeyR => "KeyR",
        KeyCode::KeyS => "KeyS",
        KeyCode::KeyT => "KeyT",
        KeyCode::KeyU => "KeyU",
        KeyCode::KeyV => "KeyV",
        KeyCode::KeyW => "KeyW",
        KeyCode::KeyX => "KeyX",
        KeyCode::KeyY => "KeyY",
        KeyCode::KeyZ => "KeyZ",
        KeyCode::Digit0 => "Digit0",
        KeyCode::Digit1 => "Digit1",
        KeyCode::Digit2 => "Digit2",
        KeyCode::Digit3 => "Digit3",
        KeyCode::Digit4 => "Digit4",
        KeyCode::Digit5 => "Digit5",
        KeyCode::Digit6 => "Digit6",
        KeyCode::Digit7 => "Digit7",
        KeyCode::Digit8 => "Digit8",
        KeyCode::Digit9 => "Digit9",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Equal => "Equal",
        KeyCode::Minus => "Minus",
        KeyCode::Enter => "Enter",
        KeyCode::Escape => "Escape",
        KeyCode::Backspace => "Backspace",
        KeyCode::Tab => "Tab",
        KeyCode::Space => "Space",
        KeyCode::Delete => "Delete",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::ArrowUp => "ArrowUp",
        KeyCode::ArrowDown => "ArrowDown",
        KeyCode::ArrowLeft => "ArrowLeft",
        KeyCode::ArrowRight => "ArrowRight",
        _ => return None,
    })
}

// ── Test-support accessors ──────────────────────────────────────────────────

/// How many callbacks the registry currently holds.
///
/// The leak this module fixes is invisible to a behavioural assertion — a stale
/// callback is inert, just never removed — so the tests below assert on the size
/// of the registry directly.
#[cfg(test)]
pub(crate) fn callback_count() -> usize {
    MENU_CALLBACKS.with(|map| map.borrow().len())
}

/// How many shortcuts the registry currently holds. See [`callback_count`].
#[cfg(test)]
fn shortcut_count() -> usize {
    MENU_SHORTCUTS.with(|shortcuts| shortcuts.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::reactive::{Scope, Signal};
    use std::cell::Cell;
    #[cfg(feature = "desktop")]
    use std::collections::BTreeSet;

    /// Every registry here is `thread_local!` and `cargo test` gives each test
    /// its own thread, so the tests are isolated without a mutex — but ids must
    /// still be distinct per test, because a test that leaves state behind would
    /// otherwise be indistinguishable from the leak being pinned.
    ///
    /// The callbacks never construct a `muda::MenuItem`: muda is GTK-backed on
    /// Linux and a headless runner has no `gtk_init`. Synthetic id strings
    /// exercise the registry directly, which is where the defect lives.
    fn probe() -> (Rc<Cell<u32>>, Rc<dyn Fn()>) {
        let fired = Rc::new(Cell::new(0u32));
        let seen = fired.clone();
        (fired, Rc::new(move || seen.set(seen.get() + 1)))
    }

    /// Register under the ambient owner, the way a caller that both creates and
    /// registers a callback in one place would. The builders instead carry the
    /// item's own owner, which is what
    /// [`the_owner_is_the_scope_that_created_the_callback_not_the_one_that_built_the_menu`]
    /// pins.
    fn register_callback(menu_id: &str, cb: Rc<dyn Fn()>) {
        register_callback_owned(menu_id, cb, current_owner());
    }

    /// The lifetime half.
    ///
    /// This test used to be
    /// `a_menu_callback_registered_in_a_scope_is_removed_when_the_scope_disposes`
    /// and asserted that disposal *removed* the entry, through
    /// `install_scoped_entry`. That mechanism is gone: it reclaimed by the
    /// **building** scope, so a component assembling a menu out of other
    /// components' items disabled all of them when it unmounted (see
    /// [`a_builder_unmounting_does_not_kill_another_components_live_item`]), and
    /// it appended a cleanup per item per rebuild. The pair that replaced it is
    /// what this asserts: the dispatch check makes a dead callback inert
    /// immediately, and the token reclaims the memory when it drops.
    #[test]
    fn a_dead_menu_callback_is_inert_at_dispatch_and_released_by_its_token() {
        let base = callback_count();
        let (fired, cb) = probe();

        // Registered from inside the scope, as the original did: here the
        // creating and building scopes coincide, which is the case the removed
        // mechanism handled and the one it was easiest to mistake for general.
        let mut registration = MenuRegistration::default();
        let scope = Scope::new();
        scope.run(|| {
            registration.register_callback("scoped-dispatched", cb, current_owner());
            registration.register_callback("scoped-untouched", Rc::new(|| {}), current_owner());
        });
        assert_eq!(callback_count(), base + 2);

        scope.dispose();
        assert_eq!(
            callback_count(),
            base + 2,
            "disposal alone reclaims nothing now — that is the token's job"
        );

        assert!(
            !dispatch_menu_event("scoped-dispatched"),
            "a disposed component's callback must not run"
        );
        assert_eq!(fired.get(), 0);

        drop(registration);
        assert_eq!(
            callback_count(),
            base,
            "dropping the token releases the build, dispatched or not"
        );
    }

    /// The contract every in-tree menu depends on: `App::menu` builds the
    /// bar from `main`, before the event loop, with no ambient owner, and
    /// `examples/ui-zoo-desktop` deliberately creates its signals there "so menu
    /// callbacks can reference them". Requiring an owner would break it.
    #[test]
    fn a_menu_callback_registered_from_main_survives_and_still_dispatches() {
        let (fired, cb) = probe();
        register_callback("main-1", cb);

        Scope::new().dispose();

        dispatch_menu_event("main-1");
        assert_eq!(
            fired.get(),
            1,
            "an ownerless registration keeps app lifetime"
        );
    }

    /// The issue's second recorded asymmetry: menu callbacks bypass
    /// `register_handler`, so they pushed no creation-time owner, and a
    /// `Signal::new` inside one landed at app lifetime instead of belonging to
    /// the menu's component.
    #[test]
    fn a_menu_callback_runs_with_its_registering_component_as_ambient_owner() {
        let scope = Scope::new();
        let before = scope.owned_counts().signals;

        scope.run(|| {
            register_callback(
                "owner-1",
                Rc::new(|| {
                    let _ = Signal::new(7u32);
                }),
            )
        });
        dispatch_menu_event("owner-1");

        assert_eq!(
            scope.owned_counts().signals,
            before + 1,
            "a Signal created inside a menu callback belongs to the menu's component"
        );
        scope.dispose();
    }

    /// The mirror image: an app-lifetime callback must not inherit whatever
    /// scope the dispatch happens to be nested inside. The owner stack is not an
    /// ancestor chain, so that scope is unrelated — and disposing it would free
    /// a signal the app-lifetime callback still holds.
    #[test]
    fn an_ownerless_menu_callback_does_not_allocate_into_the_dispatching_scope() {
        register_callback(
            "ownerless-1",
            Rc::new(|| {
                let _ = Signal::new(1u32);
            }),
        );

        let host = Scope::new();
        let before = host.owned_counts().signals;
        host.run(|| dispatch_menu_event("ownerless-1"));

        assert_eq!(
            host.owned_counts().signals,
            before,
            "an app-lifetime callback must not hand its state to whatever scope dispatched it"
        );
        host.dispose();
    }

    /// `dispatch_menu_event` used to hold the registry's `borrow()` across the
    /// callback, so a callback that rebuilt a menu panicked with
    /// `BorrowMutError`.
    #[test]
    fn a_menu_callback_may_rebuild_the_menu_from_inside_its_own_dispatch() {
        register_callback(
            "rebuild-1",
            Rc::new(|| {
                register_callback("rebuild-2", Rc::new(|| {}));
            }),
        );

        dispatch_menu_event("rebuild-1");

        assert!(
            MENU_CALLBACKS.with(|map| map.borrow().contains_key("rebuild-2")),
            "a callback must be able to rebuild the menu it was dispatched from"
        );
    }

    /// The same defect one level up: `match_shortcut` held the shortcut list's
    /// `borrow()` across `dispatch_menu_event`.
    #[test]
    fn a_shortcut_callback_may_register_a_shortcut_from_inside_its_own_dispatch() {
        register_callback(
            "chord-1",
            Rc::new(|| {
                register_shortcut("Ctrl+Shift+K", "chord-2");
            }),
        );
        register_shortcut("Ctrl+Shift+J", "chord-1");

        assert!(match_shortcut_code(true, false, false, true, "KeyJ"));
    }

    /// `MENU_SHORTCUTS` leaks identically to `MENU_CALLBACKS` — the issue names
    /// only the latter. A chord left behind keeps matching, and `match_shortcut`
    /// returning `true` swallows the key into a callback that is no longer
    /// there; the list also has to shrink, or matching goes linear in menus ever
    /// built.
    ///
    /// This was
    /// `a_shortcut_registered_in_a_scope_stops_matching_when_the_scope_disposes`
    /// and asserted the list shrank on the registering scope's disposal, through
    /// the same scope-tied removal as its callback twin, and is rewritten for
    /// the same reason — see
    /// [`a_dead_menu_callback_is_inert_at_dispatch_and_released_by_its_token`].
    /// Two mechanisms carry the contract instead: a chord whose item is dead
    /// falls through and the prune takes that item's chords with it, and the
    /// token releases whatever chords its build still holds.
    #[test]
    fn a_dead_chord_falls_through_and_the_token_releases_the_builds_chords() {
        let callbacks = callback_count();
        let shortcuts = shortcut_count();

        let mut registration = MenuRegistration::default();
        let scope = Scope::new();
        let owner = scope.run(current_owner);
        registration.register_callback("sc-1", Rc::new(|| {}), owner);
        registration.register_shortcut("Ctrl+Alt+Y", "sc-1");
        registration.register_shortcut("Ctrl+Alt+P", "sc-no-callback");
        assert_eq!(shortcut_count(), shortcuts + 2);

        scope.dispose();
        assert!(
            !match_shortcut_code(true, false, true, false, "KeyY"),
            "a disposed component's chord must fall through"
        );
        assert_eq!(
            shortcut_count(),
            shortcuts + 1,
            "and the prune takes the dead item's chords out with it"
        );

        drop(registration);
        assert_eq!(
            shortcut_count(),
            shortcuts,
            "the token releases the rest, so matching does not go O(menus ever built)"
        );
        assert_eq!(callback_count(), callbacks);
    }

    /// `match_shortcut`'s answer is what makes the runtime swallow the key, so
    /// it must mean "a callback ran". A chord whose id has no callback — an item
    /// given a `shortcut` but no `on_click` — used to eat that key combination
    /// for the life of the app.
    #[test]
    fn a_chord_that_fires_nothing_falls_through_instead_of_swallowing_the_key() {
        register_shortcut("Ctrl+Alt+U", "no-callback-here");

        assert!(
            !match_shortcut_code(true, false, true, false, "KeyU"),
            "nothing ran, so the keystroke belongs to the app"
        );
    }

    /// Only the *first* matching chord used to be tried, and it consumed the key
    /// whatever happened. A dead duplicate registered earlier would therefore
    /// shadow a live one — silently, and for one keystroke every time the dead
    /// entry was re-created.
    #[test]
    fn a_dead_chord_does_not_shadow_a_live_duplicate_registered_after_it() {
        let (fired, cb) = probe();

        let dead = Scope::new();
        let owner = dead.run(current_owner);
        register_callback_owned("shadow-dead", Rc::new(|| {}), owner);
        register_shortcut("Ctrl+Alt+I", "shadow-dead");

        register_callback("shadow-live", cb);
        register_shortcut("Ctrl+Alt+I", "shadow-live");

        dead.dispose();

        assert!(match_shortcut_code(true, false, true, false, "KeyI"));
        assert_eq!(
            fired.get(),
            1,
            "the live chord must fire on the first press, not the second"
        );
    }

    /// Menu ids come from two sources and nothing guarantees they are distinct,
    /// so removal is by `Rc` identity: an earlier unmount must not reclaim an id
    /// a later component has since taken over.
    #[test]
    fn a_later_registration_at_the_same_id_survives_the_earlier_scopes_disposal() {
        let (first_fired, first_cb) = probe();
        let (second_fired, second_cb) = probe();

        let first = Scope::new();
        first.run(|| register_callback("dup-1", first_cb));
        let second = Scope::new();
        second.run(|| register_callback("dup-1", second_cb));

        first.dispose();
        dispatch_menu_event("dup-1");

        assert_eq!(first_fired.get(), 0);
        assert_eq!(
            second_fired.get(),
            1,
            "an earlier unmount must not clobber a later registration at the same id"
        );
        second.dispose();
    }

    /// Ownership is per **item**, not per build. A component contributes an item
    /// to a menu somebody else assembles; when that component unmounts its
    /// signals are freed, and the item must stop firing even though the builder
    /// is still very much alive. Recording the owner at build time would miss
    /// exactly this, which is the shape that reintroduced the panic in PR2.
    #[test]
    fn the_owner_is_the_scope_that_created_the_callback_not_the_one_that_built_the_menu() {
        let base = callback_count();
        let (fired, cb) = probe();

        let creator = Scope::new();
        let owner = creator.run(current_owner);
        let builder = Scope::new();
        builder.run(|| {
            register_callback_owned("granular-1", cb, owner);
        });

        creator.dispose();
        dispatch_menu_event("granular-1");

        assert_eq!(
            fired.get(),
            0,
            "the component that created this callback is gone"
        );
        assert_eq!(
            callback_count(),
            base,
            "and the dead entry is pruned rather than re-checked forever"
        );
        builder.dispose();
    }

    /// The mirror image, and why *removal* must not be tied to the building
    /// scope either.
    ///
    /// Component X assembles a menu — or a tray — out of items contributed by
    /// live components Y and Z, and hands the build's token to something that
    /// outlives it: `main`, the [`MENU_BAR_REGISTRATION`] slot, a `TrayIcon`.
    /// When X unmounts, the menu is still on screen and the token is still
    /// alive, so Y's and Z's items must still fire. Reclaiming on X's disposal
    /// instead disables every id X installed — including the ones it did not
    /// create — permanently, and silently.
    #[test]
    fn a_builder_unmounting_does_not_kill_another_components_live_item() {
        let (y_fired, y_cb) = probe();
        let (z_fired, z_cb) = probe();

        let y = Scope::new();
        let y_owner = y.run(current_owner);
        let z = Scope::new();
        let z_owner = z.run(current_owner);

        // X builds; the token outlives X, as every real holder of one does.
        let mut registration = MenuRegistration::default();
        let builder = Scope::new();
        builder.run(|| {
            registration.register_callback("contrib-y", y_cb, y_owner);
            registration.register_callback("contrib-z", z_cb, z_owner);
        });

        builder.dispose();

        assert!(
            dispatch_menu_event("contrib-y"),
            "Y is still mounted and the build is still held"
        );
        assert!(dispatch_menu_event("contrib-z"));
        assert_eq!(y_fired.get(), 1);
        assert_eq!(
            z_fired.get(),
            1,
            "the builder unmounting must not silence a contributor that is still live"
        );

        y.dispose();
        z.dispose();
    }

    /// `MenuItem::on_click` is where the closure captures its signals, so that is
    /// where the owner has to come from — not from wherever the `Menu` is later
    /// assembled.
    #[test]
    fn on_click_records_the_scope_that_was_rendering_when_the_closure_was_made() {
        let scope = Scope::new();
        let item = scope.run(|| MenuItem::new("Save").on_click(|| {}));
        assert!(
            item.callback_owner
                .as_ref()
                .is_some_and(|owner| owner.is_alive())
        );

        scope.dispose();
        assert!(
            item.callback_owner.as_ref().is_some_and(|o| !o.is_alive()),
            "the item's owner dies with the component that created it"
        );

        let from_main = MenuItem::new("Quit").on_click(|| {});
        assert!(
            from_main.callback_owner.is_none(),
            "an item built from main has app lifetime"
        );
    }

    /// The leak half. `on_cleanup` cannot close it on its own: the in-tree menus
    /// are built from `main`, with no scope to hang a cleanup on, so nothing
    /// shrinks. The RAII token is what actually reclaims a build.
    #[test]
    fn dropping_a_builds_registration_returns_the_registry_to_its_baseline() {
        let callbacks = callback_count();
        let shortcuts = shortcut_count();

        {
            let mut registration = MenuRegistration::default();
            registration.register_callback("tok-1", Rc::new(|| {}), None);
            registration.register_callback("tok-2", Rc::new(|| {}), None);
            registration.register_shortcut("Ctrl+Alt+Q", "tok-1");
            assert_eq!(callback_count(), callbacks + 2);
            assert_eq!(shortcut_count(), shortcuts + 1);
        }

        assert_eq!(
            callback_count(),
            callbacks,
            "dropping the token releases it"
        );
        assert_eq!(shortcut_count(), shortcuts);
    }

    /// The ksni shape, which is the worst case: `tray.rs` mints a fresh
    /// `ksni-{N}` id from a monotonic counter for every item on every build, so
    /// a rebuilt tray never reuses a key and cannot overwrite its own previous
    /// entries. Only releasing the previous build's token bounds it.
    #[test]
    fn rebuilding_a_menu_with_fresh_ids_does_not_grow_the_registry() {
        let base = callback_count();
        let mut live = None;

        for build in 0..5u32 {
            let mut registration = MenuRegistration::default();
            for item in 0..4u32 {
                registration.register_callback(
                    &format!("ksni-{}", build * 4 + item),
                    Rc::new(|| {}),
                    None,
                );
            }
            // Assigning drops the previous build's token, exactly as replacing a
            // `TrayIcon` does.
            live = Some(registration);
            assert_eq!(
                callback_count(),
                base + 4,
                "rebuild {build} must not accumulate"
            );
        }

        drop(live);
        assert_eq!(callback_count(), base);
    }

    // ── The DOM menu bar's own registration ──────────────────────────────
    //
    // These run in *both* configurations — with `desktop` and without it — and
    // that is the point: they are the half of the menu system a `rinch-web`
    // build gets, where there is no muda to arm a chord and no winit `KeyCode`
    // to match one with. A regression that re-coupled either to the desktop
    // would take the whole web menu bar with it.

    /// The web path's equivalent of building the native bar: declare a menu,
    /// arm it, press the chord.
    #[test]
    fn register_menu_shortcuts_arms_a_declared_chord_including_inside_a_submenu() {
        let (top_fired, top_cb) = probe();
        let (nested_fired, nested_cb) = probe();

        let file = Menu::new()
            .item(
                MenuItem::new("Find")
                    .shortcut("Ctrl+Shift+F")
                    .on_click(move || top_cb()),
            )
            .separator()
            .submenu(
                "Recent",
                Menu::new().item(
                    MenuItem::new("Reopen")
                        .shortcut("Ctrl+Shift+T")
                        .on_click(move || nested_cb()),
                ),
            );

        let _chords = register_menu_shortcuts(&[("File", &file)]);

        assert!(match_shortcut_code(true, false, false, true, "KeyF"));
        assert_eq!(top_fired.get(), 1);
        assert!(
            match_shortcut_code(true, false, false, true, "KeyT"),
            "a submenu's items declare chords like any other"
        );
        assert_eq!(nested_fired.get(), 1);
    }

    /// A disabled item fires nothing, so its chord must not be armed — else the
    /// keystroke runs the callback the greyed-out item refuses to run, *and* is
    /// swallowed on the way. `build_muda_item` has always applied this rule; the
    /// DOM bar's registration has to apply it too.
    #[test]
    fn register_menu_shortcuts_skips_a_disabled_item_and_one_with_no_callback() {
        let (fired, cb) = probe();
        let shortcuts = shortcut_count();

        let view = Menu::new()
            .item(
                MenuItem::new("Zoom In")
                    .shortcut("Ctrl+Alt+B")
                    .enabled(false)
                    .on_click(move || cb()),
            )
            .item(MenuItem::new("Zoom Out").shortcut("Ctrl+Alt+M"));

        let _chords = register_menu_shortcuts(&[("View", &view)]);

        assert_eq!(
            shortcut_count(),
            shortcuts,
            "neither item may arm a chord: one is disabled, the other runs nothing"
        );
        assert!(!match_shortcut_code(true, false, true, false, "KeyB"));
        assert_eq!(fired.get(), 0);
        assert!(
            !match_shortcut_code(true, false, true, false, "KeyM"),
            "an item with no on_click must not eat its key combination"
        );
    }

    /// Re-arming a rebuilt bar must release the previous build, or a menu
    /// rebuilt at runtime grows the registry without bound — the leak
    /// `MenuRegistration` exists to close, reached by the other builder.
    #[test]
    fn re_arming_the_menu_bar_releases_the_previous_builds_chords() {
        let callbacks = callback_count();
        let shortcuts = shortcut_count();

        // Every build's token is kept alive to the end, so this asserts the
        // *replacement* releases the previous build — not a token drop.
        let mut tokens = Vec::new();
        for _ in 0..5 {
            let menu =
                Menu::new().item(MenuItem::new("Save").shortcut("Ctrl+Alt+S").on_click(|| {}));
            tokens.push(register_menu_shortcuts(&[("File", &menu)]));
            assert_eq!(
                callback_count(),
                callbacks + 1,
                "a rebuild must not accumulate"
            );
            assert_eq!(shortcut_count(), shortcuts + 1);
        }

        // Releasing the slot is what actually reclaims the last build.
        MENU_BAR_REGISTRATION.with(|slot| slot.borrow_mut().take());
        assert_eq!(callback_count(), callbacks);
        assert_eq!(shortcut_count(), shortcuts);
        drop(tokens);
    }

    /// The other half of "arming replaces": an island that *unmounts* has
    /// nothing to replace its build, so its token has to take the chords back
    /// down — otherwise a removed widget goes on running its callback and goes
    /// on calling `preventDefault` on its host page for the rest of the session
    /// (measured in Chrome 153; `rinch-web/tests/menu_bar_shortcuts.rs` is the
    /// browser half of this pair).
    #[test]
    fn dropping_the_token_disarms_the_chords_it_armed() {
        let (fired, cb) = probe();
        let menu = Menu::new().item(
            MenuItem::new("Focus Search")
                .shortcut("Ctrl+Alt+K")
                .on_click(move || cb()),
        );

        let chords = register_menu_shortcuts(&[("View", &menu)]);
        assert!(
            match_shortcut_code(true, false, true, false, "KeyK"),
            "control: armed while the bar is up"
        );
        assert_eq!(fired.get(), 1);

        drop(chords);

        assert!(
            !match_shortcut_code(true, false, true, false, "KeyK"),
            "an unmounted bar must not go on eating its chord"
        );
        assert_eq!(fired.get(), 1, "nor go on running its item");
    }

    /// Release is "only if the slot is still mine", the [`take_callback_if_ours`]
    /// discipline one level up. Two islands on one page share the chord
    /// registry and the second to arm wins; the first unmounting afterwards
    /// must leave the second's chords alone — a token that reclaimed on the
    /// strength of having *once* armed would silently disarm a live bar.
    #[test]
    fn a_stale_token_leaves_a_later_builds_chords_armed() {
        let (first_fired, first_cb) = probe();
        let (second_fired, second_cb) = probe();
        let first_menu = Menu::new().item(
            MenuItem::new("One")
                .shortcut("Ctrl+Alt+Y")
                .on_click(move || first_cb()),
        );
        let second_menu = Menu::new().item(
            MenuItem::new("Two")
                .shortcut("Ctrl+Alt+Z")
                .on_click(move || second_cb()),
        );

        let first = register_menu_shortcuts(&[("First", &first_menu)]);
        let second = register_menu_shortcuts(&[("Second", &second_menu)]);

        drop(first);

        assert!(
            match_shortcut_code(true, false, true, false, "KeyZ"),
            "the second build is the one the slot holds; a stale token must not take it"
        );
        assert_eq!(second_fired.get(), 1);
        assert!(
            !match_shortcut_code(true, false, true, false, "KeyY"),
            "the first build was released by the replacement, not by its token"
        );
        assert_eq!(first_fired.get(), 0);

        drop(second);
        assert!(
            !match_shortcut_code(true, false, true, false, "KeyZ"),
            "and the live token still disarms its own"
        );
    }

    /// The shortcut string is parsed once, into a `code` name both sides agree
    /// on. This pins that agreement: the desktop matches by winit `KeyCode`, the
    /// browser by `KeyboardEvent.code`, and one registration must answer to
    /// both.
    #[cfg(feature = "desktop")]
    #[test]
    fn the_winit_keycode_and_the_web_code_name_reach_the_same_callback() {
        for (chord, key, code) in [
            ("Ctrl+K", KeyCode::KeyK, "KeyK"),
            ("Ctrl+0", KeyCode::Digit0, "Digit0"),
            ("F5", KeyCode::F5, "F5"),
            ("Ctrl+=", KeyCode::Equal, "Equal"),
            ("Alt+ArrowUp", KeyCode::ArrowUp, "ArrowUp"),
        ] {
            let parsed =
                parse_shortcut_for_matching(chord).unwrap_or_else(|| panic!("{chord} must parse"));
            assert_eq!(parsed.code, code, "{chord}");
            assert_eq!(
                key_code_name(key),
                Some(code),
                "{chord}: winit's KeyCode must name the same key as the web code"
            );
        }
    }

    /// Every key a shortcut can name, both ways.
    ///
    /// The five pairs above are a sample, and a sample is what a hand-written
    /// 64-arm table is least safe against: flipping `KeyCode::KeyN => "KeyN"` to
    /// `"KeyM"` survived the entire `menu::` suite, and in production that is
    /// Ctrl+N on the desktop silently running the Ctrl+M item — with a green
    /// board, because the *web* half of the same registration is unaffected. The
    /// two backends diverge and nothing says so.
    ///
    /// Neither direction is asserted against a second hand-written table, which
    /// would only move the typo:
    ///
    /// * the inverse table is checked against `Debug`, which is exactly what
    ///   `key_code_name` deliberately does **not** derive from. Its reason for
    ///   spelling the arms out — that an upstream *rename* must be a compile
    ///   error rather than a silent unbinding — survives, because a renamed
    ///   variant stops compiling here too;
    /// * the forward table is checked against a spelling *derived* from the code
    ///   name (`KeyA` → `A`, `Digit0` → `0`, everything else is its own
    ///   spelling), so a typo in `parse_shortcut_for_matching` fails too.
    ///
    /// Together they pin the bijection the two tables are: a code reachable from
    /// a `KeyCode` is reachable from a shortcut string, and names the same key.
    #[cfg(feature = "desktop")]
    #[test]
    fn every_arm_of_the_key_table_round_trips() {
        let mut seen: Vec<&'static str> = Vec::new();
        for key in NAMED_KEYS {
            let debug = format!("{key:?}");
            let code = key_code_name(key)
                .unwrap_or_else(|| panic!("{debug}: no arm names this key any more"));
            assert_eq!(
                code, debug,
                "key_code_name({debug}) must name its own variant"
            );

            // The shortcut spelling this code answers to. Every code name but a
            // letter's and a digit's *is* an accepted spelling, uppercased.
            let spelling = code
                .strip_prefix("Key")
                .or_else(|| code.strip_prefix("Digit"))
                .unwrap_or(code);
            let parsed = parse_shortcut_for_matching(spelling)
                .unwrap_or_else(|| panic!("{code}: \"{spelling}\" must parse as a shortcut"));
            assert_eq!(
                parsed.code, code,
                "\"{spelling}\" must parse back to {code}"
            );
            assert!(
                !parsed.ctrl_or_cmd && !parsed.alt && !parsed.shift,
                "\"{spelling}\" is a bare key, not a modifier"
            );

            assert!(
                !seen.contains(&code),
                "{code} is reached by two KeyCodes: one of them shadows the other"
            );
            seen.push(code);
        }
    }

    /// The keys [`key_code_name`] answers to.
    ///
    /// Listed rather than iterated because `winit::keyboard::KeyCode` is
    /// `#[non_exhaustive]` with hundreds of variants and no iterator. A variant
    /// *renamed* upstream fails to compile here, which is the whole reason the
    /// source table is spelled out; a variant **added** to the table without
    /// being added here is caught by
    /// [`the_two_key_tables_name_the_same_codes`], which counts the arms.
    #[cfg(feature = "desktop")]
    const NAMED_KEYS: [KeyCode; 64] = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
        KeyCode::Equal,
        KeyCode::Minus,
        KeyCode::Enter,
        KeyCode::Escape,
        KeyCode::Backspace,
        KeyCode::Tab,
        KeyCode::Space,
        KeyCode::Delete,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
    ];

    /// The two tables name the **same set** of codes — the other direction, and
    /// the one a per-arm walk cannot reach.
    ///
    /// [`every_arm_of_the_key_table_round_trips`] starts from a `KeyCode` and so
    /// only ever visits codes the inverse table produces. A spelling added to
    /// `parse_shortcut_for_matching` whose code **no** `KeyCode` arm produces is
    /// invisible to it: measured, adding a `Comma` spelling to the forward table
    /// survives that test and the whole `menu::` suite. The consequence is
    /// #807's own bug one direction along — `MenuItem::shortcut("Ctrl+Comma")`
    /// arms and fires on the web, which matches `KeyboardEvent.code` directly,
    /// and is permanently dead on the desktop, where `key_code_name` answers
    /// `None`. Two backends, one declaration, silently different.
    ///
    /// Both tables are read out of **this file's own source**, because a set
    /// comparison has nothing else to derive them from and a hand-written third
    /// list would only move the typo. That also pins the arm *count*, so a 65th
    /// arm added to `key_code_name` without a 65th entry in [`NAMED_KEYS`] fails
    /// here rather than going unwalked.
    ///
    /// The parse is deliberately narrow — a trimmed, non-comment line whose
    /// right-hand side is a quoted string literal followed by a comma — which is
    /// the shape `cargo fmt` gives both tables and nothing else in this file. A
    /// trailing `//` comment is stripped first, because an arm outside the
    /// accepted shape is *silently uncovered* on the forward side (see the loop)
    /// where the inverse side fails loud through the count. The length
    /// assertions are the positive control: a parse that matched nothing would
    /// otherwise compare two empty sets and pass.
    #[cfg(feature = "desktop")]
    #[test]
    fn the_two_key_tables_name_the_same_codes() {
        let source = include_str!("mod.rs");
        let mut from_key_codes: BTreeSet<&str> = BTreeSet::new();
        let mut from_spellings: BTreeSet<&str> = BTreeSet::new();
        let mut inverse_arms = 0usize;

        for line in source.lines() {
            // A trailing `//` comment is stripped rather than skipped, and that
            // is not cosmetic: `cargo fmt` accepts an arm written with one, and
            // skipping the line makes a *forward*-only spelling invisible —
            // the sets stay equal and the M5 defect passes. The inverse side
            // fails loud either way, through the arm count, which is exactly the
            // asymmetry (it has something to count against and the forward side
            // does not). A `//` inside one of the string literals would be
            // wrong to strip, and neither table contains one.
            let line = line.split("//").next().unwrap_or(line).trim();
            let Some((lhs, rhs)) = line.split_once(" => ") else {
                continue;
            };
            // `"Code",` and nothing else: `_ => return None,` and the modifier
            // table (whose arms assign a bool) both fail this.
            let Some(code) = rhs
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix("\","))
                .filter(|code| !code.contains('"'))
            else {
                continue;
            };
            if let Some(variant) = lhs.strip_prefix("KeyCode::") {
                if variant.contains(' ') {
                    continue;
                }
                inverse_arms += 1;
                from_key_codes.insert(code);
            } else if lhs.starts_with('"') && lhs.ends_with('"') {
                from_spellings.insert(code);
            }
        }

        assert_eq!(
            inverse_arms,
            NAMED_KEYS.len(),
            "key_code_name has {inverse_arms} arms and NAMED_KEYS lists {}: \
             an arm nothing walks is an arm nothing checks",
            NAMED_KEYS.len()
        );
        assert_eq!(
            from_key_codes.len(),
            NAMED_KEYS.len(),
            "two arms name the same code, or the parse found the wrong table"
        );
        assert_eq!(
            from_spellings, from_key_codes,
            "a code one table can produce and the other cannot: \
             a shortcut string that arms on the web and is dead on the desktop, \
             or a KeyCode no shortcut string can name"
        );
    }

    /// A token must reclaim only what is still its own, for the same reason the
    /// scope cleanup must: a later build may have taken the id over.
    #[test]
    fn dropping_a_token_leaves_an_id_a_later_build_has_taken_over() {
        let (first_fired, first_cb) = probe();
        let (second_fired, second_cb) = probe();

        let mut first = MenuRegistration::default();
        first.register_callback("shared-1", first_cb, None);
        let mut second = MenuRegistration::default();
        second.register_callback("shared-1", second_cb, None);

        drop(first);
        dispatch_menu_event("shared-1");

        assert_eq!(first_fired.get(), 0);
        assert_eq!(
            second_fired.get(),
            1,
            "the later build still owns the id it registered"
        );
    }
}
