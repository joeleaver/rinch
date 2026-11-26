//! The structural edits: split, change of block type, wrap and lift.

use super::*;

/// Whether a [`Transform::split`] of `depth` levels at `pos` in `doc` would
/// cut no [isolating](crate::schema::NodeSpec::isolating) node: the
/// isolating half of ProseMirror's `canSplit`. The nodes a split cuts are the
/// `depth` innermost ancestors of `pos` (the parent first); a table cell is
/// isolating, so a split stops at it. Whether the halves are valid content is
/// the step's own check. `false` for a position outside `doc` or a depth of 0
/// or past the position's depth.
pub fn can_split(doc: &Node, pos: usize, depth: usize) -> bool {
    match doc.resolve(Pos(pos)) {
        Ok(r) => depth >= 1 && depth <= r.depth() && !cuts_isolating(&r, depth),
        Err(_) => false,
    }
}

fn cuts_isolating(r: &crate::pos::ResolvedPos, depth: usize) -> bool {
    (r.depth() + 1 - depth.min(r.depth())..=r.depth()).any(|d| r.node(d).node_type().is_isolating())
}

impl Transform<'_> {
    // -- structure -------------------------------------------------------

    /// Split the node(s) at `pos`, `depth` levels deep. `types_after` optionally
    /// gives the type+attrs of the block(s) created after the split (e.g. Enter at
    /// the end of a heading making a paragraph). Port of `Transform.split`.
    ///
    /// Errors, adding nothing, when the split would cut an
    /// [isolating](crate::schema::NodeSpec::isolating) node — see [`can_split`].
    /// ProseMirror leaves that check to the caller (`canSplit`); rinch makes it
    /// here too, so no command can split a table cell (#1232: a split cell is a
    /// row with one cell more than its table has columns).
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
        if cuts_isolating(&r, depth) {
            return Err(StepError::new("a split may not cut an isolating node"));
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
    /// Each block keeps its [app data attributes](crate::schema::data_attrs)
    /// when `typ` keeps them and `attrs` does not name them.
    /// Port of `Transform.setBlockType`: each block's content is first cleared of
    /// what the new type does not allow ([`clear_incompatible`](Self::clear_incompatible)),
    /// so a paragraph with bold text and a hard break made a code block is plain
    /// text with a line break (#1227), never content its schema forbids.
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
        let mut conversions: Vec<(usize, Attrs, Vec<Mark>)> = Vec::new();
        self.doc.nodes_between(from, to, &mut |node, pos, _parent| {
            if node.is_textblock() {
                // Each block keeps its own app data attributes.
                let attrs = carry_app_data(&typ, &attrs, node);
                if !node.has_markup(&typ, &attrs) {
                    conversions.push((pos, attrs, node.marks().to_vec()));
                }
            }
            true
        });
        let map_from = self.mapping.len();
        for (pos, attrs, marks) in conversions {
            // Positions were collected in the document as it was; clearing an earlier
            // block can have shrunk it.
            let pos = self.mapping.slice(map_from, self.mapping.len()).map(pos, 1);
            self.clear_incompatible(pos, &typ)?;
            let Some(size) = self.doc.node_at(pos).map(|n| n.node_size()) else {
                return Err(StepError::new(
                    "set_block_type: block not found after clearing",
                ));
            };
            let new_block =
                Node::new_branch(typ.clone(), attrs, Fragment::empty()).with_marks(marks);
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

    /// Remove from the textblock at `pos` whatever a block of type `typ` cannot
    /// hold. Port of ProseMirror's `clearIncompatible` with its `linebreakReplacement`
    /// half: a mark `typ` does not allow is removed, a `hard_break` going into a
    /// [`code`](crate::schema::NodeSpec::code) block becomes the text `"\n"`, and any
    /// other child `typ`'s content expression does not take is deleted.
    pub fn clear_incompatible(
        &mut self,
        pos: usize,
        typ: &NodeType,
    ) -> Result<&mut Self, StepError> {
        let Some(node) = self.doc.node_at(pos) else {
            return Err(StepError::new(
                "clear_incompatible: no node at the position",
            ));
        };
        let content = typ.content_match();
        let code = typ.spec().code;
        let mut state = content.start();
        let mut marks_off: Vec<(usize, usize, Mark)> = Vec::new();
        // (from, to, replacement): a child deleted or turned into a line break.
        let mut replaced: Vec<(usize, usize, Option<Node>)> = Vec::new();
        let mut cur = pos + 1;
        for i in 0..node.child_count() {
            let child = node.child(i);
            let end = cur + child.node_size();
            let next = content.advance(&state, child.type_name());
            if next.is_dead() {
                let line_break = (code && child.type_name() == "hard_break")
                    .then(|| content.advance(&state, "text"))
                    .filter(|after| !after.is_dead());
                match line_break {
                    Some(after) => {
                        let text = self
                            .schema
                            .text("\n")
                            .map_err(|e| StepError::new(format!("clear_incompatible: {e}")))?;
                        replaced.push((cur, end, Some(text)));
                        state = after;
                    }
                    None => replaced.push((cur, end, None)),
                }
            } else {
                state = next;
                for m in child.marks() {
                    if !typ.spec().marks.allows(m.type_name()) {
                        marks_off.push((cur, end, m.clone()));
                    }
                }
            }
            cur = end;
        }
        for (from, to, mark) in marks_off {
            self.step(Box::new(RemoveMarkStep::new(from, to, mark)))?;
        }
        for (from, to, with) in replaced.into_iter().rev() {
            let slice = match with {
                Some(n) => Slice::new(Fragment::from_node(n), 0, 0),
                None => Slice::empty(),
            };
            self.step(Box::new(ReplaceStep::new(from, to, slice)))?;
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
}
