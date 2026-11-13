//! How rinch gets the GPU device a surface's GPU render callback draws with
//! (#57, #2085, #1768): extra requirements for the device rinch requests
//! ([`RinchGpuConfig`]), or a whole stack the app made ([`ExternalGpu`]).
//!
//! The same two types on both halves of the API. On the desktop they go to
//! [`App::gpu_config`](crate::App::gpu_config) and
//! [`App::external_gpu`](crate::App::external_gpu); in the browser (the
//! `web-gpu` feature) to `rinch::web_gpu::set_gpu_config` and
//! `rinch::web_gpu::set_external_gpu`.

use std::sync::Arc;

use wgpu::{Adapter, Device, Features, Instance, Limits, Queue};

/// Extra GPU device requirements for the device rinch requests.
///
/// By default rinch requests its wgpu device with `Features::default()` /
/// `Limits::default()`. An application whose own renderer needs a
/// higher-capability device (extra features, larger storage buffers, more bind
/// groups, …) can raise those requirements here, so that the device rinch
/// draws with can also host the application's pipelines and textures. On the
/// desktop that is the device the compositor presents with, published via
/// `gpu_handle`, for **zero-copy** present; on the web it is the WebGPU device
/// every surface's GPU frames are drawn with.
///
/// rinch still creates the instance, picks the adapter, and owns the device,
/// so presentation is always correct. Use this via `App::gpu_config` on the
/// desktop and `rinch::web_gpu::set_gpu_config` on the web. The required
/// features and limits are passed through verbatim — if the adapter cannot
/// satisfy them, device creation fails loudly (naming the missing features)
/// rather than silently dropping a capability. Optional features are requested
/// only where the adapter has them (#2085); on the desktop
/// `GpuHandle::granted_optional_features` says which were, and on either
/// `device.features()` does.
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
    /// frames, say. What the device got is `device.features()` (and on the
    /// desktop `GpuHandle::granted_optional_features`).
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

/// A fully app-provided GPU stack.
///
/// When an application would rather own device creation with its exact
/// `DeviceDescriptor`, it can hand the whole stack to rinch. rinch does **not**
/// call `request_device`; the provided device is the one rinch draws with.
///
/// - **Desktop** (`App::external_gpu`): rinch creates only the window surface
///   (from the provided `instance`) and validates that `adapter` can present
///   to it. The device becomes the one published through `gpu_handle`.
/// - **Web** (`rinch::web_gpu::set_external_gpu`): rinch makes each surface's
///   canvas surface from `instance` and draws its GPU frames on `device`. The
///   adapter must be a WebGPU one (`Backend::BrowserWebGpu`).
///
/// The `instance` must be the one the `adapter`/`device` were created from, so
/// that the surfaces rinch creates are compatible with them.
#[derive(Clone)]
pub struct ExternalGpu {
    pub instance: Instance,
    pub adapter: Arc<Adapter>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
}
