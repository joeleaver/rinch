//! [`Transaction`] — a state-level edit: a [`Transform`] plus selection, stored
//! marks, and plugin metadata.
//!
//! A `Transaction` is what a command builds and a runtime applies
//! ([`EditorState::apply`](crate::state::EditorState::apply)). It extends the pure
//! [`Transform`] (steps + mapping) with the *editor* concerns the transform layer
//! deliberately omits: the [`Selection`] (mapped forward as steps are added),
//! `stored_marks` (the "click bold then type" tri-state), a `time` stamp (history
//! grouping), and a free-form `meta` map (e.g. `addToHistory=false`, the history
//! plugin's undo/redo marker).
//!
//! ## Ownership
//!
//! Unlike `Transform<'a>` (which borrows `&Schema`), a `Transaction` **owns** an
//! `Rc<Schema>` and has no lifetime parameter — so a [`Command`] can be a plain
//! `Fn(&EditorState, Option<&mut dyn FnMut(Transaction)>)` with no lifetime
//! gymnastics. Each gesture borrows a local `&Schema` out of the `Rc` for the
//! duration of the call ([`Transform::from_parts`]/`into_parts`), keeping the
//! gesture logic single-sourced in the transform layer.
//!
//! [`Command`]: crate::command::Command

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use crate::model::{AttrValue, Attrs, Fragment, Mark, NodeType};
use crate::model::{Node, Slice};
use crate::pos::Pos;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::tables::{self, TableProblem};
use crate::transform::Transform;
use crate::transform::node_range::NodeRange;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::Mapping;

/// Meta key: when set to `false`, the history plugin will not record this
/// transaction (used by undo/redo themselves).
pub const ADD_TO_HISTORY: &str = "addToHistory";

/// A single state-level edit. Build it from [`EditorState::tr`], mutate it with the
/// gesture/selection/mark methods, then hand it to
/// [`EditorState::apply`](crate::state::EditorState::apply).
///
/// [`EditorState::tr`]: crate::state::EditorState::tr
pub struct Transaction {
    schema: Rc<Schema>,
    // --- transform accumulation (mirrors `Transform`'s fields) ---
    doc: Node,
    steps: Vec<Box<dyn Step>>,
    docs: Vec<Node>,
    mapping: Mapping,
    // --- editor concerns ---
    cur_selection: Selection,
    /// The step index at which `cur_selection` was last set/mapped — `cur_selection`
    /// is mapped forward from here through any steps added since.
    cur_selection_for: usize,
    selection_set: bool,
    stored_marks: Option<Vec<Mark>>,
    stored_marks_set: bool,
    meta: HashMap<&'static str, Rc<dyn Any>>,
    time: u64,
}

impl Transaction {
    /// Construct a transaction from the pieces of an [`EditorState`]. Crate-internal:
    /// callers use [`EditorState::tr`](crate::state::EditorState::tr).
    pub(crate) fn new(
        schema: Rc<Schema>,
        doc: Node,
        selection: Selection,
        stored_marks: Option<Vec<Mark>>,
    ) -> Transaction {
        Transaction {
            schema,
            doc,
            steps: Vec::new(),
            docs: Vec::new(),
            mapping: Mapping::new(),
            cur_selection: selection,
            cur_selection_for: 0,
            selection_set: false,
            stored_marks,
            stored_marks_set: false,
            meta: HashMap::new(),
            time: 0,
        }
    }

    // -- accessors -------------------------------------------------------

    /// The current document (after all steps applied so far).
    pub fn doc(&self) -> &Node {
        &self.doc
    }

    /// The current selection (already mapped forward through every step added).
    pub fn selection(&self) -> Selection {
        self.cur_selection.clone()
    }

    /// The transaction's stored marks (the marks the next typed text will carry).
    pub fn stored_marks(&self) -> Option<&Vec<Mark>> {
        self.stored_marks.as_ref()
    }

    /// True if `set_selection` was called explicitly on this transaction.
    pub fn selection_set(&self) -> bool {
        self.selection_set
    }

    /// The accumulated steps.
    pub fn steps(&self) -> &[Box<dyn Step>] {
        &self.steps
    }

    /// `docs()[i]` is the document *before* step `i` — used to invert the steps.
    pub fn docs(&self) -> &[Node] {
        &self.docs
    }

    /// The accumulated position mapping.
    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    /// True if any step changed the document.
    pub fn doc_changed(&self) -> bool {
        !self.steps.is_empty()
    }

    /// The transaction's timestamp in milliseconds (history grouping). Defaults
    /// to `0`; `rinch-editor-view`'s `EditorHandle` stamps an unstamped one it
    /// dispatches with its clock (#940), and a builder may set its own via
    /// [`Transaction::set_time`] — on that same clock (see there).
    pub fn time(&self) -> u64 {
        self.time
    }

    // -- the transform bridge --------------------------------------------

    /// Run `f` against a real [`Transform`] borrowing a local `&Schema`, then take
    /// the accumulation back and fold the selection / stored marks forward.
    fn with_transform<R>(&mut self, f: impl FnOnce(&mut Transform) -> R) -> R {
        let schema = self.schema.clone();
        let mut tf = Transform::from_parts(
            &schema,
            self.doc.clone(),
            std::mem::take(&mut self.steps),
            std::mem::take(&mut self.docs),
            std::mem::take(&mut self.mapping),
        );
        let steps_before = tf.steps().len();
        let r = f(&mut tf);
        let added = tf.steps().len() - steps_before;
        let (doc, steps, docs, mapping) = tf.into_parts();
        self.doc = doc;
        self.steps = steps;
        self.docs = docs;
        self.mapping = mapping;
        if added > 0 {
            self.map_selection_to_end();
            // A content change consumes any pending stored marks (PM addStep).
            self.stored_marks = None;
            self.stored_marks_set = false;
        }
        r
    }

    /// Map `cur_selection` forward through every step added since it was last set.
    fn map_selection_to_end(&mut self) {
        let total = self.mapping.maps().len();
        if self.cur_selection_for < total {
            let sub = self.mapping.slice(self.cur_selection_for, total);
            self.cur_selection = self.cur_selection.map(&self.doc, &sub);
            self.cur_selection_for = total;
        }
    }

    /// Apply a raw step.
    pub fn step(&mut self, step: Box<dyn Step>) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.step(step).map(|_| ()))?;
        Ok(self)
    }

    // -- position-based gestures (delegated to `Transform`) --------------

    /// Replace `from..to` with `slice`.
    pub fn replace(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.replace(from, to, slice).map(|_| ()))?;
        Ok(self)
    }

    /// Replace `from..to` with `slice`, fitted to the range
    /// ([`Transform::replace_range`]). Returns the position right after the
    /// inserted content.
    pub fn replace_range(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<usize, StepError> {
        self.with_transform(|tf| tf.replace_range(from, to, slice))
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
        self.with_table_repair(from, to, |tr| tr.replace_selection_inner(from, to, slice))?;
        Ok(self)
    }

    fn replace_selection_inner(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
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
        let end = self.replace_range(from, to, slice)?;
        let near = Selection::near(&self.doc, Pos(end), if inline_end { -1 } else { 1 });
        self.set_selection(near);
        Ok(())
    }

    /// Replace `from..to` with `content`.
    pub fn replace_with(
        &mut self,
        from: usize,
        to: usize,
        content: Fragment,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.replace_with(from, to, content).map(|_| ()))?;
        Ok(self)
    }

    /// Delete `from..to`.
    pub fn delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.delete(from, to).map(|_| ()))?;
        Ok(self)
    }

    /// Split the node(s) at `pos`, `depth` levels deep.
    pub fn split(
        &mut self,
        pos: usize,
        depth: usize,
        types_after: Option<Vec<Option<(NodeType, Attrs)>>>,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.split(pos, depth, types_after).map(|_| ()))?;
        Ok(self)
    }

    /// Change the block type of every textblock overlapping `from..to`.
    pub fn set_block_type(
        &mut self,
        from: usize,
        to: usize,
        typ: NodeType,
        attrs: Attrs,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.set_block_type(from, to, typ, attrs).map(|_| ()))?;
        Ok(self)
    }

    /// Wrap the block range in the given nested wrappers (outermost first).
    pub fn wrap(
        &mut self,
        range: &NodeRange,
        wrappers: &[(NodeType, Attrs)],
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.wrap(range, wrappers).map(|_| ()))?;
        Ok(self)
    }

    /// Lift the block range out to depth `target`.
    pub fn lift(&mut self, range: &NodeRange, target: usize) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.lift(range, target).map(|_| ()))?;
        Ok(self)
    }

    /// Add `mark` across the inline range `from..to`.
    pub fn add_mark(&mut self, from: usize, to: usize, mark: Mark) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.add_mark(from, to, mark).map(|_| ()))?;
        Ok(self)
    }

    /// Remove `mark` across the inline range `from..to`.
    pub fn remove_mark(
        &mut self,
        from: usize,
        to: usize,
        mark: Mark,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.remove_mark(from, to, mark).map(|_| ()))?;
        Ok(self)
    }

    /// Apply disjoint `edits`, stated in the current document's coordinates,
    /// as one [`BatchStep`](crate::transform::BatchStep): one step however many
    /// edits (#1200). Nothing is recorded for an empty list.
    pub fn batch(
        &mut self,
        edits: Vec<crate::transform::BatchEdit>,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.batch(edits).map(|_| ()))?;
        Ok(self)
    }

    /// Set the attribute `attr` of the node at `pos`.
    pub fn set_node_attr(
        &mut self,
        pos: usize,
        attr: impl Into<String>,
        value: AttrValue,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.set_node_attr(pos, attr, value).map(|_| ()))?;
        Ok(self)
    }

    /// Set a document-level attribute.
    pub fn set_doc_attr(
        &mut self,
        attr: impl Into<String>,
        value: AttrValue,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.set_doc_attr(attr, value).map(|_| ()))?;
        Ok(self)
    }

    // -- selection-aware gestures ----------------------------------------

    /// Type `text` at the current selection: replace the selected range with a text
    /// node carrying the transaction's `stored_marks` (or the marks inherited at the
    /// insertion point), then place the cursor after it. The typing path.
    pub fn insert_text(&mut self, text: &str) -> Result<&mut Self, StepError> {
        if text.is_empty() {
            return Ok(self);
        }
        let (from, to) = self.selection_range()?;
        self.with_table_repair(from, to, |tr| tr.insert_text_inner(from, to, text))?;
        Ok(self)
    }

    fn insert_text_inner(&mut self, from: usize, to: usize, text: &str) -> Result<(), StepError> {
        let marks = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.marks_at(from));
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
        self.with_table_repair(from, to, |tr| {
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
        self.with_table_repair(from, to, |tr| tr.insert_block_inner(from, to, node))?;
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
        self.with_table_repair(from, to, |tr| {
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
    fn selection_range(&self) -> Result<(usize, usize), StepError> {
        if matches!(self.cur_selection, Selection::Cell(_)) {
            return Err(StepError::new("a cell selection is not a range to replace"));
        }
        Ok((self.cur_selection.from().0, self.cur_selection.to().0))
    }

    /// Run the selection gesture `f`, which replaces `from..to`, then repair
    /// every table the gesture left ragged that was not ragged before it
    /// (#1847): ProseMirror's `fixTables`, scoped to this edit.
    ///
    /// A range whose ends are in different tables, or with one end outside
    /// every table — a drag from a cell out of the table — has no cell
    /// selection to stand for it ([`Selection::normalized`] makes one only
    /// for two cells of one table), so the fitted replace can take cells out
    /// of a row or rows out from under a rowspan. The repair puts the grid
    /// back: a rowspan past the last row or a span into another cell's slot
    /// is cut, then each short row gets empty cells at its end.
    ///
    /// Only tables this gesture made ragged are touched: a table that was
    /// already ragged (one loaded from HTML, drawn as a browser draws it)
    /// stays as it is. A collapsed range takes nothing out, so it skips the
    /// check.
    fn with_table_repair(
        &mut self,
        from: usize,
        to: usize,
        f: impl FnOnce(&mut Self) -> Result<(), StepError>,
    ) -> Result<(), StepError> {
        if from == to {
            return f(self);
        }
        // One position either side: a table that starts or ends right at
        // the range is joined into by the fitted replace too.
        let lo = from.saturating_sub(1);
        let hi = (to + 1).min(self.doc.content_size());
        let before = ragged_tables_between(&self.doc, lo, hi);
        let first = self.mapping.maps().len();
        f(self)?;
        let total = self.mapping.maps().len();
        if total == first {
            return Ok(());
        }
        let sub = self.mapping.slice(first, total);
        let was: Vec<usize> = before
            .iter()
            .flat_map(|&p| [sub.map(p, -1), sub.map(p, 1)])
            .collect();
        let end = self.doc.content_size();
        let (lo, hi) = (sub.map(lo, -1).min(end), sub.map(hi, 1).min(end));
        let mut made = ragged_tables_between(&self.doc, lo, hi);
        made.retain(|p| !was.contains(p));
        // Innermost (later) tables first: a repair only adds or changes
        // things inside its own table, so the positions before it hold.
        for pos in made.into_iter().rev() {
            self.repair_table(pos)?;
        }
        Ok(())
    }

    /// Make the table at `table_pos` tile its grid ([`tables::table_problems`]):
    /// cut each colliding or overlong span to what fits, then give each short
    /// row empty cells at its end. Port of prosemirror-tables' `fixTable`, which
    /// cuts a colliding cell's colspan and pads the rows; a collision below a
    /// cell's first row cuts its rowspan instead.
    fn repair_table(&mut self, table_pos: usize) -> Result<(), StepError> {
        // A cut can uncover slots, which the next round pads; three rounds
        // settle any table an edit can make (the seeded #1847 check).
        for _ in 0..3 {
            let Some(table) = self.doc.node_at(table_pos).filter(tables::is_table) else {
                return Ok(());
            };
            let problems = tables::table_problems(&table);
            if problems.is_empty() {
                return Ok(());
            }
            let start = table_pos + 1;
            let mut cut = false;
            for problem in &problems {
                let (pos, attr, value) = match *problem {
                    TableProblem::OverlongRowspan { pos, rows_fit } => (pos, "rowspan", rows_fit),
                    TableProblem::Collision { pos, rows_fit, .. } if rows_fit > 0 => {
                        (pos, "rowspan", rows_fit)
                    }
                    TableProblem::Collision { pos, cols_fit, .. } if cols_fit > 0 => {
                        (pos, "colspan", cols_fit)
                    }
                    _ => continue,
                };
                self.set_node_attr(start + pos, attr, AttrValue::Int(value as i64))?;
                cut = true;
            }
            if cut {
                continue;
            }
            // Pad the short rows, the last first so the earlier rows'
            // positions hold.
            let mut row_ends = Vec::with_capacity(table.child_count());
            let mut pos = start;
            for r in 0..table.child_count() {
                pos += table.child(r).node_size();
                row_ends.push(pos - 1);
            }
            for problem in problems.iter().rev() {
                let TableProblem::Missing { row, n } = *problem else {
                    continue;
                };
                let header = table
                    .child(row)
                    .content()
                    .children()
                    .first()
                    .is_some_and(|c| c.type_name() == "table_header_cell");
                let cell = self.empty_cell(header)?;
                let cells = Fragment::from_children(vec![cell; n]);
                self.replace_with(row_ends[row], row_ends[row], cells)?;
            }
        }
        Ok(())
    }

    /// An empty cell (one empty paragraph), a header cell when `header`.
    fn empty_cell(&self, header: bool) -> Result<Node, StepError> {
        let err = |e: crate::error::EditorError| StepError::new(e.to_string());
        let para = self
            .schema
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .map_err(err)?;
        let name = if header {
            "table_header_cell"
        } else {
            "table_cell"
        };
        self.schema
            .create_node(name, Attrs::new(), Fragment::from_node(para))
            .map_err(err)
    }

    /// The marks active at document position `pos` (in this transaction's current
    /// doc) — what a character typed there would inherit.
    fn marks_at(&self, pos: usize) -> Vec<Mark> {
        self.doc
            .resolve(Pos(pos))
            .map(|r| r.marks())
            .unwrap_or_default()
    }

    // -- selection / stored marks / meta ---------------------------------

    /// Set the selection explicitly. Clears any pending stored marks (moving the
    /// caret resets the "click bold then type" state, PM `setSelection`).
    pub fn set_selection(&mut self, selection: Selection) -> &mut Self {
        self.cur_selection = selection;
        self.cur_selection_for = self.steps.len();
        self.selection_set = true;
        self.stored_marks = None;
        self.stored_marks_set = false;
        self
    }

    /// Set the stored marks (`None` = inherit on next insert; `Some(vec![])` =
    /// explicitly no marks).
    pub fn set_stored_marks(&mut self, marks: Option<Vec<Mark>>) -> &mut Self {
        self.stored_marks = marks;
        self.stored_marks_set = true;
        self
    }

    /// Add `mark` to the stored-mark set (seeded from the marks at the cursor when
    /// no stored marks are pending).
    pub fn add_stored_mark(&mut self, mark: Mark) -> &mut Self {
        let base = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.marks_at(self.cur_selection.from().0));
        let next = mark.add_to_set(&base);
        self.set_stored_marks(Some(next))
    }

    /// Remove `mark` from the stored-mark set.
    pub fn remove_stored_mark(&mut self, mark: &Mark) -> &mut Self {
        let base = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.marks_at(self.cur_selection.from().0));
        let next = mark.remove_from_set(&base);
        self.set_stored_marks(Some(next))
    }

    /// True if [`Transaction::set_stored_marks`] (or a derived stored-mark mutation)
    /// was called on this transaction.
    pub fn stored_marks_set(&self) -> bool {
        self.stored_marks_set
    }

    /// Set the transaction timestamp (history grouping window), in milliseconds.
    ///
    /// The history compares it with the times of the transactions around it, so
    /// for a transaction an `EditorHandle` dispatches it must be on the handle's
    /// clock (`rinch_editor_view::history_now_ms()`, milliseconds since the
    /// thread's first stamp). A time on another clock — epoch milliseconds, say —
    /// is far ahead of the handle's stamps, so the keystroke after it can never
    /// merge with it.
    pub fn set_time(&mut self, time: u64) -> &mut Self {
        self.time = time;
        self
    }

    /// Attach plugin metadata under `key`.
    pub fn set_meta(&mut self, key: &'static str, value: impl Any + 'static) -> &mut Self {
        self.meta.insert(key, Rc::new(value));
        self
    }

    /// Read plugin metadata at `key`.
    pub fn get_meta(&self, key: &str) -> Option<&dyn Any> {
        self.meta.get(key).map(|r| r.as_ref() as &dyn Any)
    }

    /// Convenience: typed read of a `Copy` metadata value.
    pub fn get_meta_as<T: Copy + 'static>(&self, key: &str) -> Option<T> {
        self.get_meta(key)
            .and_then(|a| a.downcast_ref::<T>())
            .copied()
    }

    /// Set `addToHistory` (false suppresses history recording, e.g. undo/redo).
    pub fn set_add_to_history(&mut self, value: bool) -> &mut Self {
        self.set_meta(ADD_TO_HISTORY, value)
    }

    /// Whether this transaction should be recorded by the history plugin (default
    /// `true`).
    pub fn add_to_history(&self) -> bool {
        self.get_meta_as::<bool>(ADD_TO_HISTORY).unwrap_or(true)
    }
}

/// The positions of the tables overlapping `from..to` whose cells do not tile
/// their grid ([`tables::table_problems`]), in document order.
fn ragged_tables_between(doc: &Node, from: usize, to: usize) -> Vec<usize> {
    let mut out = Vec::new();
    doc.nodes_between(from, to, &mut |node, pos, _| {
        if tables::is_table(node) && !tables::table_problems(node).is_empty() {
            out.push(pos);
        }
        !node.is_textblock()
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;

    fn sk() -> Rc<Schema> {
        Rc::new(Schema::starter_kit())
    }
    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }
    fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }
    fn all_text(node: &Node) -> String {
        if let Some(t) = node.text() {
            return t.to_string();
        }
        node.content().iter().map(all_text).collect()
    }

    #[test]
    fn insert_text_advances_cursor_and_carries_no_marks_by_default() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), None);
        tr.insert_text("X").unwrap();
        assert_eq!(all_text(tr.doc()), "aXb");
        // cursor was at 2 (between a,b); after inserting one char → 3
        assert_eq!(tr.selection(), Selection::cursor(Pos(3)));
        assert!(tr.doc_changed());
    }

    #[test]
    fn insert_text_consumes_stored_marks() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), Some(vec![bold]));
        tr.insert_text("X").unwrap();
        // the inserted "X" carries bold; stored marks were consumed
        let p = tr.doc().child(0);
        let bolded_x = p
            .content()
            .iter()
            .any(|n| n.text() == Some("X") && n.marks().iter().any(|m| m.type_name() == "bold"));
        assert!(bolded_x, "inserted text should carry the stored bold mark");
        assert!(
            tr.stored_marks().is_none(),
            "stored marks consumed by the insert"
        );
    }

    /// `"see "` + `mark("here")` + `" now"`, typed into at `pos`: the runs of the
    /// result as `(text, mark names)`.
    fn type_into_marked(mark: &str, pos: usize) -> Vec<(String, Vec<String>)> {
        let s = sk();
        let mt = s.mark_type(mark).unwrap().clone();
        let m = if mark == "link" {
            Mark::new(
                mt,
                crate::Attrs::from_iter([("href", crate::AttrValue::from("https://a.example"))]),
            )
        } else {
            Mark::simple(mt)
        };
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text("see ").unwrap(),
                    s.text_with_marks("here", vec![m]).unwrap(),
                    s.text(" now").unwrap(),
                ]),
            )
            .unwrap();
        let d = doc(&s, vec![p]);
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(pos)), None);
        tr.insert_text("X").unwrap();
        tr.doc()
            .child(0)
            .content()
            .iter()
            .map(|n| {
                let names = n.marks().iter().map(|m| m.type_name().to_string());
                (n.text().unwrap().to_string(), names.collect())
            })
            .collect()
    }

    fn runs(r: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        r.iter()
            .map(|(t, m)| (t.to_string(), m.iter().map(|m| m.to_string()).collect()))
            .collect()
    }

    #[test]
    fn typing_right_after_a_link_is_not_linked() {
        assert_eq!(
            type_into_marked("link", 9),
            runs(&[("see ", &[]), ("here", &["link"]), ("X now", &[])])
        );
    }

    #[test]
    fn typing_inside_a_link_is_linked() {
        assert_eq!(
            type_into_marked("link", 7),
            runs(&[("see ", &[]), ("heXre", &["link"]), (" now", &[])])
        );
    }

    #[test]
    fn typing_at_a_link_start_is_not_linked() {
        // As for every mark: at a boundary the text before decides.
        assert_eq!(
            type_into_marked("link", 5),
            runs(&[("see X", &[]), ("here", &["link"]), (" now", &[])])
        );
    }

    #[test]
    fn typing_right_after_bold_is_bold() {
        assert_eq!(
            type_into_marked("bold", 9),
            runs(&[("see ", &[]), ("hereX", &["bold"]), (" now", &[])])
        );
    }

    #[test]
    fn selection_maps_forward_through_a_leading_insert() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        // cursor at the end of "ab" (pos 3)
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(3)), None);
        // insert "YY" at the start of the text (pos 1) via a position gesture
        let yy = s.text("YY").unwrap();
        tr.replace_with(1, 1, Fragment::from_node(yy)).unwrap();
        assert_eq!(all_text(tr.doc()), "YYab");
        // the cursor at 3 shifts by +2 → 5
        assert_eq!(tr.selection(), Selection::cursor(Pos(5)));
    }

    #[test]
    fn set_selection_clears_stored_marks() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "ab")]);
        let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), None);
        tr.set_stored_marks(Some(vec![bold]));
        assert!(tr.stored_marks().is_some());
        tr.set_selection(Selection::cursor(Pos(1)));
        assert!(tr.stored_marks().is_none());
    }

    #[test]
    fn delete_selection_collapses_cursor() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "abcd")]);
        let mut tr = Transaction::new(s.clone(), d, Selection::text(Pos(2), Pos(4)), None);
        tr.delete_selection().unwrap();
        assert_eq!(all_text(tr.doc()), "ad");
        assert_eq!(tr.selection(), Selection::cursor(Pos(2)));
    }

    #[test]
    fn meta_and_add_to_history() {
        let s = sk();
        let d = doc(&s, vec![para(&s, "x")]);
        let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(1)), None);
        assert!(tr.add_to_history());
        tr.set_add_to_history(false);
        assert!(!tr.add_to_history());
        tr.set_meta("origin", "remote".to_string());
        assert_eq!(
            tr.get_meta("origin")
                .and_then(|a| a.downcast_ref::<String>()),
            Some(&"remote".to_string())
        );
    }
}
