//! [`CollabError`] — the one error type for the collab adapter.
//!
//! The most important variant is [`CollabError::Unsupported`]: per design amendment
//! **A22**, the staged first-milestone scope is **flat text-blocks + marks**. When the
//! adapter meets a model shape it cannot faithfully project onto the CRDT (a nested
//! block, an inline atom, a multi-block paste it cannot reduce), it **fails loud** with
//! `Unsupported` rather than silently dropping the change. A silent drop would
//! reintroduce the exact "the two sides disagree" divergence class the editor rewrite
//! set out to kill.

use thiserror::Error;

/// Why a collab projection / sync step failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CollabError {
    /// An underlying automerge operation failed (I/O, decode, sync-protocol error).
    #[error("automerge error: {0}")]
    Automerge(String),

    /// The model shape is outside the staged first-milestone scope (nested blocks,
    /// inline atoms, deep nesting). **Fail-loud, never a silent drop** (design A22).
    #[error("collab does not support this content yet: {0}")]
    Unsupported(String),

    /// A reconstructed model node failed schema validation, or a schema lookup the
    /// projection relied on was missing.
    #[error("schema/projection error: {0}")]
    Schema(String),
}

impl CollabError {
    /// Construct an [`CollabError::Unsupported`] from a message.
    pub fn unsupported(msg: impl Into<String>) -> CollabError {
        CollabError::Unsupported(msg.into())
    }

    /// Construct a [`CollabError::Schema`] from a message.
    pub fn schema(msg: impl Into<String>) -> CollabError {
        CollabError::Schema(msg.into())
    }
}

impl From<automerge::AutomergeError> for CollabError {
    fn from(e: automerge::AutomergeError) -> CollabError {
        CollabError::Automerge(e.to_string())
    }
}

impl From<rinch_editor_core::EditorError> for CollabError {
    fn from(e: rinch_editor_core::EditorError) -> CollabError {
        CollabError::Schema(e.to_string())
    }
}

impl From<rinch_editor_core::StepError> for CollabError {
    fn from(e: rinch_editor_core::StepError) -> CollabError {
        CollabError::Schema(e.to_string())
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, CollabError>;
