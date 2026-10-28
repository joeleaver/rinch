//! Test hooks. Not API.
//!
//! Every entry point that opens a device (a camera, a microphone, an audio
//! output) passes through [`gate`] first. A test can count those opens, and
//! deny them so that nothing reaches the hardware: an app's test that its UI
//! opens no device until asked (ui-zoo's av section, #1916) runs with
//! [`deny_devices`] on, so even the broken code it guards against touches no
//! camera. Listing devices is not an open and is not counted.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::AvError;

static OPENS: AtomicUsize = AtomicUsize::new(0);
static DENY: AtomicBool = AtomicBool::new(false);

/// While `true`, every device open fails with [`AvError::Backend`] before it
/// touches a device. It is still counted.
pub fn deny_devices(deny: bool) {
    DENY.store(deny, Ordering::SeqCst);
}

/// How many device opens were attempted in this process (denied ones too).
pub fn device_opens() -> usize {
    OPENS.load(Ordering::SeqCst)
}

/// Count an open of `what`, and refuse it while devices are denied.
#[allow(dead_code)] // a build with neither `audio` nor `camera` opens nothing
pub(crate) fn gate(what: &str) -> Result<(), AvError> {
    OPENS.fetch_add(1, Ordering::SeqCst);
    if DENY.load(Ordering::SeqCst) {
        return Err(AvError::Backend(format!(
            "{what}: devices are denied (rinch_av::testing::deny_devices)"
        )));
    }
    Ok(())
}
