//! The window's finished frame, handed to the app on the GPU (issue #2215),
//! re-exported through [`crate::window`].
//!
//! Recording or streaming a whole window — its panels and chrome as well as
//! any render surface — needs the frame the user sees, every time one is
//! drawn, without reading it back to the CPU on the UI thread.
//! `on_frame_composited` registers a handler that the desktop GPU window
//! calls once per presented frame, after the frame is composited and before
//! it is presented, with a `CompositedFrame`: the texture holding the frame
//! on the window's own device, sampleable and copyable, and a command encoder
//! that is submitted right after the handler returns. The handler records its
//! own copy or blit into textures or buffers it owns and does its own async
//! readback; nothing waits for the GPU.
//!
//! **No extra copy, whether or not a handler is registered.** The texture
//! handed over is the one the window is presented from, which every path
//! already has: the UI texture (Vello's target, `Rgba8Unorm`, straight alpha)
//! when the window has no composite layers or split surfaces, else the
//! composited texture (the surface's format, premultiplied alpha). Both are
//! made with `TEXTURE_BINDING | COPY_SRC`. A partial frame (#1718) updates
//! the UI texture in place, so it always holds the whole frame. A window with
//! no handler registered pays one thread-local check per frame.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How a [`CompositedFrame`]'s colour values carry their alpha.
///
/// The two are the same for an opaque window, whose every pixel has alpha 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameAlpha {
    /// Colour not multiplied by alpha: the UI texture, as Vello writes it.
    Straight,
    /// Colour multiplied by alpha: the composited texture (layers or split
    /// surfaces under the UI).
    Premultiplied,
}

/// One presented frame of the window, from [`on_frame_composited`].
///
/// Everything here is valid only during the call. The texture is the
/// window's own and is drawn into again by the next frame (and replaced at
/// another size when the window resizes), so copy what you need: record a
/// copy or a render pass into [`encoder`](Self::encoder), which rinch submits
/// after the handler returns and before the frame is presented, or submit
/// your own work on [`queue`](Self::queue) — the frame's commands were
/// submitted before the call, so either sees the finished frame.
///
/// **Read it only.** The texture also carries the usages rinch draws it with
/// (`STORAGE_BINDING | COPY_DST` for the UI texture, `RENDER_ATTACHMENT` for
/// the composited one), so wgpu lets you write it, but a write corrupts the
/// frame the next partial frame (#1718) builds on.
///
/// **Reading it back.** A buffer that a copy in `encoder` writes cannot be
/// mapped during the call: the copy is not submitted yet, and `map_async`
/// fails. Either record the copy into an encoder of your own, submit it on
/// `queue`, and call `map_async` — all inside the call — or record it into
/// `encoder` and map the buffer at the next call, by which time rinch has
/// submitted it. A map's callback runs only when the device is polled:
/// rinch polls (without waiting) once per presented frame, so the last
/// frame before the window goes idle is delivered only when the app polls
/// the device itself (`device.poll`, from a timer or another thread).
///
/// Its colour values are sRGB-encoded bytes in a non-sRGB format, as the
/// window presents them.
#[non_exhaustive]
pub struct CompositedFrame<'a> {
    /// The window's device (the one [`gpu_handle`](crate::shell::desktop::gpu_handle)
    /// hands out).
    pub device: &'a wgpu::Device,
    /// The device's queue.
    pub queue: &'a wgpu::Queue,
    /// Submitted right after the handler returns, before the present. Every
    /// handler of this frame records into the same encoder, in the order
    /// they were registered.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The texture holding the frame: `TEXTURE_BINDING | COPY_SRC`, one mip
    /// level, one sample, `size` texels.
    pub texture: &'a wgpu::Texture,
    /// A view of the whole of `texture`, for a sampling pass.
    pub view: &'a wgpu::TextureView,
    /// The frame's size in physical pixels: the window's surface size.
    pub size: (u32, u32),
    /// `texture`'s format: `Rgba8Unorm` for the UI texture; for the
    /// composited one the surface's, which is `Rgba8Unorm` or `Bgra8Unorm`
    /// where the surface offers either, else the first format it offers.
    pub format: wgpu::TextureFormat,
    /// How the colour values carry alpha.
    pub alpha: FrameAlpha,
}

type FrameHandler = Rc<RefCell<dyn FnMut(&mut CompositedFrame<'_>)>>;

thread_local! {
    static HANDLERS: RefCell<Vec<(u64, FrameHandler)>> = const { RefCell::new(Vec::new()) };
    static NEXT_HANDLER: Cell<u64> = const { Cell::new(1) };
}

/// A registration made by [`on_frame_composited`].
///
/// Made while a component renders, the component owns the handler: it goes
/// when the component unmounts, and dropping this does nothing. Made where
/// nothing owns it (an event handler, `rinch_core::unowned`, before the app
/// mounts), **this value is the handler's lifetime**: dropping it removes the
/// handler, so keep it for as long as frames are wanted. That is also the
/// way to keep a recorder running past the component that started it:
/// register it under `rinch_core::unowned(|| …)` and store the subscription.
#[derive(Debug)]
#[must_use = "outside a component, dropping the subscription removes the handler at once"]
pub struct FrameSubscription {
    id: u64,
    /// A component's scope removes the handler: dropping this does not.
    owned: bool,
}

impl FrameSubscription {
    /// Stop handing frames to the handler now, before its component goes.
    pub fn cancel(self) {
        remove_handler(self.id);
    }
}

impl Drop for FrameSubscription {
    fn drop(&mut self) {
        if !self.owned {
            remove_handler(self.id);
        }
    }
}

fn remove_handler(id: u64) {
    let _ = HANDLERS.try_with(|h| h.borrow_mut().retain(|(i, _)| *i != id));
}

/// Hear every frame the window presents: `handler` gets the finished,
/// composited frame as a GPU texture on the window's device (see
/// [`CompositedFrame`]), once per presented frame, on the UI thread.
///
/// - **When.** After the frame is composited and its commands submitted,
///   before it is presented. Only frames that are presented: rinch draws a
///   frame when something asks for one (an event, an animation, a
///   continuous `RenderSurface`), and a window with nothing to draw sleeps,
///   so a recorder that wants a steady rate repeats its last frame across a
///   gap. A window that is hidden, or whose surface is not available this
///   frame (occluded, timed out), presents nothing and hands nothing over.
/// - **Which window.** The app's window. The DevTools window is not handed
///   over. A window re-created by hide and show keeps its handlers.
/// - **Resize.** The frame's `size` follows the window, and the texture is a
///   new one after a resize.
/// - **Software renderer.** Never called: the tiny-skia renderer has no GPU
///   frame. [`presentation`](crate::window::presentation) answers
///   [`Unsupported`](crate::window::Presentation::Unsupported) there, which
///   an app can check. Nor on the web, Android or in a native embed (the
///   host owns the frame there).
/// - **Cost.** No copy is added for it, registered or not: the texture is the
///   one the window presents from. With a handler registered, one command
///   encoder per frame is created and submitted for the handlers' work.
///
/// Called untracked and in a batch, like every app callback. Registering or
/// cancelling a handler from inside one takes effect from the next frame.
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch::window::{on_frame_composited, CompositedFrame};
///
/// #[component]
/// fn recorder() -> NodeHandle {
///     // Removed when this component unmounts.
///     let _ = on_frame_composited(move |frame: &mut CompositedFrame<'_>| {
///         // Copy the frame into a buffer of your own with an encoder of your
///         // own, submit it, then map the buffer: its callback runs at a
///         // later `device.poll` (rinch polls once per presented frame).
///         let buffer: wgpu::Buffer = my_next_free_buffer(frame.size);
///         let mut encoder = frame.device.create_command_encoder(&Default::default());
///         encoder.copy_texture_to_buffer(frame.texture.as_image_copy(), my_copy_info(&buffer), my_extent(frame.size));
///         frame.queue.submit(Some(encoder.finish()));
///         buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
///             // `result` is Ok once the copy is done: read and encode it.
///         });
///     });
///     rsx! { div {} }
/// }
/// ```
pub fn on_frame_composited(
    handler: impl FnMut(&mut CompositedFrame<'_>) + 'static,
) -> FrameSubscription {
    let id = NEXT_HANDLER.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let handler: FrameHandler = Rc::new(RefCell::new(handler));
    HANDLERS.with(|h| h.borrow_mut().push((id, handler)));
    let owned = rinch_core::reactive::on_cleanup(move || remove_handler(id));
    FrameSubscription { id, owned }
}

/// Whether any [`on_frame_composited`] handler is registered on this thread.
#[cfg(test)]
fn has_frame_handlers() -> bool {
    HANDLERS
        .try_with(|h| !h.borrow().is_empty())
        .unwrap_or(false)
}

/// Hand `texture`, which holds the frame about to be presented, to every
/// handler, and submit what they recorded. Does nothing (and creates
/// nothing) while no handler is registered. The frame's own commands must
/// have been submitted already.
pub(crate) fn hand_over_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    alpha: FrameAlpha,
) {
    let handlers: Vec<FrameHandler> = match HANDLERS.try_with(|h| {
        h.borrow()
            .iter()
            .map(|(_, f)| f.clone())
            .collect::<Vec<_>>()
    }) {
        Ok(h) if !h.is_empty() => h,
        _ => return,
    };
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rinch composited frame handlers"),
    });
    rinch_core::batch(|| {
        rinch_core::untracked_handler(|| {
            for handler in handlers {
                // A handler that presents a frame from inside itself is not
                // run a second time.
                if let Ok(mut handler) = handler.try_borrow_mut() {
                    let mut frame = CompositedFrame {
                        device,
                        queue,
                        encoder: &mut encoder,
                        texture,
                        view: &view,
                        size: (texture.width(), texture.height()),
                        format: texture.format(),
                        alpha,
                    };
                    handler(&mut frame);
                }
            }
        });
    });
    queue.submit(Some(encoder.finish()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A component's handler goes when its scope is disposed (#2215, cancel on
    /// unmount); dropping the subscription inside it changes nothing.
    #[test]
    fn a_component_handler_goes_with_its_component() {
        let scope = rinch_core::reactive::Scope::new();
        let sub = scope.run(|| on_frame_composited(|_| {}));
        drop(sub);
        assert!(has_frame_handlers(), "owned: the drop leaves it");
        scope.dispose();
        assert!(!has_frame_handlers(), "the unmount removes it");
    }

    /// Outside a component (the opt-out: `unowned`), the subscription is the
    /// handler's lifetime, and `cancel` ends it early either way.
    #[test]
    fn an_unowned_handler_lives_as_long_as_its_subscription() {
        let sub = rinch_core::unowned(|| on_frame_composited(|_| {}));
        assert!(has_frame_handlers());
        drop(sub);
        assert!(!has_frame_handlers(), "dropped: removed");

        let scope = rinch_core::reactive::Scope::new();
        let kept = scope.run(|| rinch_core::unowned(|| on_frame_composited(|_| {})));
        scope.dispose();
        assert!(has_frame_handlers(), "unowned outlives the scope");
        let owned = scope_free_cancel(kept);
        assert!(!owned && !has_frame_handlers(), "cancelled");
    }

    fn scope_free_cancel(sub: FrameSubscription) -> bool {
        let owned = sub.owned;
        sub.cancel();
        owned
    }
}
