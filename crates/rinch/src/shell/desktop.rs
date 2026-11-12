//! Desktop backend: winit window + wgpu renderer.
//!
//! This module implements the [`PlatformWindow`] and [`PlatformRenderer`]
//! traits from `rinch-platform` for the native desktop target using
//! winit for windowing and wgpu/vello for GPU rendering.

use std::sync::{Arc, OnceLock};

use peniko::Color;
use rinch_platform::{CompositeLayer, PlatformRenderer, PlatformWindow, RenderError};
use vello::{RenderParams, Renderer as VelloRenderer, Scene};
use wgpu::{
    Adapter, Backends, CommandEncoderDescriptor, Device, Extent3d, Features, Instance,
    InstanceDescriptor, Limits, MemoryHints, PresentMode, Queue, Surface, SurfaceConfiguration,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
};
use winit::window::Window;

use crate::shell::renderer::{self, AdapterUse, Renderer};

/// Why [`WgpuRenderer::try_new_for`] made no renderer.
#[derive(Debug)]
pub(crate) enum GpuStartError {
    /// The renderer choice declined the only adapter that can present, a
    /// software driver ([`renderer::adapter_use`], #2436). Its info, for the
    /// log.
    DeclinedCpuAdapter(Box<wgpu::AdapterInfo>),
    /// The GPU could not start, and why.
    Failed(String),
}

impl From<String> for GpuStartError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

impl std::fmt::Display for GpuStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeclinedCpuAdapter(info) => write!(
                f,
                "the only GPU adapter that can present, {:?}, is a software driver",
                info.name
            ),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

// ── GpuHandle ────────────────────────────────────────────────────────────────

/// Shared GPU device, queue, and adapter handle.
///
/// Exposed after rinch creates (or adopts) the wgpu renderer so external code
/// (e.g., a game engine running on a background thread) can share the same GPU
/// device for zero-copy texture compositing. The `adapter` is included so an
/// embedder can introspect real device features/limits (e.g. to clamp buffer
/// sizes against `adapter.limits()`).
#[derive(Clone)]
pub struct GpuHandle {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub adapter: Arc<Adapter>,
}

static GPU_HANDLE: OnceLock<GpuHandle> = OnceLock::new();

/// The instance rinch created its own device from, published with
/// [`GPU_HANDLE`]: a later window (hide/show re-creates it, #1520/#1546; a
/// DevTools window) makes its surface from it and presents with the same
/// adapter, device and queue, so the device never changes under the
/// surfaces drawing with it (#1768 review).
static RINCH_INSTANCE: OnceLock<Instance> = OnceLock::new();

/// An instance and the adapter, device and queue made from it.
type GpuStack = (Instance, Arc<Adapter>, Arc<Device>, Arc<Queue>);

/// The GPU stack a new window must present with, when one is already shared:
/// the embedder's ([`App::external_gpu`](crate::App::external_gpu)), or the
/// one rinch created for its first window. `None` before the first window.
fn shared_gpu_stack(gpu_init: Option<&GpuInit>) -> Option<GpuStack> {
    match gpu_init {
        Some(GpuInit::External(g)) => Some((
            g.instance.clone(),
            g.adapter.clone(),
            g.device.clone(),
            g.queue.clone(),
        )),
        _ => {
            let instance = RINCH_INSTANCE.get()?;
            let handle = GPU_HANDLE.get()?;
            Some((
                instance.clone(),
                handle.adapter.clone(),
                handle.device.clone(),
                handle.queue.clone(),
            ))
        }
    }
}

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
pub fn gpu_handle() -> Option<&'static GpuHandle> {
    GPU_HANDLE.get()
}

/// Publish the instance rinch's own device came from, for a test that has
/// no window (the first call wins).
#[cfg(test)]
pub(crate) fn publish_instance_for_tests(instance: Instance) {
    let _ = RINCH_INSTANCE.set(instance);
}

/// [`shared_gpu_stack`] as a window made with rinch's own device sees it.
#[cfg(test)]
pub(crate) fn shared_gpu_stack_for_tests() -> Option<GpuStack> {
    shared_gpu_stack(None)
}

/// Publish a device as the shared GPU handle, for a test that has no window
/// (the first call wins, as at runtime). Returns the handle in place.
#[cfg(test)]
pub(crate) fn gpu_handle_for_tests(make: impl FnOnce() -> GpuHandle) -> &'static GpuHandle {
    GPU_HANDLE.get_or_init(make)
}

// ── GPU device injection (issue #57) ──────────────────────────────────────────

/// Extra GPU device requirements for rinch's desktop compositor.
///
/// By default rinch requests its wgpu device with `Features::default()` /
/// `Limits::default()`. An embedding application whose own renderer needs a
/// higher-capability device (extra features, larger storage buffers, more bind
/// groups, …) can raise those requirements here so that the device rinch
/// composites with — the one published via [`gpu_handle`] — can also host the
/// embedder's pipelines and textures for **zero-copy** present.
///
/// rinch still creates the instance, picks a surface-compatible adapter, and
/// owns the device, so surface presentation is always correct. Use this via
/// [`crate::App::gpu_config`]. The required features and limits are passed
/// through verbatim — if the adapter cannot satisfy them, device creation fails
/// loudly (naming the missing features) rather than silently dropping a
/// capability. Optional features are requested only where the adapter has
/// them (#2085); [`GpuHandle::granted_optional_features`] says which were.
///
/// Build it with `..Default::default()` so a field added later does not
/// break the construction:
///
/// ```ignore
/// RinchGpuConfig {
///     optional_features: wgpu::Features::TIMESTAMP_QUERY,
///     ..Default::default()
/// }
/// ```
#[derive(Clone, Default)]
pub struct RinchGpuConfig {
    /// Device features to require in addition to rinch's defaults. An adapter
    /// without one of them fails device creation, and the error names it.
    pub required_features: Features,
    /// Device limits to require (replaces `Limits::default()`).
    pub required_limits: Limits,
    /// Device features to request where the adapter has them, and to go
    /// without where it does not (#2085): `TIMESTAMP_QUERY` for timing GPU
    /// frames, say. What the device got is
    /// [`GpuHandle::granted_optional_features`] (or `device.features()`).
    ///
    /// Experimental features (`Features::all_experimental_mask()`, such as
    /// `EXPERIMENTAL_RAY_QUERY`) are never requested from this set: wgpu
    /// refuses them without an `ExperimentalFeatures` opt-in, which rinch
    /// does not give, so asking would fail the device on an adapter that has
    /// them. `Features::all()` here is therefore safe.
    pub optional_features: Features,
}

/// The features rinch requests its device with (#2085): the `required` ones,
/// the `optional` ones the adapter has (never an experimental one: wgpu
/// refuses those without an opt-in rinch does not give), and
/// `BGRA8UNORM_STORAGE` where the
/// surface is `Bgra8Unorm` and the adapter has it (any storage use of that
/// surface format needs it). `Err` holds the required features the adapter
/// lacks.
pub(crate) fn device_features(
    required: Features,
    optional: Features,
    adapter: Features,
    bgra_surface: bool,
) -> Result<Features, Features> {
    let missing = required.difference(adapter);
    if !missing.is_empty() {
        return Err(missing);
    }
    let optional = optional.difference(Features::all_experimental_mask());
    let mut features = required | optional.intersection(adapter);
    if bgra_surface && adapter.contains(Features::BGRA8UNORM_STORAGE) {
        features |= Features::BGRA8UNORM_STORAGE;
    }
    Ok(features)
}

/// The optional features the app asked for through
/// [`App::gpu_config`](crate::App::gpu_config); empty for any other start.
fn requested_optional_features() -> Features {
    match GPU_INIT.get() {
        Some(GpuInit::Config(cfg)) => cfg.optional_features,
        _ => Features::empty(),
    }
}

impl GpuHandle {
    /// Which of [`RinchGpuConfig::optional_features`] the device has (#2085):
    /// the ones the adapter supported. Empty when the app asked for none, and
    /// with [`App::external_gpu`](crate::App::external_gpu), whose device is
    /// the app's own (read `device.features()` there).
    pub fn granted_optional_features(&self) -> Features {
        self.device.features() & requested_optional_features()
    }
}

/// The size of every memory block wgpu's allocator opens for rinch's device
/// (#2473): fixed at 8 MiB, where `MemoryHints::MemoryUsage` let it double
/// with each block open (8 to 64 MiB), so that what is reserved follows what
/// is allocated. Measured on #2473: ui-zoo's VRAM fell from 72.4 to 48.4 MiB
/// (an 8..16 MiB range gave 56.4), the bar's and the editor's did not change,
/// and nothing measured cost more. wgpu gives host-visible blocks half of it
/// (4 MiB), and gpu-allocator makes an allocation larger than a block a
/// dedicated one.
const DEVICE_MEMORY_BLOCK: std::ops::Range<u64> = (8 << 20)..(8 << 20);

/// Request rinch's own device from `adapter` for a surface of `format`, with
/// the app's [`RinchGpuConfig`] (#57, #2085), or the defaults without one.
pub(crate) fn request_rinch_device(
    adapter: &Adapter,
    format: TextureFormat,
    config: Option<&RinchGpuConfig>,
) -> Result<(Device, Queue), String> {
    let (required, optional, limits) = match config {
        Some(cfg) => (
            cfg.required_features,
            cfg.optional_features,
            cfg.required_limits.clone(),
        ),
        None => (Features::default(), Features::empty(), Limits::default()),
    };
    let features = device_features(
        required,
        optional,
        adapter.features(),
        format == TextureFormat::Bgra8Unorm,
    )
    .map_err(|missing| {
        format!(
            "Failed to create device: the adapter {:?} lacks the features \
             App::gpu_config requires: {missing:?} (request them in \
             `optional_features` to go without them where they are missing)",
            adapter.get_info().name
        )
    })?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rinch-dom device"),
        required_features: features,
        required_limits: limits,
        memory_hints: MemoryHints::Manual {
            suballocated_device_memory_block_size: DEVICE_MEMORY_BLOCK,
        },
        trace: wgpu::Trace::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .map_err(|e| {
        format!(
            "Failed to create device (the adapter may not support the \
             features/limits requested via App::gpu_config): {e}"
        )
    })
}

// ── Present mode (#2095) ─────────────────────────────────────────────────────

/// The present mode a surface offering `offered` is configured with: with
/// vsync, `FifoRelaxed` where offered, else `Fifo` (wgpu's `AutoVsync`
/// order); without, `Immediate`, else `Mailbox`, else `Fifo` (wgpu's
/// `AutoNoVsync` order). Resolved here rather than left to wgpu's `Auto*`, so
/// the shell can say what the surface does. `Fifo` is the one mode every
/// surface supports.
pub(crate) fn choose_present_mode(vsync: bool, offered: &[PresentMode]) -> PresentMode {
    let has = |m: PresentMode| offered.contains(&m);
    if !vsync {
        return if has(PresentMode::Immediate) {
            PresentMode::Immediate
        } else if has(PresentMode::Mailbox) {
            PresentMode::Mailbox
        } else {
            PresentMode::Fifo
        };
    }
    if has(PresentMode::FifoRelaxed) {
        PresentMode::FifoRelaxed
    } else {
        PresentMode::Fifo
    }
}

/// What a configured present mode does, for the app.
pub(crate) fn presentation_of(mode: PresentMode) -> crate::window::Presentation {
    use crate::window::Presentation;
    match mode {
        PresentMode::Immediate => Presentation::Immediate,
        PresentMode::Mailbox => Presentation::Mailbox,
        // `Auto*` never reaches a configuration: `choose_present_mode`
        // resolves them.
        _ => Presentation::Vsync,
    }
}

/// Point a surface's configured `mode` at the vsync request: what the
/// surface does now, and whether the mode changed (the surface needs
/// configuring again). A request for what it already does changes nothing,
/// so it costs no reconfigure.
pub(crate) fn retarget_present_mode(
    mode: &mut PresentMode,
    vsync: bool,
    offered: &[PresentMode],
) -> (crate::window::Presentation, bool) {
    let wanted = choose_present_mode(vsync, offered);
    let changed = wanted != *mode;
    *mode = wanted;
    (presentation_of(wanted), changed)
}

/// A surface whose vsync the runtime switches (#2095): the GPU renderer, or a
/// fake in a test.
pub(crate) trait VsyncTarget {
    fn set_vsync(&mut self, on: bool) -> crate::window::Presentation;
}

impl VsyncTarget for WgpuRenderer {
    fn set_vsync(&mut self, on: bool) -> crate::window::Presentation {
        WgpuRenderer::set_vsync(self, on)
    }
}

/// Apply a vsync request to each of a runtime's surfaces that exists.
pub(crate) fn apply_vsync_to<'a, T: VsyncTarget + 'a>(
    on: bool,
    targets: impl IntoIterator<Item = Option<&'a mut T>>,
) {
    for target in targets.into_iter().flatten() {
        target.set_vsync(on);
    }
}

/// What a surface offering `offered` does with vsync on and with it off.
pub(crate) fn pacing_outcomes(offered: &[PresentMode]) -> crate::window::PacingOutcomes {
    crate::window::PacingOutcomes {
        on: presentation_of(choose_present_mode(true, offered)),
        off: presentation_of(choose_present_mode(false, offered)),
    }
}

/// A fully embedder-provided GPU stack for zero-copy present.
///
/// When an embedder would rather own device creation with its exact
/// `DeviceDescriptor`, it can hand the whole stack to rinch via
/// [`crate::App::external_gpu`]. rinch creates only the window surface
/// (from the provided `instance`) and validates that `adapter` can present to
/// it; it does **not** call `request_device`. The provided device becomes the
/// one published through [`gpu_handle`].
///
/// The `instance` must be the one the `adapter`/`device` were created from, so
/// that the window surface rinch creates is compatible with them.
#[derive(Clone)]
pub struct ExternalGpu {
    pub instance: Instance,
    pub adapter: Arc<Adapter>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
}

/// How the desktop `WgpuRenderer` should obtain its GPU device.
pub(crate) enum GpuInit {
    /// rinch creates the device with the given extra features/limits.
    /// Boxed: wgpu 30's `Limits` make the config much larger than `External`.
    Config(Box<RinchGpuConfig>),
    /// The embedder supplies the whole GPU stack.
    External(ExternalGpu),
}

static GPU_INIT: OnceLock<GpuInit> = OnceLock::new();

/// Install the GPU initialization strategy. Must be called before the runtime
/// creates its window (i.e. before `run_*`). A second call is ignored.
pub(crate) fn set_gpu_init(init: GpuInit) {
    let _ = GPU_INIT.set(init);
}

/// Whether the app configured the GPU device itself (`App::gpu_config` or
/// `App::external_gpu`). Such an app gets no fallback to software when the GPU
/// will not start (`shell::renderer::on_gpu_failure`).
pub(crate) fn gpu_init_installed() -> bool {
    GPU_INIT.get().is_some()
}

/// A GPU texture layer for zero-copy compositing.
///
/// Unlike [`CompositeLayer`] which carries CPU pixel data, this provides
/// a wgpu `TextureView` that the compositor reads directly — no upload needed.
pub struct GpuTextureLayer {
    /// The texture view to composite.
    pub view: TextureView,
    /// Viewport rectangle in physical pixels: (x, y, w, h).
    pub viewport: (f32, f32, f32, f32),
    /// Border radii in physical pixels: [tl, tr, br, bl].
    pub border_radius: [f32; 4],
    /// Optional clip rectangle from overflow ancestor in physical pixels: (x, y, w, h).
    pub clip_rect: Option<(f32, f32, f32, f32)>,
}

// ── WinitWindow ──────────────────────────────────────────────────────────────

/// Desktop window backed by winit.
pub struct WinitWindow {
    pub(crate) window: Arc<dyn Window>,
}

impl WinitWindow {
    pub fn new(window: Box<dyn Window>) -> Self {
        Self {
            window: Arc::from(window),
        }
    }

    /// Get the raw winit window reference.
    pub fn raw(&self) -> &dyn Window {
        &*self.window
    }
}

impl PlatformWindow for WinitWindow {
    fn inner_size(&self) -> (u32, u32) {
        let s = self.window.surface_size();
        (s.width, s.height)
    }

    fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn set_minimized(&self, minimized: bool) {
        self.window.set_minimized(minimized);
    }

    fn set_maximized(&self, maximized: bool) {
        self.window.set_maximized(maximized);
    }

    fn set_visible(&self, visible: bool) {
        self.window.set_visible(visible);
    }

    fn is_maximized(&self) -> bool {
        self.window.is_maximized()
    }

    fn drag_window(&self) -> Result<(), String> {
        self.window
            .drag_window()
            .map_err(|e| format!("drag_window failed: {e}"))
    }

    fn drag_resize_window(&self, direction: rinch_platform::ResizeDirection) -> Result<(), String> {
        use rinch_platform::ResizeDirection as RD;
        use winit::window::ResizeDirection as WRD;
        let wd = match direction {
            RD::North => WRD::North,
            RD::South => WRD::South,
            RD::East => WRD::East,
            RD::West => WRD::West,
            RD::NorthEast => WRD::NorthEast,
            RD::NorthWest => WRD::NorthWest,
            RD::SouthEast => WRD::SouthEast,
            RD::SouthWest => WRD::SouthWest,
        };
        self.window
            .drag_resize_window(wd)
            .map_err(|e| format!("drag_resize_window failed: {e}"))
    }

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }
}

// ── WgpuRenderer ─────────────────────────────────────────────────────────────

/// A surface on `window`, made from `instance`.
fn window_surface(instance: &Instance, window: &dyn Window) -> Result<Surface<'static>, String> {
    use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(
            window
                .display_handle()
                .map_err(|e| format!("no display handle for a GPU surface: {e}"))?
                .as_raw(),
        ),
        raw_window_handle: window
            .window_handle()
            .map_err(|e| format!("no window handle for a GPU surface: {e}"))?
            .as_raw(),
    };
    // SAFETY: The window outlives the surface — drop order in WgpuRenderer
    // ensures surface is dropped before the window, and a hide drops it
    // before the window it was made on (`WgpuRenderer::detach_surface`).
    unsafe { instance.create_surface_unsafe(target) }
        .map_err(|e| format!("Failed to create surface: {e}"))
}

/// How a window's `surface` is configured on `adapter`'s device: its format
/// (`Rgba8Unorm` where offered, else `Bgra8Unorm`, else the first), the
/// alpha mode for a `transparent` window, and vsync as the app asked
/// (#2095); with the present modes the surface offers.
fn surface_configuration(
    surface: &Surface<'static>,
    adapter: &Adapter,
    width: u32,
    height: u32,
    transparent: bool,
) -> Result<(SurfaceConfiguration, Vec<PresentMode>), String> {
    let caps = surface.get_capabilities(adapter);
    let alpha_mode = surface_alpha_mode(transparent, &caps.alpha_modes);
    let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
        TextureFormat::Rgba8Unorm
    } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
        TextureFormat::Bgra8Unorm
    } else {
        *caps.formats.first().ok_or_else(|| {
            format!(
                "the adapter {:?} offers no format for this window's surface",
                adapter.get_info().name
            )
        })?
    };
    let config = SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
        format,
        width,
        height,
        // Vsync on unless the app turned it off (#2095).
        present_mode: choose_present_mode(crate::window::vsync_requested(), &caps.present_modes),
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Auto,
    };
    Ok((config, caps.present_modes))
}

/// Desktop GPU renderer backed by wgpu + vello.
///
/// Field order matters for drop safety: resources that reference the device
/// (`renderer`, `render_texture`) must be declared before `device`/`queue`
/// so they are dropped first (Rust drops fields in declaration order).
/// `surface` must drop before the window handle it references is freed.
pub struct WgpuRenderer {
    pub(crate) renderer: VelloRenderer,
    pub(crate) render_texture: Texture,
    /// `None` only for a renderer made without a window, in tests.
    pub(crate) surface: Option<Surface<'static>>,
    pub(crate) surface_config: SurfaceConfiguration,
    pub(crate) device: Arc<Device>,
    pub(crate) queue: Arc<Queue>,
    /// Layer compositor (created lazily when composite layers are first set).
    pub(crate) layer_compositor: Option<super::compositor::LayerCompositor>,
    /// Frame uploader (reusable texture for decoded frames).
    pub(crate) layer_uploader: Option<super::frame_upload::FrameUploader>,
    /// Active composite layers to render underneath the UI.
    pub(crate) composite_layers: Vec<CompositeLayer>,
    /// Active GPU texture layers for zero-copy compositing.
    pub(crate) gpu_layers: Vec<GpuTextureLayer>,
    /// `RenderSurface` GPU frames registered with `renderer`, by surface id:
    /// the texture and the image it was registered as (#1768).
    surface_images: SurfaceImages,
    /// The split surfaces this window composites (#1865), their slices
    /// rendered; empty while nothing is split.
    pub(crate) split: super::split_frame::SplitFrame,
    pub(crate) split_compositor: Option<super::split_compositor::SplitCompositor>,
    /// Composited output texture (layers + UI) for screenshot capture.
    /// Created lazily, invalidated on resize.
    pub(crate) composited_texture: Option<Texture>,
    /// What `render_texture` holds (#1718). A partial frame updates it in
    /// place only while it holds a frame over the base colour this frame needs.
    ui_state: UiTextureState,
    /// The target a partial frame is rendered into, reused while it is big
    /// enough (sizes rounded up to [`PARTIAL_TARGET_STEP`]).
    pub(crate) partial_target: Option<Texture>,
    /// The present modes the surface offers (#2095).
    present_modes: Vec<PresentMode>,
    /// `surface_config.present_mode` changed since the surface was last
    /// configured: [`Self::present`] configures it before acquiring a frame.
    reconfigure: bool,
    /// Whether this window's presented frames go to the app's
    /// `on_frame_composited` handlers (#2215): the app's window, not the
    /// DevTools window.
    pub(crate) hands_over_frames: bool,
}

/// A partial frame's target grows in steps of this many device pixels, so a
/// hover that moves a few pixels reuses it (#1718).
const PARTIAL_TARGET_STEP: u32 = 128;

impl WgpuRenderer {
    /// Create a new GPU renderer for the given window.
    ///
    /// # Panics
    ///
    /// When the GPU cannot start; [`Self::try_new`] reports that instead.
    pub fn new(window: &dyn Window, width: u32, height: u32) -> Self {
        Self::try_new(window, width, height).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Create a new GPU renderer for the given window, or say why the GPU
    /// cannot present to it: no window handle, no surface, no adapter that can
    /// present to the surface, no device, or a validation error while
    /// configuring the surface and building Vello's pipelines. Nothing is
    /// published through [`gpu_handle`] unless it succeeds.
    ///
    /// # Panics
    ///
    /// With [`crate::App::external_gpu`], when the supplied adapter cannot
    /// present to the window surface: that is a configuration error in the
    /// app, not a missing GPU.
    pub fn try_new(window: &dyn Window, width: u32, height: u32) -> Result<Self, String> {
        Self::try_new_transparent(window, width, height, false)
    }

    /// [`Self::try_new`] for a window that may be transparent
    /// (`WindowProps::transparent`): the surface is then composited with its
    /// alpha, where the platform allows, so what the app leaves unpainted
    /// shows the desktop behind it. `wgpu::CompositeAlphaMode::Auto` picks
    /// `Opaque` on Vulkan, which painted it black.
    pub fn try_new_transparent(
        window: &dyn Window,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<Self, String> {
        Self::try_new_for(window, width, height, transparent, Renderer::Gpu)
            .map_err(|e| e.to_string())
    }

    /// [`Self::try_new_transparent`] for the renderer `choice`, which decides
    /// whether a software adapter is used ([`renderer::adapter_use`], #2436):
    /// under [`Renderer::Auto`] a CPU adapter is declined as soon as wgpu
    /// offers it, before a device or Vello's pipelines are made, so the
    /// window presents with rinch's software renderer without paying the
    /// software driver's start-up first. [`Renderer::Gpu`] uses any adapter.
    pub(crate) fn try_new_for(
        window: &dyn Window,
        width: u32,
        height: u32,
        transparent: bool,
        choice: Renderer,
    ) -> Result<Self, GpuStartError> {
        let width = width.max(1);
        let height = height.max(1);

        let gpu_init = GPU_INIT.get();
        // Every window presents with one device: the embedder's, or the one
        // rinch made for its first window. A window re-created by hide/show
        // used to request a second device while `gpu_handle()` kept the
        // first, so a `RenderSurface` GPU frame was drawn on one device and
        // registered with a renderer on the other.
        let shared = shared_gpu_stack(gpu_init);
        let external = matches!(gpu_init, Some(GpuInit::External(_)));

        // Instance: reuse the embedder's when an external device is supplied
        // (the surface must come from the instance the adapter/device belong to);
        // otherwise create rinch's own.
        //
        // Default to GPU-only backends (Vulkan/Metal/DX12). Skip GL/GLES probing
        // which loads Mesa gallium + LLVM (~70MB RSS) but can't run Vello anyway
        // (Vello requires compute shaders). WGPU_BACKEND env var still overrides.
        let instance = match &shared {
            Some((instance, ..)) => instance.clone(),
            None => {
                let backends = Backends::from_env()
                    .unwrap_or(Backends::VULKAN | Backends::METAL | Backends::DX12);
                Instance::new(InstanceDescriptor {
                    backends,
                    flags: wgpu::InstanceFlags::from_build_config().with_env(),
                    backend_options: wgpu::BackendOptions::from_env_or_default(),
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                    display: None,
                })
            }
        };

        let surface = window_surface(&instance, window)?;

        // Adapter: adopt the embedder's, or pick a surface-compatible one.
        let adapter: Arc<Adapter> = match &shared {
            Some((_, adapter, ..)) if external => {
                assert!(
                    adapter.is_surface_supported(&surface),
                    "App::external_gpu: the supplied adapter cannot present to \
                     rinch's window surface. The adapter/device must be created from an \
                     adapter that supports the target window (create the adapter with a \
                     compatible surface, or use App::gpu_config to let rinch own the device)."
                );
                adapter.clone()
            }
            Some((_, adapter, ..)) => {
                if !adapter.is_surface_supported(&surface) {
                    return Err(format!(
                        "the shared GPU adapter {:?} cannot present to this window",
                        adapter.get_info().name
                    )
                    .into());
                }
                adapter.clone()
            }
            None => Arc::new(
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                }))
                .map_err(|e| format!("Failed to find adapter: {e}"))?,
            ),
        };

        let info = adapter.get_info();
        if renderer::adapter_use(choice, gpu_init.is_some(), &info) == AdapterUse::Decline {
            return Err(GpuStartError::DeclinedCpuAdapter(Box::new(info)));
        }

        let (surface_config, present_modes) =
            surface_configuration(&surface, &adapter, width, height, transparent)?;
        let format = surface_config.format;

        // Device/Queue: adopt the embedder's, or create one — optionally with
        // the embedder's extra features/limits (issue #57) and the optional
        // features the adapter has (#2085). On surfaces that only offer
        // Bgra8Unorm (X11 on NVIDIA, notably) `BGRA8UNORM_STORAGE` is added
        // where the adapter has it: any storage-texture use of the surface
        // format needs it.
        let (device, queue) = match (&shared, gpu_init) {
            (Some((_, _, device, queue)), _) => (device.clone(), queue.clone()),
            (None, other) => {
                let config = match other {
                    Some(GpuInit::Config(cfg)) => Some(&**cfg),
                    _ => None,
                };
                let (raw_device, raw_queue) = request_rinch_device(&adapter, format, config)?;
                (Arc::new(raw_device), Arc::new(raw_queue))
            }
        };

        // wgpu reports errors in what follows (configuring the surface,
        // creating the texture, compiling Vello's shaders) through the device's
        // error handler, which panics by default. Scope them so a device that
        // cannot do the work is an `Err` like every step above.
        let out_of_memory_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        surface.configure(&device, &surface_config);

        // Vello's compute shaders only write rgba8unorm storage textures, so
        // the intermediate render texture is ALWAYS Rgba8Unorm regardless of
        // the surface format. When the surface is also Rgba8Unorm we present
        // with a plain copy; otherwise (e.g. Bgra8Unorm — X11/NVIDIA surfaces
        // commonly offer nothing else) render_scene blits through the layer
        // compositor's sampling pass, which swizzles for free.
        let render_texture =
            Self::create_render_texture(&device, TextureFormat::Rgba8Unorm, width, height);

        let renderer =
            crate::gpu_init::new_renderer(&device, crate::gpu_antialiasing::scene_aa_support());

        let validation = pollster::block_on(validation_scope.pop());
        let out_of_memory = pollster::block_on(out_of_memory_scope.pop());
        if let Some(e) = validation.or(out_of_memory) {
            return Err(format!("the GPU device cannot present this window: {e}").into());
        }
        let renderer = renderer.map_err(|e| format!("Failed to create Vello renderer: {e}"))?;

        // Publish the shared GPU handle for external renderers, now that the
        // device is known to work.
        if GPU_HANDLE
            .set(GpuHandle {
                device: device.clone(),
                queue: queue.clone(),
                adapter: adapter.clone(),
            })
            .is_ok()
            && !external
        {
            let _ = RINCH_INSTANCE.set(instance.clone());
        }

        renderer::log_adapter(&info, format);

        Ok(Self {
            renderer,
            render_texture,
            surface: Some(surface),
            surface_config,
            device,
            queue,
            layer_compositor: None,
            layer_uploader: None,
            composite_layers: Vec::new(),
            surface_images: SurfaceImages::default(),
            split: Default::default(),
            split_compositor: None,
            gpu_layers: Vec::new(),
            composited_texture: None,
            ui_state: UiTextureState::default(),
            partial_target: None,
            present_modes,
            reconfigure: false,
            hands_over_frames: false,
        })
    }

    /// Stop presenting to the window this renderer was made for, which is
    /// about to be destroyed (a hide, #2545): its surface goes, and so do
    /// the compositors, the composited texture and the partial-frame target
    /// (each made again when first needed, and each up to a surface's worth
    /// of GPU memory while the window is hidden). Vello's renderer and its
    /// pipelines, what a show would otherwise build again, stay for
    /// [`Self::attach_window`] to present with at the next show.
    pub(crate) fn detach_surface(&mut self) {
        self.surface = None;
        self.layer_compositor = None;
        self.split_compositor = None;
        self.composited_texture = None;
        self.partial_target = None;
    }

    /// Present to `window` with this renderer, which [`Self::detach_surface`]
    /// took off a window (#2545): a surface is made for `window` from the
    /// instance the renderer's device came from and configured as
    /// [`Self::try_new_for`] configures one, and nothing else is built again.
    /// Building Vello's renderer is most of what a new `WgpuRenderer` costs
    /// (about 52 M instructions, with the shader translation cached),
    /// against a few for a surface. The next frame is a full one, as a new
    /// renderer's first is.
    ///
    /// `Err` when the shared adapter cannot present to the new window or the
    /// surface cannot be configured; the renderer is left detached, and the
    /// caller builds a new one.
    pub(crate) fn attach_window(
        &mut self,
        window: &dyn Window,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<(), String> {
        let (width, height) = (width.max(1), height.max(1));
        let (instance, adapter, ..) = shared_gpu_stack(GPU_INIT.get())
            .ok_or("no GPU device is shared yet to present with")?;
        let surface = window_surface(&instance, window)?;
        if !adapter.is_surface_supported(&surface) {
            return Err(format!(
                "the shared GPU adapter {:?} cannot present to this window",
                adapter.get_info().name
            ));
        }
        let (config, present_modes) =
            surface_configuration(&surface, &adapter, width, height, transparent)?;
        let out_of_memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(&self.device, &config);
        let validation = pollster::block_on(validation_scope.pop());
        let out_of_memory = pollster::block_on(out_of_memory_scope.pop());
        if let Some(e) = validation.or(out_of_memory) {
            return Err(format!("the GPU device cannot present this window: {e}"));
        }
        let resized = (config.width, config.height)
            != (self.surface_config.width, self.surface_config.height);
        self.surface = Some(surface);
        self.surface_config = config;
        self.present_modes = present_modes;
        self.reconfigure = false;
        if resized {
            self.render_texture =
                Self::create_render_texture(&self.device, TextureFormat::Rgba8Unorm, width, height);
        }
        // What the document changed while hidden was never painted into the
        // UI texture: the first frame on the new surface is a full one.
        self.ui_state = UiTextureState::default();
        Ok(())
    }

    /// A renderer with no window, for tests that drive [`Self::compose`]
    /// with a texture standing in for the swapchain's (#2215): `format` is
    /// the surface format it pretends to have, opaque.
    #[cfg(test)]
    pub(crate) fn offscreen(
        device: Arc<Device>,
        queue: Arc<Queue>,
        width: u32,
        height: u32,
        format: TextureFormat,
    ) -> Self {
        let renderer =
            crate::gpu_init::new_renderer(&device, crate::gpu_antialiasing::scene_aa_support())
                .expect("vello");
        let render_texture =
            Self::create_render_texture(&device, TextureFormat::Rgba8Unorm, width, height);
        Self {
            renderer,
            render_texture,
            surface: None,
            surface_config: SurfaceConfiguration {
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
                format,
                width,
                height,
                present_mode: PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                color_space: wgpu::SurfaceColorSpace::Auto,
            },
            device,
            queue,
            layer_compositor: None,
            layer_uploader: None,
            composite_layers: Vec::new(),
            surface_images: SurfaceImages::default(),
            split: Default::default(),
            split_compositor: None,
            gpu_layers: Vec::new(),
            composited_texture: None,
            ui_state: UiTextureState::default(),
            partial_target: None,
            present_modes: vec![PresentMode::Fifo],
            reconfigure: false,
            hands_over_frames: false,
        }
    }

    /// Turn vsync on or off for this window's surface (#2095), and say what
    /// the surface does now. The surface is configured with the new mode at
    /// the next `Self::present`, before it acquires a frame, so no frame is
    /// held while it changes; the UI texture, the split slices and the frames
    /// Vello has in flight are not touched, and the next present shows the
    /// UI texture as it is.
    pub fn set_vsync(&mut self, on: bool) -> crate::window::Presentation {
        let (presentation, changed) = retarget_present_mode(
            &mut self.surface_config.present_mode,
            on,
            &self.present_modes,
        );
        self.reconfigure |= changed;
        presentation
    }

    /// What this window's surface does with vsync on and with it off.
    pub(crate) fn pacing_outcomes(&self) -> crate::window::PacingOutcomes {
        pacing_outcomes(&self.present_modes)
    }

    fn create_render_texture(
        device: &Device,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch-dom render texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            // COPY_DST: a partial frame's rects are copied in (#1718).
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Paint the given scene to the window surface.
    pub fn paint(&mut self, scene: &Scene, transparent: bool) -> Result<(), String> {
        let base_color = if transparent {
            Color::TRANSPARENT
        } else {
            Color::WHITE
        };
        self.render_scene(
            scene,
            self.surface_config.width,
            self.surface_config.height,
            base_color,
        )
        .map_err(|e| format!("{e}"))
    }

    /// The colour Vello renders the UI over this frame: transparent under
    /// composite layers (they show through), else the window's base.
    fn vello_base_color(&self, base_color: Color) -> Color {
        ui_base_color(
            base_color,
            !self.composite_layers.is_empty(),
            !self.gpu_layers.is_empty(),
        )
    }

    fn window_base_color(transparent: bool) -> Color {
        if transparent {
            Color::TRANSPARENT
        } else {
            Color::WHITE
        }
    }

    /// Why the UI texture cannot take a partial frame this frame (#1718):
    /// it holds no frame, or one rendered over another base colour (layers
    /// came or went). `None` when it holds the last frame painted.
    pub fn ui_invalid_reason(
        &self,
        transparent: bool,
    ) -> Option<rinch_dom::perf::FullRepaintReason> {
        self.ui_state
            .invalid_reason(self.vello_base_color(Self::window_base_color(transparent)))
    }

    /// What has become of the frames Vello was given (#2071), without waiting
    /// for the GPU. A frame that overflowed Vello's buffers reached the UI
    /// texture or a slice undrawn, so [`Self::ui_invalid_reason`] then asks for
    /// a full frame, which paints every split slice again as well. It takes a
    /// redraw to get one: the caller requests that on
    /// [`Lost`](vello::FrameCheck::Lost), and on
    /// [`Pending`](vello::FrameCheck::Pending) looks again later.
    pub fn check_frames(&mut self) -> vello::FrameCheck {
        let check = self.renderer.check_frames(&self.device);
        self.ui_state.note_check(check);
        check
    }

    /// Render and present a frame [`crate::app::RinchApp::build_gpu_frame`]
    /// built (#1718). A full frame renders the whole UI texture; a partial
    /// one renders its scene into a target the size of its damage and copies
    /// each damage rect into the UI texture; an unchanged one renders
    /// nothing. Each then presents the UI texture (over the layers).
    pub fn paint_frame(
        &mut self,
        frame: crate::app::GpuFrame<'_>,
        transparent: bool,
    ) -> Result<(), String> {
        use crate::app::GpuFrame;
        let base_color = Self::window_base_color(transparent);
        let (w, h) = (self.surface_config.width, self.surface_config.height);
        match frame {
            GpuFrame::Full(scene) => self.render_ui(scene, w, h, base_color),
            GpuFrame::Partial { scene, size, rects } => {
                self.render_ui_partial(scene, size, &rects, base_color)
            }
            GpuFrame::Unchanged => Ok(()),
        }
        .and_then(|()| self.present(base_color))
        .map_err(|e| format!("{e}"))
    }

    /// Render `scene` into the whole UI texture.
    fn render_ui(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<(), RenderError> {
        let base = self.vello_base_color(base_color);
        let view = self
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.ui_state.begin_render();
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &view,
                &RenderParams {
                    base_color: base,
                    width,
                    height,
                    antialiasing_method: crate::gpu_antialiasing::scene_aa(),
                },
            )
            .map_err(|e| RenderError::Internal(format!("render_to_texture failed: {:?}", e)))?;
        self.ui_state.rendered(base);
        Ok(())
    }

    /// Render a partial frame into the UI texture: see [`render_partial`].
    fn render_ui_partial(
        &mut self,
        scene: &Scene,
        size: (u32, u32),
        rects: &[(u32, u32, u32, u32)],
        base_color: Color,
    ) -> Result<(), RenderError> {
        let base = self.vello_base_color(base_color);
        debug_assert_eq!(
            self.ui_state.invalid_reason(base),
            None,
            "a partial frame over an invalid UI texture"
        );
        self.ui_state.begin_render();
        render_partial(
            &self.device,
            &self.queue,
            &mut self.renderer,
            &self.render_texture,
            &mut self.partial_target,
            PartialFrame {
                scene,
                size,
                rects,
                base,
            },
        )?;
        self.ui_state.rendered(base);
        Ok(())
    }
}

/// Whether the UI texture holds the last frame painted, which a partial or
/// unchanged frame builds on (#1718), and if not, why not.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiTextureState {
    /// The colour the frame it holds was rendered over; `None` when it holds
    /// no good frame.
    base: Option<Color>,
    /// Why `base` was last cleared, for the full frame that repairs it.
    lost: rinch_dom::perf::FullRepaintReason,
}

impl Default for UiTextureState {
    fn default() -> Self {
        Self {
            base: None,
            lost: rinch_dom::perf::FullRepaintReason::FirstFrame,
        }
    }
}

impl UiTextureState {
    /// Why it cannot take a partial frame over `base`: it holds no frame, or
    /// one rendered over another base colour (layers came or went).
    pub(crate) fn invalid_reason(&self, base: Color) -> Option<rinch_dom::perf::FullRepaintReason> {
        match self.base {
            Some(b) if b == base => None,
            Some(_) => Some(rinch_dom::perf::FullRepaintReason::Invalidated),
            None => Some(self.lost),
        }
    }

    /// A render into it starts; a failure leaves it holding no good frame.
    pub(crate) fn begin_render(&mut self) {
        self.base = None;
        self.lost = rinch_dom::perf::FullRepaintReason::Invalidated;
    }

    /// A render over `base` was submitted.
    pub(crate) fn rendered(&mut self, base: Color) {
        self.base = Some(base);
    }

    /// The texture was replaced at a new size.
    fn resized(&mut self) {
        self.base = None;
        self.lost = rinch_dom::perf::FullRepaintReason::Resize;
    }

    /// What Vello said of the frames it was given (#2071): a lost one went
    /// out undrawn, here or in a slice, so the next frame has to be full.
    pub(crate) fn note_check(&mut self, check: vello::FrameCheck) {
        if check == vello::FrameCheck::Lost {
            self.begin_render();
        }
    }
}

/// A partial frame for [`render_partial`].
pub(crate) struct PartialFrame<'a> {
    pub scene: &'a Scene,
    pub size: (u32, u32),
    pub rects: &'a [(u32, u32, u32, u32)],
    pub base: Color,
}

/// Render a partial frame's `scene` into `partial_target` (grown in
/// [`PARTIAL_TARGET_STEP`]s when smaller than `size`, never past `ui`) and
/// copy each of its `rects` from there into `ui` (#1718). The scene is drawn
/// where a full frame draws it, so each rect's pixels are a full render's
/// (`gpu_partial_frame_1718_tests`).
pub(crate) fn render_partial(
    device: &Device,
    queue: &Queue,
    renderer: &mut VelloRenderer,
    ui: &Texture,
    partial_target: &mut Option<Texture>,
    frame: PartialFrame<'_>,
) -> Result<(), RenderError> {
    let PartialFrame {
        scene,
        size,
        rects,
        base,
    } = frame;
    let (uw, uh) = (ui.width(), ui.height());
    let step =
        |v: u32, max: u32| (v.div_ceil(PARTIAL_TARGET_STEP).max(1) * PARTIAL_TARGET_STEP).min(max);
    let (want_w, want_h) = (step(size.0, uw), step(size.1, uh));
    let (have_w, have_h) = partial_target
        .as_ref()
        .map_or((0, 0), |t| (t.width(), t.height()));
    if have_w < want_w || have_h < want_h {
        *partial_target = Some(WgpuRenderer::create_render_texture(
            device,
            TextureFormat::Rgba8Unorm,
            want_w.max(have_w),
            want_h.max(have_h),
        ));
    }
    let target = partial_target.as_ref().expect("created above");
    // Rendered over the part of the target the frame needs, which a target
    // grown for an earlier, larger frame may exceed.
    let (tw, th) = (want_w, want_h);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &view,
            &RenderParams {
                base_color: base,
                width: tw,
                height: th,
                antialiasing_method: crate::gpu_antialiasing::scene_aa(),
            },
        )
        .map_err(|e| RenderError::Internal(format!("render_to_texture failed: {:?}", e)))?;
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("rinch-dom partial frame copy"),
    });
    for &(x, y, rw, rh) in rects {
        // Clamped to both textures: the surface may have been resized
        // between the app's frame and this render.
        let rw = rw.min(uw.saturating_sub(x)).min(tw.saturating_sub(x));
        let rh = rh.min(uh.saturating_sub(y)).min(th.saturating_sub(y));
        if rw == 0 || rh == 0 {
            continue;
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: ui,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: rw,
                height: rh,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit(Some(encoder.finish()));
    Ok(())
}

impl WgpuRenderer {
    /// Present the UI texture (over the composite layers, if any) to the
    /// window surface. Vello has already rendered into it: a surface that
    /// cannot be acquired this frame skips the present, and the UI texture
    /// still holds the frame for the next one.
    fn present(&mut self, base_color: Color) -> Result<(), RenderError> {
        use wgpu::CurrentSurfaceTexture as Current;
        // A new present mode (#2095), configured while this window holds no
        // frame of the surface's.
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };
        if std::mem::take(&mut self.reconfigure) {
            surface.configure(&self.device, &self.surface_config);
        }
        let surface_texture = match surface.get_current_texture() {
            Current::Success(t) | Current::Suboptimal(t) => t,
            // Swapchain went stale (resize/DPI/occlusion/GPU reset) — recreate it and retry once.
            Current::Outdated | Current::Lost => {
                surface.configure(&self.device, &self.surface_config);
                match surface.get_current_texture() {
                    Current::Success(t) | Current::Suboptimal(t) => t,
                    _ => return Ok(()), // try again on the next redraw
                }
            }
            // Transient — skip this frame.
            Current::Timeout | Current::Occluded => return Ok(()),
            Current::Validation => {
                return Err(RenderError::Internal(
                    "get_current_texture: validation error".to_string(),
                ));
            }
        };

        self.compose(&surface_texture.texture, base_color)?;
        self.queue.present(surface_texture);
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| RenderError::Internal(format!("GPU poll failed: {:?}", e)))?;

        Ok(())
    }

    /// Composite the frame into `target` (the swapchain texture, or a test's
    /// stand-in for it), submit it, and hand the presented texture to the
    /// app's `on_frame_composited` handlers (#2215). Everything
    /// [`Self::present`] does between acquiring the surface texture and
    /// presenting it.
    pub(crate) fn compose(
        &mut self,
        target: &Texture,
        base_color: Color,
    ) -> Result<(), RenderError> {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rinch-dom copy encoder"),
            });

        let has_any_layers = !self.composite_layers.is_empty() || !self.gpu_layers.is_empty();
        let split = !self.split.is_empty();

        if has_any_layers {
            // Composite layers + UI using the compositor pipeline.
            // We render to a composited_texture (not directly to swapchain)
            // so that screenshot capture can read the final composited result.
            let compositor = self.layer_compositor.get_or_insert_with(|| {
                super::compositor::LayerCompositor::new(&self.device, self.surface_config.format)
            });
            let uploader = self
                .layer_uploader
                .get_or_insert_with(super::frame_upload::FrameUploader::new);

            // Create/reuse composited texture (RENDER_ATTACHMENT + COPY_SRC)
            let sw = self.surface_config.width;
            let sh = self.surface_config.height;
            let composited = self.composited_texture.get_or_insert_with(|| {
                self.device.create_texture(&TextureDescriptor {
                    label: Some("composited texture"),
                    size: Extent3d {
                        width: sw,
                        height: sh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: self.surface_config.format,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_SRC
                        | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            });
            let composited_view = composited.create_view(&wgpu::TextureViewDescriptor::default());

            // Phase 1a: Upload CPU pixel layer frames and blit into viewport regions.
            // First layer clears to base_color, subsequent layers load (preserve prior).
            let wgpu_base_color = wgpu::Color {
                r: base_color.components[0] as f64,
                g: base_color.components[1] as f64,
                b: base_color.components[2] as f64,
                a: base_color.components[3] as f64,
            };
            let mut layer_idx = 0usize;
            for layer in self.composite_layers.iter() {
                let layer_texture = uploader.upload(
                    layer_idx,
                    &self.device,
                    &self.queue,
                    &layer.pixels,
                    layer.width,
                    layer.height,
                );

                compositor.blit_layer(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    layer_texture,
                    &composited_view,
                    layer.viewport,
                    (self.surface_config.width, self.surface_config.height),
                    if layer_idx == 0 {
                        Some(wgpu_base_color)
                    } else {
                        None
                    },
                    layer.border_radius,
                    layer.clip_rect,
                );
                layer_idx += 1;
            }

            // Phase 1b: Blit GPU texture layers directly (zero-copy, no upload).
            for gpu_layer in &self.gpu_layers {
                compositor.blit_layer_view(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &gpu_layer.view,
                    &composited_view,
                    gpu_layer.viewport,
                    (self.surface_config.width, self.surface_config.height),
                    if layer_idx == 0 {
                        Some(wgpu_base_color)
                    } else {
                        None
                    },
                    gpu_layer.border_radius,
                    gpu_layer.clip_rect,
                );
                layer_idx += 1;
            }

            // Phase 2: Alpha-blend Vello UI on top of all layers.
            compositor.overlay_ui(
                &self.device,
                &mut encoder,
                &self.render_texture,
                &composited_view,
            );

            // Copy the composited result (premultiplied) to the swapchain,
            // converted for the surface's alpha mode (#1523).
            match present_step(self.surface_config.alpha_mode, true, true) {
                PresentStep::Copy => encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: composited,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    Extent3d {
                        width: self.surface_config.width,
                        height: self.surface_config.height,
                        depth_or_array_layers: 1,
                    },
                ),
                PresentStep::Pass(alpha) => {
                    let surface_view = target.create_view(&wgpu::TextureViewDescriptor::default());
                    compositor.present(
                        &self.device,
                        &mut encoder,
                        composited,
                        &surface_view,
                        alpha,
                    );
                }
            }
        } else if split {
            // Split surfaces (#1865): below (the UI texture), each surface and
            // its above slices, composited into the composited texture, which
            // is presented as the layered path presents it.
            let sw = self.surface_config.width;
            let sh = self.surface_config.height;
            let format = self.surface_config.format;
            let composited = self.composited_texture.get_or_insert_with(|| {
                self.device.create_texture(&TextureDescriptor {
                    label: Some("composited texture"),
                    size: Extent3d {
                        width: sw,
                        height: sh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_SRC
                        | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            });
            let compositor = self.split_compositor.get_or_insert_with(|| {
                super::split_compositor::SplitCompositor::new(&self.device, format)
            });
            let below = self
                .render_texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            self.split.composite(
                compositor,
                &self.device,
                &mut encoder,
                &below,
                &composited.create_view(&wgpu::TextureViewDescriptor::default()),
                (sw, sh),
                &self.surface_images.textures(),
            );
            match present_step(self.surface_config.alpha_mode, true, true) {
                PresentStep::Copy => encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: composited,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    Extent3d {
                        width: sw,
                        height: sh,
                        depth_or_array_layers: 1,
                    },
                ),
                PresentStep::Pass(alpha) => {
                    let present = self.layer_compositor.get_or_insert_with(|| {
                        super::compositor::LayerCompositor::new(&self.device, format)
                    });
                    let surface_view = target.create_view(&wgpu::TextureViewDescriptor::default());
                    present.present(&self.device, &mut encoder, composited, &surface_view, alpha);
                }
            }
        } else {
            // Standard path (no layers). The composited texture is no longer
            // presented from: let it go (a window-sized texture), so nothing reads
            // the last frame that had layers as the window's (#1799).
            self.composited_texture = None;

            // The render texture is always Rgba8Unorm
            // (Vello's storage-texture requirement) and holds straight alpha; a
            // raw copy is only legal when the surface matches and wants straight
            // (or opaque) pixels. Otherwise one full-surface pass through the
            // compositor converts the channel order and, for a premultiplied
            // surface, the alpha (#1523).
            let rgba_surface = self.surface_config.format == TextureFormat::Rgba8Unorm;
            match present_step(self.surface_config.alpha_mode, false, rgba_surface) {
                PresentStep::Copy => encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.render_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    Extent3d {
                        width: self.surface_config.width,
                        height: self.surface_config.height,
                        depth_or_array_layers: 1,
                    },
                ),
                PresentStep::Pass(alpha) => {
                    let compositor = self.layer_compositor.get_or_insert_with(|| {
                        super::compositor::LayerCompositor::new(
                            &self.device,
                            self.surface_config.format,
                        )
                    });
                    let surface_view = target.create_view(&wgpu::TextureViewDescriptor::default());
                    compositor.present(
                        &self.device,
                        &mut encoder,
                        &self.render_texture,
                        &surface_view,
                        alpha,
                    );
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        // The app's handlers get the texture the window is presented from,
        // after the frame's commands and before the present (#2215). No copy
        // is made for them: the UI texture is the whole frame (a partial
        // frame updated it in place, #1718), and the composited one is what
        // the layered and split paths present.
        if self.hands_over_frames {
            let (texture, alpha) = presented_frame(
                self.composited_texture.as_ref(),
                &self.render_texture,
                has_any_layers || split,
            );
            crate::composited_frame::hand_over_frame(&self.device, &self.queue, texture, alpha);
        }
        Ok(())
    }
}

impl PlatformRenderer for WgpuRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.surface_config.width = width;
        self.surface_config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.surface_config);
        }
        self.reconfigure = false;
        self.render_texture =
            Self::create_render_texture(&self.device, TextureFormat::Rgba8Unorm, width, height);
        self.composited_texture = None;
        self.ui_state.resized();
    }

    fn render_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<(), RenderError> {
        self.render_ui(scene, width, height, base_color)?;
        self.present(base_color)
    }

    fn set_composite_layers(&mut self, layers: Vec<CompositeLayer>) {
        self.composite_layers = layers;
    }

    fn has_composite_layers(&self) -> bool {
        !self.composite_layers.is_empty() || !self.gpu_layers.is_empty()
    }

    fn capture_screenshot(&self) -> Result<(u32, u32, Vec<u8>), RenderError> {
        #[cfg(feature = "debug")]
        {
            let w = self.surface_config.width;
            let h = self.surface_config.height;

            // The texture the window is presented from: the composited one
            // (layers + UI blended) while there are layers, else the UI
            // texture (#1799: the composited texture outlives its layers).
            let tex = presented_texture(
                self.composited_texture.as_ref(),
                &self.render_texture,
                self.has_composite_layers() || !self.split.is_empty(),
            );

            // Ask the texture, not the surface. `composited_texture` is created
            // in the surface format, but `render_texture` is ALWAYS Rgba8Unorm
            // (Vello's storage-texture requirement) even when the surface is
            // Bgra8Unorm — so using `surface_config.format` here made
            // `capture_texture_rgba` apply a BGRA→RGBA swizzle to bytes that
            // were already RGBA, and every screenshot on an X11/Bgra8Unorm
            // surface came out with red and blue swapped while the window on
            // screen was correct.
            let fmt = tex.format();

            let rgba =
                super::screenshot::capture_texture_rgba(&self.device, &self.queue, tex, w, h, fmt)
                    .map_err(RenderError::Internal)?;
            Ok((w, h, rgba))
        }
        #[cfg(not(feature = "debug"))]
        {
            Err(RenderError::Internal(
                "Screenshot requires the 'debug' feature".into(),
            ))
        }
    }
}

impl WgpuRenderer {
    /// Set GPU texture layers for zero-copy compositing.
    ///
    /// These layers are composited alongside CPU pixel layers but skip
    /// the upload step — the compositor reads the TextureView directly.
    pub fn set_gpu_layers(&mut self, layers: Vec<GpuTextureLayer>) {
        self.gpu_layers = layers;
    }

    /// Apply a painted frame's split change (#1865): render the new slices.
    pub fn apply_split(&mut self, change: super::split_frame::SplitChange) -> Result<(), String> {
        self.split
            .apply(&self.device, &self.queue, &mut self.renderer, change)
    }

    /// The split surfaces whose texture changed size under their slice.
    pub fn split_stale_ids(&self) -> Vec<usize> {
        self.split.stale_ids(&self.surface_images.textures())
    }

    /// The surfaces this window composites split, by id.
    pub fn split_surface_ids(&self) -> Vec<usize> {
        self.split.surface_ids().collect()
    }

    /// Register this paint's `RenderSurface` GPU frames with the Vello
    /// renderer and hand back each one's image, by surface id (#1768); see
    /// `SurfaceImages::sync`.
    pub fn sync_surface_images(
        &mut self,
        frames: impl Iterator<Item = (usize, Texture)>,
    ) -> std::collections::HashMap<usize, vello::peniko::ImageData> {
        self.surface_images.sync(&mut self.renderer, frames)
    }
}

/// The texture a window is presented from, which a debug screenshot reads:
/// the composited texture (layers + UI) while there are layers, else the UI
/// texture.
///
/// The composited texture is made when layers first arrive and outlived them:
/// a screenshot taken after a `GameViewport`'s or named surface's subtree
/// unmounted read it, and showed that subtree's last frame for good, while
/// the window itself was presented from the UI texture and was right (#1799).
#[cfg(any(feature = "debug", test))]
pub(crate) fn presented_texture<'a, T: ?Sized>(
    composited: Option<&'a T>,
    ui: &'a T,
    layers: bool,
) -> &'a T {
    presented_frame(composited, ui, layers).0
}

/// [`presented_texture`] and how its colour carries alpha (#2215): the
/// composited texture holds premultiplied colour, the UI texture Vello's
/// straight colour. What `on_frame_composited` hands over.
pub(crate) fn presented_frame<'a, T: ?Sized>(
    composited: Option<&'a T>,
    ui: &'a T,
    layers: bool,
) -> (&'a T, crate::composited_frame::FrameAlpha) {
    use crate::composited_frame::FrameAlpha;
    match composited {
        Some(c) if layers => (c, FrameAlpha::Premultiplied),
        _ => (ui, FrameAlpha::Straight),
    }
}

/// The colour Vello renders the UI over: transparent under composite layers
/// (CPU-pixel layers and zero-copy GPU texture layers), so they show through
/// the UI's viewport holes, else the window's base.
///
/// It used to look at the CPU layers alone, so a window whose only layer was a
/// GPU texture (a `GpuTextureRegistrar` surface) rendered its UI over opaque
/// white, which covered the layer: the hole showed white unless the window was
/// transparent, whose base is transparent anyway (#1774).
pub(crate) fn ui_base_color(window_base: Color, cpu_layers: bool, gpu_layers: bool) -> Color {
    if cpu_layers || gpu_layers {
        Color::TRANSPARENT
    } else {
        window_base
    }
}

/// The `RenderSurface` GPU frames registered with one Vello renderer, by
/// surface id: the texture and the image it was registered as (#1768).
#[derive(Default)]
pub(crate) struct SurfaceImages {
    by_surface: std::collections::HashMap<usize, (Texture, vello::peniko::ImageData)>,
}

impl SurfaceImages {
    /// Each registered surface's texture, by id.
    pub(crate) fn textures(&self) -> std::collections::HashMap<usize, Texture> {
        self.by_surface
            .iter()
            .map(|(id, (texture, _))| (*id, texture.clone()))
            .collect()
    }

    /// Register `frames` with `renderer` and hand back each one's image, by
    /// surface id. A surface keeps its registration while its texture is the
    /// same one; a new texture (a resize) is registered again and the old
    /// one dropped, as is every surface missing from `frames`. The images are
    /// drawn premultiplied, as a compositor layer's pixels were read.
    pub(crate) fn sync(
        &mut self,
        renderer: &mut VelloRenderer,
        frames: impl Iterator<Item = (usize, Texture)>,
    ) -> std::collections::HashMap<usize, vello::peniko::ImageData> {
        let mut now = std::collections::HashMap::new();
        for (id, texture) in frames {
            let image = match self.by_surface.remove(&id) {
                Some((kept, image)) if kept == texture => {
                    // Vello 0.9+ keeps an image resident in its atlas and
                    // copies an override texture again only when told to
                    // (Vello 0.7 copied every one each render). The surface
                    // may have drawn into this texture since the last render,
                    // so mark it (#1955).
                    renderer.mark_override_image_dirty(&image);
                    image
                }
                old => {
                    if let Some((_, image)) = old {
                        renderer.unregister_texture(image);
                    }
                    let mut image = renderer.register_texture(texture.clone());
                    image.alpha_type = vello::peniko::ImageAlphaType::AlphaPremultiplied;
                    image
                }
            };
            now.insert(id, (texture, image));
        }
        // A dropped registration's atlas space is taken by the next render
        // that needs it: the patched `vello_encoding` evicts any image the
        // current render does not draw (#1955, the Vello fork in `Cargo.toml`).
        for (_, (_, image)) in std::mem::replace(&mut self.by_surface, now) {
            renderer.unregister_texture(image);
        }
        self.by_surface
            .iter()
            .map(|(id, (_, image))| (*id, image.clone()))
            .collect()
    }
}

/// The surface's alpha mode (#1523): opaque (`Auto`) unless the window is
/// transparent, then one that blends with what is behind it. `Auto` picks
/// `Opaque` on Vulkan, which painted a transparent window black.
/// `PreMultiplied` is preferred: it is what Vulkan on Wayland and X11 offers
/// (Mesa's WSI never offers `PostMultiplied`), and the composited layers are
/// premultiplied already. Either way [`present_step`] converts Vello's
/// straight alpha to what the mode reads.
pub(crate) fn surface_alpha_mode(
    transparent: bool,
    offered: &[wgpu::CompositeAlphaMode],
) -> wgpu::CompositeAlphaMode {
    use wgpu::CompositeAlphaMode as A;
    if !transparent {
        return A::Auto;
    }
    [A::PreMultiplied, A::PostMultiplied, A::Inherit]
        .into_iter()
        .find(|m| offered.contains(m))
        .unwrap_or(A::Auto)
}

/// How the frame reaches the window surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentStep {
    /// `copy_texture_to_texture`: same format, same alpha encoding.
    Copy,
    /// A full-surface pass through the compositor.
    Pass(super::compositor::PresentAlpha),
}

/// Whether a surface in `alpha_mode` reads premultiplied pixels. `Opaque`
/// and `Auto` (an opaque window) ignore alpha, and an opaque frame is the
/// same in either encoding. macOS's `PostMultiplied` is a `CAMetalLayer`
/// with `opaque = false` (wgpu-hal `metal/surface.rs`), whose contents Core
/// Animation composites as premultiplied, so it is read as premultiplied
/// there too (reasoned from the code, not tested on a Mac).
fn surface_reads_premultiplied(alpha_mode: wgpu::CompositeAlphaMode) -> Option<bool> {
    use wgpu::CompositeAlphaMode as A;
    match alpha_mode {
        A::PreMultiplied | A::Inherit => Some(true),
        A::PostMultiplied => Some(cfg!(target_os = "macos")),
        A::Opaque | A::Auto => None,
    }
}

/// The present step for a surface in `alpha_mode`: from the composited
/// layers (`layered`: premultiplied, already in the surface's format) or
/// from Vello's render texture (straight alpha, `Rgba8Unorm`, so a copy
/// needs `rgba_surface`).
pub(crate) fn present_step(
    alpha_mode: wgpu::CompositeAlphaMode,
    layered: bool,
    rgba_surface: bool,
) -> PresentStep {
    use super::compositor::PresentAlpha as P;
    let premultiplied = surface_reads_premultiplied(alpha_mode);
    match (layered, premultiplied) {
        (true, Some(false)) => PresentStep::Pass(P::Unpremultiply),
        (true, _) => PresentStep::Copy,
        (false, Some(true)) => PresentStep::Pass(P::Premultiply),
        (false, _) if rgba_surface => PresentStep::Copy,
        (false, _) => PresentStep::Pass(P::Straight),
    }
}

#[cfg(test)]
mod presented_texture_tests {
    use super::presented_texture;

    /// #1799: once the layers are gone the window is presented from the UI
    /// texture, and a screenshot must read that, not the composited texture
    /// the last frame with layers left behind.
    #[test]
    fn a_screenshot_reads_the_ui_texture_once_the_layers_are_gone() {
        let (composited, ui) = ("composited", "ui");
        assert_eq!(
            *presented_texture(Some(&composited), &ui, true),
            "composited"
        );
        assert_eq!(*presented_texture(Some(&composited), &ui, false), "ui");
        assert_eq!(*presented_texture(None, &ui, false), "ui");
        assert_eq!(*presented_texture(None, &ui, true), "ui");
    }

    /// #2215: what `on_frame_composited` is handed is the presented texture,
    /// with the alpha it holds: the composited texture's premultiplied
    /// colour while there are layers, the UI texture's straight colour else.
    #[test]
    fn the_handed_over_frame_is_the_presented_texture_with_its_alpha() {
        use super::presented_frame;
        use crate::composited_frame::FrameAlpha::{Premultiplied, Straight};
        let (composited, ui) = ("composited", "ui");
        let got = |c: Option<&'static str>, layers| {
            let (t, a) = presented_frame(c, ui, layers);
            (t, a)
        };
        assert_eq!(got(Some(composited), true), ("composited", Premultiplied));
        assert_eq!(got(Some(composited), false), ("ui", Straight));
        assert_eq!(got(None, true), ("ui", Straight));
        assert_eq!(got(None, false), ("ui", Straight));
    }
}

#[cfg(test)]
mod alpha_mode_tests {
    use super::{PresentStep, present_step, surface_alpha_mode};
    use crate::shell::compositor::PresentAlpha as P;
    use wgpu::CompositeAlphaMode as A;

    #[test]
    fn a_lost_frame_invalidates_the_ui_texture_and_nothing_else_does() {
        use super::{Color, UiTextureState};
        use rinch_dom::perf::FullRepaintReason as Reason;
        use vello::FrameCheck;
        let white = Color::WHITE;
        let mut ui = UiTextureState::default();
        assert_eq!(ui.invalid_reason(white), Some(Reason::FirstFrame));
        ui.begin_render();
        ui.rendered(white);
        assert_eq!(ui.invalid_reason(white), None);
        ui.note_check(FrameCheck::Pending);
        ui.note_check(FrameCheck::Settled);
        assert_eq!(ui.invalid_reason(white), None, "these repaint nothing");
        ui.note_check(FrameCheck::Lost);
        assert_eq!(ui.invalid_reason(white), Some(Reason::Invalidated));
        // Settling later does not forgive it.
        ui.note_check(FrameCheck::Settled);
        assert_eq!(ui.invalid_reason(white), Some(Reason::Invalidated));
        ui.rendered(white);
        assert_eq!(ui.invalid_reason(white), None, "the full frame repairs it");
        // The other reasons are unchanged.
        assert_eq!(
            ui.invalid_reason(Color::TRANSPARENT),
            Some(Reason::Invalidated)
        );
        ui.resized();
        assert_eq!(ui.invalid_reason(white), Some(Reason::Resize));
    }

    #[test]
    fn an_opaque_window_keeps_auto() {
        assert_eq!(
            surface_alpha_mode(false, &[A::Opaque, A::PreMultiplied]),
            A::Auto
        );
    }

    /// Vulkan on Wayland (RADV, KWin) offers `Opaque` and `PreMultiplied`;
    /// `Auto` took `Opaque` and a transparent overlay came out black.
    #[test]
    fn a_transparent_window_blends_where_it_can() {
        assert_eq!(
            surface_alpha_mode(true, &[A::Opaque, A::PreMultiplied]),
            A::PreMultiplied
        );
        assert_eq!(
            surface_alpha_mode(true, &[A::Opaque, A::PostMultiplied, A::PreMultiplied]),
            A::PreMultiplied
        );
        assert_eq!(
            surface_alpha_mode(true, &[A::Opaque, A::PostMultiplied]),
            A::PostMultiplied
        );
        assert_eq!(
            surface_alpha_mode(true, &[A::Opaque, A::Inherit]),
            A::Inherit
        );
        assert_eq!(surface_alpha_mode(true, &[A::Opaque]), A::Auto);
    }

    /// Vello stores straight alpha: a premultiplied surface gets it
    /// premultiplied (a straight copy showed 50% white over green as opaque
    /// white), a straight one gets it as it is, and an opaque window's
    /// frame is copied as before.
    #[test]
    fn the_present_step_matches_the_surfaces_alpha() {
        // Vello's render texture.
        assert_eq!(
            present_step(A::PreMultiplied, false, true),
            PresentStep::Pass(P::Premultiply)
        );
        assert_eq!(
            present_step(A::PreMultiplied, false, false),
            PresentStep::Pass(P::Premultiply)
        );
        assert_eq!(
            present_step(A::Inherit, false, true),
            PresentStep::Pass(P::Premultiply)
        );
        assert_eq!(present_step(A::Auto, false, true), PresentStep::Copy);
        assert_eq!(present_step(A::Opaque, false, true), PresentStep::Copy);
        assert_eq!(
            present_step(A::Auto, false, false),
            PresentStep::Pass(P::Straight),
            "a Bgra8 surface is written whole, not blended over an uncleared image"
        );
        // The composited layers (premultiplied).
        assert_eq!(
            present_step(A::PreMultiplied, true, true),
            PresentStep::Copy
        );
        assert_eq!(present_step(A::Auto, true, true), PresentStep::Copy);
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(
                present_step(A::PostMultiplied, false, true),
                PresentStep::Copy
            );
            assert_eq!(
                present_step(A::PostMultiplied, false, false),
                PresentStep::Pass(P::Straight)
            );
            assert_eq!(
                present_step(A::PostMultiplied, true, true),
                PresentStep::Pass(P::Unpremultiply)
            );
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                present_step(A::PostMultiplied, false, true),
                PresentStep::Pass(P::Premultiply)
            );
            assert_eq!(
                present_step(A::PostMultiplied, true, true),
                PresentStep::Copy
            );
        }
    }
}

/// #2095: the present mode for a vsync request, and #2085: the features a
/// device is requested with. Pure, so no GPU is needed.
#[cfg(test)]
mod pacing_and_features_tests {
    use super::*;
    use crate::window::Presentation;
    use PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};

    /// Present-mode lists as drivers report them (Vulkan on X11 and Wayland
    /// under Mesa, NVIDIA, Metal, DX12, and a FIFO-only compositor).
    const SURFACES: [(&str, &[PresentMode]); 6] = [
        ("mesa x11", &[Immediate, Mailbox, Fifo, FifoRelaxed]),
        ("mesa wayland", &[Mailbox, Fifo, Immediate]),
        ("mailbox, no immediate", &[Fifo, Mailbox]),
        ("metal", &[Fifo, Immediate]),
        ("fifo only", &[Fifo]),
        ("nothing listed", &[]),
    ];

    #[test]
    fn vsync_on_is_what_auto_vsync_picked() {
        // `FifoRelaxed` where offered, else `Fifo`: today's `AutoVsync`, so
        // turning nothing off changes nothing.
        let want = [FifoRelaxed, Fifo, Fifo, Fifo, Fifo, Fifo];
        for ((name, offered), want) in SURFACES.iter().zip(want) {
            assert_eq!(choose_present_mode(true, offered), want, "{name}");
            assert_eq!(presentation_of(want), Presentation::Vsync, "{name}");
        }
    }

    #[test]
    fn vsync_off_prefers_immediate_then_mailbox_and_falls_back_to_vsync() {
        let want = [Immediate, Immediate, Mailbox, Immediate, Fifo, Fifo];
        let says = [
            Presentation::Immediate,
            Presentation::Immediate,
            Presentation::Mailbox,
            Presentation::Immediate,
            Presentation::Vsync,
            Presentation::Vsync,
        ];
        for (((name, offered), want), says) in SURFACES.iter().zip(want).zip(says) {
            assert_eq!(choose_present_mode(false, offered), want, "{name}");
            assert_eq!(pacing_outcomes(offered).off, says, "{name}");
            assert_eq!(pacing_outcomes(offered).on, Presentation::Vsync, "{name}");
        }
    }

    /// A surface offering only `FifoRelaxed` beside `Fifo` keeps vsync off
    /// paced, on plain `Fifo` as wgpu's `AutoNoVsync` falls back: relaxed
    /// FIFO still waits for a refresh when the frame is on time, so it is not
    /// an uncapped mode.
    #[test]
    fn fifo_relaxed_is_not_uncapped() {
        assert_eq!(choose_present_mode(false, &[Fifo, FifoRelaxed]), Fifo);
        assert_eq!(choose_present_mode(true, &[Fifo, FifoRelaxed]), FifoRelaxed);
        assert!(!presentation_of(FifoRelaxed).is_uncapped());
        assert!(Presentation::Mailbox.is_uncapped() && Presentation::Immediate.is_uncapped());
    }

    #[test]
    fn a_request_reconfigures_only_when_the_mode_changes() {
        let offered = [Fifo, Mailbox, Immediate];
        let mut mode = choose_present_mode(true, &offered);
        assert_eq!(
            retarget_present_mode(&mut mode, true, &offered),
            (Presentation::Vsync, false),
            "asking for what it does is no reconfigure"
        );
        assert_eq!(
            retarget_present_mode(&mut mode, false, &offered),
            (Presentation::Immediate, true)
        );
        assert_eq!(mode, Immediate);
        assert_eq!(
            retarget_present_mode(&mut mode, false, &offered),
            (Presentation::Immediate, false)
        );
        assert_eq!(
            retarget_present_mode(&mut mode, true, &offered),
            (Presentation::Vsync, true)
        );
        assert_eq!(mode, Fifo);
        // A FIFO-only surface: off changes nothing, and says so.
        let mut fifo = Fifo;
        assert_eq!(
            retarget_present_mode(&mut fifo, false, &[Fifo]),
            (Presentation::Vsync, false)
        );
    }

    /// A surface that records what it was asked.
    #[derive(Default)]
    struct FakeSurface {
        asked: Vec<bool>,
    }

    impl VsyncTarget for FakeSurface {
        fn set_vsync(&mut self, on: bool) -> Presentation {
            self.asked.push(on);
            if on {
                Presentation::Vsync
            } else {
                Presentation::Immediate
            }
        }
    }

    /// The runtime's `apply_vsync` hands the request to the main window's
    /// surface and the DevTools window's, whichever exist.
    #[test]
    fn a_vsync_request_reaches_every_surface_that_exists() {
        let (mut main, mut devtools) = (FakeSurface::default(), FakeSurface::default());
        apply_vsync_to(false, [Some(&mut main), Some(&mut devtools)]);
        apply_vsync_to(true, [Some(&mut main), None]);
        assert_eq!(main.asked, [false, true]);
        assert_eq!(devtools.asked, [false]);
    }

    /// An adapter with some features and not others, off the empty set.
    fn adapter() -> Features {
        Features::TIMESTAMP_QUERY
            | Features::FLOAT32_FILTERABLE
            | Features::DEPTH_CLIP_CONTROL
            | Features::BGRA8UNORM_STORAGE
    }

    #[test]
    fn optional_features_are_requested_where_the_adapter_has_them() {
        let optional =
            Features::TIMESTAMP_QUERY | Features::SHADER_F16 | Features::TEXTURE_BINDING_ARRAY;
        let got = device_features(Features::DEPTH_CLIP_CONTROL, optional, adapter(), false)
            .expect("required features the adapter has");
        assert_eq!(
            got,
            Features::DEPTH_CLIP_CONTROL | Features::TIMESTAMP_QUERY,
            "the required ones, and of the optional only those the adapter has"
        );
        // None of the optional ones on this adapter: still a device.
        assert_eq!(
            device_features(Features::empty(), Features::SHADER_F16, adapter(), false),
            Ok(Features::empty())
        );
    }

    #[test]
    fn a_missing_required_feature_fails_and_is_named() {
        let required = Features::FLOAT32_FILTERABLE | Features::SHADER_F16 | Features::MULTIVIEW;
        assert_eq!(
            device_features(required, Features::TIMESTAMP_QUERY, adapter(), false),
            Err(Features::SHADER_F16 | Features::MULTIVIEW),
            "exactly the ones the adapter lacks"
        );
    }

    /// An adapter that has an experimental feature: asked for as optional
    /// (`Features::all()` included) it is still not requested, since wgpu
    /// refuses one without an opt-in and the device would fail (review of
    /// #2104).
    #[test]
    fn an_experimental_optional_feature_is_never_requested() {
        let experimental = Features::EXPERIMENTAL_RAY_QUERY;
        assert!(Features::all_experimental_mask().contains(experimental));
        let adapter = adapter() | experimental;
        assert_eq!(
            device_features(Features::empty(), experimental, adapter, false),
            Ok(Features::empty())
        );
        assert_eq!(
            device_features(Features::empty(), Features::all(), adapter, false),
            Ok(adapter - experimental),
            "everything the adapter has but the experimental one"
        );
    }

    #[test]
    fn bgra_storage_comes_with_a_bgra_surface_on_an_adapter_that_has_it() {
        let none = Features::empty();
        assert_eq!(
            device_features(none, none, adapter(), true),
            Ok(Features::BGRA8UNORM_STORAGE)
        );
        assert_eq!(device_features(none, none, adapter(), false), Ok(none));
        assert_eq!(
            device_features(none, none, adapter() - Features::BGRA8UNORM_STORAGE, true),
            Ok(none)
        );
    }

    /// #2085 on a real adapter (lavapipe in CI): optional features the
    /// adapter has are granted, one it lacks is left out rather than failing
    /// the device, and the same feature as a required one fails, naming it.
    #[test]
    #[ignore = "needs a GPU adapter (lavapipe in CI)"]
    fn gpu_optional_features_follow_the_adapter_2085() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("a GPU adapter");
        let has = adapter.features();
        // A feature this adapter lacks (and that needs no experimental
        // opt-in), and one it has, whichever adapter this is.
        let lacks = Features::all()
            .iter()
            .find(|f| !has.contains(*f) && !Features::all_experimental_mask().contains(*f))
            .expect("an adapter without every feature");
        let present = [
            Features::TIMESTAMP_QUERY,
            Features::DEPTH_CLIP_CONTROL,
            Features::FLOAT32_FILTERABLE,
            Features::INDIRECT_FIRST_INSTANCE,
        ]
        .into_iter()
        .find(|f| has.contains(*f))
        .expect("an adapter with one of the common features");
        let config = RinchGpuConfig {
            optional_features: lacks | present,
            ..Default::default()
        };
        let (device, _queue) =
            request_rinch_device(&adapter, TextureFormat::Rgba8Unorm, Some(&config))
                .expect("optional features never fail the device");
        assert!(device.features().contains(present), "{present:?} granted");
        assert!(!device.features().contains(lacks), "{lacks:?} left out");

        // Everything as optional, experimental features included (an adapter
        // that has one, RADV and llvmpipe among them, refused the device when
        // one was requested without an opt-in): the device starts, with every
        // feature the adapter has but the experimental ones.
        let everything = RinchGpuConfig {
            optional_features: Features::all(),
            ..Default::default()
        };
        let (device, _queue) =
            request_rinch_device(&adapter, TextureFormat::Rgba8Unorm, Some(&everything))
                .expect("Features::all() as optional never fails the device");
        assert_eq!(
            device.features() & !Features::all_experimental_mask(),
            has & !Features::all_experimental_mask()
        );
        assert!(
            (device.features() & Features::all_experimental_mask()).is_empty(),
            "no experimental feature requested"
        );

        let required = RinchGpuConfig {
            required_features: lacks,
            ..Default::default()
        };
        let err = request_rinch_device(&adapter, TextureFormat::Rgba8Unorm, Some(&required))
            .expect_err("a required feature the adapter lacks fails");
        assert!(
            err.contains(&format!("{lacks:?}")),
            "the error names the missing feature: {err}"
        );
    }

    /// #2473 on a real adapter (lavapipe in CI): rinch's device opens no
    /// memory block larger than 8 MiB, however much is allocated. Ten 3 MiB
    /// buffers fill several blocks; with `MemoryHints::MemoryUsage` the
    /// blocks doubled as they opened (8, 16, 32 MiB; host-visible 4, 8, 16).
    #[test]
    #[ignore = "needs a GPU adapter (lavapipe in CI)"]
    fn gpu_memory_blocks_stay_at_8_mib_2473() {
        const MIB: u64 = 1 << 20;
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("a GPU adapter");
        let (device, _queue) = request_rinch_device(&adapter, TextureFormat::Rgba8Unorm, None)
            .expect("rinch's device");
        let buffers: Vec<wgpu::Buffer> = (0..10)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu_memory_blocks_stay_at_8_mib_2473"),
                    size: 3 * MIB,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let report = device
            .generate_allocator_report()
            .expect("the backend's allocator reports its blocks");
        let sizes: Vec<u64> = report.blocks.iter().map(|block| block.size).collect();
        assert!(
            report.total_allocated_bytes >= 30 * MIB,
            "the ten buffers are in the report: {} bytes allocated",
            report.total_allocated_bytes
        );
        assert!(
            sizes.len() > 1,
            "the buffers fill more than one block: {sizes:?}"
        );
        assert!(
            sizes.iter().all(|&size| size <= 8 * MIB),
            "no block is larger than 8 MiB: {sizes:?}"
        );
        drop(buffers);
    }
}
