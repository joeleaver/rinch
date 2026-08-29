//! The app-level half of one Android frame: turn the frame clock, then say
//! what the frame still needs before the surface is presented.
//!
//! `android_runtime` compiles only for `target_os = "android"`, but what a
//! frame *is* costs nothing to run on a host, and it is the part that has to be
//! pinned down by tests — so it lives here and is compiled for the host test
//! build too, the way `touch_gesture` and `android_ime` are.

use std::time::Duration;

use rinch_platform::{AppAction, PlatformEvent};

use crate::app::RinchApp;

/// What one frame asked of the shell.
pub(crate) struct Frame {
    /// What `RinchApp` wants done — a redraw request, an exit, and so on.
    /// Read by the loop in `android_runtime`, which the host test build does
    /// not compile.
    #[cfg_attr(not(all(feature = "android", target_os = "android")), allow(dead_code))]
    pub actions: Vec<AppAction>,
    /// Layout the shell still has to resolve before it paints.
    pub pending_layout: bool,
    /// Pixels the shell still has to present.
    ///
    /// A transition on a paint-only property — `opacity`, `transform`, a
    /// colour — marks its node `PAINT`-dirty and nothing else, so
    /// `has_pending_layout` stays false and the loop's other two reasons to
    /// present (a `RequestRedraw` action, scroll momentum) never fire either.
    /// Turning the clock without this is turning it in private: the sheet
    /// slides in the tree and the surface still shows the frame from before
    /// the tap, until something unrelated forces a present — the next tap,
    /// which by then is landing on a sheet that is logically already open.
    pub needs_paint: bool,
}

/// Turn the frame clock and report what the frame needs.
///
/// [`PlatformEvent::AboutToWait`] *is* the clock. It is the one place
/// `RinchApp` advances CSS transitions and CSS animations, marks the scene
/// dirty when either moved, resolves the dirty state that the input handlers
/// deliberately leave for it to batch, and drains the pending focus requests
/// effects raise.
///
/// A shell that never sends it has no clock at all, and the failure is quiet
/// rather than obviously frozen: a transitioned property is sampled once, at
/// the instant the transition starts — which is its *old* value — and stays
/// there for ever, while every un-transitioned property on the same element
/// applies immediately. A bottom sheet written the ordinary way (a full-screen
/// root whose `pointer-events` flips, a scrim whose opacity fades, a panel that
/// slides up from below the fold) then answers a tap by going pointer-active
/// and not moving: the sheet is logically open, invisible, and covering the
/// screen, so the next tap anywhere lands on its scrim and closes it again.
/// Nothing about the app looks broken — every screen still paints, every
/// un-animated control still works — which is why this was found by tapping a
/// chip on a phone rather than by anything in CI.
///
/// The Android loop calls this once per iteration, and paces those iterations
/// against the display's own frame interval (see [`poll_timeout`]), so
/// transitions and animations get a clock that runs at whatever the panel
/// runs at — the same arrangement the winit shell has, where
/// `ActiveEventLoop::about_to_wait` fires per redraw and `ControlFlow::Wait`
/// holds the loop still in between. Card K37 is where that stopped being a
/// hard-coded 16ms: on the moto g stylus 5G, a 120Hz panel, 16ms was a 2x cap
/// on how often this could be called at all.
pub(crate) fn pump_frame(app: &mut RinchApp, window_size: (u32, u32), scale_factor: f64) -> Frame {
    let actions = app.handle_event(PlatformEvent::AboutToWait, window_size, scale_factor);
    Frame {
        pending_layout: app.has_pending_layout(),
        needs_paint: app.scene_dirty,
        actions,
    }
}

// ── Pacing ───────────────────────────────────────────────────────────────────

/// How long the loop should block in `poll_events` before its next iteration.
///
/// `None` means "until something happens": no timeout at all, sleep on the
/// looper until Android delivers input, a lifecycle event, or one of this
/// crate's own producers rings the waker.
///
/// **The 16ms this replaces, and what it cost.** The Android loop used to call
/// `poll_events(Some(Duration::from_millis(16)))` unconditionally, and
/// `poll_events` blocks for its whole timeout when nothing arrives. So 16ms
/// was two different decisions wearing one number: it was the frame clock's
/// tick *and* it was the polling interval for every queue a Java thread
/// writes. Both jobs were done badly.
///
/// It was too *long* while something was animating. Card K35 left the GPU path
/// painting a frame in 4.1 / 18.8 / 7.4 ms on three screens of SetListArray,
/// while frame-to-frame on the same traces was 20.4 / 36.0 / 24.0 — four to
/// seventeen milliseconds of every frame was this timeout, not work. The
/// software path paid it identically, because it is not a property of either
/// renderer. Two rounds of paint optimisation (K24, K35) were therefore
/// invisible: the app ran at ~42-50fps whichever painter was compiled in,
/// because the loop had decided in advance how often it was allowed to try.
///
/// And it was too *short* while nothing was. A still screen woke sixty-two
/// times a second to discover that it was still still — a full iteration each
/// time, with a frame-clock tick, six queue drains and a `has_pending_images`
/// scan that takes a process-global mutex — and then went back to sleep. That
/// is battery spent on asking.
///
/// **So the loop asks what state it is in, which it already knows.** The
/// predicate is `presented`: did this iteration actually put pixels on the
/// glass? That is exactly the loop's existing `needs_paint` — a running
/// transition or animation (via [`Frame::needs_paint`] and K23's `had_running`
/// guard), pending layout, a decoding image, scroll momentum, a requested
/// redraw — narrowed by whether the present really happened. Narrowed,
/// because the alternative is a spin: a predicate that says "keep going" while
/// nothing blocks is a busy-wait, and the surface is the only thing in the
/// iteration that blocks. If the paint was skipped or the present failed, the
/// loop must not come straight back to try it again as fast as the CPU allows.
///
/// `wake_pending` is the second half, and it closes a race rather than
/// describing a state. `REDRAW_PENDING` is swapped to false early in the
/// iteration, but a cross-thread callback that lands *after* that swap, or an
/// effect that sets a signal during the paint, sets it again — and would then
/// wait for the next unrelated event. Reading it once more, immediately before
/// the poll, means the only way to lose a wake is for it to arrive during
/// `ALooper_pollAll` itself, which is the case the waker's counted eventfd
/// already handles.
///
/// **When it is animating, the timeout is what is left of the frame.**
/// Not zero. Zero would be correct in the sense that the present blocks —
/// `Fifo` on the GPU path, `ANativeWindow_lock` waiting for a free buffer on
/// the software one — but it would put the loop's speed entirely in the hands
/// of how many buffers the compositor happens to have spare, and "render
/// frames nobody sees" is the thing K35 refused Mailbox for. Subtracting the
/// time already spent from one display frame gives back the slack when the
/// frame was cheap and gives back nothing when it was not, so a paint that
/// overruns is never delayed by its own pacing. Input still cuts the sleep
/// short: this is a poll timeout, not a sleep.
///
/// **What it measured**, on the moto g stylus 5G at 1080x2460, release build,
/// with the probe K24 established and K35 reused — a `log::info!` per
/// presented frame through `adb logcat`, medians over the frames of a
/// sixteen-second scripted trace on three screens of SetListArray: the library
/// list flicked and left to coast, the sort and group sheet sliding open and
/// shut, and a PDF page on song detail flicked the same way. `paint` is
/// resolve, build and present together; `f2f` is the interval between one
/// presented frame and the next. Both builds ran the identical `adb shell
/// input` script, minutes apart, on the same handset.
///
/// ```text
/// software, the shipping path      paint    f2f     fps   frames
/// 16ms poll     library              7.5   25.2    39.6      434
///               sheet               87.8  115.9     8.6       64
///               pdf                  7.3   24.1    41.6      613
/// this          library              7.5    8.4   119.0     1083
///               sheet               85.2   95.5    10.5       64
///               pdf                  7.9    8.3   119.9     1293
///
/// android-gpu                      paint    f2f     fps   frames
/// 16ms poll     library              3.5   21.0    47.6      794
///               sheet               21.5   39.8    25.1      104
///               pdf                  7.3   24.0    41.7      688
/// this          library             15.0   16.3    61.3     1000
///               sheet               37.4   40.9    24.5      104
///               pdf                 22.2   23.0    43.4      715
/// ```
///
/// The `frames` column is the point restated: on the software path the same
/// sixteen seconds of the same gestures carried two and a half times as many
/// frames to the glass, and nothing in the painter changed to do it.
///
/// **120fps is not a typo and this panel is why.** The moto g stylus 5G offers
/// 60, 90 and 120Hz and runs this app at 120 — `dumpsys display` says
/// `mActiveSfDisplayMode ... refreshRate=120.00001` while the app is
/// foreground, and the loop's own `InitWindow` line agrees: `frame=8.33ms`,
/// asked of the display rather than assumed. (The same command answers
/// `refreshRate=60.000004` with the app in the background, which is the mode
/// change `RATE_RECHECK` in the runtime exists to catch.) A 16ms poll on a
/// 120Hz panel is not a rough approximation of a frame, it is two of them, and
/// the software path was therefore capped at half the display's rate on both
/// screens where the paint was cheap enough to matter. Both land on 8.3-8.4ms,
/// which is the panel's frame, against paints of 7.5 and 7.9: the loop is now
/// doing the work the whole frame is made of and almost nothing else.
///
/// The sheet moved by exactly one poll's worth — 115.9ms to 95.5 — and should
/// not have moved by more: 85ms of software rasterisation is not a frame this
/// or any pacing decision can shorten, and the 20ms that did come off is the
/// timeout that used to be waited out on top of it. It is in the table to show
/// the difference between the two kinds of gap. Where paint is the cost, K37
/// removes one poll and no more; where waiting was the cost, it removes the
/// wait.
///
/// **The GPU column needs reading with K35's warning in hand**, which is that a
/// probe around a `Fifo` present measures the display's back-pressure and not
/// work. `paint` rises there — 3.5 to 15.0 on the library — while `f2f` falls,
/// which is what "the wait moved from the poll into the present" looks like
/// from inside the paint block. Broken out with a second probe on the same
/// traces, the acquire is **under 0.1ms** on all three screens (0.08 / 0.07 /
/// 0.06) and everything after it — blit, submit, `frame.present()` — is 12.5 /
/// 31.1 / 4.9. So on the library and the sheet the loop is not waiting to be
/// given an image, it is waiting for the queue to take one back, and its 16.3ms
/// frame-to-frame is two of the panel's 8.33ms vsyncs rather than the loop's
/// own deadline.
///
/// The PDF screen is the exception that keeps the reading honest: 4.9ms after
/// the acquire against a 22.2ms paint means seventeen of those milliseconds are
/// `build_scene` and vello's `render_to_texture`, which is work and not
/// back-pressure. Nothing about pacing can move it, and nothing about pacing
/// did — 24.0ms to 23.0ms. Frame-to-frame is what the user sees, and it
/// improves or holds on all three.
///
/// **And on a still screen it does nothing at all**, which was the other half
/// of the card and the half that could have been a worse bug than the one being
/// fixed. A loop that spins when nothing is happening does not show up in any
/// frame-time trace; it shows up in the battery. Measured on the same handset,
/// app foreground on the library list, ten seconds after launch to let the
/// first paint settle, then sixty seconds of nobody touching anything:
///
/// ```text
///                    loop iterations   android_main   android_main
///                        in 60.2s      utime+stime    voluntary ctxt
/// software
///   16ms poll                  3534    +137 jiffies   642 ->  4161
///   this                          0      +0 jiffies    13 ->    13
/// android-gpu
///   16ms poll                  3539    +133 jiffies   638 ->  4163
///   this                          0      +0 jiffies    89 ->    89
/// ```
///
/// The first column is a counter in the loop itself logged per iteration, so it
/// is the loop counting its own wake-ups; 3534 in 60.2 seconds is 58.7 a
/// second, which is the 16ms timeout expiring and finding nothing, over and
/// over. The other two columns come from
/// `/proc/<pid>/task/<android_main>/{stat,status}` and are the same fact told
/// by the kernel instead of by the app: 137 jiffies is 1.37 seconds of CPU
/// burned in a minute of doing nothing, and 3519 voluntary context switches is
/// the thread going to sleep 3519 times.
///
/// After the change all three are zero. Not "small" — zero: **the thread that
/// runs this loop did not wake once in a minute**, which is the strongest form
/// the claim can take, and it is the same on both renderers because the poll
/// was never a property of either.
///
/// The counterpart check, that the loop still wakes when it should, is
/// SetListArray's own attachment viewer: its chrome auto-hides through a
/// `set_timeout(4000, ..)` that fires on the timer crate's shared thread and
/// hops back through `run_on_main_thread`. Opened full-screen and then left
/// alone — no touches, nothing else on the screen moving — the bar still
/// disappears four seconds later. That is the whole waker chain (background
/// thread, `dispatch_to_main_thread`, `wake_main`, `ALooper_wake`, an iteration
/// of a loop with no timeout) demonstrated by a feature nobody wrote for the
/// purpose.
///
/// **Why this is not `AChoreographer`**, which is the obvious answer and was
/// the card's first suggestion. It is Android's real vsync signal and it would
/// be the right clock; at this dependency pin it is not reachable without
/// building the plumbing for it, and the plumbing fights the event loop:
///
/// - `android-activity` 0.6 exposes no choreographer API of any kind. What it
///   does expose is `AndroidApp::create_waker()`, which is what
///   `rinch_android::wake` is built on and what makes the untimed sleep above
///   safe.
/// - `ndk` 0.9 — the pin — has no `choreographer` module. The safe wrapper
///   exists upstream but landed after this release.
/// - `ndk-sys` 0.6 *does* declare the raw entry points
///   (`AChoreographer_getInstance`, `postFrameCallback64`, `postVsyncCallback`
///   and the rest), but `ndk` 0.9 does not re-export it, so reaching them means
///   taking a second, version-locked direct dependency and writing the `unsafe`
///   ourselves.
/// - And then the shape is wrong. `AChoreographer_getInstance()` binds to the
///   *calling thread's* looper, which for `android_main` is the same looper
///   `poll_events` polls, so a frame callback is dispatched from inside
///   `ALooper_pollAll` (which is what the native-activity backend this crate
///   builds against actually calls) as a looper callback — and its
///   `poll_events` answers `ALOOPER_POLL_CALLBACK` with
///   `error!("Spurious ALOOPER_POLL_CALLBACK from ALopper_pollAll()
///   (ignored)")`, typo and all. Every vsync would print that. The honest way
///   in is a choreographer on a looper of its own that rings the waker, which
///   is a thread and a synchronisation problem in exchange for a signal the
///   display is already giving us through the present.
///
/// **`polls_due` is the one thing the loop cannot work out for itself.**
/// `rinch_core::reactive::poll_signal` bridges an external source — an audio
/// thread's atomic, a network flag — into a signal by being *sampled once per
/// frame*, and `drain_polls` does that sampling from this loop. A loop that
/// sleeps until an event never iterates, so it never drains, so a
/// `PollRate::Hz(60)` bridge silently stops the moment the screen goes still.
/// Nothing wakes it, because there is no producer to do the waking: the value
/// is sitting in an atomic that nobody has been asked to look at. This is the
/// only queue in the process that works that way, and it is why the argument
/// exists rather than another waker. See `reactive::next_poll_due` for why
/// `PollRate::EveryFrame` is excluded from the answer, and note that the
/// answer is `None` — no constraint at all — for every app that has not called
/// `poll_signal`, which is all of them today.
///
/// Which is the substantive point: the surface *is* a vsync signal here. Under
/// `Fifo` the swapchain blocks, and on the software path `ANativeWindow_lock`
/// blocks for a free buffer — K27 measured that and called it "the display's
/// back-pressure rather than work". The deadline below is the belt to that
/// braces, and the reason for having both is that neither is sufficient alone:
/// the back-pressure only appears once the queue is full, which is one or two
/// frames of running flat out, and the deadline alone would not notice a panel
/// that changed mode after `InitWindow` read its rate.
pub(crate) fn poll_timeout(
    presented: bool,
    wake_pending: bool,
    spent: Duration,
    frame_interval: Duration,
    polls_due: Option<Duration>,
) -> Option<Duration> {
    let paced = if presented || wake_pending {
        Some(frame_interval.saturating_sub(spent))
    } else {
        None
    };
    match (paced, polls_due) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    /// 60Hz, spelled the way the runtime spells it.
    const FRAME: Duration = Duration::from_nanos(16_666_666);

    /// The card this function was written for: a frame that reached the glass
    /// means something is moving, and the next one is due at the next display
    /// deadline — not 16ms after this one finished.
    #[test]
    fn a_presented_frame_waits_only_for_the_rest_of_the_frame() {
        let left = poll_timeout(true, false, Duration::from_millis(4), FRAME, None)
            .expect("an animating loop does not sleep until an event");
        assert!(
            left < Duration::from_millis(13) && left > Duration::from_millis(12),
            "a 4ms frame on a 16.7ms display has ~12.7ms left, got {left:?}"
        );
    }

    /// The overrun case, which is the common one on the software path: the
    /// paint already took longer than a frame, so there is nothing left to
    /// wait for and the loop must not invent any.
    #[test]
    fn a_frame_that_overran_its_deadline_does_not_sleep_at_all() {
        assert_eq!(
            poll_timeout(true, false, Duration::from_millis(40), FRAME, None),
            Some(Duration::ZERO),
            "a frame that is already late must not be made later by its own \
             pacing"
        );
    }

    /// The battery half, and the one that would be a worse bug than the one
    /// K37 fixed: nothing painted, nothing pending, so the loop sleeps on the
    /// looper rather than spinning through iterations that do nothing.
    #[test]
    fn a_still_screen_waits_for_an_event_rather_than_a_deadline() {
        assert_eq!(
            poll_timeout(false, false, Duration::from_micros(80), FRAME, None),
            None,
            "an idle loop must have no timeout at all — a short one is a \
             wake-up counter, and every wake-up on a still screen is battery \
             spent asking a question whose answer is no"
        );
    }

    /// A redraw asked for after the loop had already read `REDRAW_PENDING` is
    /// not a reason to sleep until the user touches something.
    #[test]
    fn a_wake_that_landed_after_the_swap_still_gets_its_frame() {
        assert_eq!(
            poll_timeout(false, true, FRAME, FRAME, None),
            Some(Duration::ZERO),
            "the work is already queued; the next iteration is due now"
        );
    }

    /// The hole the untimed sleep opened, and the only one that could not be
    /// closed with a waker.
    ///
    /// A `poll_signal` bridge is sampled by `drain_polls`, which runs once per
    /// iteration of the loop — so an idle loop stops sampling it, and there is
    /// nobody to ring the waker because the new value is sitting in an atomic
    /// on some other thread that has never heard of this one. The loop
    /// therefore has to keep its own alarm clock for it.
    #[test]
    fn a_still_screen_still_wakes_for_a_timed_poll() {
        assert_eq!(
            poll_timeout(
                false,
                false,
                Duration::ZERO,
                FRAME,
                Some(Duration::from_millis(20)),
            ),
            Some(Duration::from_millis(20)),
            "an app that asked for a 50Hz bridge over an audio thread has to get \
             one whether or not the screen is moving"
        );
    }

    /// ...and the poll must never *lengthen* a sleep. A one-second poll on a
    /// screen that is mid-animation is not a reason to wait a second.
    #[test]
    fn a_slow_poll_does_not_delay_an_animating_frame() {
        assert_eq!(
            poll_timeout(
                true,
                false,
                Duration::ZERO,
                FRAME,
                Some(Duration::from_secs(1)),
            ),
            Some(FRAME),
            "the sooner of the two deadlines wins, always"
        );
    }
}
