//! # rinch-editor-collab
//!
//! Optional, **opt-in** collaborative-editing adapter for the Rinch editor, and the
//! **only** first-party home of a CRDT engine — [`yrs`] (Yjs) — in the workspace
//! (design §2/§8, issue #190). Gated behind the facade's `collaboration` feature so
//! default builds — desktop *and* web — link zero CRDT code. The crate itself is pure
//! model↔CRDT logic with no platform deps and is **wasm-compatible with no shims**, so
//! a Rust web editor view can reuse this *same* adapter rather than bridging to a
//! separate JS CRDT.
//!
//! The CRDT is *not* the document model — `rinch-editor-core` is. This crate projects
//! that pure model onto a yrs CRDT so concurrent edits converge, and rebuilds remote
//! CRDT changes back into editor [`Step`](rinch_editor_core::Step)s. The whole design
//! rests on one invariant:
//!
//! > **`model ≡ project(model)`** — every local step is projected onto the CRDT
//! > ([`CollabDoc::project_change`]); every remote CRDT change is rebuilt into the model
//! > ([`CollabSession::integrate_incremental`]). Convergence then follows from yrs's own
//! > convergence.
//!
//! ## Staged scope (design A22)
//!
//! The first milestone covers **flat text-blocks + marks** (`paragraph`/`heading`/
//! `code_block` with text + bold/italic/link/… marks). Anything outside that — a nested
//! block, an inline atom — is [`CollabError::Unsupported`]: the adapter **fails loud**
//! rather than silently dropping a change, because a silent drop is exactly the
//! divergence class the editor rewrite set out to kill.
//!
//! ## Quick start
//!
//! ```
//! use std::rc::Rc;
//! use rinch_editor_core::{EditorState, Schema, Fragment, default_plugins};
//! use rinch_editor_collab::CollabSession;
//!
//! let schema = Rc::new(Schema::starter_kit());
//! let para = schema.branch("paragraph", Fragment::from_node(schema.text("hi").unwrap())).unwrap();
//! let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
//! let state = EditorState::create(schema.clone(), doc, default_plugins());
//!
//! // Peer A starts a session and shares a snapshot.
//! let a = CollabSession::new(&state).unwrap();
//! let b = CollabSession::from_bytes(&a.snapshot()).unwrap();
//! # let _ = (&a, &b, &state);
//! ```

pub mod error;
pub mod plugin;
pub mod projection;
pub mod rebase;
pub mod remote;
pub mod session;
pub mod sync;

// The local projection (`CollabDoc::project_transaction` / `project_change`) is wired in
// here as an inherent-impl module.
mod project;

/// `CollabSession` and `CollabDoc` must stay **`Send`**: a server holds a session across
/// an `.await`, so losing the bound breaks downstream consumers at their next upgrade —
/// as a compile error in *their* tree, which is the worst place to find out.
///
/// It has been lost once already. Moving the broadcast delta to an observer-fed outbox
/// (#190) introduced both an `Rc<RefCell<_>>` queue and yrs's `Subscription`, which is
/// only `Send` with yrs's `sync` feature. Hence the queue is an `Arc<Mutex<_>>` and that
/// feature is on — and hence this assertion, which stops the crate compiling if either
/// ever regresses.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<CollabDoc>();
    assert_send::<CollabSession>();
};

pub use error::{CollabError, Result};
pub use plugin::{COLLAB_KEY, CollabPlugin, CollabState};
pub use projection::CollabDoc;
pub use rebase::rebase_steps;
pub use remote::{ORIGIN_REMOTE, build_remote_transaction};
pub use session::CollabSession;
