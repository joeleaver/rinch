//! The antialiasing the GPU renderer draws a window's scene with (#2286,
//! stage 1a).
//!
//! The default is [`GpuAntialiasing::Area`]: Vello's analytic area coverage,
//! about 240 coverage levels on an edge. Until #2286 the desktop and the embed
//! overlay drew with `Msaa16`, which gives an edge at most 16 levels (15
//! distinct partial ones were measured on text) and on lavapipe cost 37.7 ms
//! for a full page of 13px text where Area costs 24.8 ms; on a phone GPU
//! (Adreno 619, `shell/android_runtime.rs`) Msaa16 was 3.5x Area for the whole
//! frame. Android always drew with Area.
//!
//! `Msaa16` and `Msaa8` stay selectable, through
//! [`App::gpu_antialiasing`](crate::app_builder::App::gpu_antialiasing),
//! [`set_gpu_antialiasing`] or `RINCH_GPU_AA=area|msaa8|msaa16`, as an escape
//! hatch for a path that overlaps itself: area coverage adds up a path's
//! signed area in each pixel, so where one path crosses itself or its contours
//! overlap (a self-crossing stroke, overlapping contours in a variable font)
//! an edge pixel is shaded by the sum, where MSAA counts the samples the union
//! covers. It is not a cure for seams between separate paths that abut: Vello
//! blends each path with its own coverage in both modes, and the #2286 review
//! measured such seams about equal under tiny-skia, Msaa16 and Area.
//!
//! The mode is fixed for the process the first time a renderer asks for it,
//! because a Vello renderer is built with the pipelines for one mode: a
//! [`set_gpu_antialiasing`] after that is logged and ignored. Android ignores
//! the setting and always draws with Area.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// How the GPU renderer antialiases the scene: every shape, image edge and
/// glyph in a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GpuAntialiasing {
    /// Analytic area coverage (Vello's `AaConfig::Area`): the smoothest edges
    /// and the cheapest fine pass. The default.
    #[default]
    Area,
    /// 8 samples per pixel (`AaConfig::Msaa8`).
    Msaa8,
    /// 16 samples per pixel (`AaConfig::Msaa16`): what rinch drew with before
    /// #2286.
    Msaa16,
}

impl GpuAntialiasing {
    /// Vello's name for it.
    pub fn aa_config(self) -> vello::AaConfig {
        match self {
            GpuAntialiasing::Area => vello::AaConfig::Area,
            GpuAntialiasing::Msaa8 => vello::AaConfig::Msaa8,
            GpuAntialiasing::Msaa16 => vello::AaConfig::Msaa16,
        }
    }

    /// The pipelines a renderer drawing with it needs: this mode's only.
    pub fn aa_support(self) -> vello::AaSupport {
        vello::AaSupport {
            area: self == GpuAntialiasing::Area,
            msaa8: self == GpuAntialiasing::Msaa8,
            msaa16: self == GpuAntialiasing::Msaa16,
        }
    }

    /// `area`, `msaa8` or `msaa16`, trimmed, in any case.
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "area" => Some(GpuAntialiasing::Area),
            "msaa8" => Some(GpuAntialiasing::Msaa8),
            "msaa16" => Some(GpuAntialiasing::Msaa16),
            _ => None,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            GpuAntialiasing::Area => 1,
            GpuAntialiasing::Msaa8 => 2,
            GpuAntialiasing::Msaa16 => 3,
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(GpuAntialiasing::Area),
            2 => Some(GpuAntialiasing::Msaa8),
            3 => Some(GpuAntialiasing::Msaa16),
            _ => None,
        }
    }
}

/// The app's request, `0` for none.
static REQUESTED: AtomicU8 = AtomicU8::new(0);
/// The mode, once a renderer has asked.
static FIXED: OnceLock<GpuAntialiasing> = OnceLock::new();

/// Ask for `mode` (an app's [`App::gpu_antialiasing`] stores it this way;
/// an embedding host calls it before it creates its overlay renderer). Takes
/// effect only before the first GPU renderer is made; afterwards it is logged
/// and ignored. `RINCH_GPU_AA` overrides it.
///
/// [`App::gpu_antialiasing`]: crate::app_builder::App::gpu_antialiasing
pub fn set_gpu_antialiasing(mode: GpuAntialiasing) {
    REQUESTED.store(mode.to_u8(), Ordering::SeqCst);
    if let Some(fixed) = FIXED.get()
        && *fixed != mode
    {
        tracing::warn!(
            "set_gpu_antialiasing({mode:?}) after the GPU renderer was made with {fixed:?}: ignored"
        );
    }
}

/// The mode the GPU renderer draws with: `RINCH_GPU_AA` if it names one, else
/// the app's request, else [`GpuAntialiasing::Area`]. Fixed by the first call.
pub fn gpu_antialiasing() -> GpuAntialiasing {
    *FIXED.get_or_init(|| {
        let env = std::env::var("RINCH_GPU_AA").ok();
        resolve(
            env.as_deref(),
            GpuAntialiasing::from_u8(REQUESTED.load(Ordering::SeqCst)),
        )
    })
}

/// The environment's value over the app's request over the default; an
/// unrecognised value is logged and ignored, an empty one ignored.
fn resolve(env: Option<&str>, requested: Option<GpuAntialiasing>) -> GpuAntialiasing {
    let from_env = env.filter(|v| !v.trim().is_empty()).and_then(|v| {
        let mode = GpuAntialiasing::parse(v);
        if mode.is_none() {
            tracing::warn!("RINCH_GPU_AA={v:?} is not area, msaa8 or msaa16: ignored");
        }
        mode
    });
    from_env.or(requested).unwrap_or_default()
}

/// The `AaConfig` every window and overlay scene is rendered with.
///
/// In this crate's own test build it is `TestAa`'s per-thread mode,
/// `Msaa16` unless a test pins another: an area-coverage render of a scene is
/// not repeatable on a parallel GPU (see the module docs), and the logic
/// fixtures (damage, culls, split slices) compare renders exactly. The
/// dedicated area-coverage fixtures pin [`GpuAntialiasing::Area`] and compare
/// within 1/255.
pub(crate) fn scene_aa() -> vello::AaConfig {
    scene_mode().aa_config()
}

#[cfg(not(test))]
fn scene_mode() -> GpuAntialiasing {
    gpu_antialiasing()
}

#[cfg(test)]
fn scene_mode() -> GpuAntialiasing {
    TEST_AA.with(|m| m.get())
}

/// The `AaSupport` every window and overlay renderer is made with (every
/// mode in this crate's test build, so a test can pin either).
pub(crate) fn scene_aa_support() -> vello::AaSupport {
    if cfg!(test) {
        vello::AaSupport::all()
    } else {
        gpu_antialiasing().aa_support()
    }
}

#[cfg(test)]
thread_local! {
    static TEST_AA: std::cell::Cell<GpuAntialiasing> =
        const { std::cell::Cell::new(GpuAntialiasing::Msaa16) };
}

/// Pins this thread's [`scene_aa`] to a mode until dropped (tests only).
// Only the window GPU tests use it; gated like them so `--features embed`
// test builds (no window shell) don't warn it is dead.
#[cfg(all(test, software_shell, feature = "gpu"))]
pub(crate) struct TestAa(GpuAntialiasing);

#[cfg(all(test, software_shell, feature = "gpu"))]
impl TestAa {
    /// Pin `mode` on this thread.
    pub(crate) fn pin(mode: GpuAntialiasing) -> Self {
        TestAa(TEST_AA.with(|m| m.replace(mode)))
    }
}

#[cfg(all(test, software_shell, feature = "gpu"))]
impl Drop for TestAa {
    fn drop(&mut self) {
        TEST_AA.with(|m| m.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_is_the_default_and_the_env_wins() {
        assert_eq!(resolve(None, None), GpuAntialiasing::Area);
        assert_eq!(
            resolve(None, Some(GpuAntialiasing::Msaa16)),
            GpuAntialiasing::Msaa16
        );
        assert_eq!(
            resolve(Some(" MSAA8 "), Some(GpuAntialiasing::Msaa16)),
            GpuAntialiasing::Msaa8
        );
        assert_eq!(
            resolve(Some("bogus"), Some(GpuAntialiasing::Msaa16)),
            GpuAntialiasing::Msaa16
        );
        assert_eq!(resolve(Some(""), None), GpuAntialiasing::Area);
    }

    #[test]
    fn a_renderer_gets_its_modes_pipelines_only() {
        let s = GpuAntialiasing::Area.aa_support();
        assert!(s.area && !s.msaa8 && !s.msaa16);
        let s = GpuAntialiasing::Msaa16.aa_support();
        assert!(!s.area && !s.msaa8 && s.msaa16);
        assert_eq!(GpuAntialiasing::Msaa8.aa_config(), vello::AaConfig::Msaa8);
    }
}
