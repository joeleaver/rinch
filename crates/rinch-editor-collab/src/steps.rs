//! The places a local transaction changed, from its step maps (#1885).
//!
//! The projection diffs a changed textblock's stored text against the model's by one
//! common prefix and suffix ([`crate::projection`]'s `text_splice_bounds`), and
//! rewrites the middle: everything between the first and the last change. That is the
//! whole change for a transaction that edits a block at one place, and too much for
//! one that edits it at two or more (an undo of non-adjacent typing, an app `update`,
//! a command that touches a line at both ends): the text between the places is
//! deleted and written again as new CRDT items, so a peer's concurrent delete in it is
//! undone and a peer's concurrent insert in it lands beside a copy of the text around
//! it.
//!
//! The transaction's step maps say exactly which positions changed. While a caller
//! hands them over ([`steps_scope`], `CollabSession::record_local_mapped`), each
//! textblock the diff reconciles in place is split into the runs of text no step
//! touched ([`step_hunks`]), and only the hunks between those runs are written.
//!
//! A run is used only where it is the same text, at the same place, in the model's
//! before and after of the block being reconciled, so whatever the scope cannot
//! describe (a block it cannot find, a run whose text moved to another block, stored
//! text the model reads differently) is diffed the old way.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use rinch_editor_core::{Mapping, Node, StepMap};

/// One changed place of a textblock: the chars `old` of its stored text become the
/// chars `new` of the model's (each in its own text's offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

struct Steps {
    before: Node,
    after: Node,
    /// The transaction's maps, in order.
    maps: Vec<StepMap>,
    /// Their inverses, in the same order (applied last to first).
    inverse: Vec<StepMap>,
    /// The ranges of `after` some step wrote or deleted at, sorted and merged where
    /// they touch. An empty range is a deletion's place.
    changed: Vec<Range<usize>>,
}

thread_local! {
    static STEPS: RefCell<Option<Rc<Steps>>> = const { RefCell::new(None) };
}

/// Restores the previous scope when dropped.
pub(crate) struct StepsScope(Option<Option<Rc<Steps>>>);

impl Drop for StepsScope {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            STEPS.with(|s| *s.borrow_mut() = previous);
        }
    }
}

/// Hold `mapping`, the steps of the transaction from `before` to `after`, for the
/// projection of that change. A transaction whose changes are one place in `after`
/// sets nothing: no block can hold two of its places, and the prefix/suffix diff is
/// already exact (every keystroke takes this branch).
pub(crate) fn steps_scope(before: &Node, after: &Node, mapping: &Mapping) -> StepsScope {
    let maps = mapping.maps();
    let triples: usize = maps.iter().map(|m| m.ranges().len() / 3).sum();
    if triples < 2 {
        return StepsScope(None);
    }
    let changed = changed_ranges(maps);
    if changed.len() < 2 {
        return StepsScope(None);
    }
    let steps = Steps {
        before: before.clone(),
        after: after.clone(),
        maps: maps.to_vec(),
        inverse: maps.iter().map(StepMap::invert).collect(),
        changed,
    };
    let previous = STEPS.with(|s| s.borrow_mut().replace(Rc::new(steps)));
    StepsScope(Some(previous))
}

/// The places of the final document some map of `maps` changed: each map's new ranges,
/// mapped forward through the maps after it, widened outward so every position no map
/// touched stays outside them.
fn changed_ranges(maps: &[StepMap]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (i, m) in maps.iter().enumerate() {
        for t in m.ranges().as_chunks::<3>().0 {
            let (start, new_size) = (t[0], t[2]);
            let (mut a, mut b) = (start, start + new_size);
            for later in &maps[i + 1..] {
                a = later.map(a, -1);
                b = later.map(b, 1);
            }
            out.push(a..b.max(a));
        }
    }
    out.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The content start of the textblock `target` (by identity) in `doc`.
fn find_block(doc: &Node, target: &Node) -> Option<usize> {
    fn walk(n: &Node, at: usize, target: &Node) -> Option<usize> {
        // `at` is the position of `n`'s content start.
        let mut pos = at;
        for i in 0..n.child_count() {
            let c = n.child(i);
            if c.same_ref(target) {
                return Some(pos + 1);
            }
            if !c.is_textblock()
                && !c.is_leaf()
                && let Some(p) = walk(c, pos + 1, target)
            {
                return Some(p);
            }
            pos += c.node_size();
        }
        None
    }
    walk(doc, 0, target)
}

/// The changed places of the textblock the model had as `before` and has as `after`
/// (stored text `o_len` chars, model text `n_len`), when the transaction in scope
/// changed it at **two or more** places that are each bounded by text no step touched.
/// `None` — diff it the usual way — when no scope is held, the block is not found in
/// the scope's documents, or the steps changed it at one place at most.
pub(crate) fn step_hunks(
    before: &Node,
    after: &Node,
    o_len: usize,
    n_len: usize,
) -> Option<Vec<Hunk>> {
    let steps = STEPS.with(|s| s.borrow().clone())?;
    if before.content_size() != o_len || after.content_size() != n_len {
        return None;
    }
    let a0 = find_block(&steps.after, after)?;
    let b0 = find_block(&steps.before, before)?;
    let (a1, b1) = (a0 + n_len, b0 + o_len);
    let back =
        |pos: usize, assoc: i32| steps.inverse.iter().rev().fold(pos, |p, m| m.map(p, assoc));
    // The runs of the block's text no step touched, each as (old offset, new offset,
    // length): the gaps between the changed ranges that meet the block.
    let mut anchors: Vec<(usize, usize, usize)> = Vec::new();
    let mut cursor = a0;
    let cut = |u: usize, v: usize, anchors: &mut Vec<(usize, usize, usize)>| {
        if v <= u {
            return;
        }
        let (bu, bv) = (back(u, 1), back(v, -1));
        if bv < bu || bv - bu != v - u || bu < b0 || bv > b1 {
            return;
        }
        // A forward check: the run maps back onto itself.
        let fwd = steps.maps.iter().fold(bu, |p, m| m.map(p, 1));
        if fwd != u {
            return;
        }
        anchors.push((bu - b0, u - a0, v - u));
    };
    for r in steps.changed.iter() {
        if r.end < a0 {
            continue;
        }
        if r.start > a1 {
            break;
        }
        let (s, e) = (r.start.clamp(a0, a1), r.end.clamp(a0, a1));
        cut(cursor, s, &mut anchors);
        cursor = cursor.max(e);
    }
    cut(cursor, a1, &mut anchors);
    // The hunks between the anchors (and the block's ends), each changing something.
    let mut hunks = Vec::new();
    let (mut po, mut pn) = (0, 0);
    for &(ao, an, len) in anchors.iter().chain(std::iter::once(&(o_len, n_len, 0))) {
        if ao < po || an < pn {
            return None;
        }
        if ao > po || an > pn {
            hunks.push(Hunk {
                old: po..ao,
                new: pn..an,
            });
        }
        po = ao + len;
        pn = an + len;
    }
    (hunks.len() >= 2).then_some(hunks)
}
