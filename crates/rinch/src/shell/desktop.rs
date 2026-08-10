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

/// Get the shared GPU handle, if the renderer has been initialized.
///
/// Returns `None` before `rinch::shell::run()` creates the wgpu device.
/// After that, returns the `Arc<Device>`, `Arc<Queue>`, and `Arc<Adapter>` that
/// rinch uses internally, allowing external renderers to share the same GPU
/// device.
pub fn gpu_handle() -> Option<&'static GpuHandle> {
    GPU_HANDLE.get()
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
/// [`crate::run_with_gpu_config`]. The requested features/limits are passed
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
/// [`crate::run_with_external_device`]. rinch creates only the window surface
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
    /// Composited output texture (layers + UI) for screenshot capture.
    /// Created lazily, invalidated on resize.
    pub(crate) composited_texture: Option<Texture>,
}

impl WgpuRenderer {
    /// Create a new GPU renderer for the given window.
    pub fn new(window: &dyn Window, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let gpu_init = GPU_INIT.get();

        // Instance: reuse the embedder's when an external device is supplied
        // (the surface must come from the instance the adapter/device belong to);
        // otherwise create rinch's own.
        //
        // Default to GPU-only backends (Vulkan/Metal/DX12). Skip GL/GLES probing
        // which loads Mesa gallium + LLVM (~70MB RSS) but can't run Vello anyway
        // (Vello requires compute shaders). WGPU_BACKEND env var still overrides.
        let instance = match gpu_init {
            Some(GpuInit::External(g)) => g.instance.clone(),
            _ => {
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
                raw_display_handle: window.display_handle().unwrap().as_raw(),
                raw_window_handle: window.window_handle().unwrap().as_raw(),
            };
            instance
                .create_surface_unsafe(target)
                .expect("Failed to create surface")
        };

        // Adapter: adopt the embedder's, or pick a surface-compatible one.
        let adapter: Arc<Adapter> = match gpu_init {
            Some(GpuInit::External(g)) => {
                assert!(
                    g.adapter.is_surface_supported(&surface),
                    "run_with_external_device: the supplied adapter cannot present to \
                     rinch's window surface. The adapter/device must be created from an \
                     adapter that supports the target window (create the adapter with a \
                     compatible surface, or use run_with_gpu_config to let rinch own the device)."
                );
                g.adapter.clone()
            }
            _ => Arc::new(
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                }))
                .expect("Failed to find adapter"),
            ),
        };

        let caps = surface.get_capabilities(&adapter);

        let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
            TextureFormat::Rgba8Unorm
        } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
            TextureFormat::Bgra8Unorm
        } else {
            caps.formats[0]
        };

        // Device/Queue: adopt the embedder's, or create one — optionally with
        // the embedder's extra features/limits (issue #57).
        let (device, queue) = match gpu_init {
            Some(GpuInit::External(g)) => (g.device.clone(), g.queue.clone()),
            other => {
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
                    .expect(
                        "Failed to create device (the adapter may not support the \
                         features/limits requested via run_with_gpu_config)",
                    );
                (Arc::new(raw_device), Arc::new(raw_queue))
            }
        };

        // Publish the shared GPU handle for external renderers
        let _ = GPU_HANDLE.set(GpuHandle {
            device: device.clone(),
            queue: queue.clone(),
            adapter: adapter.clone(),
        });

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
            format,
            width,
            height,
            present_mode: PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
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
        )
        .expect("Failed to create Vello renderer");

        tracing::info!(
            "rinch-dom runtime: backend={:?}, format={:?}",
            adapter.get_info().backend,
            format
        );

        Self {
            renderer,
            render_texture,
            surface,
            surface_config,
            device,
            queue,
            layer_compositor: None,
            layer_uploader: None,
            composite_layers: Vec::new(),
            gpu_layers: Vec::new(),
            composited_texture: None,
        }
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
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
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
    }

    fn render_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<(), RenderError> {
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

        let render_texture_view = self
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // When composite layers are present, render Vello with transparent background
        // so we can composite it on top of the layer content.
        let vello_base_color = if !self.composite_layers.is_empty() {
            Color::TRANSPARENT
        } else {
            base_color
        };

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &render_texture_view,
                &RenderParams {
                    base_color: vello_base_color,
                    width,
                    height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| RenderError::Internal(format!("render_to_texture failed: {:?}", e)))?;

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
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
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

            // Copy composited result to swapchain for display
            encoder.copy_texture_to_texture(
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
            );

            self.queue.submit(Some(encoder.finish()));
            surface_texture.present();
            self.device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| RenderError::Internal(format!("GPU poll failed: {:?}", e)))?;

            return Ok(());
        }

        // Standard path (no layers). The render texture is always Rgba8Unorm
        // (Vello's storage-texture requirement); a raw copy is only legal when
        // the surface matches. Otherwise blit through the compositor's UI
        // sampling pass — the sample/write does the channel-order conversion.
        if self.surface_config.format == TextureFormat::Rgba8Unorm {
            encoder.copy_texture_to_texture(
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
            );
        } else {
            let compositor = self.layer_compositor.get_or_insert_with(|| {
                super::compositor::LayerCompositor::new(&self.device, self.surface_config.format)
            });
            let surface_view = surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            // Vello output is opaque here (base color is never transparent on
            // this path), so the alpha-over fullscreen draw writes every pixel
            // and the surface needs no clear.
            compositor.overlay_ui(
                &self.device,
                &mut encoder,
                &self.render_texture,
                &surface_view,
            );
        }

        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| RenderError::Internal(format!("GPU poll failed: {:?}", e)))?;

        Ok(())
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

            // When compositing is active, read from the composited texture
            // (which has layers + UI blended). Otherwise read from render_texture.
            let tex = self
                .composited_texture
                .as_ref()
                .unwrap_or(&self.render_texture);

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
}
