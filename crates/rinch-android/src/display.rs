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

/// The size of the window this activity draws into, in physical pixels, as
/// `(width, height)`.
///
/// **What this is for.** Everything else in this module answers a question
/// about the screen except the first one an app asks: how wide is it. The
/// width handed to `run_android_with_fonts` is not it — Android ignores that
/// number and the shell lays out against whatever the `ANativeWindow` turned
/// out to be — so an app that needs a pixel width for something it rasterises
/// has had nothing to ask. SetListArray's card K31 is where that surfaced: it
/// passes a 393-wide window because 393 is the canvas its designs were drawn
/// on, and on a moto g stylus 5G, which is 1080 physical pixels at density 400
/// and therefore 432 logical, every page it drew came out 393 wide with a
/// strip of backdrop down each side. Wrong by a different amount on every
/// handset, and invisible in a simulator that is 393 wide by construction.
///
/// Physical pixels, matching [`safe_area_insets`], because that is the unit
/// Android measures in. Divide by `density_dpi() / 160` — the same scale
/// factor `shell::android_runtime` derives at `InitWindow` — to reach the
/// logical pixels a stylesheet is written in, and the answer will agree with
/// the size the shell laid out at.
///
/// `None` if the JNI call chain fails or the platform reports a non-positive
/// size, which leaves the caller to pick its own fallback rather than have one
/// invented here — the same contract [`density_dpi`] keeps.
pub fn viewport_size() -> Option<(u32, u32)> {
    bridge::with_activity(|env, activity| {
        let obj = env
            .call_method(activity, "getViewportSize", "()[I", &[])
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let arr: jni::objects::JIntArray = obj.into();
        let mut buf = [0i32; 2];
        env.get_int_array_region(&arr, 0, &mut buf).ok()?;
        // A zero or negative dimension is not a small window, it is a window
        // that was not there to measure — the activity asked before it had one.
        // Saying `None` sends the caller to its fallback; dividing by it would
        // send it somewhere much stranger.
        (buf[0] > 0 && buf[1] > 0).then_some((buf[0] as u32, buf[1] as u32))
    })
}

/// How much of the bottom edge the soft keyboard is covering right now, in
/// physical pixels. Zero when it is down.
///
/// Kept apart from [`safe_area_insets`] on purpose, and the split is the whole
/// point rather than tidiness. The safe area is hardware: the gesture bar and
/// the cutout are where they are for the life of the process, so a caller
/// reads it once at mount and is entitled to assume it will not move. The
/// keyboard moves several times a minute. Folding the two together would
/// either make every safe-area reader poll at the keyboard's rate, or make the
/// number it cached at mount silently wrong the first time someone typed.
///
/// This is a poll and not a subscription: it reports the inset at the instant
/// it is asked, so a caller that wants to move with the keyboard asks once per
/// frame while it cares. The push version — `setDecorFitsSystemWindows(false)`
/// and an `OnApplyWindowInsetsListener` on the Java side — is deliberately not
/// wired here, because turning decor fitting off changes where the window lays
/// out underneath the `ANativeWindow` the shell draws into, and that is a
/// change to every rinch app's geometry rather than an addition to it.
///
/// `None` when the window has no insets to report yet, which happens before
/// the first layout pass and is not the same answer as "the keyboard is down".
pub fn ime_inset() -> Option<u32> {
    bridge::with_activity(|env, activity| {
        let px = env
            .call_method(activity, "getImeInset", "()I", &[])
            .ok()?
            .i()
            .ok()?;
        // The Java side returns -1 for "no insets yet" rather than throwing,
        // because a window that has not been laid out is an ordinary moment in
        // an activity's life and not a fault worth an exception.
        (px >= 0).then_some(px as u32)
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
