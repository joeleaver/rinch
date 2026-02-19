//! System tray icon support.
//!
//! On Linux, uses `ksni` (StatusNotifierItem D-Bus protocol) for proper
//! left-click → show window, right-click → context menu behavior.
//! On other platforms, uses the `tray-icon` crate.
//!
//! The tray icon runs on a dedicated background thread, completely
//! independent of the winit window lifecycle. Menu callbacks are
//! dispatched to the main thread via [`run_on_main_thread`].
//!
//! # Example
//!
//! ```ignore
//! use rinch::tray::{TrayIconBuilder, TrayMenu, TrayMenuItem};
//!
//! let menu = TrayMenu::new()
//!     .add_item(TrayMenuItem::new("Show Window").on_click(show_current_window))
//!     .add_separator()
//!     .add_item(TrayMenuItem::new("Quit").on_click(close_current_window));
//!
//! let _tray = TrayIconBuilder::new()
//!     .with_tooltip("My App")
//!     .with_icon_png(include_bytes!("icon.png"))?
//!     .with_menu(menu)
//!     .build()?;
//! ```

use std::sync::Arc;
use std::thread::JoinHandle;

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
/// The tray icon lives on a dedicated background thread. Menu events are
/// automatically dispatched to the main thread. Dropping this struct
/// detaches the thread (the tray stays alive until the process exits).
pub struct TrayIcon {
    _thread: Option<JoinHandle<()>>,
}

/// Callback type for tray menu items.
pub type TrayCallback = Box<dyn Fn() + Send + Sync>;

/// Builder for creating a system tray icon.
pub struct TrayIconBuilder {
    tooltip: Option<String>,
    icon_data: Option<(Vec<u8>, u32, u32)>,
    menu: Option<TrayMenu>,
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
        let (rgba, width, height) =
            crate::shell::rinch_runtime::decode_png_to_rgba(png_data)
                .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.icon_data = Some((rgba, width, height));
        Ok(self)
    }

    /// Set the tray icon from a PNG file path.
    pub fn with_icon_path(self, path: impl AsRef<std::path::Path>) -> TrayResult<Self> {
        let data = std::fs::read(path.as_ref())
            .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.with_icon_png(&data)
    }

    /// Set the tray menu.
    pub fn with_menu(mut self, menu: TrayMenu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Build the tray icon.
    ///
    /// Spawns a dedicated background thread for tray management. On Linux,
    /// uses the StatusNotifierItem D-Bus protocol (via `ksni`) which
    /// supports left-click → show window, right-click → context menu.
    pub fn build(self) -> TrayResult<TrayIcon> {
        #[cfg(target_os = "linux")]
        return self.build_ksni();

        #[cfg(not(target_os = "linux"))]
        return self.build_tray_icon();
    }

    /// Linux: use ksni (StatusNotifierItem) for proper left/right click.
    #[cfg(target_os = "linux")]
    fn build_ksni(self) -> TrayResult<TrayIcon> {
        use ksni::blocking::TrayMethods;

        let tooltip = self.tooltip.unwrap_or_default();
        let icon_data = self.icon_data;
        let menu = self.menu;

        // Convert RGBA → ARGB32 (network byte order) for ksni.
        let icon_pixmap = if let Some((rgba, width, height)) = icon_data {
            let mut argb = Vec::with_capacity(rgba.len());
            for pixel in rgba.chunks_exact(4) {
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

        // Convert our TrayMenu to a storable representation.
        let menu_store = menu
            .map(|m| convert_menu_entries(m.menu_items))
            .unwrap_or_default();

        let tray = RinchKsniTray {
            tooltip,
            icon_pixmap,
            menu_entries: menu_store,
        };

        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), TrayError>>(1);

        let thread = std::thread::Builder::new()
            .name("rinch-tray".into())
            .spawn(move || {
                match tray.spawn() {
                    Ok(_handle) => {
                        let _ = tx.send(Ok(()));
                        // Block forever — ksni runs its own D-Bus event loop.
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(3600));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(TrayError::CreateFailed(e.to_string())));
                    }
                }
            })
            .map_err(|e| {
                TrayError::CreateFailed(format!("failed to spawn tray thread: {}", e))
            })?;

        rx.recv()
            .map_err(|e| TrayError::CreateFailed(format!("tray thread died: {}", e)))??;

        Ok(TrayIcon {
            _thread: Some(thread),
        })
    }

    /// Non-Linux: use tray-icon crate with a background polling thread.
    #[cfg(not(target_os = "linux"))]
    fn build_tray_icon(self) -> TrayResult<TrayIcon> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder as TrayIconBuilderInner,
            TrayIconEvent,
        };

        let tooltip = self.tooltip;
        let icon_data = self.icon_data;
        let menu = self.menu;

        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), TrayError>>(1);

        let thread = std::thread::Builder::new()
            .name("rinch-tray".into())
            .spawn(move || {
                let mut builder = TrayIconBuilderInner::new();

                if let Some(tip) = tooltip {
                    builder = builder.with_tooltip(tip);
                }

                if let Some((rgba, w, h)) = icon_data {
                    match Icon::from_rgba(rgba, w, h) {
                        Ok(icon) => builder = builder.with_icon(icon),
                        Err(e) => {
                            let _ = tx.send(Err(TrayError::IconLoadFailed(e.to_string())));
                            return;
                        }
                    }
                }

                let callbacks = if let Some(tray_menu) = menu {
                    match build_tray_icon_menu(tray_menu) {
                        Ok((menu_obj, items)) => {
                            builder = builder.with_menu(Box::new(menu_obj));
                            items
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    }
                } else {
                    Vec::new()
                };

                // Disable menu on left-click so we can handle it ourselves.
                builder = builder.with_menu_on_left_click(false);

                let _tray = match builder.build() {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(Err(TrayError::CreateFailed(e.to_string())));
                        return;
                    }
                };

                let callbacks: Vec<_> = callbacks
                    .into_iter()
                    .map(|(id, label, cb)| {
                        let cb = cb.map(|b| Arc::from(b) as Arc<dyn Fn() + Send + Sync>);
                        (id, label, cb)
                    })
                    .collect();

                let _ = tx.send(Ok(()));

                loop {
                    while let Ok(event) = MenuEvent::receiver().try_recv() {
                        for (id, label, cb) in &callbacks {
                            if event.id() == id {
                                tracing::info!("Tray menu item activated: {}", label);
                                if let Some(cb) = cb {
                                    let cb = Arc::clone(cb);
                                    crate::shell::rinch_runtime::run_on_main_thread(move || {
                                        cb()
                                    });
                                }
                            }
                        }
                    }

                    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
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
                    }

                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .map_err(|e| {
                TrayError::CreateFailed(format!("failed to spawn tray thread: {}", e))
            })?;

        rx.recv()
            .map_err(|e| TrayError::CreateFailed(format!("tray thread died: {}", e)))??;

        Ok(TrayIcon {
            _thread: Some(thread),
        })
    }
}

impl Default for TrayIconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── ksni implementation (Linux) ─────────────────────────────────────────────

/// Internal tray struct implementing the ksni::Tray trait.
#[cfg(target_os = "linux")]
struct RinchKsniTray {
    tooltip: String,
    icon_pixmap: Vec<ksni::Icon>,
    menu_entries: Vec<MenuEntryStore>,
}

/// Stored menu entry with Arc-wrapped callbacks for repeated invocation.
enum MenuEntryStore {
    Item {
        label: String,
        enabled: bool,
        callback: Option<Arc<dyn Fn() + Send + Sync>>,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<MenuEntryStore>,
    },
}

/// Convert our public TrayMenuEntry list to the internal storage form.
fn convert_menu_entries(entries: Vec<TrayMenuEntry>) -> Vec<MenuEntryStore> {
    entries
        .into_iter()
        .map(|e| match e {
            TrayMenuEntry::Item {
                label,
                enabled,
                callback,
            } => MenuEntryStore::Item {
                label,
                enabled,
                callback: callback.map(|b| Arc::from(b) as Arc<dyn Fn() + Send + Sync>),
            },
            TrayMenuEntry::Separator => MenuEntryStore::Separator,
            TrayMenuEntry::Submenu { label, menu } => MenuEntryStore::Submenu {
                label,
                entries: convert_menu_entries(menu.menu_items),
            },
        })
        .collect()
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

/// Recursively convert stored menu entries to ksni MenuItems.
#[cfg(target_os = "linux")]
fn build_ksni_menu(entries: &[MenuEntryStore]) -> Vec<ksni::MenuItem<RinchKsniTray>> {
    entries
        .iter()
        .map(|entry| match entry {
            MenuEntryStore::Item {
                label,
                enabled,
                callback,
            } => {
                let cb = callback.clone();
                ksni::MenuItem::Standard(ksni::menu::StandardItem {
                    label: label.clone(),
                    enabled: *enabled,
                    activate: Box::new(move |_this: &mut RinchKsniTray| {
                        if let Some(cb) = &cb {
                            let cb = Arc::clone(cb);
                            crate::shell::rinch_runtime::run_on_main_thread(move || cb());
                        }
                    }),
                    ..Default::default()
                })
            }
            MenuEntryStore::Separator => ksni::MenuItem::Separator,
            MenuEntryStore::Submenu { label, entries } => {
                ksni::MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: label.clone(),
                    submenu: build_ksni_menu(entries),
                    ..Default::default()
                })
            }
        })
        .collect()
}

// ── tray-icon helpers (non-Linux) ───────────────────────────────────────────

/// Build a tray-icon Menu from our TrayMenu type.
#[cfg(not(target_os = "linux"))]
fn build_tray_icon_menu(
    tray_menu: TrayMenu,
) -> TrayResult<(
    tray_icon::menu::Menu,
    Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>,
)> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

    let menu = Menu::new();
    let mut items = Vec::new();

    for entry in tray_menu.menu_items {
        match entry {
            TrayMenuEntry::Item {
                label,
                enabled,
                callback,
            } => {
                let item = MenuItem::new(&label, enabled, None);
                items.push((item.id().clone(), label, callback));
                menu.append(&item)
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
            TrayMenuEntry::Separator => {
                menu.append(&PredefinedMenuItem::separator())
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
            TrayMenuEntry::Submenu {
                label,
                menu: sub,
            } => {
                let submenu = Submenu::new(&label, true);
                let sub_items = build_tray_icon_submenu(sub, &submenu)?;
                items.extend(sub_items);
                menu.append(&submenu)
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
        }
    }

    Ok((menu, items))
}

#[cfg(not(target_os = "linux"))]
fn build_tray_icon_submenu(
    tray_menu: TrayMenu,
    submenu: &tray_icon::menu::Submenu,
) -> TrayResult<Vec<(tray_icon::menu::MenuId, String, Option<TrayCallback>)>> {
    use tray_icon::menu::{MenuItem, PredefinedMenuItem, Submenu};

    let mut items = Vec::new();

    for entry in tray_menu.menu_items {
        match entry {
            TrayMenuEntry::Item {
                label,
                enabled,
                callback,
            } => {
                let item = MenuItem::new(&label, enabled, None);
                items.push((item.id().clone(), label, callback));
                submenu
                    .append(&item)
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
            TrayMenuEntry::Separator => {
                submenu
                    .append(&PredefinedMenuItem::separator())
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
            TrayMenuEntry::Submenu {
                label,
                menu: sub,
            } => {
                let nested = Submenu::new(&label, true);
                let sub_items = build_tray_icon_submenu(sub, &nested)?;
                items.extend(sub_items);
                submenu
                    .append(&nested)
                    .map_err(|e| TrayError::MenuError(e.to_string()))?;
            }
        }
    }

    Ok(items)
}

// ── Public menu types (platform-agnostic) ───────────────────────────────────

/// A menu for the system tray.
pub struct TrayMenu {
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
