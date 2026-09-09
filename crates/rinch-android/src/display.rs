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

/// The size of the window this activity draws into, in physical pixels, as
/// `(width, height)`.
///
/// **What this is for, and what it is not.** It is not a repair to rinch's own
/// layout: the shell has always laid out against the real window. It reads
/// `native_window.width()` / `height()` at both `MainEvent::InitWindow` and
/// `MainEvent::WindowResized` (`shell::android_runtime`), mounts at
/// `rinch_platform::to_logical` of that, and `App::size()` is documented inert
/// on Android for exactly this reason. Anything the DOM lays out — `100vw`, a
/// flex row, a full-bleed background — is already the right width today.
///
/// What was missing is that **app code has no way to ask**. A page the app
/// *rasterises itself* — a PDF page to a bitmap, a tile it generates at a pixel
/// width and hands back as an image — is not laid out by the DOM and so gets
/// none of that for free; it needs a number, and until now the only number
/// available was one the app chose. SetListArray's card K31 is where that bit:
/// it renders its pages at the 393-pixel canvas its designs were drawn on, and
/// on a moto g stylus 5G — 1080 physical at density 400, so 432 logical — every
/// page came out 393 wide with a strip of backdrop down each side. Nothing in
/// rinch was wrong; the app had no way to find out it was drawing at the wrong
/// size, and it was wrong by a different amount on every handset.
///
/// **This is a second answer to a question the shell already answers**, and the
/// two can disagree. The `ANativeWindow` is what rinch draws into and is
/// therefore authoritative; this asks Android independently. On API 30+ the two
/// agree — `getCurrentWindowMetrics().getBounds()` is that window's bounds. On
/// API 28-29 the fallback is `getResources().getDisplayMetrics()`, which is the
/// app-usable *display* size, and that equals the window only in a single-window
/// session. **Multi-window and split-screen are assumed away below API 30**: a
/// side-by-side split disagrees in width, not merely in height. Prefer the
/// layout you were given wherever you have one; reach for this when you are
/// generating pixels the layout never sees.
///
/// Physical pixels, matching [`safe_area_insets`], because that is the unit
/// Android measures in. To reach the logical pixels a stylesheet is written in,
/// divide by `density_dpi() / 160` — deliberately *not* wrapped in a helper
/// here, because `rinch_platform::to_logical` is the conversion the shell
/// itself uses and a second one in this crate would be the same rule written
/// twice.
///
/// `None` if the JNI call chain fails or the platform reports a non-positive
/// size, which leaves the caller to pick its own fallback rather than have one
/// invented here — the same contract [`density_dpi`] keeps. The decode is
/// [`crate::display_decode::decode_viewport_size`], which is host-tested; see
/// that module for why it is not written inline.
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
        crate::display_decode::decode_viewport_size(buf)
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
/// # Driving it: there are no frames unless you ask for them
///
/// This is a poll, not a subscription, and since #564 the Android loop **sleeps
/// with no timeout when nothing is moving** — measured at zero iterations in
/// sixty seconds on a still screen. The soft keyboard appearing raises no rinch
/// event on this shell, so "read it once per frame" has no frames to read on: an
/// effect that calls this at mount is called exactly once, ever.
///
/// The in-tree mechanism that does work is
/// [`rinch_core::reactive::poll_signal`], which
/// `shell::android_frame::poll_timeout` takes `next_poll_due()` for precisely so
/// that a polled bridge keeps the loop awake:
///
/// ```ignore
/// use rinch_core::reactive::{PollRate, poll_signal};
///
/// // Wakes the loop at 60Hz while this signal is alive, and only then.
/// let ime = poll_signal(
///     || rinch_android::display::ime_inset().unwrap_or(0),
///     PollRate::Hz(60),
/// );
/// ```
///
/// No convenience wrapper is offered for that: `poll_signal` *is* the wiring,
/// it is one call, and a wrapper could only hard-code a rate the caller is
/// better placed to choose.
///
/// # What it does not do
///
/// **It reports the settled inset, not the animation.** `getRootWindowInsets()`
/// answers where the keyboard has got to, so a per-frame poll during the
/// show/hide animation gives a step, not a ramp. Animate the layout yourself
/// (a CSS transition on the value this feeds) if you want a ramp.
///
/// **API 28-29 is unverified and probably reports nothing.** Below API 30 there
/// is no `Type.ime()`, so the Java side subtracts the stable bottom inset from
/// the system-window bottom inset — the nav bar plus the keyboard, less the nav
/// bar. That difference is non-zero only while the window is *being resized for
/// the IME*, and this shell's window declares no `windowSoftInputMode`. So on
/// 28-29 the likely answer with the keyboard up is `Some(0)`, which this API
/// defines as "the keyboard is down" — the instruction to put a footer back.
/// The device this was built against was a moto g stylus 5G on **SDK 33**, so
/// only the API 30+ branch has ever run: the sentence above is reasoning about
/// the platform, not a measurement, and nobody here has an API 28-29 handset to
/// settle it. Treat the inset as API 30+ until someone does.
///
/// # Why polled at all
///
/// The push route on the Java side — `setDecorFitsSystemWindows(false)` plus an
/// `OnApplyWindowInsetsListener` — is deliberately not wired, because turning
/// decor fitting off changes where the window lays out underneath the
/// `ANativeWindow` the shell draws into, and that is a change to every rinch
/// app's geometry rather than an addition to it.
///
/// That is not the only push route, though, and the choice should not be read as
/// poll-or-nothing. `MainEvent::ContentRectChanged` is delivered by the
/// `native-activity` backend this crate builds against, and android-activity's
/// own documentation names "the soft input window being shown or hidden" as a
/// cause of it; `shell::android_runtime` handles neither it nor anything like
/// it today. Whether it actually fires for the IME under a manifest with no
/// `windowSoftInputMode` is device-dependent and untested here. (`InsetsChanged`
/// is *not* an alternative — android-activity emits it only from the
/// `game_activity` backend.) Wiring that event and reading this getter from it
/// would be strictly better than polling, and is a follow-up rather than a
/// reason to hold this.
///
/// `None` when the window has no insets to report yet, which happens before the
/// first layout pass and is **not** the same answer as "the keyboard is down".
/// The decode is [`crate::display_decode::decode_ime_inset`], which is
/// host-tested; see that module for why it is not written inline.
pub fn ime_inset() -> Option<u32> {
    bridge::with_activity(|env, activity| {
        let px = env
            .call_method(activity, "getImeInset", "()I", &[])
            .ok()?
            .i()
            .ok()?;
        crate::display_decode::decode_ime_inset(px)
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
