//! Display metrics via JNI (accurate hardware DPI, safe area insets).

use crate::bridge;

/// Safe area insets in physical pixels (accounts for status bar, navigation bar, notch).
#[derive(Clone, Copy, Debug, Default)]
pub struct SafeAreaInsets {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

/// Get the safe area insets (status bar, navigation bar, display cutout) in physical pixels.
/// Divide by scale_factor to get logical pixels.
pub fn safe_area_insets() -> SafeAreaInsets {
    bridge::with_activity(|env, activity| {
        let mut insets = SafeAreaInsets::default();
        let result = env.call_method(activity, "getSafeAreaInsets", "()[I", &[]);
        if let Ok(val) = result {
            if let Ok(obj) = val.l() {
                if !obj.is_null() {
                    let arr: jni::objects::JIntArray = obj.into();
                    let mut buf = [0i32; 4];
                    if env.get_int_array_region(&arr, 0, &mut buf).is_ok() {
                        insets.top = buf[0];
                        insets.bottom = buf[1];
                        insets.left = buf[2];
                        insets.right = buf[3];
                    }
                }
            }
        }
        insets
    })
}

/// The refresh rate of the display this activity is on, in Hz.
///
/// **What the frame loop wants this for.** Until card K37 the Android loop
/// polled with a hard-coded 16ms timeout, and 16ms is one frame *if the panel
/// runs at 60Hz*. The moto g stylus 5G this was measured on reports three
/// modes — 60, 90 and 120Hz — and boots with 120 as its base mode, so a 16ms
/// clock was not a small approximation there, it was a hard 2x cap: the app
/// could not paint faster than 60fps on a 120Hz screen however cheap its
/// frames became. Asking the display what a frame costs makes the loop's
/// deadline the panel's deadline.
///
/// `getRefreshRate()` reports the mode that is *currently* active, and Android
/// switches modes underneath an app as it pleases — this handset answers
/// 120.00001 while the activity is foreground and 60.000004 a second or two
/// after it is not, with no notification either way. So this is an answer with
/// a shelf life, and the caller treats it as one: the shell reads it at
/// `InitWindow` and re-reads it once a second while it is drawing frames (see
/// `RATE_RECHECK` in `shell::android_runtime`), which costs one JNI call per
/// second of animation and nothing at all on an idle app.
///
/// Even stale, it is not the only thing pacing the loop: the swapchain's `Fifo`
/// present and, on the software path, `ANativeWindow_lock` waiting for a free
/// buffer both block on the display itself. Guessing too fast costs a little
/// sleep moved from our timeout into their block; guessing too slow costs
/// frames. Between those two, the honest reading of the active mode is the one
/// that is usually right and never catastrophic.
///
/// `None` if the JNI call chain fails, which leaves the caller to pick its own
/// fallback rather than have one invented here.
pub fn refresh_rate_hz() -> Option<f32> {
    bridge::with_activity(|env, activity| {
        // `getWindowManager().getDefaultDisplay().getRefreshRate()`.
        //
        // `getDefaultDisplay` is deprecated in favour of `Activity.getDisplay`
        // (API 30), and deliberately used anyway: it is present on every API
        // level this app is built against, it needs no version check at the
        // call site, and "deprecated" in Android means "still here", not
        // "gone". The newer call is the one to move to when the minimum SDK
        // makes it unconditional.
        let wm = env
            .call_method(
                activity,
                "getWindowManager",
                "()Landroid/view/WindowManager;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        let display = env
            .call_method(&wm, "getDefaultDisplay", "()Landroid/view/Display;", &[])
            .ok()?
            .l()
            .ok()?;
        let hz = env
            .call_method(&display, "getRefreshRate", "()F", &[])
            .ok()?
            .f()
            .ok()?;
        // A panel that claims an implausible rate is a panel whose answer we
        // should not be dividing by. 20Hz is below anything shipping and
        // 1000Hz is above it; either means the call chain returned something
        // that is not a refresh rate.
        (hz > 20.0 && hz < 1000.0).then_some(hz)
    })
}

pub fn density_dpi() -> Option<i32> {
    bridge::with_activity(|env, activity| {
        // getResources().getDisplayMetrics().densityDpi
        let resources = env
            .call_method(
                activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        let metrics = env
            .call_method(
                &resources,
                "getDisplayMetrics",
                "()Landroid/util/DisplayMetrics;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        let dpi = env.get_field(&metrics, "densityDpi", "I").ok()?.i().ok()?;
        Some(dpi)
    })
}

/// Tell Android that the status bar is drawn over a light background, so its
/// clock and icons should be dark.
///
/// The system draws the bar's contents, not the app, and its default is
/// light-on-dark. An app that goes edge-to-edge behind a pale background gets
/// white glyphs on cream until it says otherwise, which is unreadable rather
/// than merely wrong. `true` means "light background, dark contents".
///
/// Separate from [`set_light_navigation_bars`] because the two bars are over
/// different parts of the app: a pale page under a dark bottom bar is an
/// ordinary design, and one combined switch would force it to lie about one
/// end. Call both when the whole app is one shade.
pub fn set_light_status_bars(light: bool) {
    set_bar_appearance("setLightStatusBars", light);
}

/// The same for the navigation bar's own contents — the three buttons, or the
/// gesture pill. See [`set_light_status_bars`].
pub fn set_light_navigation_bars(light: bool) {
    set_bar_appearance("setLightNavigationBars", light);
}

fn set_bar_appearance(method: &str, light: bool) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            method,
            "(Z)V",
            &[jni::objects::JValue::Bool(light as jni::sys::jboolean)],
        ) {
            log::warn!("{method} failed: {e}");
        }
    });
}
