//! Marks, batched edits and attributes.

use super::*;

impl Transform<'_> {
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

    // -- batch -----------------------------------------------------------

    /// Apply disjoint `edits`, stated in the current document's coordinates,
    /// as one [`BatchStep`]. Nothing is recorded for an empty list.
    pub fn batch(&mut self, edits: Vec<BatchEdit>) -> Result<&mut Self, StepError> {
        if edits.is_empty() {
            return Ok(self);
        }
        self.step(Box::new(BatchStep::new(edits)?))
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
