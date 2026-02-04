//! System tray icon support.
//!
//! This module provides system tray functionality using the `tray-icon` crate.
//!
//! # Example
//!
//! ```ignore
//! use rinch::tray::{TrayIconBuilder, TrayMenu, TrayMenuItem};
//!
//! // Create a tray menu
//! let menu = TrayMenu::new()
//!     .add_item(TrayMenuItem::new("Show Window"))
//!     .add_separator()
//!     .add_item(TrayMenuItem::new("Settings"))
//!     .add_separator()
//!     .add_item(TrayMenuItem::new("Quit"));
//!
//! // Create the tray icon
//! let tray = TrayIconBuilder::new()
//!     .with_tooltip("My App")
//!     .with_menu(menu)
//!     .build()
//!     .unwrap();
//! ```

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon as TrayIconInner, TrayIconBuilder as TrayIconBuilderInner};

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

impl From<tray_icon::Error> for TrayError {
    fn from(err: tray_icon::Error) -> Self {
        TrayError::CreateFailed(err.to_string())
    }
}

impl From<tray_icon::menu::Error> for TrayError {
    fn from(err: tray_icon::menu::Error) -> Self {
        TrayError::MenuError(err.to_string())
    }
}

impl From<tray_icon::BadIcon> for TrayError {
    fn from(err: tray_icon::BadIcon) -> Self {
        TrayError::IconLoadFailed(err.to_string())
    }
}

/// Result type for tray operations.
pub type TrayResult<T> = Result<T, TrayError>;

/// A system tray icon with optional menu.
pub struct TrayIcon {
    _inner: TrayIconInner,
    menu_items: Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>,
}

/// Callback type for tray menu items.
pub type TrayCallback = Box<dyn Fn() + Send + Sync>;

impl TrayIcon {
    /// Poll for tray menu events and invoke callbacks.
    ///
    /// Call this periodically (e.g., in your event loop's about_to_wait).
    pub fn poll_events(&self) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            for (id, label, callback) in &self.menu_items {
                if event.id() == id {
                    tracing::info!("Tray menu item activated: {}", label);
                    if let Some(cb) = callback {
                        cb();
                    }
                }
            }
        }
    }
}

/// Builder for creating a system tray icon.
pub struct TrayIconBuilder {
    tooltip: Option<String>,
    icon: Option<Icon>,
    menu: Option<TrayMenu>,
}

impl TrayIconBuilder {
    /// Create a new tray icon builder.
    pub fn new() -> Self {
        Self {
            tooltip: None,
            icon: None,
            menu: None,
        }
    }

    /// Set the tooltip text shown on hover.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set the tray icon from RGBA pixel data.
    ///
    /// # Arguments
    ///
    /// * `rgba` - RGBA pixel data (4 bytes per pixel)
    /// * `width` - Icon width in pixels
    /// * `height` - Icon height in pixels
    pub fn with_icon_rgba(mut self, rgba: Vec<u8>, width: u32, height: u32) -> TrayResult<Self> {
        self.icon = Some(Icon::from_rgba(rgba, width, height)?);
        Ok(self)
    }

    /// Set the tray icon from an image file (PNG, ICO, etc.).
    ///
    /// The size parameter is optional - if not specified, the image's native size is used.
    pub fn with_icon_path(
        mut self,
        path: impl AsRef<std::path::Path>,
        size: Option<(u32, u32)>,
    ) -> TrayResult<Self> {
        self.icon = Some(Icon::from_path(path, size)?);
        Ok(self)
    }

    /// Set the tray menu.
    pub fn with_menu(mut self, menu: TrayMenu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Build the tray icon.
    pub fn build(self) -> TrayResult<TrayIcon> {
        let mut builder = TrayIconBuilderInner::new();

        if let Some(tooltip) = self.tooltip {
            builder = builder.with_tooltip(tooltip);
        }

        if let Some(icon) = self.icon {
            builder = builder.with_icon(icon);
        }

        let menu_items = if let Some(tray_menu) = self.menu {
            let (menu, items) = tray_menu.build()?;
            builder = builder.with_menu(Box::new(menu));
            items
        } else {
            Vec::new()
        };

        let inner = builder.build()?;

        Ok(TrayIcon {
            _inner: inner,
            menu_items,
        })
    }
}

impl Default for TrayIconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A menu for the system tray.
pub struct TrayMenu {
    items: Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>,
    menu_items: Vec<TrayMenuEntry>,
}

enum TrayMenuEntry {
    Item {
        label: String,
        enabled: bool,
        callback: Option<TrayCallback>,
    },
    Separator,
    Submenu {
        label: String,
        menu: TrayMenu,
    },
}

impl TrayMenu {
    /// Create a new tray menu.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            menu_items: Vec::new(),
        }
    }

    /// Add a menu item.
    pub fn add_item(mut self, item: TrayMenuItem) -> Self {
        self.menu_items.push(TrayMenuEntry::Item {
            label: item.label,
            enabled: item.enabled,
            callback: item.callback,
        });
        self
    }

    /// Add a separator.
    pub fn add_separator(mut self) -> Self {
        self.menu_items.push(TrayMenuEntry::Separator);
        self
    }

    /// Add a submenu.
    pub fn add_submenu(mut self, label: impl Into<String>, submenu: TrayMenu) -> Self {
        self.menu_items.push(TrayMenuEntry::Submenu {
            label: label.into(),
            menu: submenu,
        });
        self
    }

    fn build(
        mut self,
    ) -> TrayResult<(
        Menu,
        Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>,
    )> {
        let menu = Menu::new();

        for entry in self.menu_items {
            match entry {
                TrayMenuEntry::Item {
                    label,
                    enabled,
                    callback,
                } => {
                    let item = MenuItem::new(&label, enabled, None);
                    self.items.push((item.id().clone(), label, callback));
                    menu.append(&item)?;
                }
                TrayMenuEntry::Separator => {
                    menu.append(&PredefinedMenuItem::separator())?;
                }
                TrayMenuEntry::Submenu { label, menu: sub } => {
                    let submenu = Submenu::new(&label, true);
                    // Recursively build the submenu
                    let sub_items = sub.build_into(&submenu)?;
                    // Add submenu items to our tracking list
                    self.items.extend(sub_items);
                    menu.append(&submenu)?;
                }
            }
        }

        Ok((menu, self.items))
    }

    /// Build menu items directly into a submenu.
    fn build_into(
        mut self,
        submenu: &Submenu,
    ) -> TrayResult<Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>> {
        for entry in self.menu_items {
            match entry {
                TrayMenuEntry::Item {
                    label,
                    enabled,
                    callback,
                } => {
                    let item = MenuItem::new(&label, enabled, None);
                    self.items.push((item.id().clone(), label, callback));
                    submenu.append(&item)?;
                }
                TrayMenuEntry::Separator => {
                    submenu.append(&PredefinedMenuItem::separator())?;
                }
                TrayMenuEntry::Submenu { label, menu: sub } => {
                    let nested = Submenu::new(&label, true);
                    let sub_items = sub.build_into(&nested)?;
                    self.items.extend(sub_items);
                    submenu.append(&nested)?;
                }
            }
        }

        Ok(self.items)
    }
}

impl Default for TrayMenu {
    fn default() -> Self {
        Self::new()
    }
}

/// A menu item for the system tray menu.
pub struct TrayMenuItem {
    label: String,
    enabled: bool,
    callback: Option<TrayCallback>,
}

impl TrayMenuItem {
    /// Create a new menu item with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            callback: None,
        }
    }

    /// Set whether the item is enabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the callback to invoke when the item is clicked.
    pub fn on_click<F: Fn() + Send + Sync + 'static>(mut self, callback: F) -> Self {
        self.callback = Some(Box::new(callback));
        self
    }
}
