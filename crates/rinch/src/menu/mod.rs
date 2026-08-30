//! Unified menu system for native window menus and system tray context menus.
//!
//! Provides [`Menu`] and [`MenuItem`] builder types that work with both
//! native menu bars (via `muda`) and tray context menus (via `tray-icon`).
//! Callbacks are `Rc<dyn Fn()>` — no `Send`/`Sync` burden on users. The
//! runtime wires push-based event delivery so callbacks always run on the
//! main thread.
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
//! and `match_shortcut` all route through) and the Linux in-app menu bar, which
//! renders items straight out of the [`Menu`] and holds the `Rc<dyn Fn()>`
//! itself rather than a registry id.
//!
//! Ownership is recorded per **item**, not per menu build. One `Menu` may be
//! assembled from items contributed by several components — and the build
//! itself commonly happens somewhere else entirely (`main`, a tray builder) — so
//! a per-build owner would both silence live items and, worse, keep an unmounted
//! component's item armed. Relaxing is the unsafe direction: over-pruning only
//! drops a click, while under-pruning restores the panic.
//!
//! Registering with **no ambient owner** — from `main`, from startup code,
//! before the event loop — records no owner and keeps app lifetime, unchanged.
//! That is how every in-tree menu is built.
//!
//! Removal has two mechanisms, because they answer different questions.
//! [`on_cleanup`], through [`install_scoped_entry`], removes an id when the
//! scope that *built* the menu is disposed. But a menu built from `main` has no
//! scope, and the registry is otherwise append-only: nothing removed an entry,
//! ever, so a menu rebuilt at runtime accumulated — and the ksni tray path mints
//! a fresh `ksni-{N}` id for every item on every build, so it could never even
//! overwrite. Hence `MenuRegistration`, an RAII token holding the ids one build
//! registered, released when whoever owns that build replaces or drops it.
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
//! run_with_menu("My App", 800, 600, app, vec![("File", file_menu)]);
//!
//! // For tray context menu:
//! TrayIconBuilder::new().with_menu(menu).build()?;
//! ```

#[cfg(target_os = "linux")]
pub(crate) mod app_menu_bar;

use muda::accelerator::Accelerator;
use rinch_core::reactive::{Owner, current_owner, install_scoped_entry, on_cleanup, unowned};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::str::FromStr;
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

    /// The registration held by the most recently built native menu bar.
    ///
    /// The bar is a process-wide singleton owned by the runtime rather than by
    /// any caller, so its token lives here: building a new bar replaces this,
    /// dropping the previous build's ids. This is what keeps
    /// [`build_native_menu_bar`] from growing the registry on every rebuild.
    static MENU_BAR_REGISTRATION: RefCell<Option<MenuRegistration>> = const { RefCell::new(None) };
}

/// Register `cb` under `menu_id` as belonging to `owner`, returning the entry so
/// a [`MenuRegistration`] can hold it for the `Rc::ptr_eq` check on release.
///
/// Removal is tied to the scope that is currently rendering — the one *building*
/// the menu — via [`install_scoped_entry`], which reclaims the id only if it
/// still holds this callback. `owner` is a separate question: it is the scope
/// that created the callback, and it gates *invocation* (see
/// [`dispatch_menu_event`]). The two differ whenever a component contributes an
/// item to a menu somebody else assembles.
fn register_callback_owned(
    menu_id: &str,
    cb: Rc<dyn Fn()>,
    owner: Option<Owner>,
) -> Rc<MenuCallback> {
    let entry = Rc::new(MenuCallback { owner, cb });
    install_scoped_entry(&MENU_CALLBACKS, menu_id.to_string(), entry.clone());
    entry
}

/// Register `shortcut_str` as dispatching `menu_id`, returning the serial that
/// identifies this registration (`None` if the string does not parse).
///
/// Removal is tied to the scope that is currently rendering, so a component that
/// builds a menu takes its chords with it — otherwise the chord would keep
/// matching and returning `true`, swallowing the key into a callback that is no
/// longer there.
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
    on_cleanup(move || remove_shortcut(serial));
    Some(serial)
}

/// Drop the shortcut registered under `serial`, if it is still there.
///
/// `try_with`/`try_borrow_mut`: this runs from `Scope::dispose`, reachable from
/// a TLS destructor at thread exit (when the registry may already be gone) and
/// from a drop while unwinding, so it degrades to "not removed" rather than
/// panicking. A `ShortcutEntry` holds no user code, so dropping it under the
/// borrow is safe — unlike a callback.
fn remove_shortcut(serial: u64) {
    let _ = MENU_SHORTCUTS.try_with(|shortcuts| {
        if let Ok(mut shortcuts) = shortcuts.try_borrow_mut() {
            shortcuts.retain(|entry| entry.serial != serial);
        }
    });
}

/// Invoke `cb` on behalf of the scope that created it, returning whether it ran.
///
/// The one place the lifetime rule is applied, so every path that can fire a
/// menu item obeys it: the registry ([`dispatch_menu_event`]) *and* the Linux
/// in-app menu bar, which renders items straight from the [`Menu`] and holds the
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
        Some(owner) => {
            owner.run(|| cb());
            true
        }
        None => {
            unowned(|| cb());
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
/// is pruned. Normally [`install_scoped_entry`]'s cleanup has already removed it
/// by then; this covers the case where it could not (its `try_borrow_mut`
/// degraded) and the case where the *building* scope outlives the *creating*
/// one.
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
pub(crate) fn match_shortcut(ctrl: bool, meta: bool, alt: bool, shift: bool, key: KeyCode) -> bool {
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
                    && entry.shortcut.key == key
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
/// [`on_cleanup`] does not close this on its own. It reclaims an id when the
/// *scope that built the menu* is disposed, and the in-tree menus are built from
/// `main`, before the event loop, with no scope at all. Something has to own the
/// build; this is that thing. Whoever holds a build's token — the
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
        // `try_with`/`try_borrow_mut` for the same reason as `remove_shortcut`.
        // The removed callbacks stay in `self.callbacks` and are dropped when
        // that `Vec` is, after this function returns — they are user code whose
        // `Drop` may re-enter the registry.
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

// ── Build functions (Menu → muda types) ─────────────────────────────────────

/// Build a native menu bar from a list of `(label, Menu)` pairs.
///
/// Each pair becomes a top-level submenu in the menu bar. Callbacks are
/// registered in the thread-local registry, and the ids are recorded in
/// [`MENU_BAR_REGISTRATION`] — so building a new bar releases the previous
/// build's, instead of leaving it in the registry forever.
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
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "macos")]
pub(crate) fn attach_menu_to_window(menu: &muda::Menu, _window: &winit::window::Window) {
    menu.init_for_nsapp();
}

/// Attach a native menu bar to a window (Linux — not yet supported).
#[cfg(target_os = "linux")]
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
    pub key: KeyCode,
}

/// Parse a shortcut string like "Cmd+N" or "Ctrl+Shift+S" into a muda Accelerator.
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

    let key = match key_str.to_uppercase().as_str() {
        "A" => KeyCode::KeyA,
        "B" => KeyCode::KeyB,
        "C" => KeyCode::KeyC,
        "D" => KeyCode::KeyD,
        "E" => KeyCode::KeyE,
        "F" => KeyCode::KeyF,
        "G" => KeyCode::KeyG,
        "H" => KeyCode::KeyH,
        "I" => KeyCode::KeyI,
        "J" => KeyCode::KeyJ,
        "K" => KeyCode::KeyK,
        "L" => KeyCode::KeyL,
        "M" => KeyCode::KeyM,
        "N" => KeyCode::KeyN,
        "O" => KeyCode::KeyO,
        "P" => KeyCode::KeyP,
        "Q" => KeyCode::KeyQ,
        "R" => KeyCode::KeyR,
        "S" => KeyCode::KeyS,
        "T" => KeyCode::KeyT,
        "U" => KeyCode::KeyU,
        "V" => KeyCode::KeyV,
        "W" => KeyCode::KeyW,
        "X" => KeyCode::KeyX,
        "Y" => KeyCode::KeyY,
        "Z" => KeyCode::KeyZ,
        "0" => KeyCode::Digit0,
        "1" => KeyCode::Digit1,
        "2" => KeyCode::Digit2,
        "3" => KeyCode::Digit3,
        "4" => KeyCode::Digit4,
        "5" => KeyCode::Digit5,
        "6" => KeyCode::Digit6,
        "7" => KeyCode::Digit7,
        "8" => KeyCode::Digit8,
        "9" => KeyCode::Digit9,
        "=" | "EQUAL" | "PLUS" => KeyCode::Equal,
        "-" | "MINUS" => KeyCode::Minus,
        "F1" => KeyCode::F1,
        "F2" => KeyCode::F2,
        "F3" => KeyCode::F3,
        "F4" => KeyCode::F4,
        "F5" => KeyCode::F5,
        "F6" => KeyCode::F6,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "F9" => KeyCode::F9,
        "F10" => KeyCode::F10,
        "F11" => KeyCode::F11,
        "F12" => KeyCode::F12,
        "ENTER" | "RETURN" => KeyCode::Enter,
        "ESCAPE" | "ESC" => KeyCode::Escape,
        "BACKSPACE" => KeyCode::Backspace,
        "TAB" => KeyCode::Tab,
        "SPACE" => KeyCode::Space,
        "DELETE" | "DEL" => KeyCode::Delete,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "PAGEUP" => KeyCode::PageUp,
        "PAGEDOWN" => KeyCode::PageDown,
        "UP" | "ARROWUP" => KeyCode::ArrowUp,
        "DOWN" | "ARROWDOWN" => KeyCode::ArrowDown,
        "LEFT" | "ARROWLEFT" => KeyCode::ArrowLeft,
        "RIGHT" | "ARROWRIGHT" => KeyCode::ArrowRight,
        _ => return None,
    };

    Some(ParsedShortcut {
        ctrl_or_cmd,
        alt,
        shift,
        key,
    })
}

// ── Test-support accessors ──────────────────────────────────────────────────

/// How many callbacks the registry currently holds.
///
/// The leak this module fixes is invisible to a behavioural assertion — a stale
/// callback is inert, just never removed — so the tests below assert on the size
/// of the registry directly.
#[cfg(test)]
fn callback_count() -> usize {
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

    /// The lifetime half: a menu built during a render goes away with the
    /// component that built it — and the entry is *removed*, not just inert.
    #[test]
    fn a_menu_callback_registered_in_a_scope_is_removed_when_the_scope_disposes() {
        let base = callback_count();
        let (fired, cb) = probe();

        let scope = Scope::new();
        scope.run(|| register_callback("scoped-1", cb));
        assert_eq!(callback_count(), base + 1);

        scope.dispose();
        assert_eq!(
            callback_count(),
            base,
            "the entry must be gone, not merely inert"
        );

        dispatch_menu_event("scoped-1");
        assert_eq!(
            fired.get(),
            0,
            "a disposed component's callback must not run"
        );
    }

    /// The contract every in-tree menu depends on: `run_with_menu` builds the
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

        assert!(match_shortcut(true, false, false, true, KeyCode::KeyJ));
    }

    /// `MENU_SHORTCUTS` leaks identically to `MENU_CALLBACKS` — the issue names
    /// only the latter. A chord left behind keeps matching, and `match_shortcut`
    /// returning `true` swallows the key into a callback that is no longer
    /// there; the list also has to shrink, or matching goes linear in menus ever
    /// built.
    #[test]
    fn a_shortcut_registered_in_a_scope_stops_matching_when_the_scope_disposes() {
        let base = shortcut_count();
        let scope = Scope::new();
        scope.run(|| {
            register_shortcut("Ctrl+Alt+Y", "sc-1");
        });
        assert_eq!(shortcut_count(), base + 1);

        scope.dispose();
        assert_eq!(
            shortcut_count(),
            base,
            "shortcut matching must not go O(menus ever built)"
        );
        assert!(
            !match_shortcut(true, false, true, false, KeyCode::KeyY),
            "a disposed component's chord must fall through"
        );
    }

    /// `match_shortcut`'s answer is what makes the runtime swallow the key, so
    /// it must mean "a callback ran". A chord whose id has no callback — an item
    /// given a `shortcut` but no `on_click` — used to eat that key combination
    /// for the life of the app.
    #[test]
    fn a_chord_that_fires_nothing_falls_through_instead_of_swallowing_the_key() {
        register_shortcut("Ctrl+Alt+U", "no-callback-here");

        assert!(
            !match_shortcut(true, false, true, false, KeyCode::KeyU),
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

        assert!(match_shortcut(true, false, true, false, KeyCode::KeyI));
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
