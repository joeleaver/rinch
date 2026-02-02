//! Menu module - native menu support via muda.
//!
//! NOTE: This module provides menu infrastructure but is not yet wired up
//! to the fine-grained runtime. Menu support will be added via a builder API.

use muda::{
    accelerator::Accelerator, Menu, MenuEvent, MenuEventReceiver, MenuItem, PredefinedMenuItem,
    Submenu,
};
use rinch_core::element::{MenuItemCallback, MenuItemProps, MenuProps};
use std::collections::HashMap;
use std::str::FromStr;
use winit::keyboard::KeyCode;

/// Manages native menus for the application.
pub struct MenuManager {
    /// The root menu.
    menu: Option<Menu>,
    /// Map from menu item IDs to callback indices.
    item_callbacks: HashMap<muda::MenuId, usize>,
    /// Stored callbacks (indices into this vec).
    callbacks: Vec<MenuCallback>,
    /// Keyboard shortcuts mapped to menu item IDs for manual matching.
    shortcuts: Vec<(ParsedShortcut, muda::MenuId)>,
}

/// A parsed keyboard shortcut for matching against keyboard events.
#[derive(Debug, Clone)]
pub struct ParsedShortcut {
    pub ctrl_or_cmd: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: KeyCode,
}

/// Stores menu item information and callback.
pub struct MenuCallback {
    pub label: String,
    /// The callback to invoke when this menu item is activated.
    pub callback: Option<MenuItemCallback>,
}

/// Specification for a menu item.
pub enum MenuEntry {
    /// A clickable menu item.
    Item(MenuItemProps),
    /// A separator line.
    Separator,
    /// A submenu with nested entries.
    Submenu(MenuProps, Vec<MenuEntry>),
}

impl MenuManager {
    pub fn new() -> Self {
        Self {
            menu: None,
            item_callbacks: HashMap::new(),
            callbacks: Vec::new(),
            shortcuts: Vec::new(),
        }
    }

    /// Build native menu from a list of top-level submenus.
    pub fn build(&mut self, submenus: Vec<(MenuProps, Vec<MenuEntry>)>) -> Option<&Menu> {
        let menu = Menu::new();

        for (props, entries) in submenus {
            let submenu = self.build_submenu(&props, &entries);
            let _ = menu.append(&submenu);
        }

        self.menu = Some(menu);
        self.menu.as_ref()
    }

    /// Build a Submenu from props and entries.
    fn build_submenu(&mut self, props: &MenuProps, entries: &[MenuEntry]) -> Submenu {
        let submenu = Submenu::new(&props.label, true);

        for entry in entries {
            match entry {
                MenuEntry::Item(item_props) => {
                    let menu_item = self.build_menu_item(item_props);
                    let _ = submenu.append(&menu_item);
                }
                MenuEntry::Separator => {
                    let _ = submenu.append(&PredefinedMenuItem::separator());
                }
                MenuEntry::Submenu(nested_props, nested_entries) => {
                    let nested = self.build_submenu(nested_props, nested_entries);
                    let _ = submenu.append(&nested);
                }
            }
        }

        submenu
    }

    /// Build a MenuItem from MenuItemProps.
    fn build_menu_item(&mut self, props: &MenuItemProps) -> MenuItem {
        // Parse accelerator from shortcut string
        let accelerator = props
            .shortcut
            .as_ref()
            .and_then(|s| parse_shortcut(s));

        let item = MenuItem::new(&props.label, props.enabled, accelerator);

        // Store callback mapping
        let callback_idx = self.callbacks.len();
        self.callbacks.push(MenuCallback {
            label: props.label.clone(),
            callback: props.onclick.clone(),
        });
        self.item_callbacks.insert(item.id().clone(), callback_idx);

        // Store keyboard shortcut for manual matching
        if let Some(shortcut_str) = &props.shortcut {
            if let Some(parsed) = parse_shortcut_for_matching(shortcut_str) {
                self.shortcuts.push((parsed, item.id().clone()));
            }
        }

        item
    }

    /// Get the menu for platform initialization.
    pub fn menu(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }

    /// Initialize menu for a window (Windows/Linux).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    pub fn init_for_window(&self, window: &winit::window::Window) {
        use winit::raw_window_handle::HasWindowHandle;

        if let Some(menu) = &self.menu {
            #[cfg(target_os = "windows")]
            {
                if let Ok(handle) = window.window_handle() {
                    if let winit::raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() {
                        let hwnd = win32.hwnd.get() as isize;
                        let _ = menu.init_for_hwnd(hwnd);
                    }
                }
            }

            #[cfg(target_os = "linux")]
            {
                // Linux requires GTK integration - for now skip
                // TODO: Implement GTK menu integration
            }
        }
    }

    /// Initialize menu for the application (macOS).
    #[cfg(target_os = "macos")]
    pub fn init_for_app(&self) {
        if let Some(menu) = &self.menu {
            menu.init_for_nsapp();
        }
    }

    /// Handle a menu event, invoking the callback if one exists.
    ///
    /// Returns `true` if a callback was invoked (indicating state may have changed
    /// and a re-render may be needed), `false` otherwise.
    pub fn handle_event(&self, event: &MenuEvent) -> bool {
        if let Some(&callback_idx) = self.item_callbacks.get(event.id()) {
            if let Some(stored) = self.callbacks.get(callback_idx) {
                tracing::info!("Menu item activated: {}", stored.label);
                if let Some(cb) = &stored.callback {
                    cb.invoke();
                    return true;
                }
            }
        }
        false
    }

    /// Get the label of a menu item by event, for logging purposes.
    pub fn get_label(&self, event: &MenuEvent) -> Option<&str> {
        if let Some(&callback_idx) = self.item_callbacks.get(event.id()) {
            if let Some(stored) = self.callbacks.get(callback_idx) {
                return Some(&stored.label);
            }
        }
        None
    }

    /// Get the menu event receiver for polling.
    pub fn event_receiver() -> &'static MenuEventReceiver {
        MenuEvent::receiver()
    }

    /// Check if a keyboard event matches any registered menu shortcut.
    ///
    /// Returns the menu ID if a match is found, allowing the caller to
    /// trigger the appropriate menu event.
    pub fn match_shortcut(
        &self,
        ctrl: bool,
        meta: bool,
        alt: bool,
        shift: bool,
        key: KeyCode,
    ) -> Option<muda::MenuId> {
        let ctrl_or_cmd = ctrl || meta;

        for (shortcut, menu_id) in &self.shortcuts {
            if shortcut.ctrl_or_cmd == ctrl_or_cmd
                && shortcut.alt == alt
                && shortcut.shift == shift
                && shortcut.key == key
            {
                return Some(menu_id.clone());
            }
        }
        None
    }
}

impl Default for MenuManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a shortcut string like "Cmd+N" or "Ctrl+Shift+S" into an Accelerator.
fn parse_shortcut(shortcut: &str) -> Option<Accelerator> {
    // Convert common shortcuts to muda format
    // muda uses: "CmdOrCtrl+N", "Shift+CmdOrCtrl+S", etc.
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
