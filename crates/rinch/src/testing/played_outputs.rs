//! The outputs a test plays (#2836): published through the shell's own path
//! (`crate::outputs::publish`), so every reader of [`outputs`], [`output_ids`],
//! [`output`] and [`output_tracking`] in the app and in the windows it opens
//! sees them as it sees a compositor's, and the shell's own decisions that
//! read them (a child refused for an output that went, a content-sized
//! window's largest size and starting scale) take them.
//!
//! The list is the thread's, as the shell's is: two `TestApp`s on one
//! thread read one list, and the last [`TestApp::set_outputs`] wins for
//! both; an app mounted while it is played reads it from its first render.
//! When the last `TestApp` on the thread goes, after its documents are
//! disposed (so none of its readers runs for it), the list is emptied and
//! [`output_tracking`] is `NotListed` again ([`crate::outputs::unpublish`]),
//! so a later `TestApp` there starts with no outputs.
//!
//! [`outputs`]: crate::outputs::outputs
//! [`output_ids`]: crate::outputs::output_ids
//! [`output`]: crate::outputs::output
//! [`output_tracking`]: crate::outputs::output_tracking

use super::TestApp;
use crate::outputs::{Output, OutputTracking};

impl TestApp {
    /// Play `outputs` as the connected outputs, in this order, replacing
    /// whatever was played before: an output added, removed, renamed, moved
    /// or rescaled is a further call with the new list. It is what the shell
    /// does at each `monitors_changed` on Wayland: the list published (with
    /// [`OutputTracking::Live`]) in one batch, each content-sized window
    /// told so it can fit its output again, then a turn of the loop and the
    /// window requests it made served, so a window opened for a new output
    /// is open on return.
    ///
    /// Build each [`Output`] as a literal, with [`OutputId::from_raw`] for its
    /// id. What a played output changes, and what it does not (the
    /// compositor's part), is in the guide's
    /// [Outputs](super#outputs).
    ///
    /// [`OutputId::from_raw`]: crate::outputs::OutputId::from_raw
    pub fn set_outputs(&mut self, outputs: impl IntoIterator<Item = Output>) {
        crate::outputs::publish(outputs.into_iter().collect(), OutputTracking::Live);
        self.follow_outputs();
        for child in &mut self.children {
            child.follow_outputs();
        }
        self.turn(false);
        self.serve_window_requests();
    }
}

/// The ids of the outputs listed on this thread, which a child placed by an
/// output id must be on to be made (`RinchRuntime::refuse_stale_output`);
/// `None` while none are listed ([`OutputTracking::NotListed`]: nothing played
/// on this thread, or not since its last `TestApp` went), when no child is
/// refused.
pub(super) fn connected() -> Option<Vec<u64>> {
    rinch_core::reactive::untracked(|| {
        (crate::outputs::output_tracking() != OutputTracking::NotListed).then(|| {
            crate::outputs::outputs()
                .iter()
                .map(|output| output.id.raw())
                .collect()
        })
    })
}
