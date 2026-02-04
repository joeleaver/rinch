//! Web event loop using requestAnimationFrame and DOM event listeners.

use rinch_platform::{PlatformEvent, PlatformEventLoop, PlatformWaker};
use std::cell::RefCell;
use std::rc::Rc;

/// Web event loop using requestAnimationFrame and DOM event listeners.
///
/// A real implementation would:
/// - Set up DOM event listeners on the canvas for mouse, keyboard, wheel events
/// - Translate web events (MouseEvent, KeyboardEvent, etc.) to PlatformEvent
/// - Use requestAnimationFrame for the render loop
/// - Call the event handler callback with translated events
/// - Handle resize events via ResizeObserver
///
/// For now this is a minimal stub to enable compilation.
pub struct WebEventLoop;

/// Waker that can trigger a re-render from web callbacks.
///
/// On web, waking the event loop means scheduling a new requestAnimationFrame
/// callback. This is used when reactive signals change to trigger re-renders.
///
/// # Safety
///
/// WASM is single-threaded, so Send + Sync are safe despite using Rc.
/// The wasm32-unknown-unknown target doesn't have threads, so these traits
/// are effectively no-ops.
#[derive(Clone)]
pub struct WebWaker {
    needs_render: Rc<RefCell<bool>>,
}

// Safety: WASM is single-threaded, no actual threads exist
unsafe impl Send for WebWaker {}
unsafe impl Sync for WebWaker {}

impl PlatformWaker for WebWaker {
    fn wake(&self) {
        // Mark that we need a render and request an animation frame.
        *self.needs_render.borrow_mut() = true;

        // TODO: Call window.requestAnimationFrame() here
        // This requires storing a callback handle and managing the animation loop.
    }
}

impl PlatformEventLoop for WebEventLoop {
    type Waker = WebWaker;

    fn new() -> Result<(Self, Self::Waker), String> {
        let waker = WebWaker {
            needs_render: Rc::new(RefCell::new(false)),
        };

        Ok((Self, waker))
    }

    fn run<F>(self, mut _callback: F) -> Result<(), String>
    where
        F: FnMut(PlatformEvent) + 'static,
    {
        // TODO: Implement the web event loop
        //
        // Steps:
        // 1. Get the window and document objects from web-sys
        // 2. Get the canvas element (passed somehow - needs API design)
        // 3. Set up event listeners:
        //    - mousedown, mousemove, mouseup -> PlatformEvent::MouseButton, MouseMove
        //    - wheel -> PlatformEvent::MouseWheel
        //    - keydown, keyup -> PlatformEvent::KeyboardInput
        //    - ResizeObserver -> PlatformEvent::Resized
        // 4. Set up requestAnimationFrame loop:
        //    - Check if needs_render is true
        //    - If so, emit PlatformEvent::RedrawRequested
        //    - Call the callback with translated events
        //    - Schedule next frame
        // 5. Return Ok(()) immediately (web event loops don't block)

        Err("Web event loop not yet implemented".into())
    }
}
