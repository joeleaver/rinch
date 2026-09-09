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

/// Whether the system is in night mode right now: `Some(true)` for "dark",
/// `Some(false)` for "light", `None` when the platform declines to say.
///
/// This is `Configuration.uiMode & UI_MODE_NIGHT_MASK`, which is the same
/// answer `AppCompatDelegate` and `isSystemInDarkTheme()` are built on, read
/// through the activity's own `Resources` rather than the application's. That
/// distinction is not pedantry: an activity's configuration is the one Android
/// has already overridden per-activity — `setLocalNightMode`, a
/// `ContextThemeWrapper`, a display with its own configuration — so the
/// activity's resources answer the question the app is actually asking, which
/// is "what should the thing I am about to paint look like", not "what is the
/// device set to somewhere".
///
/// **`None` is a real third answer, not a failure code.**
/// `UI_MODE_NIGHT_UNDEFINED` is what the mask holds when nothing has decided —
/// it is what a `Configuration` carries before it is applied, and some OEM
/// skins and TV/automotive UI modes leave it that way for the life of the
/// process. An app that treats `None` as "light" has hard-coded a preference
/// where the platform explicitly said it had none; the honest thing is to hand
/// the undecided case back and let the app fall through to its own default.
/// The JNI call chain failing produces the same `None` for the same reason,
/// with a `log::warn!` so the two can be told apart in logcat.
///
/// **A reading with a shelf life**, like [`refresh_rate_hz`] and unlike
/// [`safe_area_insets`]. The user can flip the system theme while the app is
/// in the foreground, and this answers what was true when it was asked. An app
/// that wants to follow the system rather than sample it once should re-read
/// this from `rinch_core::events::set_configuration_change_handler`, which is
/// where the shell reports that a configuration changed under it. There is a
/// trap on the other side of that: an activity whose manifest
/// `android:configChanges` does not list `uiMode` is **destroyed and
/// recreated** on a night-mode flip rather than reconfigured, so the handler
/// never runs, the process restarts, and this function is only ever read at a
/// fresh mount. That is a one-word edit in the app's manifest and it is
/// invisible until someone flips the switch with the app open.
pub fn night_mode() -> Option<bool> {
    // android.content.res.Configuration's constants. Named here rather than
    // fetched as static fields over JNI because they are `public static final
    // int` in the platform API — the compiler inlines them into every app that
    // has ever been built against Android, so they cannot change without
    // breaking the world, and three extra JNI round-trips to look up numbers
    // that are frozen by ABI is a cost with nothing to buy.
    const UI_MODE_NIGHT_MASK: i32 = 0x30;
    const UI_MODE_NIGHT_NO: i32 = 0x10;
    const UI_MODE_NIGHT_YES: i32 = 0x20;

    let ui_mode = bridge::with_activity(|env, activity| -> jni::errors::Result<i32> {
        // getResources().getConfiguration().uiMode
        let resources = env
            .call_method(
                activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;
        let config = env
            .call_method(
                &resources,
                "getConfiguration",
                "()Landroid/content/res/Configuration;",
                &[],
            )?
            .l()?;
        env.get_field(&config, "uiMode", "I")?.i()
    });

    let ui_mode = match ui_mode {
        Ok(bits) => bits,
        Err(e) => {
            log::warn!("night_mode: reading Configuration.uiMode failed: {e}");
            return None;
        }
    };

    match ui_mode & UI_MODE_NIGHT_MASK {
        UI_MODE_NIGHT_YES => Some(true),
        UI_MODE_NIGHT_NO => Some(false),
        // UI_MODE_NIGHT_UNDEFINED, or a value from a future mask this build
        // has never heard of. Both mean "the platform did not answer".
        _ => None,
    }
}

/// The dominant colour of the user's wallpaper, as sRGB `(r, g, b)` — the seed
/// Material You builds a device's accent palette from.
///
/// `WallpaperManager.getWallpaperColors(FLAG_SYSTEM).getPrimaryColor()`, and
/// deliberately nothing more. The primary colour is the wallpaper's, which
/// means it can be anything a photograph can be: near-black, near-white, or a
/// saturated yellow that vanishes on any pale surface. Every app that uses this
/// has to do something about that, and what it has to do depends entirely on
/// what it is painting the colour *onto* — a contrast ratio is a fact about a
/// pair of colours, and this function only knows one of them. So the darkening,
/// the desaturating, the 4.5:1 clamp and the decision to give up and use the
/// app's own accent all belong to the caller, and returning the raw triple is
/// what makes those possible rather than second-guessed. A framework that
/// helpfully returned a "safe" colour would be a framework that had silently
/// picked a background.
///
/// **`None` is ordinary.** `getWallpaperColors` returns null more often than
/// its signature suggests: no wallpaper has been set, or the wallpaper is a
/// live wallpaper whose service does not publish colours (most of them do not),
/// or the device is showing a lock-screen-only image, or the OEM has replaced
/// the wallpaper stack with its own — Samsung and Xiaomi both have form here.
/// None of those is an error and none of them will ever fix itself, so a caller
/// should treat `None` as "this device has no accent to offer" and fall back
/// once, not retry.
///
/// The API is 27+ and every rinch Android build has a higher minimum than that,
/// so there is no version guard here; what there is instead is the same
/// `None`-on-anything-unexpected contract the rest of this module keeps.
pub fn wallpaper_primary() -> Option<(u8, u8, u8)> {
    // WallpaperManager.FLAG_SYSTEM — the home-screen wallpaper, as opposed to
    // FLAG_LOCK (2). Frozen by ABI, so a constant rather than a static-field
    // lookup, for the reason `night_mode` above spells out.
    const FLAG_SYSTEM: i32 = 1;

    let argb = bridge::with_activity(|env, activity| -> jni::errors::Result<Option<i32>> {
        // `find_class` on a framework class is safe from this thread, which is
        // the reason this module can reach WallpaperManager at all without a
        // Java-side helper the way `getSafeAreaInsets` needs one. A native
        // thread's `FindClass` resolves against the *system* class loader, so
        // it cannot see `com.rinch.*` (see `bridge::init`, which goes the long
        // way round through the activity's own loader for exactly that reason)
        // — but `android.app.WallpaperManager` is in the boot classpath, which
        // is the one thing the system loader can always find.
        let class = env.find_class("android/app/WallpaperManager")?;
        let manager = env
            .call_static_method(
                &class,
                "getInstance",
                "(Landroid/content/Context;)Landroid/app/WallpaperManager;",
                &[jni::objects::JValue::Object(activity)],
            )?
            .l()?;
        let colors = env
            .call_method(
                &manager,
                "getWallpaperColors",
                "(I)Landroid/app/WallpaperColors;",
                &[jni::objects::JValue::Int(FLAG_SYSTEM)],
            )?
            .l()?;
        // The null the doc comment above is about. It arrives as an ordinary
        // object reference that happens to be null rather than as an
        // exception, so it has to be checked before it is called through —
        // `getPrimaryColor` on a null receiver is a NullPointerException
        // thrown into Java and a `JavaException` back here, which would be a
        // warn in logcat for something that is not a fault.
        if colors.is_null() {
            return Ok(None);
        }
        let color = env
            .call_method(&colors, "getPrimaryColor", "()Landroid/graphics/Color;", &[])?
            .l()?;
        if color.is_null() {
            return Ok(None);
        }
        // `Color.toArgb()` rather than reading a field: `Color` has been a real
        // object with a colour space since API 26, and its packed `int` form is
        // what `toArgb` exists to produce. Anything wide-gamut is converted to
        // sRGB on the way out, which is what a caller comparing this against
        // CSS colours wants.
        env.call_method(&color, "toArgb", "()I", &[])?.i().map(Some)
    });

    match argb {
        Ok(Some(argb)) => Some((
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
        )),
        Ok(None) => None,
        Err(e) => {
            // Unlike everywhere else in this module, the exception is cleared
            // rather than merely reported. `jni` 0.21 turns a pending Java
            // exception into `Err(JavaException)` and leaves it pending, and a
            // pending exception makes the *next* JNI call on this thread abort
            // the process — so a SecurityException from an OEM wallpaper
            // service, thrown here, would be paid for by whichever unrelated
            // call `with_activity` made next. The rest of the module gets away
            // without this because its calls (`getResources`, `getRefreshRate`)
            // are documented not to throw; a wallpaper service is third-party
            // code and this one genuinely can.
            let _ = env_clear_exception();
            log::warn!("wallpaper_primary: WallpaperColors lookup failed: {e}");
            None
        }
    }
}

/// One tone off the palette **the system itself is themed with**, as sRGB
/// `(r, g, b)` — `android.R.color.system_accent1_<tone>`, read through the
/// activity's own `Resources`.
///
/// **Why this exists next to [`wallpaper_primary`], which already returns a
/// seed.** Android 12 (API 31) publishes the finished Material You palette as
/// ordinary framework colour resources: three accent ramps and two neutral
/// ones, thirteen tones each, regenerated by the system every time the user
/// changes their theme. That is not merely a more convenient form of the same
/// answer — it is a *different* answer, and on one common configuration the
/// wallpaper's is the wrong one. `settings get secure
/// theme_customization_overlay_packages` carries a
/// `"android.theme.customization.color_source"` key, and it reads
/// `"home_wallpaper"` only while the user is letting the wallpaper drive the
/// theme. Pick one of the basic colours in Wallpaper & style instead and it
/// reads `"preset"` — at which point the wallpaper still has a primary colour,
/// `getWallpaperColors` still returns it confidently, and it is precisely the
/// colour the user went into the settings app to *override*. A wrong answer
/// delivered with no error is worse than a missing one, so an app that wants
/// "the colour this device is themed in" should ask here first and fall back to
/// the wallpaper only for the API levels that have no palette to publish.
///
/// **The tones are a fixed, published set** — `0, 10, 50, 100, 200, 300 … 900,
/// 1000` — and this rejects anything else before it touches JNI. Dumping
/// `/system/framework/framework-res.apk` on a moto g stylus 5G (SDK 33) with
/// `aapt2` lists exactly those thirteen for each of `system_accent1`,
/// `system_accent2`, `system_accent3` and the two neutral ramps, and nothing
/// in between: there is no `system_accent1_550`. Checking here rather than
/// letting `getIdentifier` say `0` is not an optimisation, it is the
/// difference between two failures that deserve different treatment — a tone
/// nobody ever published is a caller's mistake, and a tone that is published
/// but absent is a device below API 31.
///
/// **`None` is ordinary, and covers both of those.** `getIdentifier` returns
/// `0` for a name the resource table does not have, which is what *every*
/// device below API 31 answers for every tone, and it is a documented return
/// value rather than a fault — so it produces a plain `None` with nothing in
/// logcat. A caller should treat `None` the way [`wallpaper_primary`]'s `None`
/// is treated: this device has no palette to offer, fall back once, do not
/// retry.
///
/// **A reading with a shelf life**, like [`night_mode`] and for the same
/// reason: the system regenerates these resources when the user changes their
/// wallpaper or picks a different preset, and the activity is told about it as
/// a configuration change. Re-read from
/// `rinch_core::events::set_configuration_change_handler` rather than caching
/// the answer at mount.
pub fn system_accent(tone: u16) -> Option<(u8, u8, u8)> {
    // The published tones, in the order the resource table lists them. Named
    // here rather than fetched, for the same reason `night_mode`'s uiMode
    // constants are: this is a frozen part of the platform's public resource
    // surface, and a tone that is not on this list is not a resource that has
    // ever existed on any device.
    const TONES: [u16; 13] = [0, 10, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];
    if !TONES.contains(&tone) {
        return None;
    }

    let argb = bridge::with_activity(|env, activity| -> jni::errors::Result<Option<i32>> {
        let resources = env
            .call_method(
                activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;

        // `getIdentifier(name, "color", "android")`, which is the only way to
        // reach a resource whose `R` constant this crate cannot be compiled
        // against — `android.R.color.system_accent1_500` is API 31 and rinch
        // builds for lower, so the id has to be looked up by name at runtime
        // on whatever platform the app actually landed on.
        let name = env.new_string(format!("system_accent1_{tone}"))?;
        let kind = env.new_string("color")?;
        let package = env.new_string("android")?;
        let id = env
            .call_method(
                &resources,
                "getIdentifier",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
                &[
                    jni::objects::JValue::Object(&name),
                    jni::objects::JValue::Object(&kind),
                    jni::objects::JValue::Object(&package),
                ],
            )?
            .i()?;
        // Zero is "no such resource", and on everything below API 31 that is
        // the answer for all thirteen tones. An ordinary `None`, not a fault:
        // see the doc comment above.
        if id == 0 {
            return Ok(None);
        }

        // `getColor(int, Resources.Theme)` with a null theme, rather than the
        // one-argument `getColor(int)` it replaced in API 23. The deprecated
        // form resolves against no theme at all and is deprecated for exactly
        // that reason; the two-argument form resolves theme attributes against
        // the theme it is handed, and null means "resolve nothing" — which is
        // the honest request here, because the system palette entries are
        // literal colours and there is no theme whose attributes we would want
        // consulted. Passing the activity's theme would invite an OEM overlay
        // to answer a question about the *system's* palette.
        let null_theme = jni::objects::JObject::null();
        env.call_method(
            &resources,
            "getColor",
            "(ILandroid/content/res/Resources$Theme;)I",
            &[
                jni::objects::JValue::Int(id),
                jni::objects::JValue::Object(&null_theme),
            ],
        )?
        .i()
        .map(Some)
    });

    match argb {
        Ok(Some(argb)) => Some((
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
        )),
        Ok(None) => None,
        Err(e) => {
            // **Yes, the discipline `wallpaper_primary` documents applies
            // here too**, and it is worth saying why rather than copying the
            // line. Its argument was that a wallpaper service is third-party
            // code that genuinely throws, unlike `getResources` and
            // `getRefreshRate`, which are documented not to. `Resources` is
            // not third-party — but `getColor` is documented to throw
            // `NotFoundException`, which is one more than `getResources`
            // throws, and `getIdentifier` allocates three Java strings on the
            // way in, which is one more chance at an `OutOfMemoryError` than
            // a no-argument call has. Neither is likely: the id was non-zero a
            // line earlier, so the resource is there. But "unlikely" is the
            // wrong bar, because the cost of being wrong is not paid here. A
            // pending exception makes the *next* JNI call on this thread abort
            // the process, and the next JNI call belongs to somebody else —
            // the frame loop, an input event — who will be blamed for it. One
            // `exception_clear` on a path that should never run is cheaper
            // than a crash report pointing at innocent code.
            let _ = env_clear_exception();
            log::warn!("system_accent: reading system_accent1_{tone} failed: {e}");
            None
        }
    }
}

/// Clear any Java exception left pending on this thread. See the error arm of
/// [`wallpaper_primary`] for why that matters more than it looks.
fn env_clear_exception() -> jni::errors::Result<()> {
    bridge::with_jni_env(|env| {
        if env.exception_check()? {
            env.exception_describe()?;
            env.exception_clear()?;
        }
        Ok(())
    })
}
