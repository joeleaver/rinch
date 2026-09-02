//! [`Transform`] — the builder that turns editing gestures into [`Step`]s.
//!
//! A `Transform` starts from a document and accumulates steps; each gesture
//! helper (`insert_text`, `delete`, `split`, `set_block_type`, `add_mark`, …)
//! appends one or more steps, keeping a [`Mapping`] of every position shift and a
//! per-step snapshot of the document for inversion. This is ProseMirror's
//! `Transform`; the state-layer `Transaction` (selection, stored marks, plugin
//! meta) extends it in M4.
//!
//! Helpers return `Result<&mut Self, StepError>` so failures (schema-invalid
//! steps) surface and chain with `?` — `tr.insert_text(..)?.delete(..)?`.

use crate::Slice;
use crate::model::{AttrValue, Attrs, Fragment, Mark, Node, NodeType};
use crate::pos::Pos;
use crate::schema::Schema;
use crate::transform::node_range::NodeRange;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::Mapping;
use crate::transform::steps::{
    AddMarkStep, RemoveMarkStep, ReplaceAroundStep, ReplaceStep, SetDocAttrStep, SetNodeAttrStep,
};

/// An abstraction over a document that accumulates [`Step`]s.
pub struct Transform<'a> {
    schema: &'a Schema,
    /// The current document (after all steps applied so far).
    pub doc: Node,
    steps: Vec<Box<dyn Step>>,
    docs: Vec<Node>,
    mapping: Mapping,
}

impl<'a> Transform<'a> {
    /// Start a transform from `doc` (which must belong to `schema`).
    pub fn new(schema: &'a Schema, doc: Node) -> Transform<'a> {
        Transform {
            schema,
            doc,
            steps: Vec::new(),
            docs: Vec::new(),
            mapping: Mapping::new(),
        }
    }

    /// Reconstitute a transform from previously-extracted parts. Paired with
    /// [`Transform::into_parts`] so the state-layer
    /// [`Transaction`](crate::state::Transaction) — which owns an `Rc<Schema>`
    /// rather than a borrow — can borrow a local `&Schema` for the duration of one
    /// gesture, run it through the shared gesture logic here, then take the
    /// accumulation back. This avoids duplicating every gesture on `Transaction`.
    pub(crate) fn from_parts(
        schema: &'a Schema,
        doc: Node,
        steps: Vec<Box<dyn Step>>,
        docs: Vec<Node>,
        mapping: Mapping,
    ) -> Transform<'a> {
        Transform {
            schema,
            doc,
            steps,
            docs,
            mapping,
        }
    }

    /// Extract the accumulated state (the inverse of [`Transform::from_parts`]).
    pub(crate) fn into_parts(self) -> (Node, Vec<Box<dyn Step>>, Vec<Node>, Mapping) {
        (self.doc, self.steps, self.docs, self.mapping)
    }

    /// The document before any step was applied.
    pub fn before(&self) -> &Node {
        self.docs.first().unwrap_or(&self.doc)
    }

    /// The accumulated steps.
    pub fn steps(&self) -> &[Box<dyn Step>] {
        &self.steps
    }

    /// The per-step document snapshots: `docs()[i]` is the document *before* step
    /// `i` was applied. Used to invert a whole transform (undo) and by collab.
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

    /// Apply `step`, recording it (and its map and the pre-step doc). Returns the
    /// step error if it fails to apply (e.g. the result would be schema-invalid),
    /// leaving the transform unchanged.
    pub fn step(&mut self, step: Box<dyn Step>) -> Result<&mut Self, StepError> {
        let new_doc = step.apply(&self.doc)?;
        self.docs.push(self.doc.clone());
        self.mapping.append_map(step.get_map());
        self.steps.push(step);
        self.doc = new_doc;
        Ok(self)
    }

    // -- replace family --------------------------------------------------

    /// Replace `from..to` with `slice`.
    pub fn replace(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        self.step(Box::new(ReplaceStep::new(from, to, slice)))
    }

    /// Replace `from..to` with the (flat) `content`.
    pub fn replace_with(
        &mut self,
        from: usize,
        to: usize,
        content: Fragment,
    ) -> Result<&mut Self, StepError> {
        self.replace(from, to, Slice::from_fragment(content))
    }

    /// Delete the range `from..to`.
    pub fn delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        self.replace(from, to, Slice::empty())
    }

    /// Insert `text` replacing `from..to` (collapsed when `from == to`).
    pub fn insert_text(
        &mut self,
        from: usize,
        to: usize,
        text: &str,
    ) -> Result<&mut Self, StepError> {
        let node = self
            .schema
            .text(text)
            .map_err(|e| StepError::new(e.to_string()))?;
        self.replace(from, to, Slice::from_fragment(Fragment::from_node(node)))
    }

    // -- structure -------------------------------------------------------

    /// Split the node(s) at `pos`, `depth` levels deep. `types_after` optionally
    /// gives the type+attrs of the block(s) created after the split (e.g. Enter at
    /// the end of a heading making a paragraph). Port of `Transform.split`.
    pub fn split(
        &mut self,
        pos: usize,
        depth: usize,
        types_after: Option<Vec<Option<(NodeType, Attrs)>>>,
    ) -> Result<&mut Self, StepError> {
        let r = self
            .doc
            .resolve(Pos(pos))
            .map_err(|e| StepError::new(e.to_string()))?;
        // A split deeper than the position is impossible — ProseMirror would build
        // a too-deep slice that `replace` then rejects; we reject up front (and
        // avoid the usize underflow `r.depth() - depth` would otherwise cause).
        if depth > r.depth() {
            return Err(StepError::new(
                "split depth exceeds the depth of the split position",
            ));
        }
        let mut before = Fragment::empty();
        let mut after = Fragment::empty();
        let e = r.depth() - depth;
        let mut d = r.depth();
        let mut i = depth as isize - 1;
        while d > e {
            before = Fragment::from_node(r.node(d).copy_with_content(before));
            let type_after = types_after
                .as_ref()
                .and_then(|v| v.get(i as usize).cloned().flatten());
            let after_node = match type_after {
                Some((typ, attrs)) => Node::new_branch(typ, attrs, after),
                None => r.node(d).copy_with_content(after),
            };
            after = Fragment::from_node(after_node);
            d -= 1;
            i -= 1;
        }
        let slice = Slice::new(before.append(&after), depth, depth);
        self.step(Box::new(ReplaceStep::structural(pos, pos, slice)))
    }

    /// Change the type (and attrs) of every textblock overlapping `from..to`.
    /// Port of `Transform.setBlockType` (size-preserving, so no remapping needed).
    pub fn set_block_type(
        &mut self,
        from: usize,
        to: usize,
        typ: NodeType,
        attrs: Attrs,
    ) -> Result<&mut Self, StepError> {
        if !typ.is_textblock() {
            return Err(StepError::new("set_block_type target must be a textblock"));
        }
        let mut conversions: Vec<(usize, usize, Vec<Mark>)> = Vec::new();
        self.doc.nodes_between(from, to, &mut |node, pos, _parent| {
            if node.is_textblock() && !node.has_markup(&typ, &attrs) {
                conversions.push((pos, node.node_size(), node.marks().to_vec()));
            }
            true
        });
        for (pos, size, marks) in conversions {
            let new_block =
                Node::new_branch(typ.clone(), attrs.clone(), Fragment::empty()).mark(marks);
            let slice = Slice::new(Fragment::from_node(new_block), 0, 0);
            self.step(Box::new(ReplaceAroundStep::new(
                pos,
                pos + size,
                pos + 1,
                pos + size - 1,
                slice,
                1,
                true,
            )))?;
        }
        Ok(self)
    }

    /// Wrap the block range in the given nested wrappers (outermost first). Port
    /// of `Transform.wrap`.
    pub fn wrap(
        &mut self,
        range: &NodeRange,
        wrappers: &[(NodeType, Attrs)],
    ) -> Result<&mut Self, StepError> {
        let mut content = Fragment::empty();
        for (typ, attrs) in wrappers.iter().rev() {
            content = Fragment::from_node(Node::new_branch(typ.clone(), attrs.clone(), content));
        }
        let start = range.start();
        let end = range.end();
        let slice = Slice::new(content, 0, 0);
        self.step(Box::new(ReplaceAroundStep::new(
            start,
            end,
            start,
            end,
            slice,
            wrappers.len(),
            true,
        )))
    }

    /// Lift the block range out to depth `target` (un-wrap one or more levels of
    /// list/blockquote). Port of `Transform.lift`.
    pub fn lift(&mut self, range: &NodeRange, target: usize) -> Result<&mut Self, StepError> {
        let from = &range.from;
        let to = &range.to;
        let depth = range.depth;
        // A liftable range has both endpoints nested **strictly below** its common
        // depth, so a `depth+1` ancestor exists in each path. A degenerate range —
        // e.g. a node-selection of a block atom (an `hr` inside a blockquote) where
        // an endpoint resolves *at* the common depth — has no such boundary;
        // `after(depth+1)`/`node(depth+1)` would then index the path out of bounds
        // and panic. Fail closed instead. (Reached via outdent on a node-selection
        // and via forward-delete's `build_lift_after`.)
        if from.depth() <= depth || to.depth() <= depth {
            return Err(StepError::new(
                "lift: range endpoints are not nested below its depth",
            ));
        }
        let gap_start = from
            .before(depth + 1)
            .expect("guarded: from.depth() > depth");
        let gap_end = to.after(depth + 1).expect("guarded: to.depth() > depth");

        let mut start = gap_start;
        let mut before = Fragment::empty();
        let mut open_start = 0usize;
        {
            let mut splitting = false;
            let mut d = depth;
            while d > target {
                if splitting || from.index(d) > 0 {
                    splitting = true;
                    before = Fragment::from_node(from.node(d).copy_with_content(before));
                    open_start += 1;
                } else {
                    start -= 1;
                }
                d -= 1;
            }
        }

        let mut end = gap_end;
        let mut after = Fragment::empty();
        let mut open_end = 0usize;
        {
            let mut splitting = false;
            let mut d = depth;
            while d > target {
                // `d <= depth < to.depth()`, ensured by the guard above, so every
                // `to.after(d + 1)` / `to.node(d)` access here is in bounds.
                let after_d = to.after(d + 1).expect("guarded: d < to.depth()");
                if splitting || after_d < to.end(d) {
                    splitting = true;
                    after = Fragment::from_node(to.node(d).copy_with_content(after));
                    open_end += 1;
                } else {
                    end += 1;
                }
                d -= 1;
            }
        }

        let insert = before.size() - open_start;
        let slice = Slice::new(before.append(&after), open_start, open_end);
        self.step(Box::new(ReplaceAroundStep::new(
            start, end, gap_start, gap_end, slice, insert, true,
        )))
    }

    // -- marks -----------------------------------------------------------

    /// Reject a `mark` that cannot refer to any mark in **this** document (issue #217).
    ///
    /// `MarkType` equality is `Rc::ptr_eq` — two `Schema::starter_kit()` instances mint
    /// two different `bold` handles that are never equal. So a `Mark` built from a
    /// different `Schema` than the one behind the document matches nothing, and before
    /// this guard the outcome was silent: [`Transform::remove_mark`] found no match,
    /// issued no step, and returned `Ok`; [`Transform::add_mark`] found no match either
    /// and *added a second, foreign-typed mark of the same name* beside the real one.
    /// Both are always caller bugs, and both are invisible — the removal test that
    /// exposed this asserted only convergence and passed for months while removing
    /// nothing.
    ///
    /// Two hazards, because they are genuinely different and only the second one was the
    /// bug actually found:
    ///
    /// * **The mark is foreign to this transform's schema.** Caught up front by identity
    ///   against `self.schema`.
    /// * **The *document* is foreign to this transform's schema.** The mark is this
    ///   schema's, so the check above passes, but the document was built by another
    ///   `Schema` and carries that one's handles. Caught by `foreign_twin` during the
    ///   walk each caller already does: a node carrying a mark of the *same name* but a
    ///   *different* `MarkType` handle is proof, and is never legitimate.
    ///
    /// Deliberately **not** an error: finding no such mark in the range at all. That is
    /// an ordinary no-op — `toggleBold` over unbolded text, or removing `link[href=a]`
    /// where the text carries `link[href=b]` (same type, different attrs, correctly no
    /// match). Only a *type-identity* mismatch is reported.
    fn check_mark_schema(&self, mark: &Mark) -> Result<(), StepError> {
        if self.schema.mark_type(mark.type_name()) != Some(&mark.typ) {
            return Err(StepError::new(format!(
                "mark `{}` was built from a different Schema than this document's; \
                 MarkType identity is per-Schema, so it can never match (issue #217)",
                mark.type_name()
            )));
        }
        Ok(())
    }

    /// The error for hazard 2 above: the document holds a same-named mark that is not
    /// `mark`'s type.
    fn foreign_twin(mark: &Mark) -> StepError {
        StepError::new(format!(
            "this document carries a `{}` mark from a different Schema instance than \
             this transform's; MarkType identity is per-Schema, so the two can never \
             match and the operation would silently do nothing (issue #217)",
            mark.type_name()
        ))
    }

    /// Add `mark` across the inline range `from..to`, first removing any marks the
    /// new mark excludes. Port of `Transform.addMark`.
    pub fn add_mark(&mut self, from: usize, to: usize, mark: Mark) -> Result<&mut Self, StepError> {
        self.check_mark_schema(&mark)?;
        let mut removed: Vec<RemoveMarkStep> = Vec::new();
        let mut added: Vec<AddMarkStep> = Vec::new();
        let mut foreign: Option<StepError> = None;
        self.doc.nodes_between(from, to, &mut |node, pos, parent| {
            if !node.is_inline() {
                return true;
            }
            if foreign_same_name(&mark, node.marks()) {
                foreign.get_or_insert_with(|| Transform::foreign_twin(&mark));
                return true;
            }
            let Some(parent) = parent else { return true };
            if !mark.is_in(node.marks()) && parent.node_type().spec().marks.allows(mark.type_name())
            {
                let start = pos.max(from);
                let end = (pos + node.node_size()).min(to);
                let new_set = mark.add_to_set(node.marks());
                for m in node.marks() {
                    if m.is_in(&new_set) {
                        continue;
                    }
                    let coalesced =
                        matches!(removed.last(), Some(last) if last.to == start && last.mark == *m);
                    if coalesced {
                        removed.last_mut().unwrap().to = end;
                    } else {
                        removed.push(RemoveMarkStep::new(start, end, m.clone()));
                    }
                }
                let coalesced = matches!(added.last(), Some(last) if last.to == start);
                if coalesced {
                    added.last_mut().unwrap().to = end;
                } else {
                    added.push(AddMarkStep::new(start, end, mark.clone()));
                }
            }
            true
        });
        if let Some(e) = foreign {
            return Err(e);
        }
        for s in removed {
            self.step(Box::new(s))?;
        }
        for s in added {
            self.step(Box::new(s))?;
        }
        Ok(self)
    }

    /// Remove `mark` across the inline range `from..to`. Port of
    /// `Transform.removeMark` for a concrete mark.
    pub fn remove_mark(
        &mut self,
        from: usize,
        to: usize,
        mark: Mark,
    ) -> Result<&mut Self, StepError> {
        self.check_mark_schema(&mark)?;
        let mut matched: Vec<(usize, usize)> = Vec::new();
        let mut foreign: Option<StepError> = None;
        self.doc.nodes_between(from, to, &mut |node, pos, _parent| {
            if !node.is_inline() {
                return true;
            }
            if foreign_same_name(&mark, node.marks()) {
                foreign.get_or_insert_with(|| Transform::foreign_twin(&mark));
                return true;
            }
            if mark.is_in(node.marks()) {
                let start = pos.max(from);
                let end = (pos + node.node_size()).min(to);
                let coalesced = matches!(matched.last(), Some(last) if last.1 == start);
                if coalesced {
                    matched.last_mut().unwrap().1 = end;
                } else {
                    matched.push((start, end));
                }
            }
            true
        });
        if let Some(e) = foreign {
            return Err(e);
        }
        for (f, t) in matched {
            self.step(Box::new(RemoveMarkStep::new(f, t, mark.clone())))?;
        }
        Ok(self)
    }

    // -- attrs -----------------------------------------------------------

    /// Set the attribute `attr` of the node at `pos`.
    pub fn set_node_attr(
        &mut self,
        pos: usize,
        attr: impl Into<String>,
        value: AttrValue,
    ) -> Result<&mut Self, StepError> {
        self.step(Box::new(SetNodeAttrStep::new(pos, attr, value)))
    }

    /// Set a document-level attribute.
    pub fn set_doc_attr(
        &mut self,
        attr: impl Into<String>,
        value: AttrValue,
    ) -> Result<&mut Self, StepError> {
        self.step(Box::new(SetDocAttrStep::new(attr, value)))
    }
}

/// Whether `set` holds a mark sharing `mark`'s *name* but not its `MarkType` handle —
/// proof that `set`'s document was built by a different `Schema` (issue #217). `false`
/// when every same-named mark is the same type, which includes the ordinary "no such
/// mark here" case.
fn foreign_same_name(mark: &Mark, set: &[Mark]) -> bool {
    set.iter()
        .any(|m| m.type_name() == mark.type_name() && m.typ != mark.typ)
}
