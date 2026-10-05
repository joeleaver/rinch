//! Fitting a [`Slice`] into a document range it does not fit as it is.
//!
//! [`ReplaceStep`] is strict: the slice's open depths must line up with the
//! range's, and every node it rebuilds must be valid. Content that comes from
//! somewhere else — a paste — rarely lines up: a list has no place in the
//! paragraph the caret is in. This module finds the step that puts as much of
//! the slice as possible into the range **with its structure**, closing or
//! splitting the nodes around the range where the content needs it.
//!
//! It is ProseMirror's algorithm, in two layers (`prosemirror-transform`,
//! `replace.ts`):
//!
//! - [`fit_step`] is the `Fitter`. It walks the slice from its start and
//!   places each piece in the deepest open node around the range's start that
//!   takes it — the *frontier* — closing frontier nodes that do not. Content
//!   that is open at the slice's start therefore continues the textblock the
//!   range starts in; a closed block closes that textblock and goes in beside
//!   it. At the end it joins the frontier back to what follows the range, and
//!   when the placed content ends inside a textblock, the inline content after
//!   the range moves into that textblock.
//! - [`replace_range`] is `replaceRange`: before fitting, it looks at whether
//!   the range covers whole nodes (a caret in an empty textblock does) and
//!   whether the slice starts in a [defining](crate::schema::NodeSpec::defining)
//!   node. If so, the defining node is kept and replaces the covered node,
//!   rather than its content being poured into it: a list pasted on an empty
//!   line is a list, a heading pasted on an empty line is a heading.
//!
//! Two things ProseMirror does are not done here, because the content
//! matcher ([`ContentMatch`](crate::schema::ContentMatch)) has no way to ask
//! for them: nodes are never *created* to make content valid (`fillBefore`),
//! and content is never *wrapped* in a node it was not in (`findWrapping`).
//! A fit that would need either fails. And one thing is added: the fit never
//! leaves an [isolating](crate::schema::NodeSpec::isolating) node (a table
//! cell) the range starts in.
//!
//! Nothing here can produce an invalid document: the result is a step, and
//! applying it validates every node it rebuilds
//! ([`replace::close`](super::replace)).

use crate::model::{Fragment, Mark, Node, NodeType, Slice};
use crate::pos::{Pos, ResolvedPos};
use crate::transform::step::Step;
use crate::transform::steps::{ReplaceAroundStep, ReplaceStep};

/// A fitted replacement: the step, and the position right after the content
/// it inserted (where a caret goes after a paste), in the document the step
/// produces.
pub(crate) struct Fit {
    pub step: Box<dyn Step>,
    pub end: usize,
}

/// One open node around the insertion point: its type, and the names of the
/// children it has so far (which is what the content expression is asked).
struct Frame {
    typ: NodeType,
    names: Vec<Box<str>>,
}

impl Frame {
    /// Whether a `name` child may come next.
    fn takes(&self, name: &str) -> bool {
        let mut names: Vec<&str> = self.names.iter().map(|n| &**n).collect();
        names.push(name);
        self.typ.content_match().matches_prefix(&names)
    }

    /// Whether the children so far, followed by `rest`, are a whole valid
    /// content.
    fn complete_with(&self, rest: &[Node]) -> bool {
        let names: Vec<&str> = self
            .names
            .iter()
            .map(|n| &**n)
            .chain(rest.iter().map(Node::type_name))
            .collect();
        self.typ.content_match().matches(&names)
    }
}

fn names_of(nodes: &[Node]) -> Vec<Box<str>> {
    nodes.iter().map(|n| Box::from(n.type_name())).collect()
}

/// The marks of `marks` that `typ` allows on its content.
fn allowed_marks(typ: &NodeType, marks: &[Mark]) -> Vec<Mark> {
    marks
        .iter()
        .filter(|m| typ.spec().marks.allows(m.type_name()))
        .cloned()
        .collect()
}

/// Whether `node`'s content is a whole valid content for its type.
fn content_valid(node: &Node) -> bool {
    let names: Vec<&str> = node
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    node.node_type().content_match().matches(&names)
}

/// Whether `node` and its first `levels - 1` first-descendants are valid as
/// they stand: what closing a node that was open at its start requires.
/// (A node cut at its very end is open and empty, and an empty list item is
/// not a list item. ProseMirror fills such a node; here it is not closed.)
fn start_closes(node: &Node, levels: usize) -> bool {
    let mut node = Some(node);
    for _ in 0..levels {
        let Some(n) = node else { break };
        if !content_valid(n) {
            return false;
        }
        node = n.content().children().first();
    }
    true
}

/// Whether every node of `slice` is valid, the nodes open at its edges
/// aside (their content is a part of a valid content, which the fit checks
/// where it closes or joins them).
pub(crate) fn slice_is_sound(slice: &Slice) -> bool {
    fn walk(nodes: &[Node], open_start: usize, open_end: usize) -> bool {
        let last = nodes.len().saturating_sub(1);
        nodes.iter().enumerate().all(|(i, node)| {
            let (s, e) = (
                if i == 0 { open_start } else { 0 },
                if i == last { open_end } else { 0 },
            );
            (s > 0 || e > 0 || content_valid(node))
                && walk(
                    node.content().children(),
                    s.saturating_sub(1),
                    e.saturating_sub(1),
                )
        })
    }
    walk(slice.content.children(), slice.open_start, slice.open_end)
}

/// The content `depth` first-children down.
fn content_at(fragment: &Fragment, depth: usize) -> Fragment {
    let mut fragment = fragment.clone();
    for _ in 0..depth {
        fragment = fragment.child(0).content().clone();
    }
    fragment
}

/// `fragment` without the first `count` children of the node `depth`
/// first-children down.
fn drop_from_fragment(fragment: &Fragment, depth: usize, count: usize) -> Fragment {
    if depth == 0 {
        return Fragment::from_children(fragment.children()[count..].to_vec());
    }
    let first = fragment.child(0);
    fragment.replace_child(
        0,
        first.copy_with_content(drop_from_fragment(first.content(), depth - 1, count)),
    )
}

/// `fragment` with `content` appended to the node `depth` last-children down.
fn add_to_fragment(fragment: &Fragment, depth: usize, content: &Fragment) -> Fragment {
    if depth == 0 {
        return fragment.append(content);
    }
    let last = fragment.child_count() - 1;
    let node = fragment.child(last);
    fragment.replace_child(
        last,
        node.copy_with_content(add_to_fragment(node.content(), depth - 1, content)),
    )
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

struct Fitter<'a> {
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
    fn new(from: &'a ResolvedPos, to: &'a ResolvedPos, slice: &Slice) -> Self {
        let mut frontier = Vec::with_capacity(from.depth() + 1);
        let mut floor = 0;
        for d in 0..=from.depth() {
            let node = from.node(d);
            let upto = from.index_after(d).min(node.child_count());
            frontier.push(Frame {
                typ: node.node_type().clone(),
                names: names_of(&node.content().children()[..upto]),
            });
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

    fn fit(mut self) -> Option<Fit> {
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
                self.frontier[frontier_depth]
                    .names
                    .push(Box::from(next.type_name()));
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
                self.frontier.push(Frame {
                    typ: node.node_type().clone(),
                    names: names_of(node.content().children()),
                });
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
            self.frontier[at].names.push(Box::from(node.type_name()));
            self.placed = add_to_fragment(&self.placed, at, &Fragment::from_node(empty));
            self.frontier.push(Frame {
                typ: node.node_type().clone(),
                names: Vec::new(),
            });
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

/// The step that fits `slice` into `from..to` of `doc`, when there is one.
/// Port of ProseMirror's `replaceStep`.
pub(crate) fn fit_step(doc: &Node, from: usize, to: usize, slice: &Slice) -> Option<Fit> {
    let r_from = doc.resolve(Pos(from)).ok()?;
    let r_to = doc.resolve(Pos(to)).ok()?;
    if fits_trivially(&r_from, &r_to, slice) {
        return Some(Fit {
            end: from + slice.size(),
            step: Box::new(ReplaceStep::new(from, to, slice.clone())),
        });
    }
    Fitter::new(&r_from, &r_to, slice).fit()
}

/// A closed slice into one parent that takes it as it is.
fn fits_trivially(from: &ResolvedPos, to: &ResolvedPos, slice: &Slice) -> bool {
    if slice.open_start != 0
        || slice.open_end != 0
        || from.depth() != to.depth()
        || from.start(from.depth()) != to.start(to.depth())
    {
        return false;
    }
    let parent = from.parent();
    let children = parent.content().children();
    // What stays on each side; a text node the range cuts stays on both.
    let before = &children[..from.index_after(from.depth()).min(children.len())];
    let after = &children[to.index(to.depth()).min(children.len())..];
    let names: Vec<&str> = before
        .iter()
        .chain(slice.content.children())
        .chain(after)
        .map(Node::type_name)
        .collect();
    let marks = &parent.node_type().spec().marks;
    parent.node_type().content_match().matches(&names)
        && slice
            .content
            .children()
            .iter()
            .all(|n| n.marks().iter().all(|m| marks.allows(m.type_name())))
}

/// The depths at which `from..to` covers the whole content of the node both
/// ends are in (so the node itself can be replaced), innermost first. Port of
/// `coveredDepths`.
fn covered_depths(from: &ResolvedPos, to: &ResolvedPos) -> Vec<usize> {
    let mut result = Vec::new();
    for d in (0..=from.depth().min(to.depth())).rev() {
        let start = from.start(d);
        if start + (from.depth() - d) < from.pos().0
            || to.end(d) > to.pos().0 + (to.depth() - d)
            || from.node(d).node_type().is_isolating()
            || to.node(d).node_type().is_isolating()
        {
            break;
        }
        if start == to.start(d)
            || (d == from.depth()
                && d == to.depth()
                && from.parent().is_textblock()
                && to.parent().is_textblock()
                && d > 0
                && to.start(d - 1) == start - 1)
        {
            result.push(d);
        }
    }
    result
}

/// Whether a `typ` child may be inserted in `parent` at child `index`.
fn can_insert(parent: &Node, index: usize, typ: &Node) -> bool {
    let children = parent.content().children();
    let index = index.min(children.len());
    let names: Vec<&str> = children[..index]
        .iter()
        .map(Node::type_name)
        .chain(std::iter::once(typ.type_name()))
        .chain(children[index..].iter().map(Node::type_name))
        .collect();
    parent.node_type().content_match().matches(&names)
        && typ
            .marks()
            .iter()
            .all(|m| parent.node_type().spec().marks.allows(m.type_name()))
}

/// The candidate replacements for `slice` over `from..to`, best first. Port of
/// ProseMirror's `replaceRange`; see the module doc. Each candidate is
/// `(from, to, slice)` for [`fit_step`]; the caller takes the first whose step
/// applies. (ProseMirror commits to its first candidate; here a candidate
/// that does not fit — one that would need a node created — gives way to the
/// next.)
pub(crate) fn range_candidates(
    doc: &Node,
    from: usize,
    to: usize,
    slice: &Slice,
) -> Vec<(usize, usize, Slice)> {
    let mut out = Vec::new();
    let (Ok(r_from), Ok(r_to)) = (doc.resolve(Pos(from)), doc.resolve(Pos(to))) else {
        return out;
    };
    if fits_trivially(&r_from, &r_to, slice) {
        out.push((from, to, slice.clone()));
        return out;
    }

    // Positive: replace the whole node at that depth. Negative `-d`: replace
    // from before the node at depth `d` (which `from` is at the start of) to
    // `to`.
    let mut targets: Vec<isize> = covered_depths(&r_from, &r_to)
        .into_iter()
        .map(|d| d as isize)
        .collect();
    if targets.last() == Some(&0) {
        targets.pop();
    }
    let mut preferred_target = -(r_from.depth() as isize + 1);
    targets.insert(0, preferred_target);
    let mut pos = r_from.pos().0;
    for d in (1..=r_from.depth()).rev() {
        // `pos` is where `from` would be were it at the very start of the
        // node at depth `d`, minus one: the position before that node.
        pos = pos.wrapping_sub(1);
        let spec = r_from.node(d).node_type().spec();
        if spec.defining || spec.isolating {
            break;
        }
        if targets.contains(&(d as isize)) {
            preferred_target = d as isize;
        } else if r_from.before(d) == Some(pos) {
            targets.insert(1, -(d as isize));
        }
    }
    let preferred_index = targets
        .iter()
        .position(|t| *t == preferred_target)
        .unwrap_or(0);

    // The slice's first node at each open depth.
    let mut left: Vec<Option<Node>> = Vec::new();
    let mut content = slice.content.clone();
    for i in 0..=slice.open_start {
        let node = content.maybe_child(0).cloned();
        if i < slice.open_start
            && let Some(n) = &node
        {
            content = n.content().clone();
        }
        left.push(node);
    }
    // Back the preferred depth up over defining nodes directly above it, so
    // that the defining node comes along; a non-defining textblock is skipped.
    let mut preferred_depth = slice.open_start;
    let context = r_from.node(
        (preferred_target.unsigned_abs())
            .saturating_sub(1)
            .min(r_from.depth()),
    );
    for d in (0..slice.open_start).rev() {
        let Some(node) = &left[d] else { break };
        let defines = node.node_type().spec().defining;
        if defines && !node.same_markup(context) {
            preferred_depth = d;
        } else if defines || !node.is_textblock() {
            break;
        }
    }

    let levels = slice.open_start + 1;
    for j in (0..levels).rev() {
        let open_depth = (j + preferred_depth + 1) % levels;
        let Some(insert) = &left[open_depth] else {
            continue;
        };
        for i in 0..targets.len() {
            let target = targets[(i + preferred_index) % targets.len()];
            let (depth, expand) = (target.unsigned_abs(), target > 0);
            let Some(parent_depth) = depth.checked_sub(1) else {
                continue;
            };
            if parent_depth > r_from.depth() {
                continue;
            }
            let parent = r_from.node(parent_depth);
            if !can_insert(parent, r_from.index(parent_depth), insert) {
                continue;
            }
            let start = if depth > r_from.depth() {
                Some(from)
            } else {
                r_from.before(depth)
            };
            let end = if expand { r_to.after(depth) } else { Some(to) };
            // The slice's start is closed from `open_depth` down.
            let closes = left[open_depth..slice.open_start]
                .iter()
                .all(|n| n.as_ref().is_some_and(content_valid));
            if let (Some(start), Some(end), true) = (start, end, closes) {
                out.push((
                    start,
                    end,
                    Slice::new(slice.content.clone(), open_depth, slice.open_end),
                ));
            }
        }
    }

    // Nothing takes the slice's start as it is: fit it at the range, then at
    // each covered node, outermost first.
    let (mut a, mut b) = (from, to);
    out.push((a, b, slice.clone()));
    for target in targets.iter().rev() {
        if *target < 0 {
            continue;
        }
        let d = *target as usize;
        if let (Some(before), Some(after)) = (r_from.before(d), r_to.after(d)) {
            (a, b) = (before, after);
            out.push((a, b, slice.clone()));
        }
    }
    out
}
