//! Unified menu system for native window menus and system tray context menus.
//!
//! Provides [`Menu`] and [`MenuItem`] builder types that work with both
//! native menu bars (via `muda`) and tray context menus (via `tray-icon`).
//! Callbacks are `Rc<dyn Fn()>` — no `Send`/`Sync` burden on users. The
//! runtime wires push-based event delivery so callbacks always run on the
//! main thread.
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
use std::cell::RefCell;
use std::collections::HashMap;
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
    pub fn on_click(mut self, cb: impl Fn() + 'static) -> Self {
        self.callback = Some(Rc::new(cb));
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
                },
                MenuEntryInner::Separator => MenuEntryKind::Separator,
                MenuEntryInner::Submenu { label, menu } => MenuEntryKind::Submenu { label, menu },
            })
            .collect()
    }
}

/// Register a callback in the thread-local registry with a given ID.
/// Used by the ksni tray backend on Linux.
#[cfg(feature = "system-tray")]
pub(crate) fn register_callback_public(id: &str, cb: Rc<dyn Fn()>) {
    register_callback(id, cb);
}

// ── Thread-local callback registry ──────────────────────────────────────────

thread_local! {
    /// Map from muda MenuId string → callback. Thread-local because callbacks
    /// capture `Signal` (which is `!Send`) and must run on the main thread.
    static MENU_CALLBACKS: RefCell<HashMap<String, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());

    /// Registered keyboard shortcuts mapped to their menu ID strings.
    static MENU_SHORTCUTS: RefCell<Vec<(ParsedShortcut, String)>> = const { RefCell::new(Vec::new()) };

}

fn register_callback(menu_id: &str, cb: Rc<dyn Fn()>) {
    MENU_CALLBACKS.with(|map| {
        map.borrow_mut().insert(menu_id.to_string(), cb);
    });
}

fn register_shortcut(shortcut_str: &str, menu_id: &str) {
    if let Some(parsed) = parse_shortcut_for_matching(shortcut_str) {
        MENU_SHORTCUTS.with(|shortcuts| {
            shortcuts.borrow_mut().push((parsed, menu_id.to_string()));
        });
    }
}

/// Dispatch a menu event by looking up and invoking the callback.
pub(crate) fn dispatch_menu_event(menu_id: &str) {
    MENU_CALLBACKS.with(|map| {
        if let Some(cb) = map.borrow().get(menu_id) {
            cb();
        }
    });
}

/// Check if a keyboard event matches a registered menu shortcut.
/// If so, dispatch the callback and return `true`.
pub(crate) fn match_shortcut(ctrl: bool, meta: bool, alt: bool, shift: bool, key: KeyCode) -> bool {
    let ctrl_or_cmd = ctrl || meta;

    MENU_SHORTCUTS.with(|shortcuts| {
        let shortcuts = shortcuts.borrow();
        for (shortcut, menu_id) in shortcuts.iter() {
            if shortcut.ctrl_or_cmd == ctrl_or_cmd
                && shortcut.alt == alt
                && shortcut.shift == shift
                && shortcut.key == key
            {
                dispatch_menu_event(menu_id);
                return true;
            }
        }
        false
    })
}

// ── Build functions (Menu → muda types) ─────────────────────────────────────

/// Build a native menu bar from a list of `(label, Menu)` pairs.
///
/// Each pair becomes a top-level submenu in the menu bar. Callbacks are
/// registered in the thread-local registry.
pub(crate) fn build_native_menu_bar(menus: Vec<(&str, Menu)>) -> muda::Menu {
    let menu_bar = muda::Menu::new();
    for (label, menu) in menus {
        let submenu = muda::Submenu::new(label, true);
        build_muda_entries(&submenu, menu);
        let _ = menu_bar.append(&submenu);
    }
    menu_bar
}

/// Build a muda `Menu` from a unified `Menu` (for tray context menus).
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn build_muda_menu(menu: Menu) -> muda::Menu {
    let muda_menu = muda::Menu::new();
    for entry in menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                let muda_item = build_muda_item(&item);
                let _ = muda_menu.append(&muda_item);
            }
            MenuEntryInner::Separator => {
                let _ = muda_menu.append(&muda::PredefinedMenuItem::separator());
            }
            MenuEntryInner::Submenu { label, menu } => {
                let submenu = muda::Submenu::new(&label, true);
                build_muda_entries(&submenu, menu);
                let _ = muda_menu.append(&submenu);
            }
        }
    }
    muda_menu
}

/// Recursively populate a muda Submenu from a unified Menu.
fn build_muda_entries(submenu: &muda::Submenu, menu: Menu) {
    for entry in menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                let muda_item = build_muda_item(&item);
                let _ = submenu.append(&muda_item);
            }
            MenuEntryInner::Separator => {
                let _ = submenu.append(&muda::PredefinedMenuItem::separator());
            }
            MenuEntryInner::Submenu { label, menu } => {
                let nested = muda::Submenu::new(&label, true);
                build_muda_entries(&nested, menu);
                let _ = submenu.append(&nested);
            }
        }
    }
}

/// Build a single muda MenuItem, register its callback and shortcut.
fn build_muda_item(item: &MenuItem) -> muda::MenuItem {
    let accelerator = item.shortcut.as_ref().and_then(|s| parse_shortcut(s));
    let muda_item = muda::MenuItem::new(&item.label, item.enabled, accelerator);

    // Register callback
    if let Some(cb) = &item.callback {
        register_callback(&muda_item.id().0, cb.clone());
    }

    // Register shortcut for keyboard matching
    if let Some(shortcut_str) = &item.shortcut {
        register_shortcut(shortcut_str, &muda_item.id().0);
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
pub(crate) fn attach_menu_to_window(menu: &muda::Menu, window: &winit::window::Window) {
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
pub(crate) fn attach_menu_to_window(_menu: &muda::Menu, _window: &winit::window::Window) {
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
