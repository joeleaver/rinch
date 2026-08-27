//! Generic text editing primitives for rinch.
//!
//! This crate provides core types and traits for building text editors:
//!
//! - [`Position`] and [`Range`] for text positions
//! - [`Selection`] for cursor and selection state
//! - [`EditCommand`] for the standard set of editing commands
//! - [`EditableDocument`] trait for document implementations
//! - [`StringDocument`] for single-line text
//! - [`EditableState`] for orchestrating editing operations
//! - [`RewriteDiff`] for adopting an external rewrite of the text without losing the caret
//! - [`InputHandler`] for mapping keyboard input to commands

mod command;
mod document;
mod input;
mod operation;
mod position;
mod rewrite;
mod selection;
mod state;
mod string_doc;
mod undo;

pub use command::EditCommand;
pub use document::EditableDocument;
pub use input::{InputHandler, Key, Modifiers};
pub use operation::TextOperation;
pub use position::{Position, Range};
pub use rewrite::RewriteDiff;
pub use selection::Selection;
pub use state::EditableState;
pub use string_doc::StringDocument;
pub use undo::{Invertible, UndoStack};
