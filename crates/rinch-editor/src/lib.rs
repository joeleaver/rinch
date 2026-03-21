//! Rinch Rich-Text Editor
//!
//! A comprehensive rich-text editor with CRDT collaboration support,
//! built for the Rinch GUI framework.
//!
//! # Architecture
//!
//! The editor uses a **ContentEditable** approach:
//! - A contentEditable div handles cursor rendering, selection display, and hit testing
//! - The CE API (in `rinch-core`) is the single mutation path for all operations
//! - The document model (backed by Automerge CRDT) is used for serialization
//! - Keyboard events and toolbar commands both go through the CE API
//!
//! # Core Components
//!
//! - [`EditorDocument`] - Automerge-backed document model (serialization)
//! - [`Schema`] - Document structure validation
//! - [`Extension`] - Plugin system for nodes, marks, and functionality

pub mod commands;
pub mod document;
pub mod editor;
pub mod error;
pub mod events;
pub mod extensions;
pub mod history;
pub mod input;
pub mod schema;
pub mod selection;

#[cfg(test)]
pub mod testing;

pub use document::{EditorDocument, MarkData, Position, Range};
pub use editor::Editor;
pub use error::EditorError;
pub use extensions::CommandRegistration;
pub use schema::Schema;

/// CE-to-document bridge for real-time collaboration.
///
/// Gated behind the `collaboration` feature flag.
#[cfg(feature = "collaboration")]
pub use document::bridge;

/// Sync protocol types for collaborative editing.
///
/// Gated behind the `collaboration` feature flag.
#[cfg(feature = "collaboration")]
pub use document::sync;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::document::{EditorDocument, MarkData, Position, Range};
    pub use crate::editor::{AutoFocus, Editor, EditorConfig};
    pub use crate::error::EditorError;
    pub use crate::extensions::{Extension, StarterKit};
    pub use crate::schema::Schema;

    #[cfg(feature = "collaboration")]
    pub use crate::document::bridge::CeDocBridge;
    #[cfg(feature = "collaboration")]
    pub use crate::document::sync::{ChangeHash, SyncMessage, SyncState};
}
