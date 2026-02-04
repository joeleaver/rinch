//! Main editor instance.

use crate::commands::CommandDispatcher;
use crate::document::{EditorDocument, MarkData, Range};
use crate::error::EditorError;
use crate::events::EventDispatcher;
use crate::extensions::ExtensionRegistry;
use crate::history::{History, UndoOperation};
use crate::input::{InputRuleSet, ShortcutRegistry};
use crate::schema::Schema;
use crate::selection::{Selection, SelectionState};

/// Tracks which parts of the document changed during an operation.
#[derive(Debug, Clone, Default)]
pub struct EditorChanges {
    /// Block indices that need re-rendering (content or type changed)
    pub changed_blocks: Vec<usize>,
    /// Whether block count changed (blocks added or removed)
    pub structure_changed: bool,
}

/// The main editor instance.
#[derive(Debug)]
pub struct Editor {
    /// The document being edited
    pub doc: EditorDocument,
    /// Document schema for validation
    pub schema: Schema,
    /// Current selection state
    pub selection: SelectionState,
    /// Undo/redo history
    pub history: History,
    /// Command dispatcher
    pub commands: CommandDispatcher,
    /// Registered extensions
    pub extensions: ExtensionRegistry,
    /// Input rules
    pub input_rules: InputRuleSet,
    /// Keyboard shortcuts
    pub shortcuts: ShortcutRegistry,
    /// Event dispatcher
    pub events: EventDispatcher,
    /// Editor configuration
    pub config: EditorConfig,
    /// Stored marks for cursor-position formatting.
    /// When typing at a cursor position, these marks are applied to inserted text.
    pub stored_marks: Vec<String>,
    /// Tracks which blocks changed since last take_changes() call
    pub pending_changes: EditorChanges,
}

impl Editor {
    /// Create a new editor with the given schema and configuration.
    pub fn new(schema: Schema, config: EditorConfig) -> Result<Self, EditorError> {
        let doc = EditorDocument::new();
        let selection = SelectionState::new(Selection::cursor(crate::document::Position::new(0)));

        Ok(Self {
            doc,
            schema,
            selection,
            history: History::new(),
            commands: CommandDispatcher::new(),
            extensions: ExtensionRegistry::new(),
            input_rules: InputRuleSet::new(),
            shortcuts: ShortcutRegistry::new(),
            events: EventDispatcher::new(),
            config,
            stored_marks: Vec::new(),
            pending_changes: EditorChanges::default(),
        })
    }

    /// Get the current selection.
    pub fn get_selection(&self) -> &Selection {
        &self.selection.selection
    }

    /// Set the selection.
    pub fn set_selection(&mut self, selection: Selection) {
        self.selection.set_selection(selection);
    }

    /// Toggle a stored mark. If present, remove it; if absent, add it.
    pub fn toggle_stored_mark(&mut self, mark_type: &str) {
        if let Some(pos) = self.stored_marks.iter().position(|m| m == mark_type) {
            self.stored_marks.remove(pos);
        } else {
            self.stored_marks.push(mark_type.to_string());
        }
    }

    /// Check if a stored mark is active.
    pub fn has_stored_mark(&self, mark_type: &str) -> bool {
        self.stored_marks.iter().any(|m| m == mark_type)
    }

    /// Clear stored marks (called after text is inserted or cursor moves).
    pub fn clear_stored_marks(&mut self) {
        self.stored_marks.clear();
    }

    /// Record an undo operation.
    pub fn record_undo(&mut self, op: UndoOperation) {
        self.history.push_undo(op);
    }

    /// Record a typing operation (mergeable).
    pub fn record_typing(&mut self, op: UndoOperation) {
        self.history.merge_typing(op);
    }

    /// Perform undo: pop from history, apply inverse operation to document.
    pub fn undo(&mut self) -> Result<bool, EditorError> {
        if let Some(item) = self.history.undo()? {
            self.apply_inverse(&item.operation)?;
            self.mark_all_changed();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Perform redo: pop from redo stack, apply operation to document.
    pub fn redo(&mut self) -> Result<bool, EditorError> {
        if let Some(item) = self.history.redo()? {
            self.apply_operation(&item.operation)?;
            self.mark_all_changed();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get text in a given range from the document.
    /// Iterates over blocks to extract the text spanning the range.
    pub fn text_in_range(&self, range: Range) -> String {
        let full_text = self.doc.to_text();
        let start = range.start.0.min(full_text.len());
        let end = range.end.0.min(full_text.len());
        full_text[start..end].to_string()
    }

    /// Take pending changes, resetting the tracker.
    pub fn take_changes(&mut self) -> EditorChanges {
        std::mem::take(&mut self.pending_changes)
    }

    /// Mark a specific block as changed.
    pub fn mark_block_changed(&mut self, block_index: usize) {
        if !self.pending_changes.changed_blocks.contains(&block_index) {
            self.pending_changes.changed_blocks.push(block_index);
        }
    }

    /// Mark that block structure changed (blocks added/removed).
    pub fn mark_structure_changed(&mut self) {
        self.pending_changes.structure_changed = true;
    }

    /// Mark all blocks as changed (for operations like undo/redo).
    pub fn mark_all_changed(&mut self) {
        self.pending_changes.structure_changed = true;
    }

    fn apply_inverse(&mut self, op: &UndoOperation) -> Result<(), EditorError> {
        let inverse = op.inverse();
        self.apply_operation(&inverse)
    }

    fn apply_operation(&mut self, op: &UndoOperation) -> Result<(), EditorError> {
        match op {
            UndoOperation::InsertText { position, text } => {
                self.doc.insert_text(*position, text)?;
                self.set_selection(Selection::cursor(*position + text.len()));
                // Track change
                if let Ok(rp) = self.doc.resolve_position(*position) {
                    self.mark_block_changed(rp.block_index);
                }
            }
            UndoOperation::DeleteText { range, .. } => {
                self.doc.delete_range(*range)?;
                self.set_selection(Selection::cursor(range.start));
                // Delete can span blocks, so mark structure changed
                self.mark_structure_changed();
            }
            UndoOperation::AddMark {
                range,
                mark_type,
                attrs,
            } => {
                let mark = if attrs.is_empty() {
                    MarkData::new(mark_type)
                } else {
                    MarkData::with_attrs(mark_type, attrs.clone())
                };
                self.doc.add_mark(*range, mark)?;
                // Track changes for affected blocks
                if let Ok(start_rp) = self.doc.resolve_position(range.start)
                    && let Ok(end_rp) = self.doc.resolve_position(range.end)
                {
                    for bi in start_rp.block_index..=end_rp.block_index {
                        self.mark_block_changed(bi);
                    }
                }
            }
            UndoOperation::RemoveMark { range, mark_type } => {
                self.doc.remove_mark(*range, mark_type)?;
                // Track changes for affected blocks
                if let Ok(start_rp) = self.doc.resolve_position(range.start)
                    && let Ok(end_rp) = self.doc.resolve_position(range.end)
                {
                    for bi in start_rp.block_index..=end_rp.block_index {
                        self.mark_block_changed(bi);
                    }
                }
            }
            UndoOperation::SplitBlock { position, .. } => {
                self.doc.split_block(*position)?;
                self.set_selection(Selection::cursor(*position + 1));
                self.mark_structure_changed();
            }
            UndoOperation::JoinBlocks { position, .. } => {
                // Delete the newline at position to join blocks
                self.doc
                    .delete_range(Range::new(*position, *position + 1))?;
                self.set_selection(Selection::cursor(*position));
                self.mark_structure_changed();
            }
            UndoOperation::SetBlockType {
                block_index,
                new_type,
                new_attrs,
                ..
            } => {
                let attrs = if new_attrs.is_empty() {
                    None
                } else {
                    Some(new_attrs.clone())
                };
                self.doc.set_block_type(*block_index, new_type, attrs)?;
                self.mark_block_changed(*block_index);
            }
        }
        Ok(())
    }
}

/// Editor configuration.
#[derive(Debug, Clone)]
pub struct EditorConfig {
    /// Whether the editor should auto-focus on mount
    pub autofocus: AutoFocus,
    /// Whether the editor is editable
    pub editable: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            autofocus: AutoFocus::default(),
            editable: true,
        }
    }
}

/// Auto-focus configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoFocus {
    /// Focus on mount
    Start,
    /// Focus at end of document
    End,
    /// Don't auto-focus
    #[default]
    None,
}
