//! Watching for fonts installed while the app runs (#2486, #2537).
//!
//! The process scans the system fonts once and every font context shares that
//! scan (#2467). On Linux this watch asks the kernel to say when anything the
//! scan read changes (inotify, [`inotify`]): the scan's configuration files,
//! its included configuration directories and its font directories, as the
//! scan's own fontconfig configuration lists them
//! (`rinch_dom::fonts::system_font_paths`). On a change it waits the
//! interval for more (a family installed as many files is one rescan),
//! rescans (`rinch_dom::fonts::refresh_system_fonts`), wakes the event loop,
//! whose next frame adopts the new scan (`RinchApp::adopt_refreshed_system_fonts`),
//! and watches what the new scan read.
//!
//! Between changes the thread sleeps in the kernel, in `poll` on the inotify
//! descriptor and an eventfd that [`set`] writes to: an idle app makes no
//! wake-ups for it. Before #2537 it woke every interval to `stat` every path,
//! and loaded a fontconfig configuration of its own to learn them (about
//! 45 M instructions, five seconds after start).
//!
//! One thread, started by the first [`set`] with an interval; `set(None)`
//! wakes it and it ends. Elsewhere than Linux the setting is kept and nothing
//! watches: apps call `rinch::font::refresh_system_fonts` themselves.

#[cfg(all(target_os = "linux", feature = "desktop"))]
mod inotify;

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

// Read only by the Linux watch thread; elsewhere the setting is kept and unused.
#[cfg_attr(not(all(target_os = "linux", feature = "desktop")), allow(dead_code))]
struct State {
    /// How long after a change to wait for more before rescanning; `None`
    /// stops the thread.
    interval: Option<Duration>,
    /// Whether the app has chosen (`set`), so `start_default` leaves it be.
    chosen: bool,
    /// Whether the thread is alive.
    running: bool,
    /// Wakes the thread to read a new setting, once it is watching.
    #[cfg(all(target_os = "linux", feature = "desktop"))]
    wake: Option<std::sync::Arc<inotify::Wake>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    interval: None,
    chosen: false,
    running: false,
    #[cfg(all(target_os = "linux", feature = "desktop"))]
    wake: None,
});

/// `rinch::font::watch_system_fonts`.
pub(crate) fn set(interval: Option<Duration>) {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.chosen = true;
    apply(&mut state, interval);
}

/// What `App::run` asks for: the default interval, unless the app chose.
#[cfg(feature = "desktop")]
pub(crate) fn start_default() {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if !state.chosen {
        apply(&mut state, Some(crate::font::DEFAULT_SYSTEM_FONT_WATCH));
    }
}

fn apply(state: &mut State, interval: Option<Duration>) {
    state.interval = interval;
    #[cfg(all(target_os = "linux", feature = "desktop"))]
    {
        if state.running {
            // The thread reads the new setting when it wakes; before it is
            // watching it has not slept yet, and reads it then.
            if let Some(wake) = &state.wake {
                wake.notify();
            }
        } else if interval.is_some() {
            state.running = std::thread::Builder::new()
                .name("rinch-font-watch".into())
                .spawn(run)
                .is_ok();
        }
    }
}

/// The interval to debounce with, or `None` once the watch is stopped, in
/// which case the thread is marked ended under the same lock (a `set` after
/// this starts a new one).
#[cfg(all(target_os = "linux", feature = "desktop"))]
fn current_interval() -> Option<Duration> {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if state.interval.is_none() {
        state.running = false;
        state.wake = None;
    }
    state.interval
}

#[cfg(all(target_os = "linux", feature = "desktop"))]
fn run() {
    use inotify::Wait;
    use std::time::Instant;

    let mut watch = match inotify::Watch::new() {
        Ok(watch) => watch,
        Err(e) => {
            tracing::warn!(
                "rinch: cannot watch the installed fonts ({e}); a font installed while \
                 the app runs is seen after rinch::font::refresh_system_fonts()"
            );
            let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
            state.running = false;
            return;
        }
    };
    STATE.lock().unwrap_or_else(PoisonError::into_inner).wake = Some(watch.waker());
    let mut paths = rinch_dom::fonts::system_font_paths();
    loop {
        watch.arm(&paths);
        // Asleep in the kernel until something the scan read changes, or a
        // new setting.
        let debounce = loop {
            let Some(interval) = current_interval() else {
                return;
            };
            if watch.wait(None) == Wait::Changed {
                break interval;
            }
        };
        // More changes within the interval are the same one.
        let deadline = Instant::now() + debounce;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if watch.wait(Some(deadline - now)) == Wait::Woken && current_interval().is_none() {
                return;
            }
        }
        rinch_dom::fonts::refresh_system_fonts();
        rinch_core::dispatch_main_callback(Box::new(|| {}));
        paths = rinch_dom::fonts::system_font_paths();
    }
}
