//! Whether the built-in DevTools keys work (issue #2689): F12 toggles the
//! DevTools window (desktop) and Alt+I toggles inspect mode, when nothing in
//! the app claimed the key (`RinchApp::handle_unclaimed_key_fallback`).
//!
//! On in debug builds and off in release builds by default ([`DEVTOOLS_DEFAULT`],
//! `cfg!(debug_assertions)`), so an app ships without them unless it asks.
//! [`App::devtools`](crate::App::devtools) sets it for
//! every window of an app ([`set_devtools_default`], before the windows' apps
//! are created); [`RinchApp::set_devtools`] turns it on or off for one app,
//! which is how an embedded `RinchContext` reaches it (`app_mut()`).
//! `RINCH_DEVTOOLS=on|off` in the environment overrides both, so whoever
//! starts the process can bring DevTools back to an app that ships with it
//! off, or silence it in one that does not.
//!
//! Off, the fallback does not consume either key: F12 and Alt+I go on to the
//! rest of the fallback (text input, for an Alt+I that typed a character),
//! exactly as any other unclaimed key does.

use super::RinchApp;

/// The environment variable that overrides [`App::devtools`](crate::App::devtools).
pub const DEVTOOLS_ENV: &str = "RINCH_DEVTOOLS";

/// Whether the DevTools keys work when neither the app nor `RINCH_DEVTOOLS`
/// says: on in a debug build, off in a release build (`cfg!(debug_assertions)`),
/// so a shipped app has no F12 DevTools window unless it asks for one.
pub const DEVTOOLS_DEFAULT: bool = cfg!(debug_assertions);

thread_local! {
    /// The default for a new [`RinchApp`] on this thread: `App::devtools`
    /// (`run` creates every window's app on the thread it is called on).
    static DEVTOOLS_OFF_BY_DEFAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(!DEVTOOLS_DEFAULT) };

    /// `RINCH_DEVTOOLS`, parsed: `None` until first read, then `Some(answer)`.
    /// Per thread so a test can set it (`set_env_override_for_test`) without
    /// touching the process environment other tests share; every window's
    /// app lives on the one main thread, so it is read once in an app.
    static ENV_OVERRIDE: std::cell::Cell<Option<Option<bool>>> = const { std::cell::Cell::new(None) };
}

/// Whether a `RinchApp` created from now on, on this thread, answers F12 and
/// Alt+I ([`App::devtools`](crate::App::devtools) calls this before the
/// window's app is created). `RINCH_DEVTOOLS` still overrides it.
pub fn set_devtools_default(enabled: bool) {
    DEVTOOLS_OFF_BY_DEFAULT.set(!enabled);
}

/// What a `RINCH_DEVTOOLS` value asks for: `Some(true)` for `on`, `1`,
/// `true` or `yes`, `Some(false)` for `off`, `0`, `false` or `no` (any case,
/// surrounding space ignored), and `None` — no override — for an empty or
/// unset variable or any other value.
pub(crate) fn parse_devtools_env(value: Option<&str>) -> Option<bool> {
    let v = value?.trim();
    if ["on", "1", "true", "yes"]
        .iter()
        .any(|w| v.eq_ignore_ascii_case(w))
    {
        Some(true)
    } else if ["off", "0", "false", "no"]
        .iter()
        .any(|w| v.eq_ignore_ascii_case(w))
    {
        Some(false)
    } else {
        if !v.is_empty() {
            tracing::warn!("{DEVTOOLS_ENV}={v:?} is not on/off; ignored");
        }
        None
    }
}

/// The setting in force: the environment's answer when it gives one, else the
/// app's.
pub(crate) fn devtools_resolved(env: Option<bool>, app: bool) -> bool {
    env.unwrap_or(app)
}

/// This thread's default for a new [`RinchApp`] (tests read a fresh thread's).
#[cfg(test)]
pub(crate) fn thread_default() -> bool {
    !DEVTOOLS_OFF_BY_DEFAULT.get()
}

fn env_override() -> Option<bool> {
    if let Some(cached) = ENV_OVERRIDE.get() {
        return cached;
    }
    let parsed = parse_devtools_env(std::env::var(DEVTOOLS_ENV).ok().as_deref());
    ENV_OVERRIDE.set(Some(parsed));
    parsed
}

/// Stand in for `RINCH_DEVTOOLS` on this thread (`None`: unset).
#[cfg(test)]
pub(crate) fn set_env_override_for_test(value: Option<&str>) {
    ENV_OVERRIDE.set(Some(parse_devtools_env(value)));
}

/// The DevTools-keys setting of one [`RinchApp`]: the app's own choice, before
/// the environment's override.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DevToolsKeys {
    app_enabled: bool,
}

impl Default for DevToolsKeys {
    fn default() -> Self {
        Self {
            app_enabled: !DEVTOOLS_OFF_BY_DEFAULT.get(),
        }
    }
}

impl RinchApp {
    /// Turn the built-in DevTools keys on (the default) or off for this app:
    /// F12, which toggles the DevTools window on the desktop, and Alt+I, which
    /// toggles inspect mode. Off, neither key is consumed, so an app's own
    /// handling of what is left of the key (text input) sees it. A
    /// `RINCH_DEVTOOLS=on|off` in the environment wins over this setting.
    ///
    /// Turning it off does not close a DevTools window already open (its
    /// close button still does), nor end inspect mode.
    pub fn set_devtools(&mut self, enabled: bool) {
        self.devtools_keys.app_enabled = enabled;
    }

    /// Whether F12 and Alt+I reach DevTools and inspect mode in this app: the
    /// environment's `RINCH_DEVTOOLS` if it is set to on or off, else
    /// [`set_devtools`](Self::set_devtools) (else
    /// [`App::devtools`](crate::App::devtools), else on).
    pub fn devtools_enabled(&self) -> bool {
        devtools_resolved(env_override(), self.devtools_keys.app_enabled)
    }
}
