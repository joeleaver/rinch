//! Event handling for editor interactions.

use crate::editor::Editor;
use crate::error::EditorError;

/// Editor event types.
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// Text was inserted
    TextInserted { text: String },
    /// Text was deleted
    TextDeleted { from: usize, to: usize },
    /// Selection changed
    SelectionChanged,
    /// Document updated
    DocumentUpdated,
}

/// Event handler for editor events.
pub trait EventHandler: std::fmt::Debug {
    /// Handle an editor event.
    fn handle(&mut self, event: EditorEvent, editor: &mut Editor) -> Result<(), EditorError>;
}

/// Event dispatcher for managing event handlers.
#[derive(Debug, Default)]
pub struct EventDispatcher {
    handlers: Vec<Box<dyn EventHandler>>,
}

impl EventDispatcher {
    /// Create a new event dispatcher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an event handler.
    pub fn register(&mut self, handler: Box<dyn EventHandler>) {
        self.handlers.push(handler);
    }

    /// Dispatch an event to all handlers.
    pub fn dispatch(&mut self, event: EditorEvent, editor: &mut Editor) -> Result<(), EditorError> {
        for handler in &mut self.handlers {
            handler.handle(event.clone(), editor)?;
        }
        Ok(())
    }
}
