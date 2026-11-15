//! How many frames a GPU renderer lets the GPU work on at once (issue #2688).
//!
//! wgpu keeps every hal command encoder it has made in a free list it never
//! trims, and a Vulkan driver keeps each one's command pool at the largest
//! command stream it has held (wgpu resets the pools without
//! `RELEASE_RESOURCES`): on RADV about 1.3 MiB of GTT per encoder for a short
//! stream and 2 to 4 MiB for a Vello frame's. So the memory a renderer keeps
//! is set by the most encoders that were ever alive at once, which is the
//! frames in flight times the encoders each one uses. Nothing in wgpu bounds
//! the frames in flight; a FIFO swapchain does only while it is presented to
//! (a held or occluded window still renders), and a `Mailbox`/`Immediate`
//! one does not at all.
//!
//! Each renderer (the desktop window's, the embed API's overlay) keeps a
//! `FramesInFlight`: before a frame's GPU work starts it waits for the
//! frame [`gpu_frames_in_flight()`] frames back to finish, and polls the device
//! so the encoders of every finished frame are back in wgpu's free list before
//! the frame takes its own.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The frames a GPU renderer lets be in flight at once unless the app or
/// `RINCH_GPU_FRAMES_IN_FLIGHT` says otherwise: the frame being recorded and
/// the one before it, so the CPU encodes a frame while the GPU draws the last.
pub const DEFAULT_GPU_FRAMES_IN_FLIGHT: usize = 2;

/// How long a frame waits for an earlier one before it goes ahead anyway: a
/// GPU that hangs (or a lost device) must not hang the app's thread with it.
const WAIT_LIMIT: Duration = Duration::from_secs(1);

/// The app's choice, `usize::MAX` for none.
static REQUESTED: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Let each GPU renderer have at most `frames` frames in flight on the GPU
/// (issue #2688); `0` lets them have any number, as before. The default is
/// [`DEFAULT_GPU_FRAMES_IN_FLIGHT`]. The `RINCH_GPU_FRAMES_IN_FLIGHT`
/// environment variable overrides it. It takes effect from the next frame.
///
/// A frame waits, before its GPU work starts, until the frame `frames`
/// frames back has finished. `1` waits for the previous frame every frame:
/// the least memory, and the CPU no longer encodes a frame while the GPU is
/// still drawing the last. More, or `0`, lets frames overlap, and the driver
/// keeps the memory of every command encoder those frames had alive at once
/// for the life of the process.
///
/// It is one setting for the process: every window's renderer and every
/// embed overlay renderer reads it. An overlay's wait is on a submission made
/// after everything the host had submitted by then, so in an embedding host
/// the limit holds the host's own frames in flight to it as well (with the
/// default, a `render` waits until the host's frame before last has
/// finished); `0` lifts it for every renderer in the process at once.
pub fn set_gpu_frames_in_flight(frames: usize) {
    REQUESTED.store(frames, Ordering::Relaxed);
}

/// The frames a GPU renderer lets be in flight: `RINCH_GPU_FRAMES_IN_FLIGHT`
/// if it is a number, else the app's [`set_gpu_frames_in_flight`], else
/// [`DEFAULT_GPU_FRAMES_IN_FLIGHT`]. `0` is no limit.
pub fn gpu_frames_in_flight() -> usize {
    static ENV: OnceLock<Option<usize>> = OnceLock::new();
    let env =
        *ENV.get_or_init(|| parse_env(std::env::var("RINCH_GPU_FRAMES_IN_FLIGHT").ok().as_deref()));
    resolve(env, REQUESTED.load(Ordering::Relaxed))
}

/// `RINCH_GPU_FRAMES_IN_FLIGHT`: a number, or nothing (unparsable is logged).
fn parse_env(v: Option<&str>) -> Option<usize> {
    let v = v?.trim();
    if v.is_empty() {
        return None;
    }
    let n = v.parse().ok();
    if n.is_none() {
        tracing::warn!("RINCH_GPU_FRAMES_IN_FLIGHT={v:?} is not a number: ignored");
    }
    n
}

fn resolve(env: Option<usize>, requested: usize) -> usize {
    env.unwrap_or(if requested == usize::MAX {
        DEFAULT_GPU_FRAMES_IN_FLIGHT
    } else {
        requested
    })
}

/// The submissions that ended a renderer's recent frames, oldest first.
#[derive(Debug, Default)]
pub(crate) struct FramesInFlight {
    ends: VecDeque<wgpu::SubmissionIndex>,
    /// Tests only: this renderer's limit, in place of [`gpu_frames_in_flight()`]
    /// (a process-wide setting tests running in parallel would share).
    #[cfg(test)]
    pub(crate) limit: Option<usize>,
}

impl FramesInFlight {
    /// The limit this renderer's frames have: [`gpu_frames_in_flight()`].
    pub(crate) fn limit(&self) -> usize {
        #[cfg(test)]
        if let Some(limit) = self.limit {
            return limit;
        }
        gpu_frames_in_flight()
    }

    /// Before a frame's first GPU work: wait until at most `limit - 1` earlier
    /// frames are unfinished (`0`, no limit, does not wait), then poll, so
    /// every finished frame's command encoders are back in wgpu's free list
    /// for this frame to take. Returns whether it waited.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, limit: usize) -> bool {
        let mut waited = false;
        if limit == 0 {
            self.ends.clear();
        } else {
            while self.ends.len() >= limit {
                let end = self.ends.pop_front().expect("len >= limit >= 1");
                // A timeout or an error goes ahead: see `WAIT_LIMIT`.
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: Some(end),
                    timeout: Some(WAIT_LIMIT),
                });
                waited = true;
            }
        }
        if !waited {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        waited
    }

    /// The frame's last submission went in: the GPU has finished the frame
    /// when it has finished this.
    pub(crate) fn end(&mut self, last: wgpu::SubmissionIndex) {
        self.ends.push_back(last);
    }

    /// The frames whose end has been noted and not yet waited for.
    // Read by the GPU tests and the embed overlay's test accessor (#2688);
    // a build with neither has no reader.
    #[allow(dead_code)]
    pub(crate) fn pending(&self) -> usize {
        self.ends.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_overrides_the_app_which_overrides_the_default() {
        assert_eq!(resolve(None, usize::MAX), DEFAULT_GPU_FRAMES_IN_FLIGHT);
        assert_eq!(resolve(None, 1), 1);
        assert_eq!(resolve(None, 0), 0);
        assert_eq!(resolve(Some(3), 1), 3);
        assert_eq!(resolve(Some(0), 1), 0);
    }

    #[test]
    fn the_environment_variable_is_a_number_or_nothing() {
        assert_eq!(parse_env(None), None);
        assert_eq!(parse_env(Some("")), None);
        assert_eq!(parse_env(Some(" 3 ")), Some(3));
        assert_eq!(parse_env(Some("0")), Some(0));
        assert_eq!(parse_env(Some("two")), None);
        assert_eq!(parse_env(Some("-1")), None);
    }
}
