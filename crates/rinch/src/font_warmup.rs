//! The generic families resolved on a thread of their own once the app has
//! shown its first frame (#2552): see
//! [`rinch_dom::fonts::warm_generic_families`].
//!
//! Without it, the first text of a process with a glyph its font stack's
//! first families lack (a menu's `✓` or `▸`) resolved the generic list's
//! families on the UI thread, one `FcFontSort` each: 80–560 ms before a
//! panel's first menu showed. Started after the first presented frame, so
//! it neither competes with the start-up frame nor holds the shared font
//! lock while that frame needs it; once per process.

use std::sync::atomic::{AtomicBool, Ordering};

static STARTED: AtomicBool = AtomicBool::new(false);
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Whether the warm-up runs (on unless `App::font_warmup` turned it off).
pub(crate) fn set_enabled(on: bool) {
    DISABLED.store(!on, Ordering::Relaxed);
}

/// A window presented a frame: the first time, start the warm-up.
pub(crate) fn window_presented() {
    if DISABLED.load(Ordering::Relaxed) || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rinch-font-warmup".into())
        .spawn(|| {
            let started = std::time::Instant::now();
            let families = rinch_dom::fonts::warm_generic_families();
            tracing::debug!(
                "rinch: {families} generic families resolved in {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        });
    if let Err(e) = spawned {
        tracing::debug!("rinch: no font warm-up thread ({e})");
    }
}
