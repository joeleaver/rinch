//! Error types for the editor.

use thiserror::Error;

/// Error types for editor operations.
#[derive(Debug, Error)]
pub enum EditorError {
    /// Unknown node type encountered
    #[error("Unknown node type: {0}")]
    UnknownNodeType(String),

    /// Unknown mark type encountered
    #[error("Unknown mark type: {0}")]
    UnknownMark(String),

    /// Invalid content in a node
    #[error("Invalid content: expected {expected}, found {found} in node {node}")]
    InvalidContent {
        node: String,
        expected: String,
        found: String,
    },

    /// Mark not allowed in context
    #[error("Mark not allowed: {mark} cannot be applied in {node}")]
    MarkNotAllowed { mark: String, node: String },

    /// Invalid position in document
    #[error("Invalid position: {0} (document length: {1})")]
    InvalidPosition(usize, usize),

    /// Invalid range in document
    #[error("Invalid range: {start}..{end}")]
    InvalidRange { start: usize, end: usize },

    /// Unknown command
    #[error("Unknown command: {0}")]
    UnknownCommand(String),

    /// Command execution error
    #[error("Command failed: {0}")]
    CommandFailed(String),

    /// Schema validation error
    #[error("Schema validation error: {0}")]
    SchemaValidation(String),

    /// HTML parse error
    #[error("HTML parse error: {0}")]
    HtmlParse(String),

    /// Markdown parse error
    #[error("Markdown parse error: {0}")]
    MarkdownParse(String),
}
