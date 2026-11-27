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
//! duration of the call (`Transform::from_parts`/`into_parts`), keeping the
//! gesture logic single-sourced in the transform layer.
//!
//! [`Command`]: crate::command::Command

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use crate::model::{AttrValue, Attrs, Fragment, Mark, NodeType};
use crate::model::{Node, Slice};
use crate::pos::{Pos, ResolvedPos};
use crate::schema::{DataAttrSplit, Schema};
use crate::selection::Selection;
use crate::tables::{self, TableProblem};
use crate::transform::Transform;
use crate::transform::data_attrs::Spot;
use crate::transform::node_range::NodeRange;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};

/// Meta key: when set to `false`, the history plugin will not record this
/// transaction (used by undo/redo themselves).
pub const ADD_TO_HISTORY: &str = "addToHistory";

/// A single state-level edit. Build it from [`EditorState::tr`], mutate it with the
/// gesture/selection/mark methods, then hand it to
/// [`EditorState::apply`](crate::state::EditorState::apply).
///
/// [`EditorState::tr`]: crate::state::EditorState::tr
pub struct Transaction {
    /// The state's choices for app data attributes at a split
    /// ([`Plugin::data_attr_splits`](crate::plugin::Plugin::data_attr_splits)).
    pub(crate) data_attr_splits: Option<Rc<HashMap<String, DataAttrSplit>>>,
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
    /// Construct a transaction from the pieces of an `EditorState`. Crate-internal:
    /// callers use [`EditorState::tr`](crate::state::EditorState::tr).
    pub(crate) fn new(
        schema: Rc<Schema>,
        doc: Node,
        selection: Selection,
        stored_marks: Option<Vec<Mark>>,
    ) -> Transaction {
        Transaction {
            data_attr_splits: None,
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
        tf.data_attr_splits = self.data_attr_splits.clone();
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
        self.delete_at(from, to, Spot::Unknown)
    }

    /// [`delete`](Self::delete), the caller saying what it knows of the range.
    pub(crate) fn delete_at(
        &mut self,
        from: usize,
        to: usize,
        spot: Spot<'_>,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.delete_at(from, to, spot).map(|_| ()))?;
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

    /// Set or remove an app data attribute on the textblocks overlapping
    /// `from..to`
    /// ([`Transform::set_block_data_attr`](crate::transform::Transform::set_block_data_attr)).
    pub fn set_block_data_attr(
        &mut self,
        from: usize,
        to: usize,
        name: &str,
        value: Option<&str>,
    ) -> Result<&mut Self, StepError> {
        self.with_transform(|tf| tf.set_block_data_attr(from, to, name, value).map(|_| ()))?;
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

mod gestures;
mod table_repair;
#[cfg(test)]
mod tests;

pub use table_repair::__set_full_repair_scan;
use table_repair::holds_table;
pub(crate) use table_repair::repair_every_undo;
