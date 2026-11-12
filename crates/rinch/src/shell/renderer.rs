//! Which renderer presents the desktop window, chosen at run time.
//!
//! A `desktop` build always carries the software renderer (TinySkia painting,
//! softbuffer presenting). A `desktop` + `gpu` build carries the GPU one
//! (Vello over wgpu) as well, and picks between them when it creates the
//! window: [`Renderer::Auto`] tries the GPU and falls back to software when
//! the GPU cannot start (no adapter, no surface, no device), so a machine
//! without a usable GPU still gets a window instead of a panic. A software
//! adapter (a CPU Vulkan driver such as lavapipe, Windows' WARP) counts as no
//! GPU there: `adapter_use` declines it before a device is made (#2436).
//!
//! The choice comes from [`App::renderer`](crate::App::renderer), and the
//! `RINCH_RENDERER` environment variable (`auto`, `gpu`, `software` or
//! `cpu`) overrides it, so anyone can force a renderer without a rebuild.

use std::sync::OnceLock;
#[cfg(feature = "gpu")]
use std::sync::atomic::{AtomicBool, Ordering};

/// The renderer a desktop window presents with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// The GPU when the build has it (`gpu` feature) and it starts, software
    /// otherwise. An adapter that is a software driver (device type `Cpu`:
    /// lavapipe, Windows' WARP) counts as no GPU, so where it is the only
    /// adapter that can present, the window presents with rinch's own
    /// software renderer: GPU frames (`RenderSurface::set_gpu_render_callback`)
    /// do not run and `gpu_handle()` is `None`. [`Renderer::Gpu`] (or
    /// `RINCH_RENDERER=gpu`) uses such an adapter, as does an app that
    /// configured the device (`App::gpu_config` / `App::external_gpu`).
    #[default]
    Auto,
    /// The GPU, and a panic when it cannot start: for an app that would rather
    /// not run than run slowly. Any adapter that can present is used, a
    /// software driver included (with a warning in the log). A build without the `gpu` feature has no GPU
    /// renderer to insist on and presents with software (it logs a warning).
    Gpu,
    /// Software (CPU) rendering, without touching the GPU at all. Ignored,
    /// with a warning, by an app that configured the GPU device itself
    /// (`App::gpu_config` / `App::external_gpu`), which presents on the GPU.
    Software,
}

impl Renderer {
    /// Parse a `RINCH_RENDERER` value. `cpu` is an alias of `software`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "gpu" => Some(Self::Gpu),
            "software" | "cpu" => Some(Self::Software),
            _ => None,
        }
    }
}

static REQUESTED: OnceLock<Renderer> = OnceLock::new();

/// Install the app's choice. Called once by [`App::run`](crate::App::run)
/// before the window exists; a second call is ignored, like the GPU init.
pub(crate) fn set_requested(renderer: Renderer) {
    let _ = REQUESTED.set(renderer);
}

/// The renderer to use: `RINCH_RENDERER` when it names one, else the app's
/// choice, else [`Renderer::Auto`]. An unrecognised variable is reported and
/// ignored rather than guessed at.
pub(crate) fn resolved() -> Renderer {
    let env = std::env::var("RINCH_RENDERER").ok();
    resolve(REQUESTED.get().copied(), env.as_deref())
}

/// [`resolved`] without the globals, for tests.
pub(crate) fn resolve(app: Option<Renderer>, env: Option<&str>) -> Renderer {
    if let Some(value) = env.filter(|v| !v.trim().is_empty()) {
        match Renderer::parse(value) {
            Some(renderer) => return renderer,
            None => tracing::warn!(
                "RINCH_RENDERER={value:?} is not one of auto, gpu, software, cpu; ignoring it"
            ),
        }
    }
    app.unwrap_or_default()
}

/// What to do when the GPU renderer failed to start.
#[cfg(feature = "gpu")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OnGpuFailure {
    /// Present with software and say so in the log.
    FallBack,
    /// Stop: the app asked for the GPU and nothing else.
    Panic,
}

/// Fall back only under [`Renderer::Auto`] and only when the app did not
/// configure the GPU device itself (`App::gpu_config` / `App::external_gpu`):
/// such an app is building its own pipelines on rinch's device, and quietly
/// presenting without one would break it in a way that is harder to find than
/// the panic.
#[cfg(feature = "gpu")]
pub(crate) fn on_gpu_failure(choice: Renderer, app_configured_gpu: bool) -> OnGpuFailure {
    if choice == Renderer::Auto && !app_configured_gpu {
        OnGpuFailure::FallBack
    } else {
        OnGpuFailure::Panic
    }
}

/// Whether to try the GPU at all. `Software` skips it, except for an app that
/// configured the device itself (`App::gpu_config` / `App::external_gpu`):
/// that app builds its own pipelines on rinch's device and reads it through
/// `gpu_handle()`, so a `RINCH_RENDERER=cpu` set by a *user* must not take the
/// device away from it (it would find `gpu_handle()` answering `None` for
/// good). The override is reported and ignored there.
#[cfg(feature = "gpu")]
pub(crate) fn tries_gpu(choice: Renderer, app_configured_gpu: bool) -> bool {
    if choice != Renderer::Software {
        return true;
    }
    if app_configured_gpu {
        tracing::warn!(
            "rinch: the software renderer was requested, but this app configured the GPU \
             device itself (gpu_config / external_gpu); presenting on the GPU"
        );
        return true;
    }
    false
}

/// What rinch logs about the adapter a GPU renderer got (#2436): `info`, a
/// line naming it, and `warning`, set when the adapter is a software driver.
#[cfg(feature = "gpu")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AdapterReport {
    pub(crate) info: String,
    pub(crate) warning: Option<String>,
}

/// The report for an adapter described by `adapter`, presenting in `format`.
///
/// Why: a GPU that cannot present (headless sway, a VM without passthrough)
/// leaves wgpu only a CPU Vulkan driver such as lavapipe, which compiles
/// Vello's shaders and renders on the CPU at many times the cost of rinch's
/// software renderer. wgpu names its adapter only at `debug`, so without this
/// the log said `backend=Vulkan` and nothing else.
#[cfg(feature = "gpu")]
pub(crate) fn adapter_report(
    adapter: &wgpu::AdapterInfo,
    format: wgpu::TextureFormat,
) -> AdapterReport {
    let info = format!(
        "rinch-dom runtime: backend={:?}, format={format:?}, adapter={:?}, driver={:?}, \
         driver_info={:?}, device_type={:?}",
        adapter.backend, adapter.name, adapter.driver, adapter.driver_info, adapter.device_type
    );
    let warning = (adapter.device_type == wgpu::DeviceType::Cpu).then(|| {
        format!(
            "rinch: the GPU adapter {:?} (driver {:?}) is a software driver: it renders on \
             the CPU, and costs far more than rinch's own software renderer, which \
             RINCH_RENDERER=software selects",
            adapter.name, adapter.driver
        )
    });
    AdapterReport { info, warning }
}

/// Log [`adapter_report`]: the line at `info`, the warning at `warn`.
#[cfg(feature = "gpu")]
pub(crate) fn log_adapter(adapter: &wgpu::AdapterInfo, format: wgpu::TextureFormat) {
    let report = adapter_report(adapter, format);
    tracing::info!("{}", report.info);
    if let Some(warning) = report.warning {
        tracing::warn!("{warning}");
    }
}

/// Whether the GPU renderer uses the adapter wgpu offered.
#[cfg(feature = "gpu")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdapterUse {
    /// Build the device and Vello's pipelines on it.
    Use,
    /// Present with the software renderer instead.
    Decline,
}

/// Decline a software adapter (device type `Cpu`: lavapipe, Windows' WARP)
/// exactly where [`on_gpu_failure`] would fall back: under
/// [`Renderer::Auto`], and never for a device the app configured itself
/// (`App::gpu_config` / `App::external_gpu`), whose pipelines need it. Under
/// `Auto` a CPU adapter counts as "the GPU will not start" (#2436): rinch's
/// own software renderer starts and runs at a fraction of its cost
/// (`examples/wayland-bar` under headless sway: 2.0 G instructions from
/// start to its first frames and exit, against 33.6 G on lavapipe, 81% of
/// that LLVM compiling Vello's shaders). Asking for [`Renderer::Gpu`] uses it
/// anyway.
///
/// wgpu ranks a `Cpu` adapter last, so when it offers one for the window's
/// surface no hardware adapter can present to it.
#[cfg(feature = "gpu")]
pub(crate) fn adapter_use(
    choice: Renderer,
    app_configured_gpu: bool,
    adapter: &wgpu::AdapterInfo,
) -> AdapterUse {
    if adapter.device_type == wgpu::DeviceType::Cpu
        && on_gpu_failure(choice, app_configured_gpu) == OnGpuFailure::FallBack
    {
        AdapterUse::Decline
    } else {
        AdapterUse::Use
    }
}

/// The warning for an adapter [`adapter_use`] declined: what was chosen, what
/// follows from it for the app, and how to insist on the adapter.
#[cfg(feature = "gpu")]
pub(crate) fn declined_adapter_warning(adapter: &wgpu::AdapterInfo) -> String {
    format!(
        "rinch: the only GPU adapter that can present here, {:?} (driver {:?}), is a \
         software driver that renders on the CPU, so Renderer::Auto presents with rinch's \
         software renderer instead. GPU frames (RenderSurface::set_gpu_render_callback) \
         will not run and gpu_handle() is None. RINCH_RENDERER=gpu, or \
         App::renderer(Renderer::Gpu), uses this adapter anyway",
        adapter.name, adapter.driver
    )
}

#[cfg(feature = "gpu")]
static GPU_PRESENTING: AtomicBool = AtomicBool::new(false);

/// Whether the window is presenting through the GPU compositor. Set by the
/// runtime when it creates the window's renderer; the video path asks it
/// (video goes to the compositor on the GPU and paints inline on software).
#[cfg(feature = "gpu")]
pub(crate) fn gpu_presenting() -> bool {
    GPU_PRESENTING.load(Ordering::Relaxed)
}

#[cfg(feature = "gpu")]
pub(crate) fn set_gpu_presenting(on: bool) {
    GPU_PRESENTING.store(on, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_overrides_the_app() {
        assert_eq!(
            resolve(Some(Renderer::Gpu), Some("cpu")),
            Renderer::Software
        );
        assert_eq!(
            resolve(Some(Renderer::Software), Some("gpu")),
            Renderer::Gpu
        );
        assert_eq!(
            resolve(Some(Renderer::Software), Some(" Auto ")),
            Renderer::Auto
        );
    }

    #[test]
    fn without_the_environment_the_app_decides_and_auto_is_the_default() {
        assert_eq!(resolve(Some(Renderer::Software), None), Renderer::Software);
        assert_eq!(resolve(None, None), Renderer::Auto);
        assert_eq!(resolve(Some(Renderer::Gpu), Some("")), Renderer::Gpu);
    }

    #[test]
    fn an_unrecognised_environment_value_is_ignored() {
        assert_eq!(resolve(Some(Renderer::Gpu), Some("vulkan")), Renderer::Gpu);
        assert_eq!(resolve(None, Some("fast")), Renderer::Auto);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn only_auto_without_an_app_configured_device_falls_back() {
        assert_eq!(
            on_gpu_failure(Renderer::Auto, false),
            OnGpuFailure::FallBack
        );
        assert_eq!(on_gpu_failure(Renderer::Auto, true), OnGpuFailure::Panic);
        assert_eq!(on_gpu_failure(Renderer::Gpu, false), OnGpuFailure::Panic);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn software_does_not_take_the_device_from_an_app_that_configured_it() {
        assert!(!tries_gpu(Renderer::Software, false));
        assert!(tries_gpu(Renderer::Software, true));
        assert!(tries_gpu(Renderer::Auto, false));
        assert!(tries_gpu(Renderer::Gpu, true));
    }

    /// The adapters of #2436: RADV, which presents on the GPU, and lavapipe,
    /// which Vulkan falls back to when RADV cannot present.
    #[cfg(feature = "gpu")]
    fn adapter(name: &str, driver: &str, device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: name.into(),
            vendor: 0,
            device: 0,
            device_type,
            device_pci_bus_id: String::new(),
            driver: driver.into(),
            driver_info: "Mesa 26.0.8".into(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 8,
            subgroup_max_size: 64,
            transient_saves_memory: None,
            limit_bucket: None,
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_report_names_the_adapter_and_warns_only_on_a_cpu_one() {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let gpu = adapter(
            "AMD Radeon 890M Graphics (RADV STRIX1)",
            "radv",
            wgpu::DeviceType::IntegratedGpu,
        );
        assert_eq!(
            adapter_report(&gpu, format),
            AdapterReport {
                info: "rinch-dom runtime: backend=Vulkan, format=Rgba8Unorm, \
                       adapter=\"AMD Radeon 890M Graphics (RADV STRIX1)\", driver=\"radv\", \
                       driver_info=\"Mesa 26.0.8\", device_type=IntegratedGpu"
                    .into(),
                warning: None,
            }
        );

        let cpu = adapter(
            "llvmpipe (LLVM 21.1.8, 256 bits)",
            "llvmpipe",
            wgpu::DeviceType::Cpu,
        );
        let report = adapter_report(&cpu, format);
        assert!(report.info.contains("device_type=Cpu"), "{}", report.info);
        assert_eq!(
            report.warning.as_deref(),
            Some(
                "rinch: the GPU adapter \"llvmpipe (LLVM 21.1.8, 256 bits)\" (driver \
                 \"llvmpipe\") is a software driver: it renders on the CPU, and costs far \
                 more than rinch's own software renderer, which RINCH_RENDERER=software \
                 selects"
            )
        );
    }

    /// #2436: `Auto` declines a CPU adapter and nothing else; `Gpu` uses it;
    /// a device the app configured is never declined, whatever the choice.
    #[cfg(feature = "gpu")]
    #[test]
    fn auto_declines_a_cpu_adapter_and_nothing_else_does() {
        use wgpu::DeviceType::*;
        let cpu = adapter("llvmpipe", "llvmpipe", Cpu);
        assert_eq!(
            adapter_use(Renderer::Auto, false, &cpu),
            AdapterUse::Decline
        );
        assert_eq!(adapter_use(Renderer::Gpu, false, &cpu), AdapterUse::Use);
        assert_eq!(adapter_use(Renderer::Auto, true, &cpu), AdapterUse::Use);
        assert_eq!(adapter_use(Renderer::Gpu, true, &cpu), AdapterUse::Use);
        // `Software` with an app-configured device presents on the GPU
        // (`tries_gpu`), and keeps the adapter it has.
        assert_eq!(adapter_use(Renderer::Software, true, &cpu), AdapterUse::Use);
        for device_type in [Other, IntegratedGpu, DiscreteGpu, VirtualGpu] {
            let hardware = adapter("a", "b", device_type);
            for configured in [false, true] {
                assert_eq!(
                    adapter_use(Renderer::Auto, configured, &hardware),
                    AdapterUse::Use,
                    "{device_type:?}"
                );
            }
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_decline_says_what_follows_and_how_to_insist() {
        let cpu = adapter(
            "llvmpipe (LLVM 21.1.8, 256 bits)",
            "llvmpipe",
            wgpu::DeviceType::Cpu,
        );
        assert_eq!(
            declined_adapter_warning(&cpu),
            "rinch: the only GPU adapter that can present here, \"llvmpipe (LLVM 21.1.8, \
             256 bits)\" (driver \"llvmpipe\"), is a software driver that renders on the \
             CPU, so Renderer::Auto presents with rinch's software renderer instead. GPU \
             frames (RenderSurface::set_gpu_render_callback) will not run and gpu_handle() \
             is None. RINCH_RENDERER=gpu, or App::renderer(Renderer::Gpu), uses this \
             adapter anyway"
        );
    }

    /// Only `Cpu` is a software driver: a virtual GPU (a VM's passthrough
    /// or paravirtual device) and an unknown type may be hardware.
    #[cfg(feature = "gpu")]
    #[test]
    fn only_the_cpu_device_type_warns() {
        use wgpu::DeviceType::*;
        for device_type in [Other, IntegratedGpu, DiscreteGpu, VirtualGpu, Cpu] {
            let report = adapter_report(
                &adapter("a", "b", device_type),
                wgpu::TextureFormat::Bgra8Unorm,
            );
            assert_eq!(
                report.warning.is_some(),
                device_type == Cpu,
                "{device_type:?}"
            );
        }
    }
}
