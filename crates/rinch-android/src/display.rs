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
