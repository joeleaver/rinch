//! Where a [`BorderlessWindow`] puts its window
//! controls: the layout type and each platform's convention (issue #2196).

#[cfg(doc)]
use super::BorderlessWindow;

/// One of the window control buttons a [`BorderlessWindow`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowControl {
    /// Minimize (class `rinch-borderlesswindow__control--minimize`).
    Minimize,
    /// Maximize / zoom (class `rinch-borderlesswindow__control--maximize`).
    Maximize,
    /// Close (class `rinch-borderlesswindow__control--close`).
    Close,
}

impl WindowControl {
    /// The name GTK's `gtk-decoration-layout` and GNOME's `button-layout` use.
    fn from_gtk(token: &str) -> Option<Self> {
        match token.trim() {
            "minimize" => Some(WindowControl::Minimize),
            "maximize" => Some(WindowControl::Maximize),
            "close" => Some(WindowControl::Close),
            _ => None,
        }
    }
}

/// Which window controls go at each end of a [`BorderlessWindow`]'s titlebar,
/// in order (issue #2196).
///
/// [`BorderlessWindow::controls_layout`] left unset uses
/// [`WindowControlsLayout::platform`], each platform's own convention. Set it
/// to place them yourself, with the constructors here or with GTK's
/// `gtk-decoration-layout` syntax through [`FromStr`](std::str::FromStr):
/// `"close,minimize,maximize:"` is macOS's order on the left,
/// `":minimize,maximize,close"` Windows' order on the right.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowControlsLayout {
    /// Controls at the left end, left to right.
    pub left: Vec<WindowControl>,
    /// Controls at the right end, left to right.
    pub right: Vec<WindowControl>,
}

impl WindowControlsLayout {
    /// Windows' layout: minimize, maximize, close, on the right. Also the
    /// default on a Linux desktop whose layout cannot be read, and on every
    /// other platform without a convention of its own.
    pub fn windows() -> Self {
        Self {
            left: Vec::new(),
            right: vec![
                WindowControl::Minimize,
                WindowControl::Maximize,
                WindowControl::Close,
            ],
        }
    }

    /// macOS's layout: the traffic lights close, minimize, zoom, on the left.
    pub fn macos() -> Self {
        Self {
            left: vec![
                WindowControl::Close,
                WindowControl::Minimize,
                WindowControl::Maximize,
            ],
            right: Vec::new(),
        }
    }

    /// Read a GTK `gtk-decoration-layout` / GNOME `button-layout` value, such
    /// as `"appmenu:minimize,maximize,close"`: the buttons before the colon go
    /// on the left, the ones after it on the right. Names other than
    /// `minimize`, `maximize` and `close` (`appmenu`, `icon`, `menu`,
    /// `spacer`) are skipped, as is surrounding quoting, so `gsettings`
    /// output can be passed as is. A value with no colon puts everything on
    /// the left, as GTK does.
    pub fn from_gtk(value: &str) -> Self {
        let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
        let (left, right) = value.split_once(':').unwrap_or((value, ""));
        let side = |s: &str| s.split(',').filter_map(WindowControl::from_gtk).collect();
        Self {
            left: side(left),
            right: side(right),
        }
    }

    /// This platform's convention, read once per process:
    ///
    /// - **macOS**: [`WindowControlsLayout::macos`].
    /// - **Linux**: the desktop's own setting — GNOME's
    ///   `org.gnome.desktop.wm.preferences button-layout` (through `gsettings`)
    ///   on a GNOME-family desktop, otherwise `gtk-decoration-layout` in
    ///   `$XDG_CONFIG_HOME/gtk-3.0/settings.ini` (where KDE Plasma writes its
    ///   KWin button order for GTK apps) — and [`WindowControlsLayout::windows`]
    ///   when neither can be read. GNOME's stock value is `appmenu:close`, a
    ///   close button alone, and that is what this then gives.
    /// - **Everywhere else** (Windows included): [`WindowControlsLayout::windows`].
    pub fn platform() -> Self {
        static PLATFORM: std::sync::OnceLock<WindowControlsLayout> = std::sync::OnceLock::new();
        PLATFORM.get_or_init(detect_platform_layout).clone()
    }

    /// Linux's rule, from what was read: `desktop` is `$XDG_CURRENT_DESKTOP`,
    /// `gsettings` reads GNOME's `button-layout`, `settings_ini` is the
    /// contents of GTK 3's `settings.ini`. `gsettings` spawns a process, so it
    /// is called only on a GNOME-family desktop, the one place its answer is
    /// the window manager's.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn linux(
        desktop: &str,
        gsettings: impl FnOnce() -> Option<String>,
        settings_ini: Option<&str>,
    ) -> Self {
        let gnome_family = desktop.split(':').any(|d| {
            matches!(
                d.to_ascii_lowercase().as_str(),
                "gnome" | "unity" | "budgie" | "pantheon"
            )
        });
        if gnome_family && let Some(value) = gsettings().filter(|v| !v.trim().is_empty()) {
            return Self::from_gtk(&value);
        }
        let ini_value = settings_ini.and_then(|ini| {
            ini.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "gtk-decoration-layout").then(|| value.trim().to_owned())
            })
        });
        match ini_value {
            Some(value) => Self::from_gtk(&value),
            None => Self::windows(),
        }
    }
}

impl std::str::FromStr for WindowControlsLayout {
    type Err = std::convert::Infallible;

    /// [`WindowControlsLayout::from_gtk`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_gtk(s))
    }
}

#[cfg(target_os = "macos")]
fn detect_platform_layout() -> WindowControlsLayout {
    WindowControlsLayout::macos()
}

#[cfg(target_os = "linux")]
fn detect_platform_layout() -> WindowControlsLayout {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let gsettings = || {
        let out = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    let settings_ini =
        config.and_then(|dir| std::fs::read_to_string(dir.join("gtk-3.0/settings.ini")).ok());
    WindowControlsLayout::linux(&desktop, gsettings, settings_ini.as_deref())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn detect_platform_layout() -> WindowControlsLayout {
    WindowControlsLayout::windows()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux's rule (issue #2196): GNOME's own setting on a GNOME-family
    /// desktop, GTK 3's `settings.ini` otherwise, Windows' order when neither
    /// says anything.
    #[test]
    fn linux_reads_the_desktops_layout() {
        use WindowControl::*;
        let ini = "[Settings]\ngtk-theme-name=Breeze\ngtk-decoration-layout=icon:minimize,maximize,close\n";
        let kde_left = "[Settings]\ngtk-decoration-layout = close,maximize:\n";

        let reads = |value: &'static str| move || Some(value.to_owned());
        let unread = || None;
        let must_not_run = || -> Option<String> { panic!("gsettings spawned off GNOME") };

        // GNOME: gsettings wins over the ini file.
        let l = WindowControlsLayout::linux("ubuntu:GNOME", reads("'close,minimize:'"), Some(ini));
        assert_eq!((l.left, l.right), (vec![Close, Minimize], vec![]));
        // GNOME with nothing from gsettings: the ini file.
        let l = WindowControlsLayout::linux("GNOME", unread, Some(kde_left));
        assert_eq!((l.left, l.right), (vec![Close, Maximize], vec![]));
        // KDE: gsettings is not KWin's setting, the ini file is, and the
        // process is not even spawned.
        let l = WindowControlsLayout::linux("KDE", must_not_run, Some(kde_left));
        assert_eq!((l.left, l.right), (vec![Close, Maximize], vec![]));
        // Nothing readable: Windows' order on the right.
        let l = WindowControlsLayout::linux("sway", must_not_run, Some("[Settings]\n"));
        assert_eq!(l, WindowControlsLayout::windows());
        let l = WindowControlsLayout::linux("", must_not_run, None);
        assert_eq!(l, WindowControlsLayout::windows());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_puts_the_traffic_lights_on_the_left() {
        assert_eq!(
            WindowControlsLayout::platform(),
            WindowControlsLayout::macos()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_puts_the_controls_on_the_right() {
        assert_eq!(
            WindowControlsLayout::platform(),
            WindowControlsLayout::windows()
        );
    }
}
