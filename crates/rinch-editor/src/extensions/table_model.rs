//! Table data model for representing tables in the document.

use crate::error::EditorError;
use std::collections::HashMap;

/// A table in the document model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableModel {
    /// Table rows.
    pub rows: Vec<TableRow>,
    /// Number of header rows (0 or 1 typically).
    pub header_rows: usize,
}

/// A row in a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    /// Cells in this row.
    pub cells: Vec<TableCell>,
}

/// A cell in a table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCell {
    /// Cell text content.
    pub content: String,
    /// Column span (default 1).
    pub colspan: usize,
    /// Row span (default 1).
    pub rowspan: usize,
    /// Whether this is a header cell (`<th>` vs `<td>`).
    pub is_header: bool,
    /// Additional attributes.
    pub attrs: HashMap<String, String>,
}

impl Default for TableCell {
    fn default() -> Self {
        Self {
            content: String::new(),
            colspan: 1,
            rowspan: 1,
            is_header: false,
            attrs: HashMap::new(),
        }
    }
}

impl TableCell {
    /// Create a new cell with content.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }

    /// Create a header cell with content.
    pub fn header(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_header: true,
            ..Default::default()
        }
    }
}

impl TableRow {
    /// Create a new row with the given number of empty cells.
    pub fn new(cols: usize) -> Self {
        Self {
            cells: (0..cols).map(|_| TableCell::default()).collect(),
        }
    }

    /// Create a header row with the given number of empty cells.
    pub fn header(cols: usize) -> Self {
        Self {
            cells: (0..cols)
                .map(|_| TableCell {
                    is_header: true,
                    ..Default::default()
                })
                .collect(),
        }
    }
}

impl TableModel {
    /// Create a new table with the given dimensions (no header row).
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows: (0..rows).map(|_| TableRow::new(cols)).collect(),
            header_rows: 0,
        }
    }

    /// Create a new table with a header row.
    pub fn with_header(rows: usize, cols: usize) -> Self {
        let mut table_rows = Vec::with_capacity(rows);
        if rows > 0 {
            table_rows.push(TableRow::header(cols));
            for _ in 1..rows {
                table_rows.push(TableRow::new(cols));
            }
        }
        Self {
            rows: table_rows,
            header_rows: if rows > 0 { 1 } else { 0 },
        }
    }

    /// Get the number of rows.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Get the number of columns (from the first row, or 0 if empty).
    pub fn col_count(&self) -> usize {
        self.rows.first().map_or(0, |r| r.cells.len())
    }

    /// Get a reference to a cell.
    pub fn get_cell(&self, row: usize, col: usize) -> Option<&TableCell> {
        self.rows.get(row).and_then(|r| r.cells.get(col))
    }

    /// Get a mutable reference to a cell.
    pub fn get_cell_mut(&mut self, row: usize, col: usize) -> Option<&mut TableCell> {
        self.rows.get_mut(row).and_then(|r| r.cells.get_mut(col))
    }

    /// Insert a new empty row at the given index.
    pub fn insert_row(&mut self, at: usize) {
        let cols = self.col_count();
        let at = at.min(self.rows.len());
        let is_header = at < self.header_rows;
        let row = if is_header {
            self.header_rows += 1;
            TableRow::header(cols)
        } else {
            TableRow::new(cols)
        };
        self.rows.insert(at, row);
    }

    /// Insert a new empty column at the given index.
    pub fn insert_column(&mut self, at: usize) {
        for (i, row) in self.rows.iter_mut().enumerate() {
            let at = at.min(row.cells.len());
            let is_header = i < self.header_rows;
            let cell = if is_header {
                TableCell {
                    is_header: true,
                    ..Default::default()
                }
            } else {
                TableCell::default()
            };
            row.cells.insert(at, cell);
        }
    }

    /// Delete a row. Returns error if it would leave the table empty.
    pub fn delete_row(&mut self, at: usize) -> Result<(), EditorError> {
        if self.rows.len() <= 1 {
            return Err(EditorError::CommandFailed(
                "Cannot delete the last row".into(),
            ));
        }
        if at >= self.rows.len() {
            return Err(EditorError::CommandFailed(format!(
                "Row index {} out of bounds ({})",
                at,
                self.rows.len()
            )));
        }
        if at < self.header_rows {
            self.header_rows -= 1;
        }
        self.rows.remove(at);
        Ok(())
    }

    /// Delete a column. Returns error if it would leave the table empty.
    pub fn delete_column(&mut self, at: usize) -> Result<(), EditorError> {
        let cols = self.col_count();
        if cols <= 1 {
            return Err(EditorError::CommandFailed(
                "Cannot delete the last column".into(),
            ));
        }
        if at >= cols {
            return Err(EditorError::CommandFailed(format!(
                "Column index {} out of bounds ({})",
                at, cols
            )));
        }
        for row in &mut self.rows {
            if at < row.cells.len() {
                row.cells.remove(at);
            }
        }
        Ok(())
    }

    /// Merge cells in a rectangular range `(from_row, from_col)` to `(to_row, to_col)` inclusive.
    /// The top-left cell absorbs content; spanned cells are cleared.
    pub fn merge_cells(
        &mut self,
        from: (usize, usize),
        to: (usize, usize),
    ) -> Result<(), EditorError> {
        let (r1, c1) = (from.0.min(to.0), from.1.min(to.1));
        let (r2, c2) = (from.0.max(to.0), from.1.max(to.1));

        if r2 >= self.row_count() || c2 >= self.col_count() {
            return Err(EditorError::CommandFailed(
                "Merge range out of bounds".into(),
            ));
        }
        if r1 == r2 && c1 == c2 {
            return Err(EditorError::CommandFailed(
                "Cannot merge a single cell".into(),
            ));
        }

        // Collect content from all cells in range
        let mut merged_content = String::new();
        for r in r1..=r2 {
            for c in c1..=c2 {
                let cell = &self.rows[r].cells[c];
                if !cell.content.is_empty() {
                    if !merged_content.is_empty() {
                        merged_content.push(' ');
                    }
                    merged_content.push_str(&cell.content);
                }
            }
        }

        // Set spans on top-left cell
        let rowspan = r2 - r1 + 1;
        let colspan = c2 - c1 + 1;
        self.rows[r1].cells[c1].content = merged_content;
        self.rows[r1].cells[c1].rowspan = rowspan;
        self.rows[r1].cells[c1].colspan = colspan;

        // Clear spanned cells
        for r in r1..=r2 {
            for c in c1..=c2 {
                if r == r1 && c == c1 {
                    continue;
                }
                self.rows[r].cells[c].content.clear();
                self.rows[r].cells[c].colspan = 1;
                self.rows[r].cells[c].rowspan = 1;
            }
        }

        Ok(())
    }

    /// Split a merged cell back into individual cells.
    pub fn split_cell(&mut self, row: usize, col: usize) -> Result<(), EditorError> {
        let cell = self
            .get_cell(row, col)
            .ok_or_else(|| EditorError::CommandFailed("Cell not found".into()))?;

        if cell.colspan == 1 && cell.rowspan == 1 {
            return Err(EditorError::CommandFailed("Cell is not merged".into()));
        }

        let colspan = cell.colspan;
        let rowspan = cell.rowspan;

        // Reset the merged cell's span
        self.rows[row].cells[col].colspan = 1;
        self.rows[row].cells[col].rowspan = 1;

        // The spanned cells are already present (cleared during merge); nothing else to do.
        // Just ensure they have default spans (they should already).
        for r in row..row + rowspan {
            for c in col..col + colspan {
                if r == row && c == col {
                    continue;
                }
                if let Some(cell) = self.get_cell_mut(r, c) {
                    cell.colspan = 1;
                    cell.rowspan = 1;
                }
            }
        }

        Ok(())
    }

    /// Toggle the first row as a header row.
    pub fn toggle_header_row(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        if self.header_rows > 0 {
            // Demote header row
            self.header_rows = 0;
            for cell in &mut self.rows[0].cells {
                cell.is_header = false;
            }
        } else {
            // Promote first row to header
            self.header_rows = 1;
            for cell in &mut self.rows[0].cells {
                cell.is_header = true;
            }
        }
    }

    /// Convert the table to an HTML string.
    pub fn to_html(&self) -> String {
        let mut html = String::from("<table>");

        for (i, row) in self.rows.iter().enumerate() {
            let in_header = i < self.header_rows;
            if in_header && i == 0 {
                html.push_str("<thead>");
            }
            if !in_header && (i == self.header_rows) {
                if self.header_rows > 0 {
                    html.push_str("</thead>");
                }
                html.push_str("<tbody>");
            }

            html.push_str("<tr>");
            for cell in &row.cells {
                let tag = if cell.is_header { "th" } else { "td" };
                html.push('<');
                html.push_str(tag);
                if cell.colspan > 1 {
                    html.push_str(&format!(" colspan=\"{}\"", cell.colspan));
                }
                if cell.rowspan > 1 {
                    html.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
                }
                for (k, v) in &cell.attrs {
                    html.push_str(&format!(" {}=\"{}\"", k, v));
                }
                html.push('>');
                html.push_str(&cell.content);
                html.push_str("</");
                html.push_str(tag);
                html.push('>');
            }
            html.push_str("</tr>");
        }

        // Close open sections
        if self.header_rows > 0 && self.rows.len() == self.header_rows {
            html.push_str("</thead>");
        } else if !self.rows.is_empty() {
            html.push_str("</tbody>");
        }

        html.push_str("</table>");
        html
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_correct_dimensions() {
        let t = TableModel::new(3, 4);
        assert_eq!(t.row_count(), 3);
        assert_eq!(t.col_count(), 4);
        assert_eq!(t.header_rows, 0);
    }

    #[test]
    fn with_header_creates_header_row() {
        let t = TableModel::with_header(3, 2);
        assert_eq!(t.row_count(), 3);
        assert_eq!(t.col_count(), 2);
        assert_eq!(t.header_rows, 1);
        assert!(t.get_cell(0, 0).unwrap().is_header);
        assert!(!t.get_cell(1, 0).unwrap().is_header);
    }

    #[test]
    fn get_cell_valid() {
        let t = TableModel::new(2, 2);
        assert!(t.get_cell(0, 0).is_some());
        assert!(t.get_cell(1, 1).is_some());
    }

    #[test]
    fn get_cell_out_of_bounds() {
        let t = TableModel::new(2, 2);
        assert!(t.get_cell(5, 0).is_none());
        assert!(t.get_cell(0, 5).is_none());
    }

    #[test]
    fn get_cell_mut_sets_content() {
        let mut t = TableModel::new(2, 2);
        t.get_cell_mut(0, 0).unwrap().content = "hello".into();
        assert_eq!(t.get_cell(0, 0).unwrap().content, "hello");
    }

    #[test]
    fn insert_row_at_end() {
        let mut t = TableModel::new(2, 3);
        t.insert_row(2);
        assert_eq!(t.row_count(), 3);
        assert_eq!(t.rows[2].cells.len(), 3);
    }

    #[test]
    fn insert_row_at_beginning() {
        let mut t = TableModel::new(2, 2);
        t.get_cell_mut(0, 0).unwrap().content = "original".into();
        t.insert_row(0);
        assert_eq!(t.row_count(), 3);
        assert_eq!(t.get_cell(1, 0).unwrap().content, "original");
    }

    #[test]
    fn insert_row_clamped() {
        let mut t = TableModel::new(2, 2);
        t.insert_row(100);
        assert_eq!(t.row_count(), 3);
    }

    #[test]
    fn insert_column_at_end() {
        let mut t = TableModel::new(2, 2);
        t.insert_column(2);
        assert_eq!(t.col_count(), 3);
        assert_eq!(t.rows[0].cells.len(), 3);
        assert_eq!(t.rows[1].cells.len(), 3);
    }

    #[test]
    fn insert_column_at_beginning() {
        let mut t = TableModel::new(2, 2);
        t.get_cell_mut(0, 0).unwrap().content = "first".into();
        t.insert_column(0);
        assert_eq!(t.col_count(), 3);
        assert_eq!(t.get_cell(0, 1).unwrap().content, "first");
    }

    #[test]
    fn delete_row_ok() {
        let mut t = TableModel::new(3, 2);
        t.get_cell_mut(1, 0).unwrap().content = "middle".into();
        t.delete_row(1).unwrap();
        assert_eq!(t.row_count(), 2);
    }

    #[test]
    fn delete_row_last_fails() {
        let mut t = TableModel::new(1, 2);
        assert!(t.delete_row(0).is_err());
    }

    #[test]
    fn delete_row_out_of_bounds() {
        let mut t = TableModel::new(2, 2);
        assert!(t.delete_row(5).is_err());
    }

    #[test]
    fn delete_column_ok() {
        let mut t = TableModel::new(2, 3);
        t.delete_column(1).unwrap();
        assert_eq!(t.col_count(), 2);
    }

    #[test]
    fn delete_column_last_fails() {
        let mut t = TableModel::new(2, 1);
        assert!(t.delete_column(0).is_err());
    }

    #[test]
    fn delete_column_out_of_bounds() {
        let mut t = TableModel::new(2, 2);
        assert!(t.delete_column(5).is_err());
    }

    #[test]
    fn merge_cells_basic() {
        let mut t = TableModel::new(2, 2);
        t.get_cell_mut(0, 0).unwrap().content = "A".into();
        t.get_cell_mut(0, 1).unwrap().content = "B".into();
        t.merge_cells((0, 0), (0, 1)).unwrap();
        let cell = t.get_cell(0, 0).unwrap();
        assert_eq!(cell.colspan, 2);
        assert_eq!(cell.rowspan, 1);
        assert_eq!(cell.content, "A B");
        assert!(t.get_cell(0, 1).unwrap().content.is_empty());
    }

    #[test]
    fn merge_cells_rectangular() {
        let mut t = TableModel::new(3, 3);
        t.get_cell_mut(0, 0).unwrap().content = "TL".into();
        t.merge_cells((0, 0), (1, 1)).unwrap();
        let cell = t.get_cell(0, 0).unwrap();
        assert_eq!(cell.colspan, 2);
        assert_eq!(cell.rowspan, 2);
    }

    #[test]
    fn merge_cells_single_fails() {
        let mut t = TableModel::new(2, 2);
        assert!(t.merge_cells((0, 0), (0, 0)).is_err());
    }

    #[test]
    fn merge_cells_out_of_bounds() {
        let mut t = TableModel::new(2, 2);
        assert!(t.merge_cells((0, 0), (5, 5)).is_err());
    }

    #[test]
    fn split_cell_basic() {
        let mut t = TableModel::new(2, 2);
        t.merge_cells((0, 0), (0, 1)).unwrap();
        assert_eq!(t.get_cell(0, 0).unwrap().colspan, 2);
        t.split_cell(0, 0).unwrap();
        assert_eq!(t.get_cell(0, 0).unwrap().colspan, 1);
        assert_eq!(t.get_cell(0, 0).unwrap().rowspan, 1);
    }

    #[test]
    fn split_cell_not_merged_fails() {
        let mut t = TableModel::new(2, 2);
        assert!(t.split_cell(0, 0).is_err());
    }

    #[test]
    fn split_cell_not_found_fails() {
        let mut t = TableModel::new(2, 2);
        assert!(t.split_cell(10, 10).is_err());
    }

    #[test]
    fn toggle_header_row_on() {
        let mut t = TableModel::new(2, 2);
        assert_eq!(t.header_rows, 0);
        t.toggle_header_row();
        assert_eq!(t.header_rows, 1);
        assert!(t.get_cell(0, 0).unwrap().is_header);
    }

    #[test]
    fn toggle_header_row_off() {
        let mut t = TableModel::with_header(2, 2);
        assert_eq!(t.header_rows, 1);
        t.toggle_header_row();
        assert_eq!(t.header_rows, 0);
        assert!(!t.get_cell(0, 0).unwrap().is_header);
    }

    #[test]
    fn toggle_header_row_empty_table() {
        let mut t = TableModel::new(0, 0);
        t.toggle_header_row(); // should not panic
        assert_eq!(t.header_rows, 0);
    }

    #[test]
    fn to_html_simple() {
        let mut t = TableModel::new(1, 2);
        t.get_cell_mut(0, 0).unwrap().content = "A".into();
        t.get_cell_mut(0, 1).unwrap().content = "B".into();
        let html = t.to_html();
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>A</td>"));
        assert!(html.contains("<td>B</td>"));
        assert!(html.contains("</table>"));
    }

    #[test]
    fn to_html_with_header() {
        let mut t = TableModel::with_header(2, 2);
        t.get_cell_mut(0, 0).unwrap().content = "H1".into();
        t.get_cell_mut(1, 0).unwrap().content = "D1".into();
        let html = t.to_html();
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>H1</th>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<td>D1</td>"));
    }

    #[test]
    fn to_html_with_spans() {
        let mut t = TableModel::new(2, 2);
        t.merge_cells((0, 0), (0, 1)).unwrap();
        let html = t.to_html();
        assert!(html.contains("colspan=\"2\""));
    }

    #[test]
    fn to_html_with_rowspan() {
        let mut t = TableModel::new(2, 2);
        t.merge_cells((0, 0), (1, 0)).unwrap();
        let html = t.to_html();
        assert!(html.contains("rowspan=\"2\""));
    }

    #[test]
    fn new_zero_dimensions() {
        let t = TableModel::new(0, 0);
        assert_eq!(t.row_count(), 0);
        assert_eq!(t.col_count(), 0);
    }

    #[test]
    fn with_header_single_row() {
        let t = TableModel::with_header(1, 3);
        assert_eq!(t.row_count(), 1);
        assert_eq!(t.header_rows, 1);
        assert!(t.get_cell(0, 0).unwrap().is_header);
    }

    #[test]
    fn delete_header_row_decrements_count() {
        let mut t = TableModel::with_header(3, 2);
        assert_eq!(t.header_rows, 1);
        t.delete_row(0).unwrap();
        assert_eq!(t.header_rows, 0);
    }

    #[test]
    fn insert_column_header_row_gets_header_cells() {
        let mut t = TableModel::with_header(2, 2);
        t.insert_column(1);
        assert!(t.get_cell(0, 1).unwrap().is_header);
        assert!(!t.get_cell(1, 1).unwrap().is_header);
    }

    #[test]
    fn cell_new_and_header_constructors() {
        let c = TableCell::new("hello");
        assert_eq!(c.content, "hello");
        assert!(!c.is_header);

        let h = TableCell::header("title");
        assert_eq!(h.content, "title");
        assert!(h.is_header);
    }

    #[test]
    fn merge_reversed_coords() {
        let mut t = TableModel::new(2, 2);
        t.get_cell_mut(1, 1).unwrap().content = "X".into();
        // from > to should still work (normalizes internally)
        t.merge_cells((1, 1), (0, 0)).unwrap();
        let cell = t.get_cell(0, 0).unwrap();
        assert_eq!(cell.colspan, 2);
        assert_eq!(cell.rowspan, 2);
    }
}
