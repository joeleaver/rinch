//! The GPU device, and what outlives any one window (`android-gpu`).

use std::sync::atomic::{AtomicBool, Ordering};

use super::gpu_surface::{AndroidWindow, GpuSurface};

/// The device, and everything standing on it that outlives any one window.
///
/// **This struct exists because of when things die.** A Vulkan device, and the
/// twenty-odd compute pipelines vello compiles against it, belong to the
/// process: nothing Android does to an Activity invalidates them. A swapchain
/// belongs to the `ANativeWindow`, and Android destroys that window every time
/// the Activity is stopped — every press of Home, every task switch, every
/// screen rotation on some devices. Those are different lifetimes, so they are
/// different structs, and the loop holds one binding for each.
///
/// The version of this file that read the frame back to the CPU had no
/// swapchain and therefore no reason to make the distinction: it kept device
/// and target in one `GpuSurface` and dropped the lot on `TerminateWindow`.
/// Doing that now would recompile vello's pipeline set on every resume, and
/// the logcat timestamps say what that costs. On a moto g stylus 5G, cold:
///
/// ```text
/// 18:28:44.113  vello: initialising shader modules in parallel using 6 threads
/// 18:28:45.350  GPU: Adreno (TM) 619 (Vulkan)          <- 1,237 ms later
/// ```
///
/// and then, pressing Home and returning twice:
///
/// ```text
/// 18:32:12.476  InitWindow: 1080x2460
/// 18:32:12.484  GPU surface: formats=[...]             <- 8 ms
/// 18:32:17.620  InitWindow: 1080x2460
/// 18:32:17.624  GPU surface: formats=[...]             <- 4 ms
/// ```
///
/// Those two lines are a whole swapchain rebuild, and no `GPU: Adreno` line
/// appears between them, which is the observable proof that the device was
/// reused. One struct instead of two would have made every resume a
/// 1.2-second black screen in exchange for throwing away a device Vulkan
/// never invalidated.
/// Set by the device-lost callback, cleared by [`GpuContext::attach`]. The
/// callback is a plain `fn` with no access to the loop's bindings, so the flag
/// is how it reaches them.
#[cfg(feature = "android-gpu")]
static DEVICE_LOST: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "android-gpu")]
pub(super) struct GpuContext {
    /// Kept because a surface must be created by the same instance that will
    /// present it, and the surfaces are created long after this struct is —
    /// once per window, for as many windows as the Activity is given.
    instance: wgpu::Instance,
    /// Kept because `get_capabilities` is a question about an (adapter,
    /// surface) *pair*: the formats and present modes a window supports have
    /// to be asked again for each new window, not cached from the first.
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) renderer: vello::Renderer,
}

#[cfg(feature = "android-gpu")]
impl GpuContext {
    /// Give the window a swapchain, bringing the device up if this is the
    /// first window the process has seen.
    ///
    /// Called from `InitWindow` and nowhere else. The `slot` argument is the
    /// loop's `gpu` binding: on the first call it is filled in and on every
    /// later one it is reused, so a resume costs a swapchain and not a device.
    ///
    /// Reused, that is, unless Vulkan took it away. Caching the device across
    /// windows is what makes a resume cheap, and it is also the one way this
    /// shell could hold a *dead* device for ever: the version of this file that
    /// built a device per window recovered from a device loss on the next
    /// `InitWindow` without anyone noticing it was a recovery. `DEVICE_LOST`
    /// keeps that property — the callback below can only log (it is handed no
    /// state), so it records the loss and the next window throws the context
    /// away and builds a fresh one.
    pub(super) fn attach(
        slot: &mut Option<GpuContext>,
        window: &ndk::native_window::NativeWindow,
        width: u32,
        height: u32,
    ) -> Option<GpuSurface> {
        if DEVICE_LOST.swap(false, Ordering::AcqRel) && slot.is_some() {
            log::warn!("GPU: device was lost; rebuilding the context for this window");
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(GpuContext::new(window)?);
        }
        let ctx = slot.as_ref()?;

        let surface = match ctx.instance.create_surface(AndroidWindow {
            native: window.clone(),
        }) {
            Ok(s) => s,
            Err(e) => {
                log::error!("GPU: create_surface failed: {e}");
                return None;
            }
        };

        // The device was chosen for the *first* window this process saw. A
        // later window is a different `ANativeWindow` and, in principle, a
        // different `VkSurfaceKHR` with its own presentation-support answer.
        // In practice every Android surface on a device is supported by the
        // same queue family, so this has never been seen to fail — but
        // presenting to a surface the adapter cannot present to is undefined
        // behaviour, not an error, and the check costs one call at window
        // creation.
        if !ctx.adapter.is_surface_supported(&surface) {
            log::error!("GPU: adapter cannot present to this window; no GPU frames");
            return None;
        }

        GpuSurface::new(ctx, surface, width, height)
    }

    /// Bring up the instance, adapter, device and renderer.
    ///
    /// Takes the window because the adapter has to be chosen *for* a surface:
    /// `request_adapter`'s `compatible_surface` is what makes wgpu pick a
    /// queue family that can actually present, and a device whose queue cannot
    /// present is a device that can draw the frame and not show it. The
    /// surface built here is a probe — it is dropped at the end of this
    /// function and the real one is built by [`attach`](Self::attach) from the
    /// instance this leaves behind.
    ///
    /// **The probe must not outlive this function.**
    /// `vkCreateAndroidSurfaceKHR` is where the `ANativeWindow` is connected to
    /// `NATIVE_WINDOW_API_EGL` — libvulkan calls `native_window_api_connect`
    /// there, not at swapchain creation, and returns
    /// `VK_ERROR_NATIVE_WINDOW_IN_USE_KHR` for a second connect. So the probe
    /// and the real surface can only ever exist one at a time, and this works
    /// because `probe` is a local that drops before `attach` creates the other
    /// one. Do not hoist it into the returned struct.
    fn new(window: &ndk::native_window::NativeWindow) -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::empty(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });

        let probe = match instance.create_surface(AndroidWindow {
            native: window.clone(),
        }) {
            Ok(s) => s,
            Err(e) => {
                log::error!("GPU: probe surface failed: {e}");
                return None;
            }
        };

        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&probe),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("GPU: adapter failed: {e}");
                    return None;
                }
            };

        // Every adapter feature but the experimental ones (which need
        // `experimental_features` opted in) and passthrough shaders (an
        // experimental feature until wgpu 28 renamed it; still excluded as
        // before). wgpu 30 adds experimental features the old hand-written
        // list did not name, so take wgpu's own mask (#1955).
        let experimental =
            wgpu::Features::all_experimental_mask() | wgpu::Features::PASSTHROUGH_SHADERS;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rinch-android-gpu"),
            required_features: adapter.features() - experimental,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .map_err(|e| log::error!("GPU: device failed: {e}"))
        .ok()?;

        device.set_device_lost_callback(|reason, msg| {
            log::error!("GPU DEVICE LOST: reason={reason:?} msg={msg}");
            DEVICE_LOST.store(true, Ordering::Release);
        });

        // `use_cpu: false`, which is the whole point of having a device.
        //
        // It read `true` until card K27 measured it. Vello's `use_cpu` swaps
        // its compute pipeline for CPU implementations of the same stages —
        // a debugging aid — so this feature initialised Vulkan, allocated a
        // GPU texture, and then rasterised on the same four cores the
        // software painter uses. On a moto g stylus 5G at 1080x2460 that cost
        // 31ms a frame on the library list and 56ms on a screen with a
        // rasterised PDF page; with the flag corrected the same frames took
        // **2.3ms and 5.0ms**. Nothing else about the path changed.
        // Through `gpu_init` like the desktop's (#2468): at most four
        // shader-compile threads, `RINCH_GPU_INIT_THREADS` to choose.
        let renderer = crate::gpu_init::new_renderer(&device, vello::AaSupport::area_only())
            .map_err(|e| log::error!("GPU: vello renderer failed: {e}"))
            .ok()?;

        let info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}), driver {}",
            info.name,
            info.backend,
            info.driver_info
        );

        Some(Self {
            instance,
            adapter,
            device,
            queue,
            renderer,
        })
    }
}
