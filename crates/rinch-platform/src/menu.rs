//! Platform menu abstraction.
//!
//! Native menus are a desktop-only concept. On web and mobile, this
//! trait can be implemented as a no-op, or applications can use
//! rinch widgets to render custom menus.

use crate::{KeyCode, Modifiers};

/// Abstraction over the platform menu system.
///
/// Desktop: wraps `muda` for native menus.
/// Web/Mobile: typically a no-op ([`NoopMenu`]).
pub trait PlatformMenu {
    /// Build menus from a specification.
    fn build(&mut self, submenus: Vec<(MenuSpec, Vec<MenuEntrySpec>)>);

    /// Attach menus to a window.
    ///
    /// On macOS this sets the app menu; on Windows/Linux it attaches to the window.
    /// On web/mobile this is a no-op.
    fn attach_to_window(&self, window: &dyn super::PlatformWindow);

    /// Poll for menu activation events.
    ///
    /// Returns callback indices for any menu items that were activated.
    fn poll_events(&self) -> Vec<usize>;

    /// Check if a keyboard shortcut matches a registered menu shortcut.
    ///
    /// Returns the callback index if matched.
    fn match_shortcut(&self, modifiers: Modifiers, key: KeyCode) -> Option<usize>;
}

/// Specification for a top-level menu or submenu.
#[derive(Debug, Clone)]
pub struct MenuSpec {
    pub label: String,
}

/// Specification for a menu entry.
#[derive(Debug, Clone)]
pub enum MenuEntrySpec {
    /// A clickable menu item.
    Item {
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        callback_index: usize,
    },
    /// A separator line.
    Separator,
    /// A nested submenu.
    Submenu(MenuSpec, Vec<MenuEntrySpec>),
}

/// A no-op menu implementation for platforms without native menus.
#[derive(Debug, Default)]
pub struct NoopMenu;

impl PlatformMenu for NoopMenu {
    fn build(&mut self, _submenus: Vec<(MenuSpec, Vec<MenuEntrySpec>)>) {}

    fn attach_to_window(&self, _window: &dyn super::PlatformWindow) {}

    fn poll_events(&self) -> Vec<usize> {
        Vec::new()
    }

    fn match_shortcut(&self, _modifiers: Modifiers, _key: KeyCode) -> Option<usize> {
        None
    }
}
