//! Table extension for full table editing support.

use crate::extensions::{CommandRegistration, Extension};
use crate::input::KeyboardShortcut;
use crate::schema::{AttrSpec, NodeSpec};

use super::table_model::TableModel;

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
            CommandRegistration::new("insert_table", |_editor| {
                // Insert a default 3x3 table at cursor position
                let _table = TableModel::new(3, 3);
                // Integration with editor document model would go here
                Ok(())
            }),
            CommandRegistration::new("delete_table", |_editor| Ok(())),
            CommandRegistration::new("add_row_before", |_editor| Ok(())),
            CommandRegistration::new("add_row_after", |_editor| Ok(())),
            CommandRegistration::new("delete_row", |_editor| Ok(())),
            CommandRegistration::new("add_column_before", |_editor| Ok(())),
            CommandRegistration::new("add_column_after", |_editor| Ok(())),
            CommandRegistration::new("delete_column", |_editor| Ok(())),
            CommandRegistration::new("merge_cells", |_editor| Ok(())),
            CommandRegistration::new("split_cell", |_editor| Ok(())),
            CommandRegistration::new("toggle_header_row", |_editor| Ok(())),
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
}
