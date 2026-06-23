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

    /// The transaction's timestamp (history grouping). Defaults to `0`; the
    /// platform sets it via [`Transaction::set_time`].
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
        let from = self.cur_selection.from().0;
        let to = self.cur_selection.to().0;
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
        Ok(self)
    }

    /// Replace the current selection with a single node (e.g. a hard break, image,
    /// or horizontal rule), placing the cursor after it.
    pub fn replace_selection_with(&mut self, node: Node) -> Result<&mut Self, StepError> {
        let from = self.cur_selection.from().0;
        let to = self.cur_selection.to().0;
        let size = node.node_size();
        self.replace_with(from, to, Fragment::from_node(node))?;
        let after = Selection::near(&self.doc, Pos(from + size), 1);
        self.set_selection(after);
        Ok(self)
    }

    /// Delete the current (non-empty) selection. A no-op for a collapsed cursor.
    pub fn delete_selection(&mut self) -> Result<&mut Self, StepError> {
        let from = self.cur_selection.from().0;
        let to = self.cur_selection.to().0;
        if from == to {
            return Ok(self);
        }
        self.delete(from, to)?;
        Ok(self)
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

    /// Set the transaction timestamp (history grouping window).
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
