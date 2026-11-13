//! The GPU render callback in the browser (#1768, the `web-gpu` feature).
//!
//! [`RenderSurfaceHandle::set_gpu_render_callback`] draws a surface with the
//! GPU on the desktop and on the web alike. On the web rinch requests **one
//! WebGPU device** for the page, the first time a mounted surface with a GPU
//! render callback has a size, and hands each of that surface's frames the
//! canvas's current texture at the canvas's device-pixel size. The browser
//! composites the canvas with the page.
//!
//! - **The device.** By default rinch requests it with `Features::default()` /
//!   `Limits::default()`; [`set_gpu_config`] raises that, with the same
//!   [`RinchGpuConfig`] the desktop's `App::gpu_config` takes. An app that
//!   must make the device itself hands it over with [`set_external_gpu`],
//!   the desktop's [`ExternalGpu`].
//! - **WebGPU only.** No WebGL: where the browser has no WebGPU adapter the
//!   surface reports no GPU frames
//!   ([`gpu_frames_available`](RenderSurfaceHandle::gpu_frames_available) stays
//!   `false`) and its ordinary [render callback](RenderSurfaceHandle::set_render_callback)
//!   runs instead, if it has one — set both to have a fallback that draws
//!   with [`SurfaceWriter::submit_frame`](crate::render_surface::SurfaceWriter::submit_frame).
//!   rinch asks for an adapter before it touches a canvas, so a canvas that
//!   falls back is never claimed by a WebGPU context.
//! - **Waiting for the device.** Acquiring it is asynchronous. Until it is
//!   ready, a surface with a GPU render callback draws nothing (neither
//!   callback runs, so a fallback frame does not claim the canvas for a 2D
//!   context first); then `gpu_frames_available` turns `true` and every such
//!   surface draws a frame.
//! - **Device loss.** A device rinch made and lost for a reason other than
//!   `destroy()` is **requested again**: the surfaces wait, as at the start,
//!   and draw with the new device, whose frames carry a new
//!   [`generation`](crate::render_surface::GpuSurfaceFrame::generation) —
//!   rebuild every pipeline, buffer and texture made on the old one when it
//!   changes. If no new device can be had (the browser blocks WebGPU after
//!   repeated GPU-process crashes), or the device was destroyed, the surfaces
//!   report no GPU frames from then on and the render callback runs. rinch
//!   does not watch an app-supplied device: an app that loses its own hands
//!   rinch the new one with [`set_external_gpu`] again.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use wgpu::{Backend, Backends, DeviceLostReason, Features, Limits};

use crate::gpu_device::device_features;
pub use crate::gpu_device::{ExternalGpu, RinchGpuConfig};
use crate::render_surface::{GpuSurfaceFrame, RenderSurfaceHandle};

/// [`set_gpu_config`] was called after rinch had begun requesting its device,
/// so the config would not apply to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebGpuAlreadyStarted;

impl std::fmt::Display for WebGpuAlreadyStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "rinch has already requested its WebGPU device; call set_gpu_config before the first \
             surface with a GPU render callback mounts",
        )
    }
}

impl std::error::Error for WebGpuAlreadyStarted {}

/// Raise the requirements of the WebGPU device rinch requests for the page's
/// GPU render callbacks: the web's `App::gpu_config`.
///
/// Call it before the first surface with a GPU render callback mounts (in
/// `main`, before `rinch_web::mount`, say); after that rinch has begun
/// requesting its device with what it had, and this returns
/// [`WebGpuAlreadyStarted`]. A required feature or limit the adapter lacks
/// fails the device: the surfaces then report no GPU frames, and the error is
/// logged naming what was missing. It does nothing once an app device was
/// handed over with [`set_external_gpu`].
pub fn set_gpu_config(config: RinchGpuConfig) -> Result<(), WebGpuAlreadyStarted> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !matches!(s.phase, Phase::Idle) {
            return Err(WebGpuAlreadyStarted);
        }
        s.config = Some(Box::new(config));
        Ok(())
    })
}

/// Draw the page's GPU render callbacks with the app's own WebGPU device: the
/// web's `App::external_gpu`.
///
/// rinch makes each surface's canvas surface from `gpu.instance` and draws on
/// `gpu.device`; it never requests a device of its own after this. Callable at
/// any time: a later call replaces the device (after the app's device was lost,
/// say), and every surface with a GPU render callback draws a frame on the new
/// one, with a new [`generation`](GpuSurfaceFrame::generation). An adapter that
/// is not a WebGPU one (`Backend::BrowserWebGpu`) is refused with an error in
/// the log, and the surfaces report no GPU frames.
pub fn set_external_gpu(gpu: ExternalGpu) {
    let backend = gpu.adapter.get_info().backend;
    let phase = if backend == Backend::BrowserWebGpu {
        Phase::Ready(Box::new(gpu))
    } else {
        tracing::error!(
            "rinch::web_gpu::set_external_gpu: the adapter is a {backend:?} one; GPU render \
             callbacks on the web draw with WebGPU only"
        );
        Phase::Unavailable
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.external = true;
        s.settle(phase);
    });
    wake_gpu_surfaces();
}

/// Whether the page's WebGPU device is ready to draw GPU frames with.
pub(crate) fn device_ready() -> bool {
    STATE.with(|s| matches!(s.borrow().phase, Phase::Ready(_)))
}

/// Where the page's device is, for a surface about to draw.
pub(crate) enum DeviceFor {
    /// Ready: the stack and its generation.
    Ready(Box<ExternalGpu>, u64),
    /// Being requested; the surfaces are woken when it settles.
    Pending,
    /// None to be had: the surface draws with its CPU render callback.
    Unavailable,
}

/// The device a surface draws its GPU frame with. The first ask starts the
/// request for rinch's own device.
pub(crate) fn device_for_frame() -> DeviceFor {
    let start = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match &s.phase {
            Phase::Ready(gpu) => return Err(DeviceFor::Ready(gpu.clone(), s.generation)),
            Phase::Pending => return Err(DeviceFor::Pending),
            Phase::Unavailable => return Err(DeviceFor::Unavailable),
            Phase::Idle => {}
        }
        s.phase = Phase::Pending;
        Ok(s.config.clone())
    });
    match start {
        Ok(config) => {
            request_device(config);
            DeviceFor::Pending
        }
        Err(now) => now,
    }
}

// ── The page's device ────────────────────────────────────────────────────────

enum Phase {
    /// Nothing asked for a device yet.
    Idle,
    /// rinch is requesting its device.
    Pending,
    /// The device every GPU frame is drawn with.
    Ready(Box<ExternalGpu>),
    /// No device: GPU frames are not available.
    Unavailable,
}

struct State {
    phase: Phase,
    /// What [`set_gpu_config`] asked for.
    config: Option<Box<RinchGpuConfig>>,
    /// The app handed its own device over: rinch requests none.
    external: bool,
    /// The current device's generation: 1 for the first, one more for each
    /// device after it. Frames carry it.
    generation: u64,
}

impl State {
    /// Enter `phase` after a request or a hand-over; a new device is a new
    /// generation.
    fn settle(&mut self, phase: Phase) {
        if matches!(phase, Phase::Ready(_)) {
            self.generation += 1;
        }
        self.phase = phase;
    }
}

thread_local! {
    static STATE: RefCell<State> = const {
        RefCell::new(State {
            phase: Phase::Idle,
            config: None,
            external: false,
            generation: 0,
        })
    };
}

/// Request rinch's own device in a task, then settle and wake the surfaces.
fn request_device(config: Option<Box<RinchGpuConfig>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let acquired = acquire(config.as_deref()).await;
        let device = acquired.as_ref().ok().map(|gpu| gpu.device.clone());
        let phase = match acquired {
            Ok(gpu) => Phase::Ready(Box::new(gpu)),
            Err(why) => {
                tracing::warn!(
                    "no WebGPU device for RenderSurface GPU frames: {why}; surfaces draw with \
                     their render callback instead"
                );
                Phase::Unavailable
            }
        };
        let generation = STATE.with(|s| {
            let mut s = s.borrow_mut();
            // The app's own device arrived meanwhile: it stands.
            if s.external {
                return None;
            }
            s.settle(phase);
            Some(s.generation)
        });
        let Some(generation) = generation else {
            return;
        };
        if let Some(device) = device {
            watch_for_loss(&device, generation);
        }
        wake_gpu_surfaces();
    });
}

/// Request an adapter (WebGPU only, no surface: a canvas is not claimed by a
/// WebGPU context before there is a device to draw it with) and a device on it
/// with the app's config, or the defaults.
async fn acquire(config: Option<&RinchGpuConfig>) -> Result<ExternalGpu, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| format!("no WebGPU adapter ({e})"))?;
    if adapter.get_info().backend != Backend::BrowserWebGpu {
        return Err(format!(
            "the adapter is a {:?} one, not WebGPU",
            adapter.get_info().backend
        ));
    }
    let (required, optional, limits) = match config {
        Some(cfg) => (
            cfg.required_features,
            cfg.optional_features,
            cfg.required_limits.clone(),
        ),
        None => (Features::default(), Features::empty(), Limits::default()),
    };
    // The canvas format is the browser's preferred one, `Bgra8Unorm` on most
    // desktops: its storage use needs `BGRA8UNORM_STORAGE`, asked for where the
    // adapter has it, as on the desktop.
    let features =
        device_features(required, optional, adapter.features(), true).map_err(|missing| {
            format!(
                "the adapter lacks the features rinch::web_gpu::set_gpu_config requires: \
                 {missing:?} (request them in `optional_features` to go without them where \
                 they are missing)"
            )
        })?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("rinch web device"),
            required_features: features,
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        })
        .await
        .map_err(|e| {
            format!(
                "the device could not be created (the adapter may not support the \
                 features/limits requested via set_gpu_config): {e}"
            )
        })?;
    // The stack is the desktop's `ExternalGpu`, whose fields are `Arc`s; on
    // the web wgpu's types are neither `Send` nor `Sync`, and the `Arc` is
    // only the shared type's shape.
    #[allow(clippy::arc_with_non_send_sync)]
    Ok(ExternalGpu {
        instance,
        adapter: Arc::new(adapter),
        device: Arc::new(device),
        queue: Arc::new(queue),
    })
}

/// On the loss of the device rinch made as `generation`, request a new one
/// (or report none, when it was destroyed on purpose). A loss reported for a
/// device that is no longer the current one changes nothing.
fn watch_for_loss(device: &wgpu::Device, generation: u64) {
    device.set_device_lost_callback(move |reason, message| {
        let config = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let current = matches!(s.phase, Phase::Ready(_)) && s.generation == generation;
            if !current || s.external {
                return None;
            }
            if reason == DeviceLostReason::Destroyed {
                tracing::warn!(
                    "the WebGPU device of RenderSurface GPU frames was destroyed; surfaces draw \
                     with their render callback from now on"
                );
                s.phase = Phase::Unavailable;
                return Some(None);
            }
            tracing::warn!("the WebGPU device was lost ({message}); requesting a new one");
            s.phase = Phase::Pending;
            Some(Some(s.config.clone()))
        });
        match config {
            Some(Some(config)) => request_device(config),
            Some(None) => wake_gpu_surfaces(),
            None => {}
        }
    });
}

/// Every surface with a GPU render callback draws a frame: the device
/// settled, or changed.
fn wake_gpu_surfaces() {
    crate::render_surface::wake_gpu_surfaces();
}

// ── A surface's canvas ───────────────────────────────────────────────────────

/// A surface's canvas as a wgpu surface, configured for one device.
pub(crate) enum CanvasGpu {
    /// Configured on the device of `generation`.
    Ready {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        generation: u64,
    },
    /// The canvas would not take a WebGPU context (an app claimed another
    /// first): nothing is drawn on it until it is mounted again.
    Refused,
}

/// Whether a surface's next frame must be drawn whatever its pacing: its
/// canvas is not configured at `target`, or for the current device.
pub(crate) fn target_changed(
    surface: &RenderSurfaceHandle,
    target: (u32, u32),
    generation: u64,
) -> bool {
    match &*surface.web_gpu_canvas.borrow() {
        Some(CanvasGpu::Ready {
            config,
            generation: g,
            ..
        }) => (config.width, config.height) != target || *g != generation,
        Some(CanvasGpu::Refused) => false,
        None => true,
    }
}

/// The texture usages of a frame's target: those of the desktop's target, so
/// the same app code can copy out of it, sample it or copy into it.
const CANVAS_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

/// Draw one GPU frame of `surface` on `gpu`'s device into its canvas's current
/// texture at `target` (the canvas's size, scaled to the device's limit).
pub(crate) fn draw_frame(
    surface: &RenderSurfaceHandle,
    gpu: &ExternalGpu,
    generation: u64,
    target: (u32, u32),
) {
    let Some(canvas) = surface.canvas.borrow().clone() else {
        return; // not mounted: the canvas's init asks for a frame
    };
    let mut state = surface.web_gpu_canvas.borrow_mut();
    let stale = !matches!(&*state, Some(CanvasGpu::Ready { generation: g, .. }) if *g == generation)
        && !matches!(&*state, Some(CanvasGpu::Refused));
    if stale {
        *state = Some(configure_canvas(canvas.clone(), gpu, generation, target));
    }
    let Some(CanvasGpu::Ready {
        surface: canvas_surface,
        config,
        ..
    }) = state.as_mut()
    else {
        return;
    };
    // The canvas's backing store is the texture's size: wgpu sizes it when it
    // configures, and rinch's ResizeObserver sizes it to the layout, which
    // passes the target when the device's limit scales it down.
    let canvas_size = (canvas.width(), canvas.height());
    if (config.width, config.height) != target || canvas_size != target {
        config.width = target.0;
        config.height = target.1;
        canvas_surface.configure(&gpu.device, config);
    }
    let frame = match acquire_texture(canvas_surface, &gpu.device, config) {
        Some(frame) => frame,
        None => return,
    };
    let format = config.format;
    // The view and the texture are handles: no borrow of the canvas state is
    // held while the callback runs.
    drop(state);
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let frame_info = GpuSurfaceFrame {
        device: &gpu.device,
        queue: &gpu.queue,
        texture: &frame.texture,
        target: &view,
        format,
        size: target,
        generation,
    };
    if let Some(callback) = surface.gpu_render_callback.borrow_mut().as_mut() {
        callback(&frame_info);
    }
    gpu.queue.present(frame);
}

/// The canvas's current texture, configuring the canvas again once when it
/// says it is out of date or lost.
fn acquire_texture(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> Option<wgpu::SurfaceTexture> {
    use wgpu::CurrentSurfaceTexture as Current;
    for _ in 0..2 {
        match surface.get_current_texture() {
            Current::Success(t) | Current::Suboptimal(t) => return Some(t),
            Current::Outdated | Current::Lost => surface.configure(device, config),
            _ => return None,
        }
    }
    None
}

/// Make `canvas`'s wgpu surface for `gpu` and configure it at `target`: the
/// browser's preferred canvas format (the first the surface offers), with its
/// `…Srgb` view allowed as on the desktop, premultiplied alpha (the desktop
/// draws a frame as premultiplied too), and the desktop target's usages.
fn configure_canvas(
    canvas: web_sys::HtmlCanvasElement,
    gpu: &ExternalGpu,
    generation: u64,
    target: (u32, u32),
) -> CanvasGpu {
    let surface = match gpu
        .instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
    {
        Ok(surface) => surface,
        Err(e) => {
            warn_refused_once(&e.to_string());
            return CanvasGpu::Refused;
        }
    };
    let caps = surface.get_capabilities(&gpu.adapter);
    let Some(&format) = caps.formats.first() else {
        warn_refused_once("the canvas offers no texture format");
        return CanvasGpu::Refused;
    };
    let srgb = format.add_srgb_suffix();
    let config = wgpu::SurfaceConfiguration {
        usage: CANVAS_USAGES,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: target.0,
        height: target.1,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        view_formats: if srgb == format { vec![] } else { vec![srgb] },
    };
    surface.configure(&gpu.device, &config);
    CanvasGpu::Ready {
        surface,
        config,
        generation,
    }
}

thread_local! {
    static REFUSED_WARNED: Cell<bool> = const { Cell::new(false) };
}

fn warn_refused_once(why: &str) {
    if !REFUSED_WARNED.with(|w| w.replace(true)) {
        tracing::warn!(
            "a RenderSurface canvas would not take a WebGPU context ({why}); its GPU frames are \
             not drawn. Was another context made on it (canvas_element().get_context(..), or a \
             submit_frame before the GPU render callback was set)?"
        );
    }
}
