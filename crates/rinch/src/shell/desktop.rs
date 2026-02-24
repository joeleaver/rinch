//! Desktop backend: winit window + wgpu renderer.
//!
//! This module implements the [`PlatformWindow`] and [`PlatformRenderer`]
//! traits from `rinch-platform` for the native desktop target using
//! winit for windowing and wgpu/vello for GPU rendering.

use std::sync::Arc;

use peniko::Color;
use rinch_platform::{CompositeLayer, PlatformRenderer, PlatformWindow, RenderError};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Backends, CommandEncoderDescriptor, Device, Extent3d, Instance, InstanceDescriptor, Limits,
    MemoryHints, PresentMode, Queue, Surface, SurfaceConfiguration, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages,
};
use winit::window::Window;

// ── WinitWindow ──────────────────────────────────────────────────────────────

/// Desktop window backed by winit.
pub struct WinitWindow {
    pub(crate) window: Arc<Window>,
}

impl WinitWindow {
    pub fn new(window: Arc<Window>) -> Self {
        Self { window }
    }

    /// Get the raw winit window reference.
    pub fn raw(&self) -> &Arc<Window> {
        &self.window
    }
}

impl PlatformWindow for WinitWindow {
    fn inner_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
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
    pub(crate) device: Device,
    pub(crate) queue: Queue,
    /// Layer compositor (created lazily when composite layers are first set).
    pub(crate) layer_compositor: Option<super::compositor::LayerCompositor>,
    /// Frame uploader (reusable texture for decoded frames).
    pub(crate) layer_uploader: Option<super::frame_upload::FrameUploader>,
    /// Active composite layers to render underneath the UI.
    pub(crate) composite_layers: Vec<CompositeLayer>,
    /// Composited output texture (layers + UI) for screenshot capture.
    /// Created lazily, invalidated on resize.
    pub(crate) composited_texture: Option<Texture>,
}

impl WgpuRenderer {
    /// Create a new GPU renderer for the given window.
    pub fn new(window: &Arc<Window>, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let backends = Backends::from_env().unwrap_or_default();
        let instance = Instance::new(&InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("Failed to create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Failed to find adapter");

        let caps = surface.get_capabilities(&adapter);

        let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
            TextureFormat::Rgba8Unorm
        } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
            TextureFormat::Bgra8Unorm
        } else {
            caps.formats[0]
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rinch-dom device"),
            required_features: wgpu::Features::default(),
            required_limits: Limits::default(),
            memory_hints: MemoryHints::MemoryUsage,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .expect("Failed to create device");

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

        let render_texture = Self::create_render_texture(&device, format, width, height);

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
            Self::create_render_texture(&self.device, self.surface_config.format, width, height);
        self.composited_texture = None;
    }

    fn render_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<(), RenderError> {
        let surface_texture = self
            .surface
            .get_current_texture()
            .map_err(|_| RenderError::SurfaceLost)?;

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

        if !self.composite_layers.is_empty() {
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

            // Phase 1: Upload all layer frames to separate GPU textures,
            // then blit each into its viewport region.
            // First layer clears to base_color, subsequent layers load (preserve prior).
            let wgpu_base_color = wgpu::Color {
                r: base_color.components[0] as f64,
                g: base_color.components[1] as f64,
                b: base_color.components[2] as f64,
                a: base_color.components[3] as f64,
            };
            for (i, layer) in self.composite_layers.iter().enumerate() {
                let layer_texture = uploader.upload(
                    i,
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
                    if i == 0 {
                        Some(wgpu_base_color)
                    } else {
                        None
                    },
                );
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

        // Standard path: simple texture copy (no video)
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
        !self.composite_layers.is_empty()
    }

    fn capture_screenshot(&self) -> Result<(u32, u32, Vec<u8>), RenderError> {
        #[cfg(feature = "debug")]
        {
            let w = self.surface_config.width;
            let h = self.surface_config.height;
            let fmt = self.surface_config.format;

            // When compositing is active, read from the composited texture
            // (which has layers + UI blended). Otherwise read from render_texture.
            let tex = self
                .composited_texture
                .as_ref()
                .unwrap_or(&self.render_texture);

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
