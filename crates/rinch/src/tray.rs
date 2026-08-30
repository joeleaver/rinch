//! System tray icon support.
//!
//! On Linux, uses `ksni` (StatusNotifierItem D-Bus protocol) for proper
//! left-click → show window, right-click → context menu behavior.
//! On other platforms, uses the `tray-icon` crate with push-based event
//! delivery (no polling thread).
//!
//! Uses the unified [`Menu`](crate::menu::Menu) / [`MenuItem`](crate::menu::MenuItem)
//! types shared with native window menus. Menu callbacks are dispatched to the
//! main thread via the global muda event handler.
//!
//! # Example
//!
//! ```ignore
//! use rinch::tray::TrayIconBuilder;
//! use rinch::menu::{Menu, MenuItem};
//!
//! let menu = Menu::new()
//!     .item(MenuItem::new("Show Window").on_click(show_current_window))
//!     .separator()
//!     .item(MenuItem::new("Quit").on_click(close_current_window));
//!
//! let _tray = TrayIconBuilder::new()
//!     .with_tooltip("My App")
//!     .with_icon_png(include_bytes!("icon.png"))?
//!     .with_menu(menu)
//!     .build()?;
//! ```

use crate::menu::Menu;

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
/// On non-Linux: the tray icon is created on the main thread. Menu events
/// are handled by the global push-based handler — no polling thread.
///
/// On Linux: a background thread runs the ksni D-Bus event loop.
///
/// Dropping this struct removes the tray icon, and releases the menu callbacks
/// it registered — so replacing a tray reclaims the previous one's ids rather
/// than leaving them in the registry forever (issue #183).
///
/// Keep the handle for as long as you want the tray. On Linux the icon itself
/// currently outlives a dropped handle (the ksni thread is detached and parks
/// forever), but its menu items no longer do anything once the callbacks are
/// released — the icon lingering there is the pre-existing bug, not the release.
pub struct TrayIcon {
    /// On Linux: background thread running ksni.
    /// On non-Linux: None (tray icon lives on main thread).
    _thread: Option<std::thread::JoinHandle<()>>,
    /// On non-Linux: the tray-icon handle (must be kept alive).
    #[cfg(not(target_os = "linux"))]
    _tray: Option<tray_icon::TrayIcon>,
    /// The menu callbacks this tray registered.
    ///
    /// Dropping the tray releases them (issue #183). Without this the registry
    /// only grew: a tray rebuilt at runtime left its whole previous menu behind,
    /// and on Linux it could not even overwrite, because each build mints fresh
    /// `ksni-{N}` ids from a monotonic counter.
    _menu: Option<crate::menu::MenuRegistration>,
}

/// Builder for creating a system tray icon.
pub struct TrayIconBuilder {
    tooltip: Option<String>,
    icon_data: Option<(Vec<u8>, u32, u32)>,
    menu: Option<Menu>,
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
        let (rgba, width, height) = crate::shell::rinch_runtime::decode_png_to_rgba(png_data)
            .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.icon_data = Some((rgba, width, height));
        Ok(self)
    }

    /// Set the tray icon from a PNG file path.
    pub fn with_icon_path(self, path: impl AsRef<std::path::Path>) -> TrayResult<Self> {
        let data =
            std::fs::read(path.as_ref()).map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
        self.with_icon_png(&data)
    }

    /// Set the tray context menu using the unified [`Menu`] type.
    pub fn with_menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Build the tray icon.
    ///
    /// On Linux, spawns a background thread for the ksni D-Bus event loop.
    /// On other platforms, creates the tray icon on the main thread with
    /// push-based event delivery.
    pub fn build(self) -> TrayResult<TrayIcon> {
        #[cfg(target_os = "linux")]
        return self.build_ksni();

        #[cfg(not(target_os = "linux"))]
        return self.build_tray_icon();
    }

    /// Non-Linux: create tray icon on main thread with push-based events.
    #[cfg(not(target_os = "linux"))]
    fn build_tray_icon(self) -> TrayResult<TrayIcon> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder as TrayIconBuilderInner,
            TrayIconEvent,
        };

        let mut builder = TrayIconBuilderInner::new();

        if let Some(tip) = self.tooltip {
            builder = builder.with_tooltip(tip);
        }

        if let Some((rgba, w, h)) = self.icon_data {
            let icon = Icon::from_rgba(rgba, w, h)
                .map_err(|e| TrayError::IconLoadFailed(e.to_string()))?;
            builder = builder.with_icon(icon);
        }

        // Convert unified Menu → muda Menu (registers callbacks in thread-local registry)
        let mut registration = None;
        if let Some(menu) = self.menu {
            let (muda_menu, reg) = crate::menu::build_muda_menu(menu);
            registration = Some(reg);
            builder = builder.with_menu(Box::new(muda_menu));
        }

        // Left-click shows the window (not the menu)
        builder = builder.with_menu_on_left_click(false);

        let tray = builder
            .build()
            .map_err(|e| TrayError::CreateFailed(e.to_string()))?;

        // Push-based tray icon event handling for left-click → show window.
        // Menu events are already handled by the global MenuEvent::set_event_handler
        // installed in the runtime (covers both native and tray menus).
        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
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
        }));

        Ok(TrayIcon {
            _thread: None,
            _tray: Some(tray),
            _menu: registration,
        })
    }

    /// Linux: use ksni (StatusNotifierItem) with a background thread.
    #[cfg(target_os = "linux")]
    fn build_ksni(self) -> TrayResult<TrayIcon> {
        use ksni::blocking::TrayMethods;

        let tooltip = self.tooltip.unwrap_or_default();
        let icon_data = self.icon_data;

        // Convert RGBA → ARGB32 (network byte order) for ksni.
        let icon_pixmap = if let Some((rgba, width, height)) = icon_data {
            let mut argb = Vec::with_capacity(rgba.len());
            for pixel in rgba.as_chunks::<4>().0 {
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

        // Register callbacks on the main thread and collect entries for ksni.
        let (menu_entries, registration) = if let Some(menu) = self.menu {
            let (entries, reg) = convert_menu_to_ksni_entries(menu);
            (entries, Some(reg))
        } else {
            (Vec::new(), None)
        };

        let tray = RinchKsniTray {
            tooltip,
            icon_pixmap,
            menu_entries,
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
            .map_err(|e| TrayError::CreateFailed(format!("failed to spawn tray thread: {}", e)))?;

        rx.recv()
            .map_err(|e| TrayError::CreateFailed(format!("tray thread died: {}", e)))??;

        Ok(TrayIcon {
            _thread: Some(thread),
            _menu: registration,
        })
    }
}

impl Default for TrayIconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── ksni implementation (Linux) ─────────────────────────────────────────────

/// Stored menu entry for ksni with string ID for callback dispatch.
enum KsniMenuEntry {
    Item {
        label: String,
        enabled: bool,
        /// Menu ID string for dispatching via the thread-local callback registry.
        menu_id: Option<String>,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<KsniMenuEntry>,
    },
}

/// Counter for generating unique menu IDs on Linux (ksni doesn't use muda).
static KSNI_MENU_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Convert a unified Menu into ksni entries, registering callbacks on the main thread.
///
/// Returns the [`MenuRegistration`](crate::menu::MenuRegistration) holding this
/// build's ids. Every build mints fresh ids from [`KSNI_MENU_COUNTER`], so a
/// rebuilt tray can never overwrite its own previous entries — without the token
/// the registry would grow by the item count on every rebuild, forever.
fn convert_menu_to_ksni_entries(menu: Menu) -> (Vec<KsniMenuEntry>, crate::menu::MenuRegistration) {
    let mut registration = crate::menu::MenuRegistration::default();
    let entries = convert_menu_entries_inner(menu, &mut registration);
    (entries, registration)
}

fn convert_menu_entries_inner(
    menu: Menu,
    registration: &mut crate::menu::MenuRegistration,
) -> Vec<KsniMenuEntry> {
    menu.take_entries()
        .into_iter()
        .map(|entry| match entry {
            crate::menu::MenuEntryKind::Item {
                label,
                enabled,
                callback,
                callback_owner,
            } => {
                let menu_id = callback.map(|cb| {
                    let id = format!(
                        "ksni-{}",
                        KSNI_MENU_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    );
                    registration.register_callback(&id, cb, callback_owner);
                    id
                });
                KsniMenuEntry::Item {
                    label,
                    enabled,
                    menu_id,
                }
            }
            crate::menu::MenuEntryKind::Separator => KsniMenuEntry::Separator,
            crate::menu::MenuEntryKind::Submenu { label, menu } => KsniMenuEntry::Submenu {
                label,
                entries: convert_menu_entries_inner(menu, registration),
            },
        })
        .collect()
}

#[cfg(target_os = "linux")]
struct RinchKsniTray {
    tooltip: String,
    icon_pixmap: Vec<ksni::Icon>,
    menu_entries: Vec<KsniMenuEntry>,
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

#[cfg(target_os = "linux")]
fn build_ksni_menu(entries: &[KsniMenuEntry]) -> Vec<ksni::MenuItem<RinchKsniTray>> {
    entries
        .iter()
        .map(|entry| match entry {
            KsniMenuEntry::Item {
                label,
                enabled,
                menu_id,
            } => {
                let id = menu_id.clone();
                ksni::MenuItem::Standard(ksni::menu::StandardItem {
                    label: label.clone(),
                    enabled: *enabled,
                    activate: Box::new(move |_this: &mut RinchKsniTray| {
                        if let Some(id) = &id {
                            let id = id.clone();
                            crate::shell::rinch_runtime::run_on_main_thread(move || {
                                crate::menu::dispatch_menu_event(&id);
                            });
                        }
                    }),
                    ..Default::default()
                })
            }
            KsniMenuEntry::Separator => ksni::MenuItem::Separator,
            KsniMenuEntry::Submenu { label, entries } => {
                ksni::MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: label.clone(),
                    submenu: build_ksni_menu(entries),
                    ..Default::default()
                })
            }
        })
        .collect()
}
