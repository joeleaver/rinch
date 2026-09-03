//! **Test-only** determinism seam: sessions whose yrs client id is pinned by the
//! caller (issue #214). Compiled only under the crate's `test-util` feature, which
//! nothing but this crate's own `[dev-dependencies]` turns on — a downstream release
//! build cannot reach these functions at all.
//!
//! # Why this exists
//!
//! The fuzz suites (`tests/fuzz.rs`) drive a swarm of peers from a fixed-seed PRNG, so
//! the *edit script* is deterministic. The resulting document was not. yrs breaks a tie
//! between two concurrent inserts at the same position by **client id**, and
//! `Options::default()` calls `ClientID::random()` — so the converged document, and with
//! it every later random position computed from that document, differed run to run.
//! Two runs of the same binary at the same seed reached different documents and even
//! different edit counts, which meant a failing trial could not be reproduced from its
//! `(seed, peers, rounds)` triple. Pinning the ids makes a trial replayable bit-for-bit.
//!
//! # Why it must not be reachable from production
//!
//! The client id is a replica's **identity** in the CRDT. Two live peers sharing one
//! produce blocks with colliding ids, which yrs treats as the same block: the replicas
//! silently stop converging and the shared document is corrupt with no error anywhere.
//! That is precisely the divergence class this crate exists to kill, so the seam is
//! gated rather than merely `#[doc(hidden)]`:
//!
//! * the `test-util` feature is **off by default** and is enabled only by this crate's
//!   self dev-dependency, so it is on for `cargo test -p rinch-editor-collab` and off
//!   everywhere else;
//! * the facade (`rinch`'s `collaboration` feature) never names it, so no downstream
//!   feature unification can switch it on;
//! * every public path stays random — [`CollabSession::new`] and
//!   [`CollabSession::from_bytes`] are untouched.
//!
//! # Use
//!
//! Give each peer of a trial a **distinct** id derived from the trial's own seed, never
//! a process-global counter: tests run in parallel threads, so a shared counter would
//! reintroduce exactly the scheduling-dependent non-determinism this seam removes.
//!
//! ```ignore
//! let host = testing::session_with_client_id(&state, client_id(seed, 0))?;
//! let guest = testing::session_from_bytes_with_client_id(&host.snapshot(), client_id(seed, 1))?;
//! ```

use rinch_editor_core::EditorState;

use crate::error::Result;
use crate::session::CollabSession;

/// [`CollabSession::new`] with this replica's yrs client id pinned to `client_id`.
///
/// Every peer of one collaboration must be given a **different** id — see the module
/// docs for what sharing one does to the shared document.
pub fn session_with_client_id(state: &EditorState, client_id: u64) -> Result<CollabSession> {
    CollabSession::new_with_client_id(state, checked(client_id))
}

/// [`CollabSession::from_bytes`] with this replica's yrs client id pinned to
/// `client_id` — the joining half of [`session_with_client_id`].
///
/// Every peer of one collaboration must be given a **different** id, the host included.
pub fn session_from_bytes_with_client_id(bytes: &[u8], client_id: u64) -> Result<CollabSession> {
    CollabSession::from_bytes_with_client_id(bytes, checked(client_id))
}

/// yrs client ids are **53-bit** (`ClientID::new` debug-asserts it, and a release build
/// would silently fold the high bits into the mask instead), so two ids that differ only
/// above bit 52 would collide — the corruption this module exists to warn about. Reject
/// that here rather than let it through in a release-profile test run.
fn checked(client_id: u64) -> u64 {
    assert!(
        client_id < (1u64 << 53),
        "collab client ids are 53-bit; {client_id} does not fit and would collide with \
         another id that differs only in its high bits"
    );
    client_id
}
