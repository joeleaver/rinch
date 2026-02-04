//! Platform event loop abstraction.

use crate::PlatformEvent;

/// Abstraction over the platform event loop.
///
/// Event loops are fundamentally different across platforms:
/// - Desktop: blocking `winit::EventLoop::run_app()` -- never returns
/// - Web: `requestAnimationFrame` callback chain -- returns immediately
/// - Mobile: platform-specific run loops
///
/// The trait is intentionally minimal. Each backend implements the loop
/// mechanics internally, and calls the provided handler with translated
/// `PlatformEvent`s.
pub trait PlatformEventLoop: Sized {
    /// The waker type for this event loop.
    type Waker: PlatformWaker;

    /// Create the event loop and return a waker handle.
    ///
    /// The waker can be used from any thread to wake the event loop
    /// (e.g., when a reactive signal changes).
    fn new() -> Result<(Self, Self::Waker), String>;

    /// Run the event loop with the given event handler.
    ///
    /// On desktop, this function does not return (it takes over the main thread).
    /// On web, it sets up the callback chain and returns immediately.
    ///
    /// The handler receives `PlatformEvent`s translated from native events.
    fn run<F>(self, handler: F) -> Result<(), String>
    where
        F: FnMut(PlatformEvent) + 'static;
}

/// A thread-safe handle for waking the event loop.
///
/// Used by `rinch_core::set_on_signal_change()` to trigger re-renders
/// when reactive signals change from any thread.
///
/// # Implementations
/// - Desktop: wraps `winit::event_loop::EventLoopProxy`
/// - Web: calls `requestAnimationFrame` or `postMessage`
pub trait PlatformWaker: Clone + Send + Sync + 'static {
    /// Wake the event loop, causing it to emit a `UserEvent::ReRender`.
    fn wake(&self);
}
