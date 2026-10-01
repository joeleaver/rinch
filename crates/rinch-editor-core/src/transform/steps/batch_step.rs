//! [`BatchStep`] — many disjoint replaces and attribute changes as one step.
//!
//! A table command edits every row it touches: a column insert puts a cell in
//! each row, a split fills each row the cell spanned. As one [`ReplaceStep`]
//! per row, each step rebuilt the table's row list, the transform kept the
//! document from before every step, and each position was mapped through every
//! step before it — quadratic in the rows, time and memory alike (#1200:
//! `addColumnBefore` on 16,000 rows took 6.4 s and 2.1 GB).
//!
//! A `BatchStep` is those edits as one step, stated in the coordinates of the
//! document it applies to. It rebuilds each node on the way to an edit once,
//! sharing every subtree no edit reaches, so its cost is the nodes it edits
//! plus their ancestors' child lists. Its [`StepMap`] has one range per
//! replace, which maps a position exactly as the equivalent sequence of
//! [`ReplaceStep`]s did: a caret in a row the step did not touch stays where
//! it was. ProseMirror has no such step (prosemirror-tables takes one step per
//! row); rinch's `Fragment` is a flat vector, not ProseMirror's, and pays the
//! row list per step where ProseMirror's copy is the same cost — the batch is
//! what keeps a table command linear here.
//!
//! **Semantics.** Replaces are disjoint ranges of the original document,
//! applied as if one by one from the last to the first, so no edit moves
//! another's positions; several inserts at one position land in the order
//! given. An attribute change names the node that starts at its position in the
//! original document (of two changes of one attribute of one node, the last
//! given wins); an insert at that position goes before the node, and a
//! replace may not cover it. Unlike a sequence of steps, the content of each
//! rebuilt node is checked once, on the result: an intermediate state a
//! sequence would have passed through is never built.

use crate::Slice;
use crate::model::{AttrValue, Fragment, Node};
use crate::transform::replace::close;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};
use crate::transform::steps::{ReplaceStep, SetNodeAttrStep};
use std::any::Any;

/// One edit of a [`BatchStep`], in the coordinates of the document the step
/// applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchEdit {
    /// Replace `from..to` with `slice` (a [`ReplaceStep`]'s edit).
    Replace {
        /// Start of the replaced range.
        from: usize,
        /// End of the replaced range.
        to: usize,
        /// The content to put in its place.
        slice: Slice,
    },
    /// Set (`Some`) or remove (`None`) the attribute `attr` of the node that
    /// starts at `pos` (a [`SetNodeAttrStep`]'s edit).
    SetAttr {
        /// Position before the target node.
        pos: usize,
        /// The attribute name.
        attr: String,
        /// The new value, or `None` to remove the attribute.
        value: Option<AttrValue>,
    },
}

impl BatchEdit {
    /// Insert `content` (closed) at `pos`.
    pub fn insert(pos: usize, content: Fragment) -> BatchEdit {
        BatchEdit::replace(pos, pos, content)
    }

    /// Delete `from..to`.
    pub fn delete(from: usize, to: usize) -> BatchEdit {
        BatchEdit::Replace {
            from,
            to,
            slice: Slice::empty(),
        }
    }

    /// Replace `from..to` with the closed `content`.
    pub fn replace(from: usize, to: usize, content: Fragment) -> BatchEdit {
        BatchEdit::Replace {
            from,
            to,
            slice: Slice::from_fragment(content),
        }
    }

    /// Set the attribute `attr` of the node at `pos` to `value`.
    pub fn set_attr(pos: usize, attr: impl Into<String>, value: AttrValue) -> BatchEdit {
        BatchEdit::SetAttr {
            pos,
            attr: attr.into(),
            value: Some(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rep {
    from: usize,
    to: usize,
    slice: Slice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attr {
    pos: usize,
    attr: String,
    value: Option<AttrValue>,
}

/// Disjoint replaces and attribute changes applied as one step (see the
/// module docs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchStep {
    /// Sorted by `(from, to)` and disjoint. Inserts at one point, and
    /// deletions that meet end to end, are merged into one range.
    reps: Vec<Rep>,
    /// Sorted by `pos`; none inside a replaced range.
    attrs: Vec<Attr>,
}

fn closed(slice: &Slice) -> bool {
    slice.open_start == 0 && slice.open_end == 0
}

/// Whether two replaces that meet end to end can be one range that maps every
/// position as the two did one at a time: inserts at one point, or deletions.
/// Any other pair that meets is refused. One step map with both ranges stops
/// at the first for their shared point, where one at a time can leave it
/// between them; and the inverse of such a pair need not apply as a batch
/// (an open slice restored first moves the point the other is restored at).
fn mergeable(a: &Rep, b: &Rep) -> bool {
    let inserts = a.from == a.to && b.from == b.to && closed(&a.slice) && closed(&b.slice);
    let deletions = a.slice.size() == 0 && b.slice.size() == 0;
    inserts || deletions
}

impl BatchStep {
    /// Build a step from `edits`, in any order (inserts at one position keep
    /// theirs). Inserts at one point become one range, and so do deletions
    /// that meet end to end; the step's [`StepMap`] then maps every position
    /// as the edits applied one at a time do. Fails when two replaces overlap
    /// or meet end to end otherwise, a replace's `from` is past its `to`, or an
    /// attribute change names a node a replace removes.
    pub fn new(edits: Vec<BatchEdit>) -> Result<BatchStep, StepError> {
        let mut reps = Vec::new();
        let mut attrs = Vec::new();
        for edit in edits {
            match edit {
                BatchEdit::Replace { from, to, slice } => {
                    if from > to {
                        return Err(StepError::new("batch replace ends before it starts"));
                    }
                    reps.push(Rep { from, to, slice });
                }
                BatchEdit::SetAttr { pos, attr, value } => attrs.push(Attr { pos, attr, value }),
            }
        }
        reps.sort_by_key(|r| (r.from, r.to));
        // Two changes of one attribute of one node: the last given wins, as
        // it does one step after the other. Keeping one makes the step mean
        // the same on both of `rebuild`'s paths (the splice applies changes
        // in order, `one_by_one` from the last).
        attrs.sort_by_key(|a| a.pos);
        attrs.reverse();
        let mut seen = std::collections::HashSet::new();
        attrs.retain(|a| seen.insert((a.pos, a.attr.clone())));
        attrs.reverse();
        let mut merged: Vec<Rep> = Vec::with_capacity(reps.len());
        for rep in reps {
            match merged.last_mut() {
                Some(last) if last.to > rep.from => {
                    return Err(StepError::new("batch replaces overlap"));
                }
                Some(last) if last.to == rep.from && !mergeable(last, &rep) => {
                    return Err(StepError::new("batch replaces meet end to end"));
                }
                Some(last) if last.to == rep.from => {
                    last.to = rep.to;
                    last.slice = Slice::new(last.slice.content.append(&rep.slice.content), 0, 0);
                }
                _ => merged.push(rep),
            }
        }
        for a in &attrs {
            // The last replace starting at or before `pos` is the only one
            // that can cover it: the ones before it end where it starts.
            let k = merged.partition_point(|r| r.from <= a.pos);
            if k > 0 && a.pos < merged[k - 1].to {
                return Err(StepError::new("batch attribute change on a replaced node"));
            }
        }
        Ok(BatchStep {
            reps: merged,
            attrs,
        })
    }

    /// The number of replaced ranges (after merging) and attribute changes.
    pub fn len(&self) -> usize {
        self.reps.len() + self.attrs.len()
    }

    /// Whether the step changes nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where each edit of one node's content goes: into one of its children, or
/// onto the node itself.
struct Parts {
    /// Replaces whose range is not inside one child's content.
    level: Vec<usize>,
    /// `(child index, attr index)`: attribute changes on a child.
    level_attrs: Vec<(usize, usize)>,
    /// `(child index, child content offset, replaces, attrs)`, by child.
    groups: Vec<(usize, usize, Vec<usize>, Vec<usize>)>,
    /// Every level replace starts and ends between children and carries a
    /// closed slice of non-text nodes, so the node's children can be spliced
    /// in one pass.
    splice: bool,
}

fn can_hold(child: &Node) -> bool {
    !child.is_text() && !child.is_leaf()
}

/// Sort the edits `reps`/`attrs` (indices into the step's lists, positions
/// absolute) of `node`, whose content starts at `offset`.
fn partition(
    node: &Node,
    offset: usize,
    reps: &[Rep],
    rep_ix: &[usize],
    attrs: &[Attr],
    attr_ix: &[usize],
) -> Result<Parts, StepError> {
    let children = node.content().children();
    let n = children.len();
    let end = offset + node.content().size();
    let mut parts = Parts {
        level: Vec::new(),
        level_attrs: Vec::new(),
        groups: Vec::new(),
        splice: !node.is_textblock(),
    };
    let group_for =
        |groups: &mut Vec<(usize, usize, Vec<usize>, Vec<usize>)>, i: usize, cs: usize| {
            if groups.last().map(|g| g.0) != Some(i) {
                groups.push((i, cs + 1, Vec::new(), Vec::new()));
            }
            groups.len() - 1
        };
    // Is `pos` between two children (or at either end)? `i`/`cs` walk forward.
    let boundary = |mut i: usize, mut cs: usize, pos: usize| -> bool {
        while i < n && cs + children[i].node_size() <= pos {
            cs += children[i].node_size();
            i += 1;
        }
        pos == cs || (i == n && pos == end)
    };

    // Replaces and attrs are merged by position so each walk moves forward.
    let (mut i, mut cs) = (0usize, offset);
    let (mut ri, mut ai) = (0usize, 0usize);
    loop {
        let rep_pos = rep_ix.get(ri).map(|&k| reps[k].from);
        let attr_pos = attr_ix.get(ai).map(|&k| attrs[k].pos);
        let (pos, is_rep) = match (rep_pos, attr_pos) {
            (None, None) => break,
            (Some(r), Some(a)) => {
                if r <= a {
                    (r, true)
                } else {
                    (a, false)
                }
            }
            (Some(r), None) => (r, true),
            (None, Some(a)) => (a, false),
        };
        if pos < offset || pos > end {
            return Err(StepError::new("batch edit outside the document"));
        }
        while i < n && cs + children[i].node_size() <= pos {
            cs += children[i].node_size();
            i += 1;
        }
        if is_rep {
            let k = rep_ix[ri];
            ri += 1;
            let rep = &reps[k];
            if rep.to > end {
                return Err(StepError::new("batch edit outside the document"));
            }
            // A closed slice rebuilds no node above the one holding its
            // ends; an open one can split the child itself, so it is the
            // level's (and makes the level go one edit at a time).
            let inside = i < n
                && can_hold(&children[i])
                && closed(&rep.slice)
                && rep.from > cs
                && rep.to < cs + children[i].node_size();
            if inside {
                let g = group_for(&mut parts.groups, i, cs);
                parts.groups[g].2.push(k);
            } else {
                parts.level.push(k);
                let clean = closed(&rep.slice)
                    && !rep.slice.content.children().iter().any(Node::is_text)
                    && pos == cs
                    && boundary(i, cs, rep.to);
                parts.splice &= clean;
            }
        } else {
            let k = attr_ix[ai];
            ai += 1;
            if i == n {
                return Err(StepError::new("no node at attr step position"));
            }
            if pos == cs {
                if children[i].is_text() {
                    return Err(StepError::new("cannot set attributes on a text node"));
                }
                parts.level_attrs.push((i, k));
            } else if can_hold(&children[i]) && pos < cs + children[i].node_size() - 1 {
                let g = group_for(&mut parts.groups, i, cs);
                parts.groups[g].3.push(k);
            } else if children[i].is_text() {
                return Err(StepError::new("cannot set attributes on a text node"));
            } else {
                return Err(StepError::new("no node at attr step position"));
            }
        }
    }
    Ok(parts)
}

/// `attr` applied to `node` (not a text node).
fn with_attr(node: &Node, attr: &Attr) -> Node {
    let attrs = match &attr.value {
        Some(v) => node.attrs().with(attr.attr.clone(), v.clone()),
        None => node.attrs().without(&attr.attr),
    };
    node.with_attrs(attrs)
}

/// `node` (content at `offset`) with the edits `rep_ix`/`attr_ix` applied.
fn rebuild(
    node: &Node,
    offset: usize,
    reps: &[Rep],
    rep_ix: &[usize],
    attrs: &[Attr],
    attr_ix: &[usize],
) -> Result<Node, StepError> {
    if rep_ix.is_empty() && attr_ix.is_empty() {
        return Ok(node.clone());
    }
    let parts = partition(node, offset, reps, rep_ix, attrs, attr_ix)?;
    if !parts.splice {
        return one_by_one(node, offset, reps, rep_ix, attrs, attr_ix);
    }
    let children = node.content().children();
    let n = children.len();
    let mut out: Vec<Node> = Vec::with_capacity(n);
    let (mut i, mut pos) = (0usize, offset);
    let (mut li, mut gi, mut ai) = (0usize, 0usize, 0usize);
    loop {
        while li < parts.level.len() && reps[parts.level[li]].from == pos {
            let rep = &reps[parts.level[li]];
            li += 1;
            out.extend(rep.slice.content.children().iter().cloned());
            while pos < rep.to {
                pos += children[i].node_size();
                i += 1;
            }
        }
        if i == n {
            break;
        }
        let mut child = children[i].clone();
        if gi < parts.groups.len() && parts.groups[gi].0 == i {
            let (_, child_offset, g_reps, g_attrs) = &parts.groups[gi];
            child = rebuild(&child, *child_offset, reps, g_reps, attrs, g_attrs)?;
            gi += 1;
        }
        while ai < parts.level_attrs.len() && parts.level_attrs[ai].0 == i {
            child = with_attr(&child, &attrs[parts.level_attrs[ai].1]);
            ai += 1;
        }
        out.push(child);
        pos += children[i].node_size();
        i += 1;
    }
    close(node, Fragment::from_children(out))
}

/// The edits applied to `node` one at a time, from the last position to the
/// first, as the equivalent [`ReplaceStep`]s and [`SetNodeAttrStep`]s would be
/// (an attribute change before an insert at its own position, so it names the
/// original node). For a node whose children cannot simply be spliced: a
/// textblock, or a replace that cuts into a child or carries an open slice.
fn one_by_one(
    node: &Node,
    offset: usize,
    reps: &[Rep],
    rep_ix: &[usize],
    attrs: &[Attr],
    attr_ix: &[usize],
) -> Result<Node, StepError> {
    // `rep_ix`/`attr_ix` hold every edit inside `node`, deepest ones
    // included, in position order.
    let (all_reps, all_attrs) = (rep_ix, attr_ix);
    let mut doc = node.clone();
    let (mut r, mut a) = (all_reps.len(), all_attrs.len());
    while r > 0 || a > 0 {
        let take_attr =
            a > 0 && (r == 0 || attrs[all_attrs[a - 1]].pos >= reps[all_reps[r - 1]].from);
        if take_attr {
            a -= 1;
            let attr = &attrs[all_attrs[a]];
            let step = SetNodeAttrStep {
                pos: attr.pos - offset,
                attr: attr.attr.clone(),
                value: attr.value.clone(),
            };
            doc = step.apply(&doc)?;
        } else {
            r -= 1;
            let rep = &reps[all_reps[r]];
            let step = ReplaceStep::new(rep.from - offset, rep.to - offset, rep.slice.clone());
            doc = step.apply(&doc)?;
        }
    }
    Ok(doc)
}

/// What each edit finds in the document before the step: the content each
/// replace removes, and each attribute's prior value.
#[allow(clippy::too_many_arguments)]
fn originals(
    node: &Node,
    offset: usize,
    reps: &[Rep],
    rep_ix: &[usize],
    attrs: &[Attr],
    attr_ix: &[usize],
    slices: &mut [Option<Slice>],
    priors: &mut [Option<AttrValue>],
) -> Result<(), StepError> {
    if rep_ix.is_empty() && attr_ix.is_empty() {
        return Ok(());
    }
    let parts = partition(node, offset, reps, rep_ix, attrs, attr_ix)?;
    for &k in &parts.level {
        let rep = &reps[k];
        let slice = node
            .slice(rep.from - offset, rep.to - offset)
            .map_err(|e| StepError::new(e.to_string()))?;
        slices[k] = Some(slice);
    }
    for &(i, k) in &parts.level_attrs {
        priors[k] = node.child(i).attrs().get(&attrs[k].attr).cloned();
    }
    for (i, child_offset, g_reps, g_attrs) in &parts.groups {
        originals(
            node.child(*i),
            *child_offset,
            reps,
            g_reps,
            attrs,
            g_attrs,
            slices,
            priors,
        )?;
    }
    Ok(())
}

impl BatchStep {
    fn all_indices(&self) -> (Vec<usize>, Vec<usize>) {
        (
            (0..self.reps.len()).collect(),
            (0..self.attrs.len()).collect(),
        )
    }
}

impl Step for BatchStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let (rep_ix, attr_ix) = self.all_indices();
        rebuild(doc, 0, &self.reps, &rep_ix, &self.attrs, &attr_ix)
    }

    fn get_map(&self) -> StepMap {
        let mut ranges = Vec::with_capacity(self.reps.len() * 3);
        for rep in &self.reps {
            ranges.extend([rep.from, rep.to - rep.from, rep.slice.size()]);
        }
        StepMap::new(ranges)
    }

    fn invert(&self, doc: &Node) -> Box<dyn Step> {
        let (rep_ix, attr_ix) = self.all_indices();
        let mut slices = vec![None; self.reps.len()];
        let mut priors = vec![None; self.attrs.len()];
        originals(
            doc,
            0,
            &self.reps,
            &rep_ix,
            &self.attrs,
            &attr_ix,
            &mut slices,
            &mut priors,
        )
        .expect("invert: the step must be valid in the pre-step document");
        let mut edits = Vec::with_capacity(self.len());
        // `delta`: how far the replaces before a position moved it.
        let mut delta: isize = 0;
        let mut r = 0;
        for (a, attr) in self.attrs.iter().enumerate() {
            while r < self.reps.len() && self.reps[r].to <= attr.pos {
                let rep = &self.reps[r];
                edits.push(inverse_rep(rep, slices[r].take(), delta));
                delta += rep.slice.size() as isize - (rep.to - rep.from) as isize;
                r += 1;
            }
            edits.push(BatchEdit::SetAttr {
                pos: (attr.pos as isize + delta) as usize,
                attr: attr.attr.clone(),
                value: priors[a].take(),
            });
        }
        while r < self.reps.len() {
            let rep = &self.reps[r];
            edits.push(inverse_rep(rep, slices[r].take(), delta));
            delta += rep.slice.size() as isize - (rep.to - rep.from) as isize;
            r += 1;
        }
        Box::new(BatchStep::new(edits).expect("invert: the inverse edits are disjoint"))
    }

    /// Each replaced range mapped as one [`ReplaceStep`] of it would be, and
    /// each attribute change as a [`SetNodeAttrStep`], dropping what those
    /// would drop. A range is what [`BatchStep::new`] made of the edits:
    /// inserts at one point, and deletions that meet, are one range, so a
    /// concurrent insert between two such deletions is deleted with them where
    /// two separate deletions would keep it. Two kept ranges the mapping brings
    /// to meet end to end (it deleted what lay between them) are merged when
    /// [`mergeable`], folded into one replace of both ranges by both slices
    /// when both slices are closed — the document the two make in turn — and
    /// otherwise (an open slice) the later one is dropped and **its content is
    /// lost**, where separate steps would apply it. An attribute change on a
    /// node a kept range covers after mapping is dropped too; a separate step
    /// would have set it before the replace removed or kept the node.
    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let mut reps: Vec<Rep> = Vec::with_capacity(self.reps.len());
        for rep in &self.reps {
            let from = mapping.map_result(rep.from, 1);
            let to = mapping.map_result(rep.to, -1);
            if from.deleted_across() && to.deleted_across() {
                continue;
            }
            let rep = Rep {
                from: from.pos,
                to: from.pos.max(to.pos),
                slice: rep.slice.clone(),
            };
            // Mapping is monotonic, so a kept replace can meet the one
            // before it but never overlap it.
            match reps.last_mut() {
                Some(last) if last.to == rep.from && mergeable(last, &rep) => {
                    last.to = rep.to;
                    last.slice = Slice::new(last.slice.content.append(&rep.slice.content), 0, 0);
                }
                Some(last) if last.to == rep.from => {
                    if closed(&last.slice) && closed(&rep.slice) {
                        last.to = rep.to;
                        last.slice =
                            Slice::new(last.slice.content.append(&rep.slice.content), 0, 0);
                    }
                }
                _ => reps.push(rep),
            }
        }
        let ranges: Vec<(usize, usize)> = reps.iter().map(|r| (r.from, r.to)).collect();
        let mut edits: Vec<BatchEdit> = reps
            .into_iter()
            .map(|r| BatchEdit::Replace {
                from: r.from,
                to: r.to,
                slice: r.slice,
            })
            .collect();
        for attr in &self.attrs {
            let pos = mapping.map_result(attr.pos, 1);
            if pos.deleted_after() {
                continue;
            }
            // `BatchStep::new`'s rule: the last range starting at or before
            // the node is the only one that can cover it. Reached when one
            // step map has adjacent ranges: the position ends one range, the
            // next deletes what follows it, and the result reports no
            // deletion after it (`tests/batch_step_map.rs`,
            // `attr_guard_is_reachable`). Without it `new` refuses the step.
            let k = ranges.partition_point(|&(f, _)| f <= pos.pos);
            if k > 0 && pos.pos < ranges[k - 1].1 {
                continue;
            }
            edits.push(BatchEdit::SetAttr {
                pos: pos.pos,
                attr: attr.attr.clone(),
                value: attr.value.clone(),
            });
        }
        if edits.is_empty() {
            return None;
        }
        BatchStep::new(edits)
            .ok()
            .map(|s| Box::new(s) as Box<dyn Step>)
    }

    fn clone_box(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn inverse_rep(rep: &Rep, recovered: Option<Slice>, delta: isize) -> BatchEdit {
    let from = (rep.from as isize + delta) as usize;
    BatchEdit::Replace {
        from,
        to: from + rep.slice.size(),
        slice: recovered.expect("invert: every replace was located"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Attrs;
    use crate::schema::Schema;
    use crate::transform::step_map::Mapping;

    /// xorshift64*: deterministic, no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn para(s: &Schema, text: &str) -> Node {
        let content = if text.is_empty() {
            Fragment::empty()
        } else {
            Fragment::from_node(s.text(text).unwrap())
        };
        s.branch("paragraph", content).unwrap()
    }

    fn cell(s: &Schema, text: &str) -> Node {
        s.create_node(
            "table_cell",
            Attrs::new(),
            Fragment::from_node(para(s, text)),
        )
        .unwrap()
    }

    /// Paragraphs, a heading, a list and a table: nodes at several depths.
    fn doc(s: &Schema, rng: &mut Rng) -> Node {
        let mut blocks = Vec::new();
        for b in 0..1 + rng.below(4) {
            match rng.below(4) {
                0 => blocks.push(para(s, &"ab".repeat(rng.below(3)))),
                1 => blocks.push(
                    s.create_node(
                        "heading",
                        Attrs::from_iter([("level", AttrValue::Int(1))]),
                        Fragment::from_node(s.text("h").unwrap()),
                    )
                    .unwrap(),
                ),
                2 => {
                    let item = s
                        .create_node(
                            "list_item",
                            Attrs::new(),
                            Fragment::from_node(para(s, "li")),
                        )
                        .unwrap();
                    blocks.push(
                        s.create_node("bullet_list", Attrs::new(), Fragment::from_node(item))
                            .unwrap(),
                    )
                }
                _ => {
                    let rows = (0..1 + rng.below(3))
                        .map(|r| {
                            let cells = (0..1 + rng.below(3))
                                .map(|c| cell(s, if (r + c + b) % 2 == 0 { "x" } else { "" }))
                                .collect();
                            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                                .unwrap()
                        })
                        .collect();
                    blocks.push(
                        s.create_node("table", Attrs::new(), Fragment::from_children(rows))
                            .unwrap(),
                    )
                }
            }
        }
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }

    /// Random disjoint edits of `doc`, adjacent ones included.
    fn edits(s: &Schema, rng: &mut Rng, doc: &Node) -> Vec<BatchEdit> {
        let size = doc.content_size();
        let mut points: Vec<usize> = (0..2 * (1 + rng.below(4)))
            .map(|_| rng.below(size + 1))
            .collect();
        points.sort_unstable();
        let mut out = Vec::new();
        for pair in points.chunks(2) {
            let (from, mut to) = (pair[0], pair[1]);
            if rng.below(3) == 0 {
                to = from;
            }
            let content = match rng.below(5) {
                0 => Fragment::from_node(s.text("zz").unwrap()),
                1 => Fragment::from_node(para(s, "p")),
                2 => Fragment::from_node(cell(s, "c")),
                _ => Fragment::empty(),
            };
            if rng.below(4) == 0 {
                out.push(BatchEdit::SetAttr {
                    pos: from,
                    attr: "level".into(),
                    value: Some(AttrValue::Int(2)),
                });
            } else {
                out.push(BatchEdit::replace(from, to, content));
            }
        }
        out
    }

    /// The edits one at a time from the last position to the first: what a
    /// batch means.
    fn one_at_a_time(doc: &Node, step: &BatchStep) -> Result<(Node, Mapping), StepError> {
        let mut steps: Vec<(usize, bool, Box<dyn Step>)> = Vec::new();
        for (i, r) in step.reps.iter().enumerate() {
            steps.push((
                r.from,
                false,
                Box::new(ReplaceStep::new(r.from, r.to, r.slice.clone())),
            ));
            let _ = i;
        }
        for a in &step.attrs {
            steps.push((
                a.pos,
                true,
                Box::new(SetNodeAttrStep {
                    pos: a.pos,
                    attr: a.attr.clone(),
                    value: a.value.clone(),
                }),
            ));
        }
        // Descending position; at one position an attribute change first,
        // then the replaces from last to first.
        let n_reps = step.reps.len();
        let mut order: Vec<usize> = (0..steps.len()).collect();
        order.sort_by(|&x, &y| {
            let kx = (steps[x].0, steps[x].1, if x < n_reps { x } else { 0 });
            let ky = (steps[y].0, steps[y].1, if y < n_reps { y } else { 0 });
            ky.cmp(&kx)
        });
        let mut doc = doc.clone();
        let mut mapping = Mapping::new();
        for i in order {
            doc = steps[i].2.apply(&doc)?;
            mapping.append_map(steps[i].2.get_map());
        }
        Ok((doc, mapping))
    }

    /// The edits as separate steps, each mapped over `over` on its own (the
    /// steps that map to nothing dropped), applied to `doc` (the document
    /// `over` leads to) from the last position to the first. Mapping keeps
    /// the order of positions, so the order is the original one.
    fn sequence_mapped(doc: &Node, step: &BatchStep, over: &Mapping) -> Result<Node, StepError> {
        type Keyed = ((usize, bool, usize), Box<dyn Step>);
        let mut keyed: Vec<Keyed> = Vec::new();
        for (i, r) in step.reps.iter().enumerate() {
            let s = ReplaceStep::new(r.from, r.to, r.slice.clone());
            keyed.push(((r.from, false, i), Box::new(s)));
        }
        for a in &step.attrs {
            let s = SetNodeAttrStep {
                pos: a.pos,
                attr: a.attr.clone(),
                value: a.value.clone(),
            };
            keyed.push(((a.pos, true, 0), Box::new(s)));
        }
        keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
        let mut doc = doc.clone();
        for (_, s) in keyed {
            if let Some(m) = s.map(over) {
                doc = m.apply(&doc)?;
            }
        }
        Ok(doc)
    }

    /// A batch applies as its edits one at a time, maps positions as they
    /// did, inverts, and shares the subtrees it does not edit.
    #[test]
    fn a_batch_is_its_edits_one_at_a_time() {
        let s = Schema::starter_kit();
        let mut rng = Rng(0x1200_ba7c_0000_0001);
        let (mut same, mut stricter, mut refused, mut mapped) = (0, 0, 0, 0);
        for case in 0..20000 {
            let doc = doc(&s, &mut rng);
            let Ok(step) = BatchStep::new(edits(&s, &mut rng, &doc)) else {
                refused += 1;
                continue;
            };
            let got = step.apply(&doc);
            match (got, one_at_a_time(&doc, &step)) {
                (Ok(g), Ok((w, mapping))) => {
                    assert!(g == w, "case {case}: {step:?}\n got {g:?}\n want {w:?}");
                    let map = step.get_map();
                    for p in 0..=doc.content_size() {
                        for a in [-1, 1] {
                            assert_eq!(
                                map.map(p, a),
                                mapping.map(p, a),
                                "case {case}: {p} {a} {step:?}"
                            );
                        }
                    }
                    let inv = step.invert(&doc);
                    let back = inv.apply(&g).unwrap_or_else(|e| {
                        panic!("case {case}: {e} {step:?} inverse {inv:?} doc {doc:?}")
                    });
                    assert!(back == doc, "case {case}: invert {step:?}");
                    same += 1;
                    // Mapped over another change (a random deletion), the
                    // batch makes the document its edits make as separate
                    // steps mapped one by one, wherever both apply.
                    let size = doc.content_size();
                    let x = rng.below(size + 1);
                    let y = (x + rng.below(5)).min(size);
                    let other = ReplaceStep::new(x, y, Slice::empty());
                    if let Ok(doc1) = other.apply(&doc) {
                        let mut over = Mapping::new();
                        over.append_map(other.get_map());
                        let seq = sequence_mapped(&doc1, &step, &over);
                        match (step.map(&over), seq) {
                            (Some(m), Ok(w)) => {
                                let got = m.apply(&doc1).unwrap_or_else(|e| {
                                    panic!("case {case}: {m:?} refused ({e}); the edits apply")
                                });
                                assert!(
                                    got == w,
                                    "case {case}: mapped over {x}..{y}: {step:?}\n \
                                     as {m:?}\n got {got:?}\n want {w:?}"
                                );
                                mapped += 1;
                            }
                            (None, Ok(w)) => assert!(
                                w == doc1,
                                "case {case}: the batch mapped over {x}..{y} to nothing, \
                                 where its edits change the document: {step:?}"
                            ),
                            _ => {}
                        }
                    }
                }
                // The batch checks each node's content once, on the result;
                // one at a time can pass through an invalid state.
                (Ok(_), Err(_)) => stricter += 1,
                (Err(_), Ok((w, _))) => {
                    panic!("case {case}: the batch refused {step:?}, giving {w:?}")
                }
                (Err(_), Err(_)) => refused += 1,
            }
        }
        eprintln!(
            "same {same} (mapped {mapped}), one at a time refused {stricter}, \
             both refused {refused}"
        );
        assert!(same > 2000, "positive control: {same}");
        assert!(mapped > 600, "positive control: {mapped} mapped");
    }

    #[test]
    fn inserts_at_one_position_keep_their_order_and_merge() {
        let s = Schema::starter_kit();
        let doc = s
            .branch(
                "doc",
                Fragment::from_children(vec![para(&s, "a"), para(&s, "b")]),
            )
            .unwrap();
        let step = BatchStep::new(vec![
            BatchEdit::insert(3, Fragment::from_node(para(&s, "1"))),
            BatchEdit::delete(4, 5),
            BatchEdit::insert(3, Fragment::from_node(para(&s, "2"))),
        ])
        .unwrap();
        // The two inserts are one range; the deletion (the text "b") its own.
        assert_eq!(step.len(), 2);
        let got = step.apply(&doc).unwrap();
        let texts: Vec<String> = got
            .content()
            .children()
            .iter()
            .map(|p| {
                p.content()
                    .maybe_child(0)
                    .and_then(Node::text)
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        assert_eq!(texts, ["a", "1", "2", ""]);
        // The inserts map their point as one after the other did.
        assert_eq!(step.get_map().map(3, -1), 3);
        assert_eq!(step.get_map().map(3, 1), 9);
        assert_eq!(step.get_map().map(4, -1), 10);
        // Deletions end to end are one range, and map as two did.
        let del = BatchStep::new(vec![BatchEdit::delete(0, 3), BatchEdit::delete(3, 6)]).unwrap();
        assert_eq!(del.len(), 1);
        assert!(del.get_map().map_result(3, -1).deleted());
    }

    #[test]
    fn overlaps_and_attributes_on_replaced_nodes_are_refused() {
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), BatchEdit::delete(4, 6)]).is_err());
        let p = || Fragment::from_node(para(&Schema::starter_kit(), "p"));
        // Meeting end to end: only inserts at a point, or deletions.
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), BatchEdit::insert(5, p())]).is_err());
        assert!(BatchStep::new(vec![BatchEdit::insert(2, p()), BatchEdit::delete(2, 5)]).is_err());
        assert!(
            BatchStep::new(vec![BatchEdit::replace(2, 5, p()), BatchEdit::delete(5, 6)]).is_err()
        );
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), BatchEdit::delete(5, 6)]).is_ok());
        assert!(BatchStep::new(vec![BatchEdit::delete(5, 2)]).is_err());
        let attr = |pos| BatchEdit::set_attr(pos, "level", AttrValue::Int(2));
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), attr(2)]).is_err());
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), attr(4)]).is_err());
        // At the end of a deleted range, or under an insert, the node stays.
        assert!(BatchStep::new(vec![BatchEdit::delete(2, 5), attr(5)]).is_ok());
        assert!(BatchStep::new(vec![BatchEdit::insert(2, Fragment::empty()), attr(2)]).is_ok());
    }

    /// An attribute change names the node that started at its position, not
    /// what an insert there put before it.
    #[test]
    fn an_attribute_change_names_the_original_node() {
        let s = Schema::starter_kit();
        let heading = s
            .create_node(
                "heading",
                Attrs::from_iter([("level", AttrValue::Int(1))]),
                Fragment::from_node(s.text("h").unwrap()),
            )
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_node(heading.clone()))
            .unwrap();
        let other = s
            .create_node(
                "heading",
                Attrs::from_iter([("level", AttrValue::Int(5))]),
                Fragment::empty(),
            )
            .unwrap();
        for edits in [
            vec![
                BatchEdit::set_attr(0, "level", AttrValue::Int(3)),
                BatchEdit::insert(0, Fragment::from_node(other.clone())),
            ],
            vec![
                BatchEdit::insert(0, Fragment::from_node(other.clone())),
                BatchEdit::set_attr(0, "level", AttrValue::Int(3)),
            ],
        ] {
            let got = BatchStep::new(edits).unwrap().apply(&doc).unwrap();
            let levels: Vec<_> = got
                .content()
                .children()
                .iter()
                .map(|h| h.attrs().get_int("level"))
                .collect();
            assert_eq!(levels, [Some(5), Some(3)]);
        }
    }

    /// Mapped over a change before them, the edits move with it; mapped over
    /// the deletion of a replaced range, that replace is dropped.
    #[test]
    fn a_batch_maps_over_other_changes() {
        let step = BatchStep::new(vec![
            BatchEdit::delete(10, 12),
            BatchEdit::set_attr(20, "level", AttrValue::Int(2)),
        ])
        .unwrap();
        let mut over = Mapping::new();
        over.append_map(StepMap::new(vec![0, 0, 3]));
        let moved = step.map(&over).unwrap();
        let moved = moved.as_any().downcast_ref::<BatchStep>().unwrap();
        assert_eq!(
            moved,
            &BatchStep::new(vec![
                BatchEdit::delete(13, 15),
                BatchEdit::set_attr(23, "level", AttrValue::Int(2)),
            ])
            .unwrap()
        );
        let mut gone = Mapping::new();
        gone.append_map(StepMap::new(vec![9, 4, 0]));
        let kept = step.map(&gone).unwrap();
        let kept = kept.as_any().downcast_ref::<BatchStep>().unwrap();
        assert_eq!(
            kept,
            &BatchStep::new(vec![BatchEdit::set_attr(16, "level", AttrValue::Int(2))]).unwrap()
        );
    }
}
