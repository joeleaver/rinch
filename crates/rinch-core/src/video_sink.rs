//! The named video sink: a hook through which a crate that decodes video
//! (rinch-video's players, rinch-rtc's remote tracks) shows frames in the
//! element carrying `data-viewport = <name>`, without depending on `rinch`
//! (design record `docs/design/rtc-media.md`, M6).
//!
//! The shell installs the factory once at startup
//! (`rinch::render_surface::create_video_frame_sink` on desktop). With no
//! shell (a test, a headless tool) there is no factory and
//! [`create_video_sink`] answers `None`.

use std::sync::{Arc, OnceLock};

/// Takes decoded RGBA8 frames `(pixels, width, height)`, rows tightly
/// packed, from any thread. The surface it feeds lives as long as any clone
/// of the sink.
pub type VideoFrameSink = Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>;

type Factory = Box<dyn Fn(&str) -> VideoFrameSink + Send + Sync>;

static FACTORY: OnceLock<Factory> = OnceLock::new();

/// Install the factory that makes a sink for a viewport name. The first
/// call wins; later ones are ignored (the shell may start more than once in
/// a process's tests).
pub fn set_video_sink_factory(f: impl Fn(&str) -> VideoFrameSink + Send + Sync + 'static) {
    INSTALLS.with(|n| n.set(n.get() + 1));
    let _ = FACTORY.set(Box::new(f));
}

thread_local! {
    static INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has called [`set_video_sink_factory`] (a test
/// hook: the shell's install is pinned by it, per thread so another test's
/// install cannot stand in for it).
#[doc(hidden)]
pub fn installs_on_this_thread() -> usize {
    INSTALLS.with(|n| n.get())
}

/// A sink showing frames in the element with `data-viewport = viewport`,
/// when a shell has installed the factory. Call it on the main thread: the
/// surface registry it registers with is the main thread's.
pub fn create_video_sink(viewport: &str) -> Option<VideoFrameSink> {
    FACTORY.get().map(|f| f(viewport))
}

/// Whether a factory is installed.
pub fn has_video_sink_factory() -> bool {
    FACTORY.get().is_some()
}
