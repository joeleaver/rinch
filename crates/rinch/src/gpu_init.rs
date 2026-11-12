//! How a GPU renderer is built: the threads that compile Vello's shaders,
//! and what is returned to the system afterwards (issue #2468).
//!
//! A [`vello::Renderer`] compiles its shaders on `num_init_threads` threads.
//! Left to Vello (`None`), that is `max(available_parallelism, 4) - 2`, one
//! per shader at most: 20 on a 24-CPU machine. With a cold driver shader
//! cache, each thread's compiler leaves freed memory behind in that thread's
//! malloc arena, and the process keeps it for as long as it runs. A
//! `Renderer::new` measured on a Radeon 890M (RADV, Mesa 26.0.8), cold, kept
//! this much anonymous memory:
//!
//! | init threads | wall | kept | kept after `malloc_trim` |
//! |---|---|---|---|
//! | Vello's default (20) | 124-148 ms | 51-53 MB | 19-22 MB |
//! | 4 | 182-250 ms | 34-36 MB | 12-16 MB |
//! | 2 | 250-317 ms | 22-23 MB | 7-12 MB |
//! | 1 | 432-454 ms | 16 MB | 4 MB |
//!
//! With a warm cache, every count takes 11-50 ms and keeps 3-6 MB.
//!
//! So rinch builds its renderers (desktop, embed and Android) with at most `DEFAULT_MAX_INIT_THREADS`
//! threads (one on macOS, as Vello's own default is there), and on glibc
//! calls `malloc_trim` once a renderer is built. Together they keep about
//! 14 MB where Vello's default kept 52 MB, for at most about 0.1 s more on a
//! cold cache, which a user meets on the first start after a driver update.
//! An app that wants every core for a faster cold start, or one thread,
//! chooses with [`set_gpu_init_threads`],
//! [`App::gpu_init_threads`](crate::app_builder::App::gpu_init_threads) or
//! `RINCH_GPU_INIT_THREADS`.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The most threads rinch compiles a renderer's shaders on by default (one
/// on macOS). See the module docs for what was measured.
pub const DEFAULT_MAX_INIT_THREADS: usize = 4;

/// The app's request; `0` for none.
static REQUESTED: AtomicUsize = AtomicUsize::new(0);

/// Compile each GPU renderer's shaders on `threads` threads from now on
/// (issue #2468). `RINCH_GPU_INIT_THREADS` overrides it. Renderers already
/// built are not affected.
pub fn set_gpu_init_threads(threads: NonZeroUsize) {
    REQUESTED.store(threads.get(), Ordering::SeqCst);
}

/// The threads a new GPU renderer compiles its shaders on:
/// `RINCH_GPU_INIT_THREADS` if it is a positive number, else the app's
/// [`set_gpu_init_threads`], else the default.
pub fn gpu_init_threads() -> NonZeroUsize {
    let env = std::env::var("RINCH_GPU_INIT_THREADS").ok();
    resolve(
        env.as_deref(),
        NonZeroUsize::new(REQUESTED.load(Ordering::SeqCst)),
        std::thread::available_parallelism().ok(),
    )
}

/// The environment's value over the app's request over the default, which
/// is the machine's parallelism capped at [`DEFAULT_MAX_INIT_THREADS`] (one
/// on macOS). An unparsable or zero value is logged and ignored, an empty one
/// ignored.
fn resolve(
    env: Option<&str>,
    requested: Option<NonZeroUsize>,
    parallelism: Option<NonZeroUsize>,
) -> NonZeroUsize {
    let from_env = env.filter(|v| !v.trim().is_empty()).and_then(|v| {
        let n = v.trim().parse::<usize>().ok().and_then(NonZeroUsize::new);
        if n.is_none() {
            tracing::warn!("RINCH_GPU_INIT_THREADS={v:?} is not a positive number: ignored");
        }
        n
    });
    from_env.or(requested).unwrap_or_else(|| {
        let max = if cfg!(target_os = "macos") {
            1
        } else {
            DEFAULT_MAX_INIT_THREADS
        };
        let n = parallelism.map_or(1, NonZeroUsize::get).min(max);
        NonZeroUsize::new(n).unwrap_or(NonZeroUsize::MIN)
    })
}

/// The options rinch builds a window's or an overlay's Vello renderer with.
pub(crate) fn renderer_options(antialiasing_support: vello::AaSupport) -> vello::RendererOptions {
    vello::RendererOptions {
        antialiasing_support,
        use_cpu: false,
        num_init_threads: Some(gpu_init_threads()),
        pipeline_cache: None,
    }
}

/// [`vello::Renderer::new`] with [`renderer_options`], then hand the memory
/// the shader compilers freed back to the system (glibc keeps it in the
/// compile threads' arenas otherwise; see the module docs).
pub(crate) fn new_renderer(
    device: &wgpu::Device,
    antialiasing_support: vello::AaSupport,
) -> Result<vello::Renderer, vello::Error> {
    let renderer = vello::Renderer::new(device, renderer_options(antialiasing_support));
    release_freed_memory();
    renderer
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_freed_memory() {
    // SAFETY: `malloc_trim` takes no pointers; it only returns free memory
    // at the top of each glibc arena (and whole free pages inside them) to
    // the system.
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_freed_memory() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: usize) -> Option<NonZeroUsize> {
        NonZeroUsize::new(v)
    }

    #[test]
    fn the_default_caps_the_machines_parallelism() {
        let cap = if cfg!(target_os = "macos") { 1 } else { 4 };
        assert_eq!(resolve(None, None, n(24)).get(), cap);
        assert_eq!(resolve(None, None, n(3)).get(), 3.min(cap));
        assert_eq!(resolve(None, None, n(1)).get(), 1);
        assert_eq!(resolve(None, None, None).get(), 1);
    }

    #[test]
    fn the_environment_wins_over_the_apps_request_over_the_default() {
        assert_eq!(resolve(None, n(12), n(24)).get(), 12);
        assert_eq!(resolve(Some(" 2 "), n(12), n(24)).get(), 2);
        assert_eq!(resolve(Some("16"), None, n(4)).get(), 16);
        // Unusable values are ignored.
        assert_eq!(resolve(Some("0"), n(12), n(24)).get(), 12);
        assert_eq!(resolve(Some("many"), n(12), n(24)).get(), 12);
        assert_eq!(resolve(Some(""), n(12), n(24)).get(), 12);
    }

    #[test]
    fn renderers_get_the_resolved_count() {
        let opts = renderer_options(vello::AaSupport::area_only());
        assert_eq!(opts.num_init_threads, Some(gpu_init_threads()));
        assert!(!opts.use_cpu);
    }
}
