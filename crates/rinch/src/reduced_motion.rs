//! Reduced motion: the OS's setting, the app's override, and the reactive
//! getter (#2209).
//!
//! `@media (prefers-reduced-motion: reduce)` matches on desktop when the
//! effective preference says so: the app's override
//! ([`App::reduced_motion`](crate::app_builder::App::reduced_motion) or
//! [`set_reduced_motion`]) where it gives one, else the OS's setting, else
//! "no preference". rinch-dom answers the query from it
//! (`rinch_dom::reduced_motion`, which writes the answer into the sheets
//! Stylo's servo build cannot evaluate it in), and every open window adopts a
//! change on its next frame (`App::adopt_reduced_motion`).
//!
//! The OS's setting, as Chromium and GTK read it:
//!
//! - **Linux, KDE** (`XDG_CURRENT_DESKTOP` names KDE): `kdeglobals`'
//!   `[KDE] AnimationDurationFactor`; `0` reduces.
//! - **Linux, elsewhere**: GNOME's `org.gnome.desktop.interface
//!   enable-animations` through `gsettings`; `false` reduces. Without
//!   `gsettings`, GTK 3's `settings.ini` `gtk-enable-animations`.
//! - **Windows**: `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)`; off
//!   reduces.
//! - **macOS**: `NSWorkspace.accessibilityDisplayShouldReduceMotion`.
//!
//! It is read when the app starts and again whenever a window gains focus,
//! which is how a change made in the system settings reaches a running app
//! (the settings window took the focus to make it). On Linux the read runs on
//! a short-lived thread (it may start `gsettings`); elsewhere it is one call.
//!
//! The browser evaluates the query itself, so none of this runs on the web.

use std::sync::atomic::{AtomicU8, Ordering};

use rinch_core::Signal;

const UNSET: u8 = 0;
const NO: u8 = 1;
const YES: u8 = 2;

static OVERRIDE: AtomicU8 = AtomicU8::new(UNSET);
static SYSTEM: AtomicU8 = AtomicU8::new(UNSET);

fn encode(v: Option<bool>) -> u8 {
    match v {
        None => UNSET,
        Some(false) => NO,
        Some(true) => YES,
    }
}

fn decode(v: u8) -> Option<bool> {
    match v {
        NO => Some(false),
        YES => Some(true),
        _ => None,
    }
}

thread_local! {
    /// Bumped on this thread whenever the effective preference may have
    /// changed, so [`prefers_reduced_motion`] can be read reactively.
    static CHANGED: Signal<u64> = Signal::new(0).leak();
}

/// The preference an override and the OS's setting give: the override wins,
/// and neither means no preference.
pub(crate) fn effective_of(override_: Option<bool>, system: Option<bool>) -> bool {
    override_.or(system).unwrap_or(false)
}

/// Whether the app prefers reduced motion now: the override, else the OS's
/// setting, else `false`. Reactive: an effect or a `{|| …}` closure that reads
/// it runs again when it changes.
///
/// ```ignore
/// let duration = || if rinch::prefers_reduced_motion() { 0 } else { 300 };
/// ```
pub fn prefers_reduced_motion() -> bool {
    CHANGED.with(|s| s.get());
    effective_of(reduced_motion_override(), system_reduced_motion())
}

/// Force the reduced-motion preference (`Some`), or follow the OS again
/// (`None`). [`App::reduced_motion`](crate::app_builder::App::reduced_motion)
/// sets it before the first window; calling it later restyles every open
/// window on its next frame.
pub fn set_reduced_motion(value: Option<bool>) {
    OVERRIDE.store(encode(value), Ordering::Relaxed);
    publish();
}

/// The app's override, if it set one.
pub fn reduced_motion_override() -> Option<bool> {
    decode(OVERRIDE.load(Ordering::Relaxed))
}

/// The OS's setting as last read, `None` before the first read and where it
/// cannot be read.
pub fn system_reduced_motion() -> Option<bool> {
    decode(SYSTEM.load(Ordering::Relaxed))
}

/// Hand the effective preference to rinch-dom (which new documents start
/// from, and which every window's frame compares its own document against),
/// and tell the reactive getter, on the main thread, with a wake of the loop.
fn publish() {
    rinch_dom::reduced_motion::set_prefers_reduced_motion(effective_of(
        reduced_motion_override(),
        system_reduced_motion(),
    ));
    rinch_core::dispatch_main_callback(Box::new(|| {
        CHANGED.with(|s| s.update(|n| *n += 1));
    }));
}

#[cfg(feature = "windowing")]
fn store_system(value: Option<bool>) {
    let old = SYSTEM.swap(encode(value), Ordering::Relaxed);
    if old != encode(value) {
        publish();
    }
}

/// Read the OS's setting again, and publish a change. Called when the app
/// starts and when a window gains focus.
#[cfg(feature = "windowing")]
pub(crate) fn refresh_from_os() {
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        use std::sync::atomic::AtomicBool;
        static READING: AtomicBool = AtomicBool::new(false);
        if READING.swap(true, Ordering::AcqRel) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("rinch-reduced-motion".into())
            .spawn(|| {
                store_system(os::read());
                READING.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            READING.store(false, Ordering::Release);
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    store_system(os::read());
}

#[cfg(all(
    feature = "windowing",
    unix,
    not(target_os = "macos"),
    not(target_os = "android")
))]
mod os {
    use std::path::PathBuf;

    fn config_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }

    pub(super) fn read() -> Option<bool> {
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|d| d.split(':').any(|d| d.eq_ignore_ascii_case("KDE")));
        if kde {
            let text = config_dir()
                .and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
                .unwrap_or_default();
            return Some(super::parse::kdeglobals(&text));
        }
        let gsettings = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| super::parse::gsettings(&String::from_utf8_lossy(&o.stdout)));
        gsettings.or_else(|| {
            config_dir()
                .and_then(|d| std::fs::read_to_string(d.join("gtk-3.0/settings.ini")).ok())
                .and_then(|text| super::parse::gtk_settings_ini(&text))
        })
    }
}

#[cfg(all(feature = "windowing", target_os = "windows"))]
mod os {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };

    pub(super) fn read() -> Option<bool> {
        let mut on: i32 = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one `BOOL` to `pvParam`.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut on as *mut i32).cast(),
                0,
            )
        };
        (ok != 0).then_some(on == 0)
    }
}

#[cfg(all(feature = "windowing", target_os = "macos"))]
mod os {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};

    pub(super) fn read() -> Option<bool> {
        // SAFETY: `sharedWorkspace` answers the process's NSWorkspace, and
        // `accessibilityDisplayShouldReduceMotion` is a BOOL getter on it
        // (macOS 10.12+).
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return None;
            }
            let reduce: Bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
            Some(reduce.as_bool())
        }
    }
}

/// The Linux settings files' and `gsettings`' text, read into a preference.
/// Plain functions, so they are tested on every host.
#[cfg_attr(
    not(all(
        feature = "windowing",
        unix,
        not(target_os = "macos"),
        not(target_os = "android")
    )),
    allow(dead_code)
)]
mod parse {
    /// `kdeglobals`: `[KDE] AnimationDurationFactor=0` reduces; absent, KDE
    /// animates (its default factor is 1).
    pub(super) fn kdeglobals(text: &str) -> bool {
        ini_value(text, "KDE", "AnimationDurationFactor")
            .and_then(|v| v.parse::<f64>().ok())
            .is_some_and(|factor| factor == 0.0)
    }

    /// `gsettings get … enable-animations` prints `true` or `false`.
    pub(super) fn gsettings(stdout: &str) -> Option<bool> {
        match stdout.trim() {
            "false" => Some(true),
            "true" => Some(false),
            _ => None,
        }
    }

    /// GTK 3's `settings.ini`: `[Settings] gtk-enable-animations=false` (or
    /// `0`) reduces.
    pub(super) fn gtk_settings_ini(text: &str) -> Option<bool> {
        match ini_value(text, "Settings", "gtk-enable-animations")?
            .to_ascii_lowercase()
            .as_str()
        {
            "false" | "0" => Some(true),
            "true" | "1" => Some(false),
            _ => None,
        }
    }

    /// The value of `key` in `[section]` of an INI-style file.
    fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                in_section = name == section;
            } else if in_section
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_and_neither_means_no_preference() {
        assert!(!effective_of(None, None));
        assert!(effective_of(None, Some(true)));
        assert!(!effective_of(Some(false), Some(true)));
        assert!(effective_of(Some(true), Some(false)));
        assert!(effective_of(Some(true), None));
    }

    #[test]
    fn kdeglobals_reduces_at_factor_zero_only() {
        assert!(parse::kdeglobals("[KDE]\nAnimationDurationFactor=0\n"));
        assert!(parse::kdeglobals(
            "[General]\nx=1\n[KDE]\nAnimationDurationFactor = 0.0"
        ));
        assert!(!parse::kdeglobals("[KDE]\nAnimationDurationFactor=0.5\n"));
        assert!(!parse::kdeglobals("[Other]\nAnimationDurationFactor=0\n"));
        assert!(!parse::kdeglobals(""));
    }

    #[test]
    fn gsettings_false_reduces() {
        assert_eq!(parse::gsettings("false\n"), Some(true));
        assert_eq!(parse::gsettings("true\n"), Some(false));
        assert_eq!(parse::gsettings("No such schema"), None);
    }

    #[test]
    fn gtk_settings_ini_false_reduces() {
        assert_eq!(
            parse::gtk_settings_ini("[Settings]\ngtk-enable-animations=false\n"),
            Some(true)
        );
        assert_eq!(
            parse::gtk_settings_ini("[Settings]\ngtk-enable-animations=1"),
            Some(false)
        );
        assert_eq!(parse::gtk_settings_ini("[Settings]\n"), None);
    }
}
