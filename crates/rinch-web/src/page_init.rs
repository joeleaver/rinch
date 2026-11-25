//! One-time page-global setup: the runtime, the first mount's global init,
//! and document-level event delegation.

use std::cell::Cell;

use rinch_core::events;

use crate::{WebDocument, pointer_lock, setup_event_delegation, web_document};

thread_local! {
    /// Page-global, first-mount-only initialization guard.
    static GLOBAL_INIT: Cell<bool> = const { Cell::new(false) };
    /// Document-level event listeners are installed exactly once per page.
    static DELEGATION_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Install the web runtime without mounting anything (issue #1956): the
/// timer backend `set_timeout` and `sleep` run on, the scheduler that polls
/// `spawn_local` tasks, and the drain that samples `poll_signal`s.
///
/// Every `mount*` calls this, so a page that mounts a root needs nothing more.
/// It is for a page that mounts no rinch DOM but runs rinch code that arms
/// timers, spawns tasks or polls — a game that owns the canvas, say, or
/// `rinch-ws` / `rinch-rtc` code with no UI of its own. Call it at startup,
/// before the first timer: a `set_timeout` armed earlier never fires (a task
/// spawned earlier is picked up). Idempotent, page-global, and it builds no
/// DOM: no event delegation, no theme `<style>`, no root.
///
/// The same function as `rinch::install_web_runtime`, for an app that
/// depends on `rinch` alone. An embedded `rinch::embed::RinchContext` installs
/// the timer backend and task scheduler itself on wasm32.
pub fn install_runtime() {
    // A no-op on host builds of this crate (the workspace's clippy and check):
    // `rinch::install_web_runtime` installs nothing off wasm32.
    rinch::install_web_runtime();
}

/// First-mount-only global init shared by every mount. Clearing handlers/context
/// must NOT run per-root, or a second island would wipe the first.
pub(crate) fn ensure_global_init() {
    GLOBAL_INIT.with(|f| {
        if f.get() {
            return;
        }
        f.set(true);

        // Clear stale handlers/context from any previous run of this wasm module.
        events::clear_handlers();
        rinch_core::clear_context();

        // Timers, tasks and polls: the same install a page with no root makes
        // (issue #1956). Idempotent, so a page that called it first is fine.
        install_runtime();

        // Theme CSS is page-global (one shared `<style>`). On any signal change
        // (e.g. dark-mode toggle), refresh it once — regardless of which root
        // changed. Fine-grained DOM updates are handled by effects directly.
        //
        // Keyed on the theme's generation, so a flush that did not touch the
        // theme costs one integer compare rather than a clone of the whole
        // stylesheet string.
        rinch_core::set_on_signal_change(|| {
            thread_local! {
                static APPLIED_THEME_GENERATION: std::cell::Cell<Option<u64>> =
                    const { std::cell::Cell::new(None) };
            }
            let generation = rinch_core::current_theme_css_generation();
            if APPLIED_THEME_GENERATION.with(|g| g.replace(Some(generation))) == Some(generation) {
                return;
            }
            if let Some(css) = rinch_core::get_current_theme_css() {
                web_document::update_theme_style_global(&css);
            }
        });
    });
}

/// Install document-level event delegation exactly once. The listeners are
/// page-global (they key off `[data-rid]` / `closest(...)`, not any one
/// document), so a single install serves every root.
pub(crate) fn ensure_event_delegation(doc: &WebDocument) {
    DELEGATION_INSTALLED.with(|f| {
        if f.get() {
            return;
        }
        f.set(true);
        setup_event_delegation(doc);
        // The browser's pointer lock behind `rinch::window`'s (#2092).
        pointer_lock::install();
    });
}
