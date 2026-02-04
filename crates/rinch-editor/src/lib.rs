//! Rinch Rich-Text Editor
//!
//! A comprehensive rich-text editor with CRDT collaboration support,
//! built for the Rinch GUI framework.
//!
//! # Architecture
//!
//! The editor uses a **Hidden Textarea + Custom DOM Rendering** approach:
//! - A hidden `<textarea>` captures keyboard input, IME, and clipboard operations
//! - The document model (backed by Automerge CRDT) is the source of truth
//! - DOM nodes are rendered from the document model using Rinch's NodeHandle API
//! - Custom cursor and selection overlays provide visual feedback
//!
//! # Core Components
//!
//! - [`Editor`] - The main editor instance
//! - [`EditorDocument`] - Automerge-backed document model
//! - [`Schema`] - Document structure validation
//! - [`Extension`] - Plugin system for nodes, marks, and functionality
//! - [`Commands`] - All document mutations go through commands

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
pub mod view;

#[cfg(test)]
pub mod testing;

pub use document::{EditorDocument, MarkData, Position, Range};
pub use editor::{Editor, EditorChanges};
pub use error::EditorError;
pub use extensions::CommandRegistration;
pub use schema::Schema;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::document::{EditorDocument, MarkData, Position, Range};
    pub use crate::editor::{AutoFocus, Editor, EditorChanges, EditorConfig};
    pub use crate::error::EditorError;
    pub use crate::extensions::{Extension, StarterKit};
    pub use crate::schema::Schema;
}
