//! The [`App`] builder's rendering options: the renderer, the GPU device,
//! vsync, text rasterization, antialiasing and frame pacing.

use super::*;

impl<F, M> App<F, M>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    /// Raise the compositor GPU device's features and limits.
    ///
    /// rinch still owns the instance, picks a surface-compatible adapter and
    /// creates the device, so window presentation is always correct. Use this
    /// when an embedding renderer needs a higher-capability device so it can
    /// build its pipelines and textures on *rinch's* device and hand back a
    /// `TextureView` for zero-copy present. After startup, obtain the shared
    /// device via [`gpu_handle`](crate::shell::desktop::gpu_handle).
    ///
    /// Mutually exclusive with [`external_gpu`](App::external_gpu) — the last
    /// one set on *this* builder wins, because the builder holds one field and
    /// installs it once.
    ///
    /// It is installed into a process-wide `OnceLock` at [`run`](App::run),
    /// which ignores a second install, so a second `App` could not change it.
    /// That is unreachable rather than dangerous: a second `run` panics first,
    /// at `EventLoop::new`, because winit permits one event loop per process
    /// and answers `RecreationAttempt` after the first.
    #[cfg(feature = "gpu-renderer")]
    pub fn gpu_config(mut self, gpu: crate::shell::desktop::RinchGpuConfig) -> Self {
        self.gpu = Some(crate::shell::desktop::GpuInit::Config(Box::new(gpu)));
        self
    }

    /// Choose the renderer the window presents with.
    ///
    /// [`Renderer::Auto`](crate::Renderer::Auto), the default, presents on the
    /// GPU when the build has the GPU renderer (`gpu-renderer`, which `gpu`
    /// implies; `gpu` is one of rinch's default features) and the GPU starts,
    /// and falls back to the software renderer (with a warning in the log)
    /// when it does not. A software adapter (a CPU Vulkan driver such as lavapipe, or
    /// Windows' WARP) counts as a GPU that does not start: rinch's own
    /// software renderer is cheaper, so `Auto` takes it, and GPU frames and
    /// `gpu_handle` are then unavailable.
    /// [`Renderer::Software`](crate::Renderer::Software) never touches
    /// the GPU; [`Renderer::Gpu`](crate::Renderer::Gpu) panics rather than
    /// fall back. The `RINCH_RENDERER` environment variable (`auto`, `gpu`,
    /// `software` or `cpu`) overrides this, so a user can force a renderer
    /// without a rebuild.
    ///
    /// To leave the GPU renderer out of the build altogether, depend on rinch
    /// with `default-features = false, features = ["desktop"]` (or
    /// `["windowing"]`): the binary is smaller and the build shorter than the
    /// GPU build's (`hello-rinch-dom`, release, stripped: 31.4 MB against
    /// 39.3 MB, and about 276 s less release build time), and the app
    /// never opens a GPU device or holds its memory, which suits a small tool.
    /// Such a build has only the software renderer, whatever this asks for;
    /// `Renderer::Gpu` (or `RINCH_RENDERER=gpu`) there logs a warning that
    /// names the feature to enable.
    ///
    /// An app that configures the GPU device itself
    /// ([`gpu_config`](App::gpu_config), [`external_gpu`](App::external_gpu))
    /// always presents on the GPU, because its own pipelines need that device:
    /// it gets no fallback (a GPU that will not start panics), and a
    /// `Software` choice, from this method or from `RINCH_RENDERER`, is logged
    /// as a warning and ignored.
    #[cfg(feature = "windowing")]
    pub fn renderer(mut self, renderer: crate::shell::renderer::Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    /// Start with vsync on (the default) or off (issue #2095). Off, the GPU
    /// renderer presents uncapped (`Immediate`, or `Mailbox` where the
    /// surface has no `Immediate`), so frames that follow each other — a
    /// continuous `RenderSurface`, an animation — are drawn as fast as they
    /// can be rather than once per display refresh. An idle window still
    /// sleeps.
    ///
    /// Change it while the app runs with
    /// [`rinch::window::set_vsync`](crate::window::set_vsync), which also
    /// says what the surface honoured; [`rinch::window::presentation`](crate::window::presentation)
    /// reads it. The software renderer has no present mode, and ignores it.
    #[cfg(feature = "windowing")]
    pub fn vsync(mut self, on: bool) -> Self {
        self.vsync = Some(on);
        self
    }

    /// Choose how text is rendered (issue #2286). Every field of the default
    /// [`TextRasterization`](crate::text_rasterization::TextRasterization) is `Auto`,
    /// which follows the OS's text settings
    /// ([`system_text_settings`](crate::text_rasterization::system_text_settings):
    /// fontconfig on Linux, `SystemParametersInfo` on Windows, grayscale and
    /// unhinted on macOS), as Chrome does.
    ///
    /// Light text is thickened a little by default ("stem darkening"), which
    /// makes light-on-dark text as heavy as the same text dark-on-light; turn
    /// it off with
    /// `.text_rasterization(TextRasterization::default().stem_darkening(StemDarkening::Off))`.
    /// Hinting follows the OS unless set. Both renderers apply both.
    #[cfg(any(feature = "windowing", feature = "android"))]
    pub fn text_rasterization(
        mut self,
        rasterization: crate::text_rasterization::TextRasterization,
    ) -> Self {
        self.text_rasterization = rasterization;
        self
    }

    /// Choose the antialiasing the GPU renderer draws every window with
    /// (issue #2286): [`GpuAntialiasing::Area`](crate::GpuAntialiasing::Area),
    /// the default, or MSAA, for paths that overlap themselves (self-crossing
    /// strokes, overlapping contours), whose edges area coverage shades by the
    /// sum of the overlaps. The `RINCH_GPU_AA` environment variable
    /// (`area`, `msaa8`, `msaa16`) overrides it. The software renderer and
    /// Android ignore it.
    #[cfg(feature = "gpu-renderer")]
    pub fn gpu_antialiasing(mut self, mode: crate::GpuAntialiasing) -> Self {
        self.gpu_antialiasing = Some(mode);
        self
    }

    /// Choose whether the GPU renderer paces its presents on Wayland by frame
    /// callbacks (issue #2669), so that a window the compositor does not show
    /// (a layer surface under another, a toplevel on a hidden workspace)
    /// cannot stall the app. The default,
    /// [`WaylandFramePacing::Auto`](crate::WaylandFramePacing::Auto), paces
    /// only where the compositor does not offer `wp_fifo_v1` (with it, the
    /// driver paces `Fifo` itself and a hidden window cannot stall). The
    /// `RINCH_WAYLAND_FRAME_PACING` environment variable (`auto`, `on`,
    /// `off`) overrides it. The same as [`crate::set_wayland_frame_pacing`],
    /// which it calls.
    #[cfg(feature = "gpu-renderer")]
    pub fn wayland_frame_pacing(self, mode: crate::WaylandFramePacing) -> Self {
        crate::shell::frame_pacing::set_wayland_frame_pacing(mode);
        self
    }

    /// Let the GPU renderer have at most `frames` frames in flight on the GPU
    /// (issue #2688): before a frame's GPU work starts, it waits until the
    /// frame `frames` frames back has finished. The default,
    /// [`DEFAULT_GPU_FRAMES_IN_FLIGHT`](crate::gpu_frames_in_flight::DEFAULT_GPU_FRAMES_IN_FLIGHT)
    /// (2), lets the CPU encode a frame while the GPU draws the last; `1`
    /// keeps the least memory; `0` sets no limit. Each frame in flight holds
    /// wgpu command encoders, and the driver keeps the memory of the most that
    /// were ever alive at once for the life of the process (on RADV 1.3 to
    /// 4 MiB each). The `RINCH_GPU_FRAMES_IN_FLIGHT` environment variable
    /// overrides it. The same as [`crate::set_gpu_frames_in_flight`], which
    /// it calls.
    #[cfg(feature = "gpu-renderer")]
    pub fn gpu_frames_in_flight(self, frames: usize) -> Self {
        crate::gpu_frames_in_flight::set_gpu_frames_in_flight(frames);
        self
    }

    /// Compile each GPU renderer's shaders on `threads` threads (issue
    /// #2468). The default is the machine's parallelism, at most
    /// [`DEFAULT_MAX_INIT_THREADS`](crate::gpu_init::DEFAULT_MAX_INIT_THREADS)
    /// (one on macOS). More threads make a start with a cold driver shader
    /// cache a little faster, and leave more of the compilers' freed memory
    /// in the process. The `RINCH_GPU_INIT_THREADS` environment variable
    /// overrides it. The same as [`crate::set_gpu_init_threads`], which it
    /// calls.
    #[cfg(feature = "gpu-renderer")]
    pub fn gpu_init_threads(self, threads: std::num::NonZeroUsize) -> Self {
        crate::gpu_init::set_gpu_init_threads(threads);
        self
    }

    /// Run on an embedder-provided GPU device.
    ///
    /// The embedder creates the whole GPU stack with its own `DeviceDescriptor`
    /// and hands it over; rinch creates only the window surface, validates that
    /// the adapter can present to it, and composites directly onto the provided
    /// device — no `request_device`, no CPU readback.
    ///
    /// Prefer [`gpu_config`](App::gpu_config) when you only need to raise
    /// features and limits: letting rinch own device creation guarantees
    /// surface compatibility.
    ///
    /// # Panics
    ///
    /// [`run`](App::run) panics if the supplied adapter cannot present to
    /// rinch's window surface.
    #[cfg(feature = "gpu-renderer")]
    pub fn external_gpu(mut self, gpu: crate::shell::desktop::ExternalGpu) -> Self {
        self.gpu = Some(crate::shell::desktop::GpuInit::External(gpu));
        self
    }
}
