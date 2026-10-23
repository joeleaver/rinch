//! Desktop backend: winit window + wgpu renderer.
//!
//! This module implements the [`PlatformWindow`] and [`PlatformRenderer`]
//! traits from `rinch-platform` for the native desktop target using
//! winit for windowing and wgpu/vello for GPU rendering.

use std::sync::{Arc, OnceLock};

use peniko::Color;
use rinch_platform::{CompositeLayer, PlatformRenderer, PlatformWindow, RenderError};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Adapter, Backends, CommandEncoderDescriptor, Device, Extent3d, Features, Instance,
    InstanceDescriptor, Limits, MemoryHints, PresentMode, Queue, Surface, SurfaceConfiguration,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
};
use winit::window::Window;

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
/// [`crate::App::gpu_config`]. The requested features/limits are passed
/// through verbatim — if the adapter cannot satisfy them, device creation fails
/// loudly rather than silently dropping a capability.
#[derive(Clone, Default)]
pub struct RinchGpuConfig {
    /// Device features to require in addition to rinch's defaults.
    pub required_features: Features,
    /// Device limits to require (replaces `Limits::default()`).
    pub required_limits: Limits,
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
    Config(RinchGpuConfig),
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

/// Desktop GPU renderer backed by wgpu + vello.
///
/// Field order matters for drop safety: resources that reference the device
/// (`renderer`, `render_texture`) must be declared before `device`/`queue`
/// so they are dropped first (Rust drops fields in declaration order).
/// `surface` must drop before the window handle it references is freed.
pub struct WgpuRenderer {
    pub(crate) renderer: VelloRenderer,
    pub(crate) render_texture: Texture,
    pub(crate) surface: Surface<'static>,
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
    /// Composited output texture (layers + UI) for screenshot capture.
    /// Created lazily, invalidated on resize.
    pub(crate) composited_texture: Option<Texture>,
    /// What `render_texture` holds (#1718): `None` before the first frame
    /// and after a resize or a failed render, else the base colour Vello
    /// rendered it over. A partial frame updates it in place only while it
    /// holds a frame over the base colour this frame needs.
    pub(crate) ui_base: Option<Color>,
    /// Why `ui_base` was last cleared, for the full frame that repairs it.
    pub(crate) ui_lost: rinch_dom::perf::FullRepaintReason,
    /// The target a partial frame is rendered into, reused while it is big
    /// enough (sizes rounded up to [`PARTIAL_TARGET_STEP`]).
    pub(crate) partial_target: Option<Texture>,
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
                Instance::new(&InstanceDescriptor {
                    backends,
                    flags: wgpu::InstanceFlags::from_build_config().with_env(),
                    backend_options: wgpu::BackendOptions::from_env_or_default(),
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                })
            }
        };

        // SAFETY: The window outlives the surface — drop order in WgpuRenderer
        // ensures surface is dropped before the window.
        let surface = unsafe {
            use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
            let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: window
                    .display_handle()
                    .map_err(|e| format!("no display handle for a GPU surface: {e}"))?
                    .as_raw(),
                raw_window_handle: window
                    .window_handle()
                    .map_err(|e| format!("no window handle for a GPU surface: {e}"))?
                    .as_raw(),
            };
            instance
                .create_surface_unsafe(target)
                .map_err(|e| format!("Failed to create surface: {e}"))?
        };

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
                    ));
                }
                adapter.clone()
            }
            None => Arc::new(
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                }))
                .map_err(|e| format!("Failed to find adapter: {e}"))?,
            ),
        };

        let caps = surface.get_capabilities(&adapter);
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

        // Device/Queue: adopt the embedder's, or create one — optionally with
        // the embedder's extra features/limits (issue #57).
        let (device, queue) = match (&shared, gpu_init) {
            (Some((_, _, device, queue)), _) => (device.clone(), queue.clone()),
            (None, other) => {
                let (mut features, limits) = match other {
                    Some(GpuInit::Config(cfg)) => {
                        (cfg.required_features, cfg.required_limits.clone())
                    }
                    _ => (Features::default(), Limits::default()),
                };
                // On surfaces that only offer Bgra8Unorm (X11 on NVIDIA,
                // notably), any storage-texture use of the surface format
                // needs this feature explicitly. Harmless to request whenever
                // the adapter has it.
                if format == TextureFormat::Bgra8Unorm
                    && adapter.features().contains(Features::BGRA8UNORM_STORAGE)
                {
                    features |= Features::BGRA8UNORM_STORAGE;
                }
                let (raw_device, raw_queue) =
                    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                        label: Some("rinch-dom device"),
                        required_features: features,
                        required_limits: limits,
                        memory_hints: MemoryHints::MemoryUsage,
                        trace: wgpu::Trace::default(),
                        experimental_features: wgpu::ExperimentalFeatures::default(),
                    }))
                    .map_err(|e| {
                        format!(
                            "Failed to create device (the adapter may not support the \
                             features/limits requested via App::gpu_config): {e}"
                        )
                    })?;
                (Arc::new(raw_device), Arc::new(raw_queue))
            }
        };

        // wgpu reports errors in what follows (configuring the surface,
        // creating the texture, compiling Vello's shaders) through the device's
        // error handler, which panics by default. Scope them so a device that
        // cannot do the work is an `Err` like every step above.
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        device.push_error_scope(wgpu::ErrorFilter::Validation);

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
            format,
            width,
            height,
            present_mode: PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };

        surface.configure(&device, &surface_config);

        // Vello's compute shaders only write rgba8unorm storage textures, so
        // the intermediate render texture is ALWAYS Rgba8Unorm regardless of
        // the surface format. When the surface is also Rgba8Unorm we present
        // with a plain copy; otherwise (e.g. Bgra8Unorm — X11/NVIDIA surfaces
        // commonly offer nothing else) render_scene blits through the layer
        // compositor's sampling pass, which swizzles for free.
        let render_texture =
            Self::create_render_texture(&device, TextureFormat::Rgba8Unorm, width, height);

        let renderer = VelloRenderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        );

        let validation = pollster::block_on(device.pop_error_scope());
        let out_of_memory = pollster::block_on(device.pop_error_scope());
        if let Some(e) = validation.or(out_of_memory) {
            return Err(format!("the GPU device cannot present this window: {e}"));
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

        tracing::info!(
            "rinch-dom runtime: backend={:?}, format={:?}",
            adapter.get_info().backend,
            format
        );

        Ok(Self {
            renderer,
            render_texture,
            surface,
            surface_config,
            device,
            queue,
            layer_compositor: None,
            layer_uploader: None,
            composite_layers: Vec::new(),
            surface_images: SurfaceImages::default(),
            gpu_layers: Vec::new(),
            composited_texture: None,
            ui_base: None,
            ui_lost: rinch_dom::perf::FullRepaintReason::FirstFrame,
            partial_target: None,
        })
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
        let base = self.vello_base_color(Self::window_base_color(transparent));
        match self.ui_base {
            Some(b) if b == base => None,
            Some(_) => Some(rinch_dom::perf::FullRepaintReason::Invalidated),
            None => Some(self.ui_lost),
        }
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
        self.ui_base = None;
        self.ui_lost = rinch_dom::perf::FullRepaintReason::Invalidated;
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
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| RenderError::Internal(format!("render_to_texture failed: {:?}", e)))?;
        self.ui_base = Some(base);
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
            self.ui_base,
            Some(base),
            "a partial frame over an invalid UI texture"
        );
        self.ui_base = None;
        self.ui_lost = rinch_dom::perf::FullRepaintReason::Invalidated;
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
        self.ui_base = Some(base);
        Ok(())
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
                antialiasing_method: AaConfig::Msaa16,
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
        let surface_texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            // Swapchain went stale (resize/DPI/occlusion/GPU reset) — recreate it and retry once.
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.surface_config);
                match self.surface.get_current_texture() {
                    Ok(t) => t,
                    Err(_) => return Ok(()), // try again on the next redraw
                }
            }
            // Transient — skip this frame.
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(e) => return Err(RenderError::Internal(format!("get_current_texture: {e:?}"))),
        };

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rinch-dom copy encoder"),
            });

        let has_any_layers = !self.composite_layers.is_empty() || !self.gpu_layers.is_empty();

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
                        texture: &surface_texture.texture,
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
                    let surface_view = surface_texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    compositor.present(
                        &self.device,
                        &mut encoder,
                        composited,
                        &surface_view,
                        alpha,
                    );
                }
            }

            self.queue.submit(Some(encoder.finish()));
            surface_texture.present();
            self.device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| RenderError::Internal(format!("GPU poll failed: {:?}", e)))?;

            return Ok(());
        }

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
                    texture: &surface_texture.texture,
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
                let surface_view = surface_texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                compositor.present(
                    &self.device,
                    &mut encoder,
                    &self.render_texture,
                    &surface_view,
                    alpha,
                );
            }
        }

        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| RenderError::Internal(format!("GPU poll failed: {:?}", e)))?;

        Ok(())
    }
}

impl PlatformRenderer for WgpuRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.render_texture =
            Self::create_render_texture(&self.device, TextureFormat::Rgba8Unorm, width, height);
        self.composited_texture = None;
        self.ui_base = None;
        self.ui_lost = rinch_dom::perf::FullRepaintReason::Resize;
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
                self.has_composite_layers(),
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

    /// Register this paint's `RenderSurface` GPU frames with the Vello
    /// renderer and hand back each one's image, by surface id (#1768); see
    /// [`SurfaceImages::sync`].
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
    match composited {
        Some(c) if layers => c,
        _ => ui,
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
                Some((kept, image)) if kept == texture => image,
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
}

#[cfg(test)]
mod alpha_mode_tests {
    use super::{PresentStep, present_step, surface_alpha_mode};
    use crate::shell::compositor::PresentAlpha as P;
    use wgpu::CompositeAlphaMode as A;

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
