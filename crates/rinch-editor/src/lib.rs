//! Rinch Rich-Text Editor
//!
//! A comprehensive rich-text editor with CRDT collaboration support,
//! built for the Rinch GUI framework.
//!
//! # Architecture
//!
//! The editor uses a **ContentEditable** approach:
//! - A contentEditable div handles cursor rendering, selection display, and hit testing
//! - The document model (backed by Automerge CRDT) is the source of truth
//! - The bridge layer reconciles the document model into DOM nodes
//! - Keyboard events are intercepted and routed through editor commands
//!
//! # Core Components
//!
//! - [`Editor`] - The main editor instance
//! - [`EditorDocument`] - Automerge-backed document model
//! - [`EditorBridge`] - Connects the editor to contentEditable rendering
//! - [`Schema`] - Document structure validation
//! - [`Extension`] - Plugin system for nodes, marks, and functionality
//! - [`Commands`] - All document mutations go through commands

pub mod bridge;
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

pub use bridge::EditorBridge;
pub use document::{EditorDocument, MarkData, Position, Range};
pub use editor::Editor;
pub use error::EditorError;
pub use extensions::CommandRegistration;
pub use schema::Schema;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::bridge::EditorBridge;
    pub use crate::document::{EditorDocument, MarkData, Position, Range};
    pub use crate::editor::{AutoFocus, Editor, EditorConfig};
    pub use crate::error::EditorError;
    pub use crate::extensions::{Extension, StarterKit};
    pub use crate::schema::Schema;
}
