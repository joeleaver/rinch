//! The GPU device rinch's desktop windows share, and its replacement after a
//! device loss (#1788).
//!
//! Every desktop window presents with one device ([`gpu_handle`]): the one
//! rinch made for its first window, or the app's own
//! ([`App::external_gpu`](crate::App::external_gpu)). A driver reset, a GPU
//! taken away or a TDR on Windows loses that device, and everything made on it
//! (Vello's renderer, the compositors, the surfaces' GPU targets, the
//! textures apps registered) goes with it.
//!
//! **Detection.** rinch watches the device it made with
//! `Device::set_device_lost_callback` ([`GpuSlot::watch`]). The callback runs
//! on whichever thread wgpu notices the loss on; it marks the slot due and
//! wakes the event loop. A callback from a device the slot no longer holds
//! (one already replaced) is ignored. An app's own device is never watched:
//! wgpu keeps one callback per device, and setting rinch's would take the
//! app's away. The app watches it, and hands rinch the replacement with
//! [`replace_external_gpu`].
//!
//! **Replacement.** The event loop, at its next wake or redraw
//! (`RinchRuntime::settle_gpu_device`), drops every window's renderer,
//! [`retires`](GpuSlot::retire) the device, drops every surface's GPU target
//! and registered texture (`render_surface::forget_gpu_device`), and builds
//! each window's renderer again, as at startup: the first one makes (or
//! adopts) the new device and [publishes](GpuSlot::publish) it. Each publish
//! and each retirement moves the [generation](gpu_generation) on.
//!
//! **Telling the app.** Once the windows are rebuilt the listeners
//! [`on_gpu_device_replaced`] registered are called with the new handle (or
//! `None`, when no GPU would start and the window presents with software),
//! on the main thread, before the next frame. A surface's GPU render callback
//! sees the new device and generation in its next [`GpuSurfaceFrame`](crate::render_surface::GpuSurfaceFrame).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use wgpu::{Device, DeviceLostReason, Instance};

use super::desktop::{ExternalGpu, GpuHandle};

/// The shared device, its generation, and what is owed about it.
pub(crate) struct GpuSlot {
    /// The published device and the instance it came from: what a new window
    /// presents with. `None` before the first window and between a
    /// retirement and the next publish.
    shared: RwLock<Option<(Instance, GpuHandle)>>,
    /// Moved on by every publish and every retirement: `0` before the first
    /// device.
    generation: AtomicU64,
    /// The device must be replaced: it was lost, or the app handed over
    /// another ([`Self::replace_external`]).
    due: AtomicBool,
    /// The generation of the last device reported lost (`0`: none).
    lost: AtomicU64,
    /// The device changed since the app was last told: a retirement, or a
    /// publish after the first.
    announce: AtomicBool,
    /// The device an app given [`App::external_gpu`](crate::App::external_gpu)
    /// handed over to replace its first one.
    replacement: Mutex<Option<ExternalGpu>>,
}

/// The process's shared device.
pub(crate) static GPU: GpuSlot = GpuSlot::new();

impl GpuSlot {
    pub(crate) const fn new() -> Self {
        Self {
            shared: RwLock::new(None),
            generation: AtomicU64::new(0),
            due: AtomicBool::new(false),
            lost: AtomicU64::new(0),
            announce: AtomicBool::new(false),
            replacement: Mutex::new(None),
        }
    }

    /// The published device, stamped with its generation.
    pub(crate) fn handle(&self) -> Option<GpuHandle> {
        self.read().as_ref().map(|(_, handle)| handle.clone())
    }

    /// The published device and the instance it came from.
    pub(crate) fn stack(&self) -> Option<(Instance, GpuHandle)> {
        self.read().clone()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Option<(Instance, GpuHandle)>> {
        self.shared.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish `handle` as the shared device, unless one is published already
    /// (a later window presents with that one). Returns the generation it was
    /// published at.
    pub(crate) fn publish(&self, instance: Instance, mut handle: GpuHandle) -> Option<u64> {
        let mut shared = self.shared.write().unwrap_or_else(|e| e.into_inner());
        if shared.is_some() {
            return None;
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        handle.generation = generation;
        *shared = Some((instance, handle));
        // The first device is not news; every later one replaces one.
        if generation > 1 {
            self.announce.store(true, Ordering::Release);
        }
        Some(generation)
    }

    /// Stop handing out the published device: the next window makes (or
    /// adopts) another. Its handle is dropped after the lock is released.
    pub(crate) fn retire(&self) {
        let old = self
            .shared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.announce.store(true, Ordering::Release);
        drop(old);
    }

    /// Watch `device`, published at `generation`, for its loss, and keep the
    /// loss from panicking the app.
    ///
    /// Once a device is lost every call on it fails, until the event loop
    /// replaces it: a frame being drawn, the app's render thread. wgpu drops
    /// the errors it classes as device-lost, but not a validation error on a
    /// resource made invalid by the loss (mapping a buffer created on the dead
    /// device, say), and its default handler panics on those. So the device
    /// gets a handler of its own: an error from a device known to be lost is
    /// logged once and dropped; any other panics, as wgpu's default does.
    /// `Device::destroy` reports its loss only at a poll that finds the queue
    /// empty, so before deciding the handler polls the device, waiting for the
    /// work in flight, and a destroyed device has reported itself by then.
    ///
    /// An app that sets its own `on_uncaptured_error` on the device replaces
    /// this handler and takes on those errors.
    pub(crate) fn watch(&'static self, device: &Arc<Device>, generation: u64) {
        device.set_device_lost_callback(move |reason, message| {
            self.lost(generation, reason, &message);
        });
        let weak = Arc::downgrade(device);
        let logged = AtomicBool::new(false);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            if !self.is_gone(generation) {
                if let Some(device) = weak.upgrade() {
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());
                }
            }
            if !self.is_gone(generation) {
                panic!("wgpu error: {error}");
            }
            if !logged.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "rinch: GPU errors on the lost device are ignored until it is replaced \
                     (first: {error})"
                );
            }
        }));
    }

    /// Whether the published device is known to be lost (and not replaced
    /// yet): what is left of a frame is not drawn on it.
    pub(crate) fn current_is_lost(&self) -> bool {
        let generation = self.generation();
        generation != 0 && self.lost.load(Ordering::Acquire) == generation
    }

    /// Give the published device a poll, without waiting: a device that was
    /// destroyed reports its loss only at a poll that finds its queue empty,
    /// so the event loop polls before it decides whether to replace it.
    pub(crate) fn poll_published(&self) {
        if let Some(handle) = self.handle() {
            let _ = handle.device.poll(wgpu::PollType::Poll);
        }
    }

    /// Whether the device published at `generation` was lost or replaced.
    fn is_gone(&self, generation: u64) -> bool {
        self.lost.load(Ordering::Acquire) == generation || self.generation() != generation
    }

    /// The device published at `generation` was lost. Ignored when the slot
    /// has moved on since (a retired device reporting late).
    pub(crate) fn lost(&self, generation: u64, reason: DeviceLostReason, message: &str) {
        if self.generation() != generation {
            return;
        }
        self.lost.store(generation, Ordering::Release);
        tracing::error!(
            "rinch: the GPU device was lost ({reason:?}{}{message}); replacing it",
            if message.is_empty() { "" } else { ": " }
        );
        self.mark_due();
    }

    /// The app's replacement for the device it gave
    /// [`App::external_gpu`](crate::App::external_gpu).
    pub(crate) fn replace_external(&self, gpu: ExternalGpu) {
        *self.replacement.lock().unwrap_or_else(|e| e.into_inner()) = Some(gpu);
        self.mark_due();
    }

    /// The app's device a new window presents with: its replacement, when it
    /// handed one over, else the one it configured.
    pub(crate) fn external_source(&self, configured: &ExternalGpu) -> ExternalGpu {
        self.replacement
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| configured.clone())
    }

    fn mark_due(&self) {
        if !self.due.swap(true, Ordering::AcqRel) {
            wake_event_loop();
        }
    }

    /// Whether the device must be replaced now; clears the request.
    pub(crate) fn take_due(&self) -> bool {
        self.due.swap(false, Ordering::AcqRel)
    }

    /// The device to tell the app about, when it changed since it was last
    /// told: `Some(None)` when there is none (no GPU would start, or no window
    /// is shown to make one).
    pub(crate) fn take_announcement(&self) -> Option<Option<GpuHandle>> {
        self.announce
            .swap(false, Ordering::AcqRel)
            .then(|| self.handle())
    }
}

/// Wake the event loop from any thread, so it replaces the device.
fn wake_event_loop() {
    if let Some(proxy) = crate::shell::rinch_runtime::GLOBAL_PROXY.get() {
        proxy.wake_up();
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Get the shared GPU handle, if the renderer has been initialized.
///
/// Returns `None` until the window's GPU renderer has started. After that,
/// returns the `Arc<Device>`, `Arc<Queue>`, and `Arc<Adapter>` that rinch uses
/// internally, allowing external renderers to share the same GPU device.
///
/// It stays `None` for the whole session when the window presents with the
/// software renderer: `Renderer::Software` was chosen (by
/// [`App::renderer`](crate::App::renderer) or `RINCH_RENDERER`), or the GPU
/// would not start under `Renderer::Auto` and rinch fell back. An app that
/// configured the device itself ([`App::gpu_config`](crate::App::gpu_config),
/// [`App::external_gpu`](crate::App::external_gpu)) is exempt from both: it
/// always presents on the GPU, or panics at startup when the GPU will not
/// start, so once its window is up the handle is there.
///
/// **The device can change** (#1788): after a device loss rinch makes a new
/// one, and this returns that one, with a higher
/// [`generation`](GpuHandle::generation). Hold the handle for a frame, not
/// for the session; [`on_gpu_device_replaced`] says when to fetch it again.
pub fn gpu_handle() -> Option<GpuHandle> {
    GPU.handle()
}

/// The shared device's generation, from any thread, without a lock: `0`
/// before the first device, then moved on each time the device is replaced
/// (#1788). A thread that renders on the shared device compares it with the
/// [`GpuHandle::generation`] it holds: a difference means that device is gone
/// and [`gpu_handle`] has its successor (or `None`, between the two).
pub fn gpu_generation() -> u64 {
    GPU.generation()
}

/// Call `listener` on the main thread each time the shared device is replaced
/// (#1788): with the new handle, after every window has been rebuilt on it and
/// before the next frame; or with `None` when the device was lost and none
/// replaces it yet (no GPU would start and the windows present with software,
/// or no window is shown to make one; a later window's device is announced
/// when it comes).
///
/// Everything made on the old device is gone: pipelines, buffers, textures.
/// In particular rinch dropped every texture registered with
/// [`GpuTextureRegistrar::set_texture_source`](crate::render_surface::GpuTextureRegistrar::set_texture_source):
/// register one made on the new device. A surface's
/// [GPU render callback](crate::render_surface::RenderSurfaceHandle::set_gpu_render_callback)
/// needs no listener: it is handed a fresh target on the new device, and a
/// frame, with the new [`generation`](crate::render_surface::GpuSurfaceFrame::generation).
///
/// Not called for the first device. Listeners stay registered for the
/// session; register one at startup, not per component.
pub fn on_gpu_device_replaced(listener: impl FnMut(Option<&GpuHandle>) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(listener)));
}

/// Hand rinch a new device to replace the one given
/// [`App::external_gpu`](crate::App::external_gpu), from any thread (#1788).
///
/// rinch does not watch an app's own device for its loss: wgpu keeps one
/// device-lost callback per device, and rinch's would replace the app's. The
/// app watches it (`Device::set_device_lost_callback`), makes a new stack, and
/// calls this. At the event loop's next turn rinch rebuilds every window on
/// `gpu` (the adapter must be able to present to them, or that panics as at
/// startup), drops the surfaces' GPU targets and registered textures, and
/// calls the [`on_gpu_device_replaced`] listeners. It also serves to move the
/// app to another GPU with no loss at all.
///
/// Ignored, with a warning, in an app that did not start with
/// `App::external_gpu`: rinch owns that device and replaces it itself.
pub fn replace_external_gpu(gpu: ExternalGpu) {
    if !super::desktop::external_gpu_configured() {
        tracing::warn!(
            "rinch: replace_external_gpu ignored: the app did not start with App::external_gpu, \
             and rinch replaces its own device itself"
        );
        return;
    }
    GPU.replace_external(gpu);
}

type Listener = Box<dyn FnMut(Option<&GpuHandle>)>;

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// Call the [`on_gpu_device_replaced`] listeners with `handle`. A listener
/// may register another; it is called from the next replacement on.
pub(crate) fn announce(handle: Option<&GpuHandle>) {
    let mut listeners = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    for listener in &mut listeners {
        listener(handle);
    }
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        let added = std::mem::take(&mut *l);
        listeners.extend(added);
        *l = listeners;
    });
}

#[cfg(test)]
#[path = "gpu_device_1788_tests.rs"]
mod tests;
