//! Main editor instance.

use std::collections::HashMap;

use crate::commands::CommandDispatcher;
use crate::document::{EditorDocument, MarkData, Range};
use crate::error::EditorError;
use crate::events::EventDispatcher;
use crate::extensions::table_model::TableModel;
use crate::extensions::{ExtensionRegistry, StarterKit};
use crate::history::{History, UndoOperation};
use crate::input::{InputRuleSet, ShortcutRegistry};
use crate::schema::Schema;
use crate::selection::{Selection, SelectionState};

/// Reference to a specific cell in a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellRef {
    pub table_id: String,
    pub block_index: usize,
    pub row: usize,
    pub col: usize,
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
    /// - `None` = no preference, inherit marks from cursor context
    /// - `Some(vec)` = explicitly use these marks (even if empty = no marks)
    pub stored_marks: Option<Vec<String>>,
    /// Table data storage: maps table_id -> TableModel.
    /// Table blocks reference their data via a "table_id" attribute.
    pub tables: HashMap<String, TableModel>,
    /// Counter for generating unique table IDs.
    next_table_id: usize,
    /// Currently selected table cell (for interactive table editing).
    pub table_selection: Option<TableCellRef>,
}

impl Editor {
    /// Create a new editor with the given schema and configuration.
    pub fn new(schema: Schema, config: EditorConfig) -> Result<Self, EditorError> {
        let doc = EditorDocument::new();
        let selection = SelectionState::new(Selection::cursor(crate::document::Position::new(0)));

        // Register StarterKit extensions for input rules, shortcuts, and commands
        let mut extensions = ExtensionRegistry::new();
        for ext in StarterKit::extensions() {
            extensions.register(ext);
        }
        let input_rules = extensions.build_input_rules();
        let shortcuts = extensions.build_shortcuts();

        Ok(Self {
            doc,
            schema,
            selection,
            history: History::new(),
            commands: CommandDispatcher::new(),
            extensions,
            input_rules,
            shortcuts,
            events: EventDispatcher::new(),
            config,
            stored_marks: None,
            tables: HashMap::new(),
            next_table_id: 0,
            table_selection: None,
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
    ///
    /// When stored_marks is `None` (no preference), initializes from the
    /// document's marks at the cursor position before toggling. This ensures
    /// that toggling bold OFF at the end of bold text produces `Some([])`,
    /// meaning "explicitly no marks", rather than just `None` (inherit).
    pub fn toggle_stored_mark(&mut self, mark_type: &str) {
        let marks = self.stored_marks.get_or_insert_with(|| {
            let pos = self.selection.selection.head;
            self.doc
                .marks_at(pos)
                .iter()
                .map(|m| m.mark_type.clone())
                .collect()
        });

        if let Some(pos) = marks.iter().position(|m| m == mark_type) {
            marks.remove(pos);
        } else {
            marks.push(mark_type.to_string());
        }
    }

    /// Check if a stored mark is active.
    ///
    /// When stored_marks is `None`, falls back to checking the document's
    /// marks at the cursor position (inheriting from context).
    pub fn has_stored_mark(&self, mark_type: &str) -> bool {
        match &self.stored_marks {
            Some(marks) => marks.iter().any(|m| m == mark_type),
            None => {
                let pos = self.selection.selection.head;
                self.doc
                    .marks_at(pos)
                    .iter()
                    .any(|m| m.mark_type == mark_type)
            }
        }
    }

    /// Clear stored marks (called after text is inserted or cursor moves).
    /// Resets to `None` (no preference — inherit from context).
    pub fn clear_stored_marks(&mut self) {
        self.stored_marks = None;
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
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Perform redo: pop from redo stack, apply operation to document.
    pub fn redo(&mut self) -> Result<bool, EditorError> {
        if let Some(item) = self.history.redo()? {
            self.apply_operation(&item.operation)?;
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

    /// Insert a table at the current cursor position.
    ///
    /// Creates a new block of type "table" after the current block,
    /// stores a TableModel with the given dimensions, and moves the cursor
    /// past the table block.
    pub fn insert_table(&mut self, rows: usize, cols: usize) -> Result<String, EditorError> {
        let sel = self.get_selection().clone();
        let resolved = self.doc.resolve_position(sel.head)?;
        let insert_at = resolved.block_index + 1;

        // Generate unique table ID
        let table_id = format!("t{}", self.next_table_id);
        self.next_table_id += 1;

        // Create table model with header row
        let table = TableModel::with_header(rows, cols);

        // Create attrs with table metadata
        let mut attrs = HashMap::new();
        attrs.insert("table_id".to_string(), table_id.clone());
        attrs.insert("rows".to_string(), rows.to_string());
        attrs.insert("cols".to_string(), cols.to_string());

        // Insert the table block into the document
        self.doc.insert_block_at(insert_at, "table", Some(attrs))?;

        // Store the table model
        self.tables.insert(table_id.clone(), table);

        // Move cursor past the table block
        let table_block_start = self.doc.block_start_position(insert_at);
        self.set_selection(Selection::cursor(crate::document::Position::new(
            table_block_start,
        )));

        Ok(table_id)
    }

    /// Delete a table block and its associated data.
    pub fn delete_table(&mut self, block_index: usize) -> Result<(), EditorError> {
        // Get table_id from block attrs before deleting
        if let Some(table_id) = self.table_id_for_block(block_index) {
            self.tables.remove(&table_id);
        }
        // Delete the block from the document
        self.doc.delete_block(block_index)
    }

    /// Get the table_id for a block (reads "table_id" from block attrs).
    pub fn table_id_for_block(&self, block_index: usize) -> Option<String> {
        self.doc
            .block_attrs(block_index)
            .and_then(|attrs| attrs.get("table_id").cloned())
    }

    /// Get a reference to a table by its ID.
    pub fn get_table(&self, table_id: &str) -> Option<&TableModel> {
        self.tables.get(table_id)
    }

    /// Get a mutable reference to a table by its ID.
    pub fn get_table_mut(&mut self, table_id: &str) -> Option<&mut TableModel> {
        self.tables.get_mut(table_id)
    }

    /// Get a reference to a table by block index.
    pub fn table_at_block(&self, block_index: usize) -> Option<&TableModel> {
        let table_id = self.table_id_for_block(block_index)?;
        self.tables.get(&table_id)
    }

    /// Get a mutable reference to a table by block index.
    pub fn table_at_block_mut(&mut self, block_index: usize) -> Option<&mut TableModel> {
        let table_id = self.table_id_for_block(block_index)?;
        self.tables.get_mut(&table_id)
    }

    /// Select a specific table cell for interactive editing.
    pub fn select_table_cell(
        &mut self,
        table_id: String,
        block_index: usize,
        row: usize,
        col: usize,
    ) {
        self.table_selection = Some(TableCellRef {
            table_id,
            block_index,
            row,
            col,
        });
    }

    /// Clear the current table cell selection.
    pub fn clear_table_selection(&mut self) {
        self.table_selection = None;
    }

    /// Get the currently selected table cell, if any.
    pub fn selected_table_cell(&self) -> Option<&TableCellRef> {
        self.table_selection.as_ref()
    }

    /// Export the current document as markdown.
    pub fn get_markdown(&self) -> String {
        self.doc.to_markdown()
    }

    /// Replace document content from a markdown string.
    pub fn set_markdown(&mut self, md: &str) -> Result<(), EditorError> {
        use crate::document::Fragment;

        let fragment = Fragment::from_markdown(md);

        // Clear document - delete all blocks except the first, clear first block
        while self.doc.block_count() > 1 {
            let _ = self.doc.delete_block(self.doc.block_count() - 1);
        }
        // Clear the first block's content
        self.doc
            .delete_range(crate::document::Range::new(0usize, self.doc.text_length()))?;

        // Insert fragment blocks
        let mut first = true;
        for block in &fragment.blocks {
            if !first {
                let pos = crate::document::Position::new(self.doc.text_length());
                self.doc.split_block(pos)?;
            }

            // Set block type
            let block_index = if first { 0 } else { self.doc.block_count() - 1 };
            if block.block_type != "paragraph" || !block.attrs.is_empty() {
                let attrs = if block.attrs.is_empty() {
                    None
                } else {
                    Some(block.attrs.clone())
                };
                self.doc
                    .set_block_type(block_index, &block.block_type, attrs)?;
            }

            // Insert text content
            let mut full_text = String::new();
            let mut mark_ranges: Vec<(usize, usize, crate::document::MarkData)> = Vec::new();

            for inline in &block.content {
                match inline {
                    crate::document::FragmentInline::Text { text, marks } => {
                        let start = full_text.len();
                        full_text.push_str(text);
                        let end = full_text.len();
                        for mark in marks {
                            mark_ranges.push((start, end, mark.clone()));
                        }
                    }
                    crate::document::FragmentInline::HardBreak => {
                        full_text.push('\n');
                    }
                }
            }

            if !full_text.is_empty() {
                let insert_pos =
                    crate::document::Position::new(self.doc.block_start_position(block_index));
                self.doc.insert_text(insert_pos, &full_text)?;

                for (start, end, mark) in &mark_ranges {
                    let abs_start = self.doc.block_start_position(block_index) + start;
                    let abs_end = self.doc.block_start_position(block_index) + end;
                    let _ = self.doc.add_mark(
                        crate::document::Range::new(abs_start, abs_end),
                        mark.clone(),
                    );
                }
            }

            first = false;
        }

        self.set_selection(crate::selection::Selection::cursor(
            crate::document::Position::new(0),
        ));
        Ok(())
    }

    /// Insert markdown content at the current cursor position.
    pub fn insert_markdown(&mut self, md: &str) -> Result<(), EditorError> {
        use crate::commands::{StructureCommands, TextCommands};
        use crate::document::Fragment;

        let fragment = Fragment::from_markdown(md);

        // Delete selection first if any
        let sel = self.get_selection().clone();
        if !sel.is_cursor() {
            let range = sel.range();
            TextCommands::delete_range(self, range)?;
        }

        let mut first = true;
        for block in &fragment.blocks {
            if !first {
                StructureCommands::split_block(self)?;
            }

            // Set block type for current block
            if block.block_type != "paragraph" || !block.attrs.is_empty() {
                let resolved = self.doc.resolve_position(self.get_selection().head)?;
                let attrs = if block.attrs.is_empty() {
                    None
                } else {
                    Some(block.attrs.clone())
                };
                self.doc
                    .set_block_type(resolved.block_index, &block.block_type, attrs)?;
            }

            // Insert inline content
            let mut full_text = String::new();
            let mut mark_ranges: Vec<(usize, usize, crate::document::MarkData)> = Vec::new();

            for inline in &block.content {
                match inline {
                    crate::document::FragmentInline::Text { text, marks } => {
                        let start = full_text.len();
                        full_text.push_str(text);
                        let end = full_text.len();
                        for mark in marks {
                            mark_ranges.push((start, end, mark.clone()));
                        }
                    }
                    crate::document::FragmentInline::HardBreak => {
                        full_text.push('\n');
                    }
                }
            }

            if !full_text.is_empty() {
                let pos = self.get_selection().head;
                self.doc.insert_text(pos, &full_text)?;

                for (start, end, mark) in &mark_ranges {
                    let mark_range = crate::document::Range::new(pos + *start, pos + *end);
                    let _ = self.doc.add_mark(mark_range, mark.clone());
                }

                self.set_selection(crate::selection::Selection::cursor(pos + full_text.len()));
            }

            first = false;
        }

        Ok(())
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
            }
            UndoOperation::DeleteText { range, .. } => {
                self.doc.delete_range(*range)?;
                self.set_selection(Selection::cursor(range.start));
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
            }
            UndoOperation::RemoveMark { range, mark_type } => {
                self.doc.remove_mark(*range, mark_type)?;
            }
            UndoOperation::SplitBlock { position, .. } => {
                self.doc.split_block(*position)?;
                self.set_selection(Selection::cursor(*position + 1));
            }
            UndoOperation::JoinBlocks { position, .. } => {
                // Delete the newline at position to join blocks
                self.doc
                    .delete_range(Range::new(*position, *position + 1))?;
                self.set_selection(Selection::cursor(*position));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;

    #[test]
    fn table_cell_selection() {
        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        let table_id = editor.insert_table(3, 3).unwrap();

        assert!(editor.selected_table_cell().is_none());

        editor.select_table_cell(table_id.clone(), 1, 0, 0);
        let sel = editor.selected_table_cell().unwrap();
        assert_eq!(sel.table_id, table_id);
        assert_eq!(sel.block_index, 1);
        assert_eq!(sel.row, 0);
        assert_eq!(sel.col, 0);

        // Move to a different cell
        editor.select_table_cell(table_id.clone(), 1, 1, 2);
        let sel = editor.selected_table_cell().unwrap();
        assert_eq!(sel.row, 1);
        assert_eq!(sel.col, 2);

        editor.clear_table_selection();
        assert!(editor.selected_table_cell().is_none());
    }

    #[test]
    fn table_cell_selection_independent_of_cursor() {
        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        let table_id = editor.insert_table(3, 3).unwrap();

        // Select a table cell
        editor.select_table_cell(table_id.clone(), 1, 0, 0);

        // Move the document cursor — table selection should remain
        editor.set_selection(crate::selection::Selection::cursor(
            crate::document::Position::new(0),
        ));
        assert!(editor.selected_table_cell().is_some());
        assert_eq!(editor.selected_table_cell().unwrap().table_id, table_id);
    }

    #[test]
    fn table_cell_ref_equality() {
        let a = TableCellRef {
            table_id: "t0".into(),
            block_index: 1,
            row: 0,
            col: 0,
        };
        let b = TableCellRef {
            table_id: "t0".into(),
            block_index: 1,
            row: 0,
            col: 0,
        };
        let c = TableCellRef {
            table_id: "t0".into(),
            block_index: 1,
            row: 1,
            col: 0,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
