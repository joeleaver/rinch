//! Real-time collaboration wiring for the desktop editor (the `collaboration`
//! feature, M9).
//!
//! The pure model↔CRDT adapter lives in
//! [`rinch-editor-collab`](rinch_editor_collab); this module is the thin seam that
//! drives it from an [`EditorHandle`](super::EditorHandle):
//!
//! * **outbound** — every *local* edit funnels through `EditorCore::commit`
//!   (`handle.rs`), which projects the `before → after` change onto the CRDT
//!   ([`CollabSession::record_local`](rinch_editor_collab::CollabSession::record_local))
//!   and hands the resulting delta to this bridge's `outbound` sink.
//! * **inbound** — a peer's delta arrives via
//!   [`EditorHandle::collab_receive`](super::EditorHandle::collab_receive), which
//!   merges it into the CRDT, rebuilds the model from the *converged* CRDT, and
//!   re-projects the host. A remote integration is applied as a non-undoable,
//!   remote-origin transaction and is **not** re-broadcast (it is already shared).
//!
//! The transport itself is left to the caller (design §2: "transport is the app's
//! concern"). The seam is a pair of closures/calls: `outbound` carries bytes out,
//! [`post_remote_delta`](super::post_remote_delta) carries bytes back in from any
//! thread. The in-process loopback in `examples/collab-editor-demo` wires two
//! handles to each other directly; a network transport forwards the same bytes over
//! a socket / data channel.
//!
//! This driver uses **immediate projection + converged rebuild**, so it deliberately
//! does *not* add [`CollabPlugin`](rinch_editor_collab::CollabPlugin) to the editor's
//! plugins — the version/unconfirmed-steps bookkeeping that plugin folds is only
//! meaningful for a future throttled / rebase-based driver.

use rinch_editor_collab::{CollabError, CollabSession};

/// The collaboration state attached to one editor: its CRDT session plus the sink
/// that carries outbound deltas to peers.
pub(super) struct CollabBridge {
    /// This editor's model↔CRDT projection and sync lifecycle.
    pub(super) session: CollabSession,
    /// Where to send a delta produced by a *local* edit. Invoked on the main
    /// thread, immediately after the edit is projected onto the CRDT. For a real
    /// transport it forwards bytes to the network thread (e.g. over an
    /// `mpsc::Sender`); for the in-process loopback demo it calls the peer handle's
    /// [`collab_receive`](super::EditorHandle::collab_receive) directly.
    ///
    /// It must **not** re-enter the same handle (the handle is borrowed while the
    /// edit is committed) — a real transport just enqueues bytes, and the loopback
    /// targets the *other* handle.
    pub(super) outbound: Box<dyn Fn(Vec<u8>)>,
    /// The last projection error. Per design A22 the staged scope is flat
    /// text-blocks + marks; an edit outside it fails loud rather than silently
    /// diverging. Surfaced (and cleared) via
    /// [`EditorHandle::collab_take_error`](super::EditorHandle::collab_take_error).
    pub(super) last_error: Option<CollabError>,
}

impl CollabBridge {
    /// Attach `session` with the given outbound delta sink.
    pub(super) fn new(session: CollabSession, outbound: Box<dyn Fn(Vec<u8>)>) -> CollabBridge {
        CollabBridge {
            session,
            outbound,
            last_error: None,
        }
    }
}
