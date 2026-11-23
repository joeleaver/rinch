//! #2843: a software window that has drawn nothing for its idle period
//! drops its softbuffer surface, and with it both presentation buffers.
//!
//! softbuffer's pool cannot be observed without a compositor, so these drive
//! the shell's own bookkeeping ([`Presenter`], [`IdleTrimmed`]) with a
//! stand-in for softbuffer's Wayland surface: two full-size buffers, made at
//! the first present and each resident from the first frame written into it
//! (what `RssShmem` counts), presented by writing the back one and swapping.
//! The live figures under a nested sway are in the PR.

use super::{IdleTrimmed, PresentTarget, Presenter, TimerOps};
use rinch_core::TimeoutHandle;

thread_local! {
    /// The timers armed through [`RECORDED`] and not cleared yet.
    static LIVE_TIMERS: RefCell<Vec<TimeoutHandle>> = const { RefCell::new(Vec::new()) };
}

/// `rinch_core`'s timers, recording which are armed: a timer that is not
/// cleared still wakes the event loop at its deadline.
const RECORDED: TimerOps = TimerOps {
    set: |ms| {
        let handle = (TimerOps::CORE.set)(ms);
        LIVE_TIMERS.with(|t| t.borrow_mut().push(handle));
        handle
    },
    clear: |handle| {
        LIVE_TIMERS.with(|t| t.borrow_mut().retain(|h| *h != handle));
        (TimerOps::CORE.clear)(handle);
    },
};

fn live_timers() -> usize {
    LIVE_TIMERS.with(|t| t.borrow().len())
}
use rinch_core::element::{IdleTrim, WindowProps};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

const W: u32 = 6;
const H: u32 = 4;

/// What the stand-in surfaces hold, across every one made.
#[derive(Default)]
struct Shm {
    /// Surfaces made.
    made: usize,
    /// Buffers written at least once, of the surfaces alive.
    resident: usize,
}

/// softbuffer's Wayland surface, as far as memory goes.
struct FakeSurface {
    shm: Rc<RefCell<Shm>>,
    buffers: [Option<Vec<u8>>; 2],
    /// The buffer the next frame is written into.
    back: usize,
}

impl FakeSurface {
    /// The buffer the compositor was last given.
    fn front(&self) -> &[u8] {
        self.buffers[1 - self.back].as_deref().unwrap()
    }
}

impl PresentTarget for FakeSurface {
    fn present(&mut self, pixels: &[u8], width: u32, height: u32) {
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        let back = &mut self.buffers[self.back];
        if back.is_none() {
            self.shm.borrow_mut().resident += 1;
        }
        *back = Some(pixels.to_vec());
        self.back = 1 - self.back;
    }
}

impl Drop for FakeSurface {
    fn drop(&mut self) {
        let written = self.buffers.iter().flatten().count();
        self.shm.borrow_mut().resident -= written;
    }
}

/// A presentation of a surface that keeps its contents (Wayland's) with
/// `trim`, and what its surfaces hold.
fn presentation(retains: bool, trim: IdleTrim) -> (IdleTrimmed<FakeSurface>, Rc<RefCell<Shm>>) {
    let shm = Rc::new(RefCell::new(Shm::default()));
    let make = {
        let shm = shm.clone();
        move |w: u32, h: u32| {
            assert_eq!((w, h), (W, H));
            shm.borrow_mut().made += 1;
            FakeSurface {
                shm: shm.clone(),
                buffers: [None, None],
                back: 0,
            }
        }
    };
    let presenter = Presenter::new(retains, trim.period(), make);
    LIVE_TIMERS.with(|t| t.borrow_mut().clear());
    (IdleTrimmed::with_timers(presenter, RECORDED), shm)
}

/// A frame of `W` x `H` filled with `fill`, with one pixel of `mark`.
fn frame(fill: u8, mark: (usize, u8)) -> Vec<u8> {
    let mut px = vec![fill; (W * H * 4) as usize];
    px[mark.0 * 4..mark.0 * 4 + 4].fill(mark.1);
    px
}

const MS: Duration = Duration::from_millis(1);
const PERIOD: Duration = Duration::from_secs(2);

fn resident(shm: &Rc<RefCell<Shm>>) -> usize {
    shm.borrow().resident
}

#[test]
fn two_frames_then_idle_past_the_period_release_both_buffers() {
    let (mut p, shm) = presentation(true, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    assert!(p.present_frame(&frame(10, (0, 1)), W, H, 1, t0));
    assert_eq!(resident(&shm), 1, "one frame writes one buffer");
    // The timer is armed for the first frame's deadline...
    assert_eq!(p.timer_deadline(), Some(t0 + PERIOD));
    let t1 = t0 + 16 * MS;
    assert!(p.present_frame(&frame(10, (1, 2)), W, H, 2, t1));
    assert_eq!(resident(&shm), 2, "the second frame writes the spare");
    // ...and a frame while it is armed does not re-arm it.
    assert_eq!(p.timer_deadline(), Some(t0 + PERIOD));

    // A wake within the period drops nothing.
    assert!(!p.service_idle_trim(t0 + 1000 * MS));
    assert_eq!(resident(&shm), 2);
    // The timer's wake: the second frame moved the deadline on by 16 ms, so
    // it drops nothing and arms for the new one.
    assert!(!p.service_idle_trim(t0 + PERIOD));
    assert_eq!(resident(&shm), 2);
    assert_eq!(p.timer_deadline(), Some(t1 + PERIOD));
    // That one's wake drops the surface and both buffers, and arms nothing:
    // an idle window is woken no more.
    assert!(p.service_idle_trim(t1 + PERIOD));
    assert_eq!(resident(&shm), 0, "both buffers released");
    assert!(p.presenter().target().is_none());
    assert_eq!(p.timer_deadline(), None);
    assert!(!p.service_idle_trim(t1 + 10 * PERIOD));
    assert_eq!(live_timers(), 0, "no timer left to wake the loop");
}

#[test]
fn a_redraw_that_presents_nothing_still_serves_the_trim() {
    // The timer's wake can also ask for a frame: the runtime then leaves the
    // trim to that frame, and a frame that changed no pixel presents
    // nothing. It has to serve the trim itself, or the timer is spent and
    // nothing ever trims.
    let (mut p, shm) = presentation(true, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    let b = frame(10, (1, 2));
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    let t1 = t0 + 16 * MS;
    p.present_frame(&b, W, H, 2, t1);
    assert_eq!(live_timers(), 1);
    // The timer's wake at t0 + 2 s, served by a redraw of the same frame:
    // not idle yet, so it arms for the later deadline.
    assert!(!p.present_frame(&b, W, H, 2, t0 + PERIOD));
    assert_eq!(resident(&shm), 2);
    assert_eq!(p.timer_deadline(), Some(t1 + PERIOD));
    assert_eq!(live_timers(), 1);
    // That timer's wake, again with a redraw that presents nothing: idle.
    assert!(!p.present_frame(&b, W, H, 2, t1 + PERIOD));
    assert_eq!(resident(&shm), 0, "both buffers released");
    assert_eq!(live_timers(), 0);
}

#[test]
fn dropping_a_surface_with_a_trim_pending_clears_its_timer() {
    // A hidden or closed window drops its renderer: its timer must not wake
    // the event loop at the deadline for nothing.
    let (mut p, shm) = presentation(true, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    p.present_frame(&frame(10, (1, 2)), W, H, 2, t0 + 16 * MS);
    assert_eq!(live_timers(), 1);
    drop(p);
    assert_eq!(live_timers(), 0, "the timer is cleared");
    assert_eq!(resident(&shm), 0);
}

#[test]
fn the_frame_after_a_trim_is_a_whole_frame_on_a_new_surface() {
    let (mut p, shm) = presentation(true, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    let b = frame(10, (1, 2));
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    p.present_frame(&b, W, H, 2, t0 + 16 * MS);
    assert!(p.service_idle_trim(t0 + 16 * MS + PERIOD));
    assert_eq!(resident(&shm), 0);

    // A redraw of the frame the compositor still shows (a configure's)
    // presents nothing, and so allocates nothing.
    let t2 = t0 + 5 * PERIOD;
    assert!(!p.present_frame(&b, W, H, 2, t2));
    assert_eq!(shm.borrow().made, 1);
    assert!(p.presenter().target().is_none());
    assert_eq!(p.timer_deadline(), None);

    // The next painted frame differs from the last one in one pixel: the
    // painter repainted only that. The new surface is given all of it.
    let c = frame(10, (5, 3));
    assert!(p.present_frame(&c, W, H, 3, t2));
    assert_eq!(shm.borrow().made, 2, "a new surface");
    assert_eq!(resident(&shm), 1, "one buffer again");
    let target = p.presenter().target().unwrap();
    assert_eq!(target.front(), &c[..], "the whole frame, pixel for pixel");
    // The frame after it writes the spare again, and arms the trim again.
    let d = frame(20, (2, 4));
    assert!(p.present_frame(&d, W, H, 4, t2 + 16 * MS));
    assert_eq!(resident(&shm), 2);
    assert_eq!(p.presenter().target().unwrap().front(), &d[..]);
    assert_eq!(p.timer_deadline(), Some(t2 + PERIOD));
}

#[test]
fn never_keeps_both_buffers() {
    let (mut p, shm) = presentation(true, IdleTrim::Never);
    let t0 = Instant::now();
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    p.present_frame(&frame(10, (1, 2)), W, H, 2, t0 + 16 * MS);
    assert_eq!(p.timer_deadline(), None, "nothing to wake for");
    assert!(!p.service_idle_trim(t0 + 3600 * PERIOD));
    assert_eq!(resident(&shm), 2);
    assert!(p.presenter().target().is_some());
}

#[test]
fn a_frame_before_the_period_cancels_the_trim() {
    let (mut p, shm) = presentation(true, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    p.present_frame(&frame(10, (1, 2)), W, H, 2, t0 + 16 * MS);
    // A frame 1.5 s in: the timer armed for t0 + 2 s finds it.
    let t1 = t0 + 1500 * MS;
    p.present_frame(&frame(10, (2, 3)), W, H, 3, t1);
    assert!(!p.service_idle_trim(t0 + PERIOD));
    assert_eq!(resident(&shm), 2, "kept: the window drew 0.5 s ago");
    assert_eq!(p.timer_deadline(), Some(t1 + PERIOD));
    assert!(!p.service_idle_trim(t1 + PERIOD - MS));
    assert!(p.service_idle_trim(t1 + PERIOD));
    assert_eq!(resident(&shm), 0);
}

#[test]
fn a_surface_that_does_not_keep_its_contents_is_not_trimmed() {
    // An X11 window's: its next expose presents anyway, and elsewhere
    // (macOS) dropping the surface takes the picture off the window.
    let (mut p, shm) = presentation(false, IdleTrim::After(PERIOD));
    let t0 = Instant::now();
    p.present_frame(&frame(10, (0, 1)), W, H, 1, t0);
    p.present_frame(&frame(10, (1, 2)), W, H, 2, t0 + 16 * MS);
    assert_eq!(p.timer_deadline(), None);
    assert!(!p.service_idle_trim(t0 + 100 * PERIOD));
    assert_eq!(resident(&shm), 2);
    // And says so: its `RINCH_PERF` line is `idle_trim=never` (#2917).
    assert_eq!(p.idle_period(), None);
    let (p, _) = presentation(true, IdleTrim::After(PERIOD));
    assert_eq!(p.idle_period(), Some(PERIOD));
}

#[test]
fn a_window_trims_after_two_seconds_by_default() {
    assert_eq!(WindowProps::default().idle_trim, IdleTrim::After(PERIOD));
    assert_eq!(IdleTrim::default().period(), Some(PERIOD));
    assert_eq!(IdleTrim::Never.period(), None);
}
