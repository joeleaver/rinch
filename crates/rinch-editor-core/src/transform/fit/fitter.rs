//! The `Fitter`: places a slice piece by piece into the open nodes around a
//! range (ProseMirror's `replaceStep`).

use super::*;

/// One open node around the insertion point: its type, and how far the
/// children it has so far have got through its content expression. The state
/// is stepped once per child: asking the expression about the whole child
/// list for every node placed made a paste of n blocks cost n².
struct Frame {
    typ: NodeType,
    state: MatchState,
}

impl Frame {
    /// A frame of `typ` that already holds `children`.
    fn holding(typ: &NodeType, children: &[Node]) -> Frame {
        let mut frame = Frame {
            typ: typ.clone(),
            state: typ.content_match().start(),
        };
        for child in children {
            frame.push(child.type_name());
        }
        frame
    }

    fn step(&self, state: &MatchState, name: &str) -> MatchState {
        MATCH_STEPS.with(|c| c.set(c.get() + 1));
        self.typ.content_match().advance(state, name)
    }

    /// Whether a `name` child may come next.
    fn takes(&self, name: &str) -> bool {
        !self.step(&self.state, name).is_dead()
    }

    /// Record a `name` child.
    fn push(&mut self, name: &str) {
        self.state = self.step(&self.state, name);
    }

    /// Whether the children so far, followed by `rest`, are a whole valid
    /// content.
    fn complete_with(&self, rest: &[Node]) -> bool {
        let mut state = self.state.clone();
        for node in rest {
            state = self.step(&state, node.type_name());
        }
        self.typ.content_match().accepts_end(&state)
    }
}

/// Whether the content after `to` in its ancestor at `depth` may follow what
/// `frame` holds. `open` says the child `to` is inside is not part of it.
fn content_after_fits(to: &ResolvedPos, depth: usize, frame: &Frame, open: bool) -> bool {
    let node = to.node(depth);
    let index = if open {
        to.index_after(depth)
    } else {
        to.index(depth)
    };
    let index = index.min(node.child_count());
    if index == node.child_count() && !frame.typ.compatible_content(node.node_type()) {
        return false;
    }
    let rest = &node.content().children()[index..];
    frame.complete_with(rest)
        && rest.iter().all(|child| {
            child
                .marks()
                .iter()
                .all(|m| frame.typ.spec().marks.allows(m.type_name()))
        })
}

pub(super) struct Fitter<'a> {
    from: &'a ResolvedPos,
    to: &'a ResolvedPos,
    /// What is left of the slice.
    unplaced: Slice,
    /// The open nodes at the insertion point, outermost first.
    frontier: Vec<Frame>,
    /// The content placed so far, inside empty copies of `from`'s ancestors.
    placed: Fragment,
    /// The shallowest frontier depth content may be placed at: the innermost
    /// isolating ancestor of `from`.
    floor: usize,
}

struct Fittable {
    slice_depth: usize,
    frontier_depth: usize,
    /// The slice node whose content is placed, when `slice_depth > 0`.
    parent: Option<Node>,
}

impl<'a> Fitter<'a> {
    pub(super) fn new(from: &'a ResolvedPos, to: &'a ResolvedPos, slice: &Slice) -> Self {
        let mut frontier = Vec::with_capacity(from.depth() + 1);
        let mut floor = 0;
        for d in 0..=from.depth() {
            let node = from.node(d);
            let upto = from.index_after(d).min(node.child_count());
            frontier.push(Frame::holding(
                node.node_type(),
                &node.content().children()[..upto],
            ));
            if d > 0 && node.node_type().is_isolating() {
                floor = d;
            }
        }
        let mut placed = Fragment::empty();
        for d in (1..=from.depth()).rev() {
            placed = Fragment::from_node(from.node(d).copy_with_content(placed));
        }
        Fitter {
            from,
            to,
            unplaced: slice.clone(),
            frontier,
            placed,
            floor,
        }
    }

    fn depth(&self) -> usize {
        self.frontier.len() - 1
    }

    pub(super) fn fit(mut self) -> Option<Fit> {
        // Each turn places content, opens the slice one node further, or
        // drops a node: all three shrink what is left, so this ends. The
        // bound is a guard against a mistake in that argument, not a limit a
        // real slice meets.
        let mut turns = 16 * (self.unplaced.content.size() + 16);
        while self.unplaced.size() > 0 {
            turns = turns.checked_sub(1)?;
            match self.find_fittable() {
                Some(fit) => self.place_nodes(fit)?,
                None => {
                    if !self.open_more() {
                        self.drop_node();
                    }
                }
            }
        }
        // When the placed content ends in a textblock and so does the range,
        // the inline content after the range has to move into that textblock:
        // the fit then runs to the end of the range's textblock.
        let move_inline = self.must_move_inline();
        let placed_size = (self.placed.size()).checked_sub(self.depth() + self.from.depth())?;
        let doc = self.from.node(0).clone();
        let moved;
        let target = match move_inline {
            Some(pos) => {
                moved = doc.resolve(Pos(pos)).ok()?;
                &moved
            }
            None => self.to,
        };
        let to = self.close(target)?;

        let mut content = self.placed.clone();
        let (mut open_start, mut open_end) = (self.from.depth(), to.depth());
        while open_start > 0 && open_end > 0 && content.child_count() == 1 {
            content = content.child(0).content().clone();
            open_start -= 1;
            open_end -= 1;
        }
        let slice = Slice::new(content, open_start, open_end);
        let from = self.from.pos().0;
        if let Some(outer) = move_inline {
            let (gap_from, gap_to) = (self.to.pos().0, self.to.end(self.to.depth()));
            return Some(Fit {
                step: Box::new(ReplaceAroundStep::new(
                    from,
                    outer,
                    gap_from,
                    gap_to,
                    slice,
                    placed_size,
                    false,
                )),
                end: from + placed_size,
            });
        }
        if slice.size() == 0 && from == self.to.pos().0 {
            return None;
        }
        Some(Fit {
            end: from + slice.size(),
            step: Box::new(ReplaceStep::new(from, to.pos().0, slice)),
        })
    }

    /// The deepest content of the slice's open start that some frontier node
    /// takes, at the deepest such node.
    fn find_fittable(&self) -> Option<Fittable> {
        let start_depth = self.unplaced.open_start;
        for slice_depth in (0..=start_depth).rev() {
            let (fragment, parent) = if slice_depth > 0 {
                let parent = content_at(&self.unplaced.content, slice_depth - 1)
                    .child(0)
                    .clone();
                (parent.content().clone(), Some(parent))
            } else {
                (self.unplaced.content.clone(), None)
            };
            let first = fragment.maybe_child(0);
            for frontier_depth in (self.floor..=self.depth()).rev() {
                let frame = &self.frontier[frontier_depth];
                let fits = match (first, &parent) {
                    (Some(first), _) => frame.takes(first.type_name()),
                    // An open node with nothing in it: any frontier node of
                    // compatible content stands for it.
                    (None, Some(parent)) => frame.typ.compatible_content(parent.node_type()),
                    (None, None) => false,
                };
                if fits {
                    return Some(Fittable {
                        slice_depth,
                        frontier_depth,
                        parent,
                    });
                }
                // Do not look further up when the node itself would go here.
                if let Some(parent) = &parent
                    && frame.takes(parent.type_name())
                {
                    break;
                }
            }
        }
        None
    }

    /// Open the slice's first node, when it has content that is not a leaf.
    fn open_more(&mut self) -> bool {
        let Slice {
            content,
            open_start,
            open_end,
        } = &self.unplaced;
        let inner = content_at(content, *open_start);
        match inner.maybe_child(0) {
            Some(first) if !first.is_leaf() => {}
            _ => return false,
        }
        // The node being opened is also the slice's last when nothing follows
        // it: then the end is open at least as far.
        let reaches_end = inner.size() + open_start >= content.size().saturating_sub(*open_end);
        let open_end = (*open_end).max(if reaches_end { open_start + 1 } else { 0 });
        self.unplaced = Slice::new(content.clone(), open_start + 1, open_end);
        true
    }

    /// Give up on the slice's first node.
    fn drop_node(&mut self) {
        let Slice {
            content,
            open_start,
            open_end,
        } = self.unplaced.clone();
        let inner = content_at(&content, open_start);
        if inner.child_count() <= 1 && open_start > 0 {
            let open_at_end =
                content.size().saturating_sub(open_start) <= open_start + inner.size();
            self.unplaced = Slice::new(
                drop_from_fragment(&content, open_start - 1, 1),
                open_start - 1,
                if open_at_end {
                    open_start - 1
                } else {
                    open_end
                },
            );
        } else {
            self.unplaced = Slice::new(
                drop_from_fragment(&content, open_start, 1),
                open_start,
                open_end,
            );
        }
    }

    /// Move as many nodes as fit from the slice into the frontier node at
    /// `frontier_depth`. `None` when a frontier node that has to close is not
    /// valid as it stands.
    fn place_nodes(&mut self, fit: Fittable) -> Option<()> {
        let Fittable {
            slice_depth,
            frontier_depth,
            parent,
        } = fit;
        while self.depth() > frontier_depth {
            self.close_frontier_node()?;
        }
        let slice = self.unplaced.clone();
        let fragment = match &parent {
            Some(parent) => parent.content().clone(),
            None => slice.content.clone(),
        };
        // How deep the first node is still open.
        let open_start = slice.open_start - slice_depth;
        // How many nodes are open at the fragment's end: 0 when only the
        // parent is, negative when nothing is.
        let mut open_end_count = (fragment.size() + slice_depth) as isize
            - (slice.content.size() as isize - slice.open_end as isize);

        let mut taken = 0;
        let mut add: Vec<Node> = Vec::new();
        let mut last_added = false;
        let typ = self.frontier[frontier_depth].typ.clone();
        while taken < fragment.child_count() {
            let next = fragment.child(taken);
            if !self.frontier[frontier_depth].takes(next.type_name()) {
                break;
            }
            taken += 1;
            // An open node with nothing in it is dropped.
            last_added = taken > 1 || open_start == 0 || next.content().size() > 0;
            if last_added {
                // Placed whole, a node that was open at its start is closed
                // there.
                if taken == 1 && !start_closes(next, open_start) {
                    return None;
                }
                self.frontier[frontier_depth].push(next.type_name());
                let marks = allowed_marks(&typ, next.marks());
                add.push(if marks.len() == next.marks().len() {
                    next.clone()
                } else {
                    next.with_marks(marks)
                });
            }
        }
        let to_end = taken == fragment.child_count();
        if !to_end {
            open_end_count = -1;
        }
        self.placed = add_to_fragment(&self.placed, frontier_depth, &Fragment::from_children(add));

        // The whole of a closed node of the frontier's own type went in:
        // that frontier node is done.
        if to_end
            && open_end_count < 0
            && parent
                .as_ref()
                .is_some_and(|p| *p.node_type() == self.frontier[self.depth()].typ)
            && self.frontier.len() > 1
        {
            self.close_frontier_node()?;
        }

        // The nodes open at the fragment's end are frontier nodes now.
        if last_added {
            let mut cur = fragment.clone();
            for _ in 0..open_end_count.max(0) {
                let node = cur.child(cur.child_count() - 1).clone();
                self.frontier
                    .push(Frame::holding(node.node_type(), node.content().children()));
                cur = node.content().clone();
            }
        }

        self.unplaced = if !to_end {
            Slice::new(
                drop_from_fragment(&slice.content, slice_depth, taken),
                slice.open_start,
                slice.open_end,
            )
        } else if slice_depth == 0 {
            Slice::empty()
        } else {
            Slice::new(
                drop_from_fragment(&slice.content, slice_depth - 1, 1),
                slice_depth - 1,
                if open_end_count < 0 {
                    slice.open_end
                } else {
                    slice_depth - 1
                },
            )
        };
        Some(())
    }

    /// Where the fit has to end for the inline content after the range to
    /// move into the textblock the placed content ends in, if it has to.
    fn must_move_inline(&self) -> Option<usize> {
        let to = self.to;
        if !to.parent().is_textblock() {
            return None;
        }
        let top = &self.frontier[self.depth()];
        if !top.typ.is_textblock() || !content_after_fits(to, to.depth(), top, false) {
            return None;
        }
        // The frontier's textblock is the range's own: an ordinary join.
        if to.depth() == self.depth()
            && self
                .find_close_level(to)
                .is_some_and(|(depth, _)| depth == self.depth())
        {
            return None;
        }
        let mut depth = to.depth();
        let mut after = to.after(depth)?;
        while depth > 1 {
            depth -= 1;
            if after != to.end(depth) {
                break;
            }
            after += 1;
        }
        Some(after)
    }

    /// The deepest frontier depth at which what follows `to` can be joined
    /// on, and whether `to` then moves past the end of the node it is in.
    fn find_close_level(&self, to: &ResolvedPos) -> Option<(usize, bool)> {
        'scan: for i in (0..=self.depth().min(to.depth())).rev() {
            let drop_inner = i < to.depth() && to.end(i + 1) == to.pos().0 + (to.depth() - (i + 1));
            if !content_after_fits(to, i, &self.frontier[i], drop_inner) {
                continue;
            }
            for d in (0..i).rev() {
                if !content_after_fits(to, d, &self.frontier[d], true) {
                    continue 'scan;
                }
            }
            return Some((i, drop_inner));
        }
        None
    }

    /// Close the frontier down to where `to`'s content joins on, and open
    /// `to`'s own ancestors below that. Returns where the replaced range
    /// ends.
    fn close(&mut self, to: &ResolvedPos) -> Option<ResolvedPos> {
        let (depth, drop_inner) = self.find_close_level(to)?;
        while self.depth() > depth {
            self.close_frontier_node()?;
        }
        let to = if drop_inner {
            let doc = to.node(0).clone();
            doc.resolve(Pos(to.after(depth + 1)?)).ok()?
        } else {
            to.clone()
        };
        for d in depth + 1..=to.depth() {
            let node = to.node(d);
            let empty = node.copy_with_content(Fragment::empty());
            let at = self.depth();
            self.frontier[at].push(node.type_name());
            self.placed = add_to_fragment(&self.placed, at, &Fragment::from_node(empty));
            self.frontier.push(Frame::holding(node.node_type(), &[]));
        }
        Some(to)
    }

    /// Close the innermost frontier node. `None` when its content is not
    /// valid as it stands (ProseMirror would create the missing nodes).
    fn close_frontier_node(&mut self) -> Option<()> {
        let open = self.frontier.pop()?;
        open.complete_with(&[]).then_some(())
    }
}
