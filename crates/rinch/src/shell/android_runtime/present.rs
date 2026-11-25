//! Putting a turn's frame on the glass, and asking again for one that did not
//! get there.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[cfg(feature = "android-gpu")]
use super::VELLO_CHECK_INTERVAL;
use super::loop_state::LoopState;
use super::{RATE_RECHECK, REDRAW_PENDING};
use crate::shell::android_frame::{self, Frame};

impl LoopState {
    /// Paint the frame if one is owed, then make sure one that is still owed
    /// is asked for again.
    pub(super) fn paint(&mut self, frame: &Frame, logical_size: (u32, u32)) {
        // Check if app has pending layout (signal changes create pending updates)
        let has_momentum = self.gesture.has_momentum();
        let redraw = REDRAW_PENDING.swap(false, Ordering::AcqRel);
        // `pending_layout` also covers a finished image decode, which has
        // dirtied no node (`RinchApp::has_pending_images`) — without that this
        // loop spins past the decode forever and the `<img>` paints as a blank
        // card. Found on a moto g stylus showing a rasterised PDF page out of
        // app-private storage: the page was on disk, the box was the right
        // size, and the pixels only arrived when something else happened to
        // make the screen redraw.
        let pending = frame.pending_layout;
        let needs_paint = redraw || pending || has_momentum || frame.needs_paint;
        // Whether this iteration is going to try to put pixels on the glass.
        // Named because the present's *outcome* is only meaningful beside it —
        // see the `retry_owed_frame` call below.
        let attempted = needs_paint && self.mounted;

        if attempted {
            self.present(pending, logical_size);
        }

        #[cfg(feature = "android-gpu")]
        self.check_vello_frames();

        self.retry_unpresented_frame(attempted);
    }

    /// Draw the frame and hand it to the surface.
    fn present(&mut self, pending: bool, logical_size: (u32, u32)) {
        // Only on frames that are actually being drawn, and at most once a
        // second — see `RATE_RECHECK` for the mode change that made this
        // necessary.
        if self.frame_interval_read.elapsed() >= RATE_RECHECK {
            self.frame_interval_read = Instant::now();
            if let Some(hz) = rinch_android::display::refresh_rate_hz() {
                self.frame_interval = Duration::from_secs_f32(1.0 / hz);
            }
        }

        if pending {
            self.app
                .resolve_and_repaint(logical_size.0 as f32, logical_size.1 as f32);
        }

        #[cfg(feature = "android-gpu")]
        if let (Some(ctx), Some(s)) = (&mut self.gpu, &mut self.surface) {
            let scene = self.app.build_scene(self.scale_factor, logical_size);
            let present_start = std::time::Instant::now();
            self.presented = s.present_scene(&mut ctx.renderer, scene);
            self.app.perf_add(
                rinch_dom::perf::Counter::TimePresentNs,
                present_start.elapsed().as_nanos() as u64,
            );
        }

        #[cfg(not(feature = "android-gpu"))]
        if let Some(ref mut s) = self.surface {
            let (pixels, w, h) = self
                .app
                .build_pixels(self.scale_factor, logical_size, false);
            let present_start = std::time::Instant::now();
            self.presented = s.present_pixels(pixels, w, h);
            self.app.perf_add(
                rinch_dom::perf::Counter::TimePresentNs,
                present_start.elapsed().as_nanos() as u64,
            );
        }

        // One owed frame is one performance frame (`rinch_dom::perf`),
        // as one redraw is on the desktop; this is also what prints the
        // `RINCH_PERF` line on Android.
        self.app.end_perf_frame();
    }

    /// A frame that overflowed Vello's buffers went out undrawn, and nothing
    /// else would draw it again on a screen at rest (#2071). This path
    /// renders the whole scene every time, so a redraw is all it takes.
    #[cfg(feature = "android-gpu")]
    fn check_vello_frames(&mut self) {
        if let (Some(ctx), Some(s)) = (&mut self.gpu, &self.surface) {
            self.vello_check_in = None;
            match ctx.renderer.check_frames(&s.device) {
                vello::FrameCheck::Settled => {}
                vello::FrameCheck::Pending => {
                    self.vello_check_in = Some(VELLO_CHECK_INTERVAL);
                }
                vello::FrameCheck::Lost => REDRAW_PENDING.store(true, Ordering::Release),
            }
        }
    }

    /// The frame was owed and did not reach the glass, so ask for it again
    /// (issue #563). Nothing else will: `redraw` was swapped out of
    /// `REDRAW_PENDING` in [`paint`](Self::paint), the layout `pending` named
    /// was resolved by the `resolve_and_repaint` in [`present`](Self::present),
    /// `build_scene` / `build_pixels` clear `scene_dirty` *before* the present,
    /// and a running CSS transition marks its node dirty without ever queueing
    /// a redraw *action* — so the loop's next `poll_timeout` would see
    /// `presented == false` with no wake pending, answer `None`, and sleep
    /// until something unrelated happened to arrive. The re-arm costs one
    /// display frame of sleep and clears itself on the first success; see
    /// `android_frame::retry_owed_frame` for the sequence.
    ///
    /// **The clock reset is half the repair, not a tidy-up.** `poll_timeout`
    /// answers `frame_interval - spent`, and the `spent` it is handed is
    /// `frame_start.elapsed()` — this whole iteration, paint included. A
    /// paint that overran the display's frame (85.2ms against an 8.33ms
    /// panel in K37's own software `sheet` row) saturates that to
    /// `Duration::ZERO`, so without this line the retry is immediate and the
    /// loop spins at whatever rate the CPU allows, failing every time: the
    /// exact busy-wait the `bool` returns of `present_pixels` /
    /// `present_scene` exist to prevent, reintroduced by the cure. Restart
    /// the clock here and the retry is measured from the arm instead, so it
    /// gets a whole frame. A frame that *reached the glass* still gets
    /// `ZERO` when it overran, and should — that is catching up, not failing
    /// faster.
    fn retry_unpresented_frame(&mut self, attempted: bool) {
        if android_frame::retry_owed_frame(attempted, self.presented) {
            REDRAW_PENDING.store(true, Ordering::Release);
            self.frame_start = Instant::now();
        }
    }
}
