//! Which renderer presents the desktop window, chosen at run time.
//!
//! A `desktop` build always carries the software renderer (TinySkia painting,
//! softbuffer presenting). A `desktop` + `gpu` build carries the GPU one
//! (Vello over wgpu) as well, and picks between them when it creates the
//! window: [`Renderer::Auto`] tries the GPU and falls back to software when
//! the GPU cannot start (no adapter, no surface, no device), so a machine
//! without a usable GPU still gets a window instead of a panic.
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
    /// otherwise.
    #[default]
    Auto,
    /// The GPU, and a panic when it cannot start: for an app that would rather
    /// not run than run slowly. A build without the `gpu` feature has no GPU
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
}
