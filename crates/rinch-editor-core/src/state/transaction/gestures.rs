//! The selection-aware gestures: typing, replacing or deleting the selection,
//! inserting a node or a block.

use super::*;

impl Transaction {
    // -- selection-aware gestures ----------------------------------------

    /// Type `text` at the current selection: replace the selected range with a text
    /// node carrying the transaction's `stored_marks` (or the marks inherited at the
    /// insertion point), then place the cursor after it. The typing path.
    pub fn insert_text(&mut self, text: &str) -> Result<&mut Self, StepError> {
        if text.is_empty() {
            return Ok(self);
        }
        let (from, to) = self.selection_range()?;
        // One resolve of `from`, for the inherited marks and for the table
        // check alike: resolving walks every row before it in a table.
        let r = self
            .doc
            .resolve(Pos(from))
            .map_err(|e| StepError::new(e.to_string()))?;
        let marks = self.stored_marks.clone().unwrap_or_else(|| r.marks());
        self.with_table_repair(from, to, false, Some(r), |tr| {
            tr.insert_text_inner(from, to, text, marks)
        })?;
        Ok(self)
    }

    fn insert_text_inner(
        &mut self,
        from: usize,
        to: usize,
        text: &str,
        marks: Vec<Mark>,
    ) -> Result<(), StepError> {
        let node = self
            .schema
            .text_with_marks(text, marks)
            .map_err(|e| StepError::new(e.to_string()))?;
        let len = node.text_len();
        self.replace_with(from, to, Fragment::from_node(node))?;
        self.set_selection(Selection::cursor(Pos(from + len)));
        Ok(())
    }

    /// Replace the current selection with a single node (e.g. a hard break, image,
    /// or horizontal rule), placing the cursor after it.
    pub fn replace_selection_with(&mut self, node: Node) -> Result<&mut Self, StepError> {
        let (from, to) = self.selection_range()?;
        let tables = holds_table(&node);
        self.with_table_repair(from, to, tables, None, |tr| {
            let size = node.node_size();
            tr.replace_with(from, to, Fragment::from_node(node))?;
            let after = Selection::near(&tr.doc, Pos(from + size), 1);
            tr.set_selection(after);
            Ok(())
        })?;
        Ok(self)
    }

    /// Insert a **block-level** `node` (horizontal rule, table) at the current
    /// selection, opening a block gap for it: delete any selected range, then place
    /// the node before or after the enclosing textblock — splitting the textblock when
    /// the cursor is in its interior. Unlike [`Self::replace_selection_with`] (which
    /// drops the node *at* the selection, so a block node only fits when the selection
    /// already spans a doc-level block gap), this handles the common case of a collapsed
    /// cursor inside a paragraph. Leaves the cursor just after the inserted node.
    pub fn insert_block(&mut self, node: Node) -> Result<&mut Self, StepError> {
        let (from, to) = self.selection_range()?;
        let tables = holds_table(&node);
        self.with_table_repair(from, to, tables, None, |tr| {
            tr.insert_block_inner(from, to, node)
        })?;
        Ok(self)
    }

    fn insert_block_inner(&mut self, from: usize, to: usize, node: Node) -> Result<(), StepError> {
        if from != to {
            self.delete(from, to)?;
        }
        let size = node.node_size();
        // `from` is the (post-delete) collapsed cursor — the deletion start is invariant
        // under its own delete, so it stays valid.
        let r = self
            .doc
            .resolve(Pos(from))
            .map_err(|e| StepError::new(e.to_string()))?;
        let gap = if r.depth() < 1 || !r.parent().is_textblock() {
            // Already a gap between blocks (no enclosing textblock): at the doc
            // level, or inside a container — never split the container (a table
            // cell split there left a ragged row, #1232).
            from
        } else if r.parent_offset() == 0 {
            // Start of the textblock — insert directly before it (no split).
            r.before(r.depth())
                .ok_or_else(|| StepError::new("no position before block"))?
        } else if r.parent_offset() == r.parent().content().size() {
            // End of the textblock — insert directly after it (no split).
            r.after(r.depth())
                .ok_or_else(|| StepError::new("no position after block"))?
        } else {
            // Interior — split the textblock; the gap is just before the second half.
            self.split(from, 1, None)?;
            let mapped = self.mapping().map(from, 1);
            let r2 = self
                .doc
                .resolve(Pos(mapped))
                .map_err(|e| StepError::new(e.to_string()))?;
            r2.before(r2.depth())
                .ok_or_else(|| StepError::new("no position at split boundary"))?
        };
        self.replace_with(gap, gap, Fragment::from_node(node))?;
        self.set_selection(Selection::near(&self.doc, Pos(gap + size), 1));
        Ok(())
    }

    /// Delete the current (non-empty) selection. A no-op for a collapsed cursor.
    pub fn delete_selection(&mut self) -> Result<&mut Self, StepError> {
        let (from, to) = self.selection_range()?;
        if from == to {
            return Ok(self);
        }
        self.with_table_repair(from, to, false, None, |tr| {
            tr.delete(from, to)?;
            // A fitted delete whose tail could not join the textblock it began
            // in (code, then marked text) maps the range's end past `from`: the
            // caret still goes where the range began (ProseMirror's
            // `selectionToInsertionEnd`).
            if !tr.cur_selection.is_empty() {
                let caret = Selection::near(&tr.doc, Pos(from), -1);
                tr.set_selection(caret);
            }
            Ok(())
        })?;
        Ok(self)
    }

    /// The selection's `from..to`, or an error for a **cell selection**,
    /// whose `from()..to()` is a coarse bound (the positions before its two
    /// corner cells), not a range of content: replacing it would take cells
    /// out of their rows. Clear the cells and collapse into one first
    /// ([`clear_cells`](crate::commands::table_ops::clear_cells)).
    pub(super) fn selection_range(&self) -> Result<(usize, usize), StepError> {
        if matches!(self.cur_selection, Selection::Cell(_)) {
            return Err(StepError::new("a cell selection is not a range to replace"));
        }
        Ok((self.cur_selection.from().0, self.cur_selection.to().0))
    }

    /// Replace the selection with `slice`, fitted
    /// ([`Transform::replace_range`]), and put the caret after the inserted
    /// content: ProseMirror's `replaceSelection`, the default paste.
    ///
    /// A **cell selection** is refused: its `from()..to()` is a coarse bound
    /// (the positions before its two corner cells), not a range of content,
    /// and replacing it would take cells out of their rows. Clear the cells
    /// and collapse into one first
    /// ([`clear_cells`](crate::commands::table_ops::clear_cells)), as the
    /// paste does.
    pub fn replace_selection(&mut self, slice: Slice) -> Result<&mut Self, StepError> {
        let (from, to) = self.selection_range()?;
        let tables = slice.content.children().iter().any(holds_table);
        self.with_table_repair(from, to, tables, None, |tr| {
            tr.replace_selection_inner(from, to, slice, false)
        })?;
        Ok(self)
    }

    /// [`replace_selection`](Self::replace_selection) with pasted lines of plain
    /// text: the default plain-text paste. An app's data attributes on the
    /// block the lines split follow the text, as at Enter
    /// ([`Transform::replace_range_with_lines`]).
    pub fn replace_selection_with_lines(&mut self, slice: Slice) -> Result<&mut Self, StepError> {
        let (from, to) = self.selection_range()?;
        self.with_table_repair(from, to, false, None, |tr| {
            tr.replace_selection_inner(from, to, slice, true)
        })?;
        Ok(self)
    }

    fn replace_selection_inner(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
        lines: bool,
    ) -> Result<(), StepError> {
        // Whether the slice ends in inline content: the caret then stays in
        // it rather than moving on to what follows.
        let mut last = slice.content.children().last().cloned();
        let mut last_parent: Option<Node> = None;
        for _ in 0..slice.open_end {
            let Some(node) = last else { break };
            last = node.content().children().last().cloned();
            last_parent = Some(node);
        }
        let inline_end = match (&last, &last_parent) {
            (Some(node), _) => node.is_inline(),
            (None, Some(parent)) => parent.is_textblock(),
            (None, None) => false,
        };
        let end = if lines {
            self.with_transform(|tf| tf.replace_range_with_lines(from, to, slice))?
        } else {
            self.replace_range(from, to, slice)?
        };
        let near = Selection::near(&self.doc, Pos(end), if inline_end { -1 } else { 1 });
        self.set_selection(near);
        Ok(())
    }

    /// The marks active at document position `pos` (in this transaction's current
    /// doc) — what a character typed there would inherit.
    pub(super) fn marks_at(&self, pos: usize) -> Vec<Mark> {
        self.doc
            .resolve(Pos(pos))
            .map(|r| r.marks())
            .unwrap_or_default()
    }
}
