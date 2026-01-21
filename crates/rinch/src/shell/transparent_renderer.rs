//! Custom Vello window renderer with proper transparency support.
//!
//! On Windows, true window transparency requires:
//! 1. DirectComposition swapchain (via WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual)
//! 2. DX12 backend
//! 3. PreMultiplied alpha mode
//! 4. Transparent base color
//!
//! Implementation notes:
//! - Uses a patched wgpu-fork that enables Rgba8Unorm storage texture support on DX12
//!   (see ../../../wgpu-fork for the patches)
//! - Since swapchain textures don't support STORAGE_BINDING, we render to an
//!   intermediate texture first, then copy to the surface

use anyrender_vello::VelloScenePainter;
use peniko::Color;
use std::num::NonZero;
use std::sync::Arc;
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Backends, CommandEncoderDescriptor, CompositeAlphaMode, Device, Extent3d, Features, Instance,
    InstanceDescriptor, Limits, MemoryHints, PresentMode, Queue, Surface, SurfaceConfiguration,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use winit::window::Window;

const DEFAULT_THREADS: Option<NonZero<usize>> = None;

struct ActiveRenderState {
    renderer: VelloRenderer,
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    // Intermediate texture for Vello's compute shaders (needs STORAGE_BINDING)
    render_texture: Texture,
}

enum RenderState {
    Active(ActiveRenderState),
    Suspended,
}

/// Options for configuring the transparent window renderer.
#[derive(Clone)]
pub struct TransparentRendererOptions {
    pub features: Option<Features>,
    pub limits: Option<Limits>,
    pub base_color: Color,
    pub antialiasing_method: AaConfig,
    pub transparent: bool,
}

impl Default for TransparentRendererOptions {
    fn default() -> Self {
        Self {
            features: None,
            limits: None,
            base_color: Color::WHITE,
            antialiasing_method: AaConfig::Msaa16,
            transparent: false,
        }
    }
}

/// A Vello-based window renderer with proper transparency support.
pub struct TransparentWindowRenderer {
    render_state: RenderState,
    window_handle: Option<Arc<Window>>,
    scene: Scene,
    config: TransparentRendererOptions,
}

impl TransparentWindowRenderer {
    pub fn new() -> Self {
        Self::with_options(TransparentRendererOptions::default())
    }

    pub fn with_options(config: TransparentRendererOptions) -> Self {
        Self {
            config,
            render_state: RenderState::Suspended,
            window_handle: None,
            scene: Scene::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.render_state, RenderState::Active(_))
    }

    pub fn resume(&mut self, window: Arc<Window>, width: u32, height: u32) {
        // For transparency on Windows, use DX12 with DirectComposition
        let backends = if self.config.transparent && cfg!(target_os = "windows") {
            // Enable DirectComposition for true window transparency
            // SAFETY: Setting environment variable before wgpu initialization
            unsafe {
                std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "DxgiFromVisual");
            }
            tracing::info!("Using DX12 with DirectComposition for transparent window");
            Backends::DX12
        } else {
            Backends::from_env().unwrap_or_default()
        };

        let state = self.create_render_state(&window, width, height, backends);
        self.window_handle = Some(window);
        self.render_state = RenderState::Active(state);
    }

    fn create_render_texture(device: &Device, format: TextureFormat, width: u32, height: u32) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("vello render texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            // STORAGE_BINDING for Vello's compute shaders, TEXTURE_BINDING for Vello internals, COPY_SRC to copy to surface
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn create_render_state(
        &self,
        window: &Arc<Window>,
        width: u32,
        height: u32,
        backends: Backends,
    ) -> ActiveRenderState {
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

        tracing::info!("Backend: {:?}", adapter.get_info().backend);
        tracing::info!("Adapter: {:?}", adapter.get_info().name);
        tracing::info!("Available alpha modes: {:?}", caps.alpha_modes);

        // For transparency, we need PreMultiplied alpha mode (supported on DX12 with DirectComposition)
        // Fall back to Auto (usually Opaque) if PreMultiplied isn't available
        let alpha_mode = if self.config.transparent
            && caps.alpha_modes.contains(&CompositeAlphaMode::PreMultiplied)
        {
            tracing::info!("Using PreMultiplied alpha mode for transparency");
            CompositeAlphaMode::PreMultiplied
        } else {
            if self.config.transparent {
                tracing::warn!(
                    "Transparency requested but PreMultiplied alpha mode not available. \
                     Available modes: {:?}",
                    caps.alpha_modes
                );
            }
            CompositeAlphaMode::Auto
        };

        // Vello prefers Rgba8Unorm
        let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
            TextureFormat::Rgba8Unorm
        } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
            TextureFormat::Bgra8Unorm
        } else {
            caps.formats[0]
        };

        // Request minimal features - let Vello/wgpu determine what's needed
        let required_features = self.config.features.unwrap_or_default();
        let available_features = adapter.features();
        let features = required_features & available_features;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rinch device"),
            required_features: features,
            required_limits: self.config.limits.clone().unwrap_or_default(),
            memory_hints: MemoryHints::MemoryUsage,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .expect("Failed to create device");

        // Surface only needs RENDER_ATTACHMENT and COPY_DST (for receiving the copy)
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

        // Create intermediate render texture for Vello
        let render_texture = Self::create_render_texture(&device, format, width, height);

        let renderer = VelloRenderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::all(),
                use_cpu: false,
                num_init_threads: DEFAULT_THREADS,
                pipeline_cache: None,
            },
        )
        .expect("Failed to create Vello renderer");

        tracing::info!(
            "Created renderer: backend={:?}, alpha_mode={:?}, format={:?}",
            adapter.get_info().backend,
            alpha_mode,
            format
        );

        ActiveRenderState {
            renderer,
            surface,
            surface_config,
            device,
            queue,
            render_texture,
        }
    }

    pub fn suspend(&mut self) {
        self.render_state = RenderState::Suspended;
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        if let RenderState::Active(state) = &mut self.render_state {
            state.surface_config.width = width;
            state.surface_config.height = height;
            state.surface.configure(&state.device, &state.surface_config);
            // Recreate the render texture with new size
            state.render_texture = Self::create_render_texture(
                &state.device,
                state.surface_config.format,
                width,
                height,
            );
        }
    }

    pub fn render<F>(&mut self, draw_fn: F)
    where
        F: for<'a, 'b> FnOnce(&'a mut VelloScenePainter<'b, 'b>),
    {
        let RenderState::Active(state) = &mut self.render_state else {
            return;
        };

        // Get current surface texture
        let surface_texture = match state.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(e) => {
                tracing::warn!("Failed to get surface texture: {:?}", e);
                return;
            }
        };

        // Create view of our intermediate render texture
        let render_texture_view = state
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Draw to scene using VelloScenePainter wrapper
        let mut painter = VelloScenePainter::new(&mut self.scene);
        draw_fn(&mut painter);

        // Render to intermediate texture (which has STORAGE_BINDING)
        state
            .renderer
            .render_to_texture(
                &state.device,
                &state.queue,
                &self.scene,
                &render_texture_view,
                &RenderParams {
                    base_color: self.config.base_color,
                    width: state.surface_config.width,
                    height: state.surface_config.height,
                    antialiasing_method: self.config.antialiasing_method,
                },
            )
            .expect("failed to render to texture");

        // Copy from render texture to surface texture
        let mut encoder = state
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("copy encoder"),
            });

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &state.render_texture,
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
                width: state.surface_config.width,
                height: state.surface_config.height,
                depth_or_array_layers: 1,
            },
        );

        state.queue.submit(Some(encoder.finish()));

        // Present
        surface_texture.present();

        // Wait for GPU
        state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();

        // Clear the scene for next frame
        self.scene.reset();
    }
}

impl Default for TransparentWindowRenderer {
    fn default() -> Self {
        Self::new()
    }
}
