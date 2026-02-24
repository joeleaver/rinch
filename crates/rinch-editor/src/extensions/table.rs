//! Table extension for full table editing support.

use crate::editor::Editor;
use crate::error::EditorError;
use crate::extensions::{CommandRegistration, Extension};
use crate::input::KeyboardShortcut;
use crate::schema::{AttrSpec, NodeSpec};

/// Table extension providing table editing commands and node types.
#[derive(Debug)]
pub struct TableExtension;

impl Extension for TableExtension {
    fn name(&self) -> &str {
        "table"
    }

    fn priority(&self) -> i32 {
        50
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("table")
                .content("table_row+")
                .group("block")
                .isolating(true)
                .build(),
            NodeSpec::builder("table_row")
                .content("table_cell+")
                .build(),
            NodeSpec::builder("table_cell")
                .content("block+")
                .isolating(true)
                .attr("colspan", AttrSpec::optional("1"))
                .attr("rowspan", AttrSpec::optional("1"))
                .build(),
            NodeSpec::builder("table_header")
                .content("block+")
                .isolating(true)
                .attr("colspan", AttrSpec::optional("1"))
                .attr("rowspan", AttrSpec::optional("1"))
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("insert_table", cmd_insert_table),
            CommandRegistration::new("delete_table", cmd_delete_table),
            CommandRegistration::new("add_row_before", cmd_add_row_before),
            CommandRegistration::new("add_row_after", cmd_add_row_after),
            CommandRegistration::new("delete_row", cmd_delete_row),
            CommandRegistration::new("add_column_before", cmd_add_column_before),
            CommandRegistration::new("add_column_after", cmd_add_column_after),
            CommandRegistration::new("delete_column", cmd_delete_column),
            CommandRegistration::new("merge_cells", cmd_merge_cells),
            CommandRegistration::new("split_cell", cmd_split_cell),
            CommandRegistration::new("toggle_header_row", cmd_toggle_header_row),
        ]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![
            (
                KeyboardShortcut::new("Tab", "Move to next cell"),
                "table_next_cell".into(),
            ),
            (
                KeyboardShortcut::new("Shift-Tab", "Move to previous cell"),
                "table_prev_cell".into(),
            ),
        ]
    }
}

/// Find the table block at or near the cursor position.
/// Returns (block_index, table_id) if the cursor is at a table block.
///
/// Checks the explicit table_selection first (set by clicking a cell),
/// then falls back to cursor-based detection.
fn find_cursor_table(editor: &Editor) -> Option<(usize, String)> {
    // First check if we have an explicit table selection
    if let Some(ref cell_ref) = editor.table_selection {
        return Some((cell_ref.block_index, cell_ref.table_id.clone()));
    }

    // Fall back to cursor-based detection
    let sel = editor.get_selection();
    let resolved = editor.doc.resolve_position(sel.head).ok()?;
    let block_idx = resolved.block_index;

    // Check if current block is a table
    if let Some(bt) = editor.doc.block_type(block_idx)
        && bt == "table"
    {
        let table_id = editor.table_id_for_block(block_idx)?;
        return Some((block_idx, table_id));
    }

    // Check adjacent blocks (cursor might be right before/after a table)
    if block_idx > 0 && editor.doc.block_type(block_idx - 1).as_deref() == Some("table") {
        let table_id = editor.table_id_for_block(block_idx - 1)?;
        return Some((block_idx - 1, table_id));
    }
    if block_idx + 1 < editor.doc.block_count()
        && editor.doc.block_type(block_idx + 1).as_deref() == Some("table")
    {
        let table_id = editor.table_id_for_block(block_idx + 1)?;
        return Some((block_idx + 1, table_id));
    }

    None
}

fn cmd_insert_table(editor: &mut Editor) -> Result<(), EditorError> {
    let _ = editor.insert_table(3, 3)?;
    Ok(())
}

fn cmd_delete_table(editor: &mut Editor) -> Result<(), EditorError> {
    let (block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    editor.tables.remove(&table_id);
    editor.doc.delete_block(block_idx)?;
    Ok(())
}

fn cmd_add_row_before(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    // Insert before the first body row (after header if present)
    let insert_at = table.header_rows;
    table.insert_row(insert_at);
    Ok(())
}

fn cmd_add_row_after(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    // Insert at end
    let row_count = table.row_count();
    table.insert_row(row_count);
    Ok(())
}

fn cmd_delete_row(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    // Delete last body row
    let last_row = table.row_count().saturating_sub(1);
    table.delete_row(last_row)?;
    Ok(())
}

fn cmd_add_column_before(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    table.insert_column(0);
    Ok(())
}

fn cmd_add_column_after(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    let col_count = table.col_count();
    table.insert_column(col_count);
    Ok(())
}

fn cmd_delete_column(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    let last_col = table.col_count().saturating_sub(1);
    table.delete_column(last_col)?;
    Ok(())
}

fn cmd_merge_cells(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    // Merge first two cells as a demo (full implementation needs cell selection)
    if table.col_count() >= 2 {
        table.merge_cells((0, 0), (0, 1))?;
    }
    Ok(())
}

fn cmd_split_cell(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    // Split first cell if it's merged
    table.split_cell(0, 0)?;
    Ok(())
}

fn cmd_toggle_header_row(editor: &mut Editor) -> Result<(), EditorError> {
    let (_block_idx, table_id) = find_cursor_table(editor)
        .ok_or_else(|| EditorError::CommandFailed("No table at cursor".into()))?;
    let table = editor
        .tables
        .get_mut(&table_id)
        .ok_or_else(|| EditorError::CommandFailed("Table data not found".into()))?;
    table.toggle_header_row();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_name() {
        let ext = TableExtension;
        assert_eq!(ext.name(), "table");
    }

    #[test]
    fn extension_priority() {
        let ext = TableExtension;
        assert_eq!(ext.priority(), 50);
    }

    #[test]
    fn provides_node_specs() {
        let ext = TableExtension;
        let nodes = ext.nodes();
        assert_eq!(nodes.len(), 4);
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"table"));
        assert!(names.contains(&"table_row"));
        assert!(names.contains(&"table_cell"));
        assert!(names.contains(&"table_header"));
    }

    #[test]
    fn table_node_is_isolating() {
        let ext = TableExtension;
        let nodes = ext.nodes();
        let table = nodes.iter().find(|n| n.name == "table").unwrap();
        assert!(table.isolating);
    }

    #[test]
    fn table_cell_has_span_attrs() {
        let ext = TableExtension;
        let nodes = ext.nodes();
        let cell = nodes.iter().find(|n| n.name == "table_cell").unwrap();
        assert!(cell.attrs.contains_key("colspan"));
        assert!(cell.attrs.contains_key("rowspan"));
        assert_eq!(cell.attrs["colspan"].default, Some("1".into()));
        assert_eq!(cell.attrs["rowspan"].default, Some("1".into()));
    }

    #[test]
    fn table_header_has_span_attrs() {
        let ext = TableExtension;
        let nodes = ext.nodes();
        let hdr = nodes.iter().find(|n| n.name == "table_header").unwrap();
        assert!(hdr.attrs.contains_key("colspan"));
        assert!(hdr.attrs.contains_key("rowspan"));
    }

    #[test]
    fn provides_commands() {
        let ext = TableExtension;
        let cmds = ext.commands();
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"insert_table"));
        assert!(names.contains(&"delete_table"));
        assert!(names.contains(&"add_row_before"));
        assert!(names.contains(&"add_row_after"));
        assert!(names.contains(&"delete_row"));
        assert!(names.contains(&"add_column_before"));
        assert!(names.contains(&"add_column_after"));
        assert!(names.contains(&"delete_column"));
        assert!(names.contains(&"merge_cells"));
        assert!(names.contains(&"split_cell"));
        assert!(names.contains(&"toggle_header_row"));
    }

    #[test]
    fn provides_keyboard_shortcuts() {
        let ext = TableExtension;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].1, "table_next_cell");
        assert_eq!(shortcuts[1].1, "table_prev_cell");
    }

    #[test]
    fn tab_shortcut_matches() {
        let ext = TableExtension;
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[0].0.matches("Tab"));
    }

    #[test]
    fn shift_tab_shortcut_matches() {
        let ext = TableExtension;
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[1].0.matches("Shift-Tab"));
    }

    #[test]
    fn command_count() {
        let ext = TableExtension;
        assert_eq!(ext.commands().len(), 11);
    }

    #[test]
    fn marks_is_empty() {
        let ext = TableExtension;
        assert!(ext.marks().is_empty());
    }

    #[test]
    fn input_rules_is_empty() {
        let ext = TableExtension;
        assert!(ext.input_rules().is_empty());
    }

    #[test]
    fn insert_table_command() {
        use crate::editor::{Editor, EditorConfig};
        use crate::schema::Schema;

        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        cmd_insert_table(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 2); // original + table
        assert_eq!(editor.doc.block_type(1), Some("table".into()));
        assert_eq!(editor.tables.len(), 1);
    }

    #[test]
    fn delete_table_command() {
        use crate::editor::{Editor, EditorConfig};
        use crate::schema::Schema;

        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        cmd_insert_table(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 2);

        // Cursor should be at the table block
        cmd_delete_table(&mut editor).unwrap();
        assert_eq!(editor.doc.block_count(), 1);
        assert!(editor.tables.is_empty());
    }

    #[test]
    fn add_row_after_command() {
        use crate::editor::{Editor, EditorConfig};
        use crate::schema::Schema;

        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        let table_id = editor.insert_table(3, 3).unwrap();
        assert_eq!(editor.tables[&table_id].row_count(), 3);

        cmd_add_row_after(&mut editor).unwrap();
        assert_eq!(editor.tables[&table_id].row_count(), 4);
    }

    #[test]
    fn add_column_after_command() {
        use crate::editor::{Editor, EditorConfig};
        use crate::schema::Schema;

        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        let table_id = editor.insert_table(3, 3).unwrap();
        assert_eq!(editor.tables[&table_id].col_count(), 3);

        cmd_add_column_after(&mut editor).unwrap();
        assert_eq!(editor.tables[&table_id].col_count(), 4);
    }

    #[test]
    fn toggle_header_command() {
        use crate::editor::{Editor, EditorConfig};
        use crate::schema::Schema;

        let mut editor = Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap();
        let table_id = editor.insert_table(3, 3).unwrap();
        // Table created with header via with_header
        assert_eq!(editor.tables[&table_id].header_rows, 1);

        cmd_toggle_header_row(&mut editor).unwrap();
        assert_eq!(editor.tables[&table_id].header_rows, 0);

        cmd_toggle_header_row(&mut editor).unwrap();
        assert_eq!(editor.tables[&table_id].header_rows, 1);
    }
}
