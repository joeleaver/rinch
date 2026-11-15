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
//!
//! # Cost (#2526)
//!
//! The scope folds the transaction's maps into one list of [`Segment`]s when it is
//! set — the places of the whole transaction, each with where it was in the document
//! before and where it is after — by composing halves of the map list pairwise, so a
//! transaction of `n` places costs `n log n`. A block's runs are then the gaps between
//! the segments that meet it, found by a binary search, and the block itself is found
//! by a walk of each document for the first few lookups and through an index of each
//! document's blocks after that. A replace-all over
//! every paragraph of a document costs in proportion to the document; mapping each
//! block's places back through every step and searching the document for each block
//! cost (paragraphs × steps), 10.8 s at 4,000 paragraphs.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::rc::Rc;

use rinch_editor_core::{Mapping, Node, StepMap};

use crate::projection::node_key;

/// One changed place of a textblock: the chars `old` of its stored text become the
/// chars `new` of the model's (each in its own text's offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// One place a sequence of step maps changed: positions `b0..b1` of the document
/// before them became `a0..a1` of the document after. Outside every segment the maps
/// move positions without changing what is between them, so a position in the gap
/// after a segment maps back by that segment's `b1 - a1` (and one before the first
/// segment maps to itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Segment {
    b0: usize,
    b1: usize,
    a0: usize,
    a1: usize,
}

impl Segment {
    fn growth(&self) -> isize {
        (self.a1 - self.a0) as isize - (self.b1 - self.b0) as isize
    }
}

struct Steps {
    before: Node,
    after: Node,
    /// The transaction's places, sorted, disjoint and never touching (two that touch
    /// are one). A segment empty on both sides is a deletion of what an earlier step
    /// inserted.
    segments: Vec<Segment>,
    /// How many blocks [`step_hunks`] has looked for. The first
    /// [`WALKS_BEFORE_INDEX`] are found by walking the documents up to them (a
    /// transaction that changes a few blocks costs what it did); the next builds
    /// [`Steps::blocks`].
    lookups: Cell<usize>,
    /// The content start of every node of `before` and of `after` that is not inside
    /// a textblock, by [`node_key`] (the first, where a node occurs twice).
    blocks: OnceCell<[Blocks; 2]>,
}

/// Blocks looked up by walking the documents before the rest are indexed.
const WALKS_BEFORE_INDEX: usize = 4;

/// A content start by [`node_key`].
type Blocks = HashMap<usize, usize, BuildHasherDefault<KeyHasher>>;

/// A set of [`node_key`]s.
pub(crate) type KeySet = HashSet<usize, BuildHasherDefault<KeyHasher>>;

/// A hasher for a [`node_key`], an address: one 64×64→128-bit multiply, its two
/// halves folded together. `std`'s map takes the bucket from the hash's low bits,
/// and an address's low bits are zero (alignment), so a plain multiply's low bits
/// would be too; the fold brings the high half, mixed from all of it, down into them.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_usize(&mut self, n: usize) {
        let p = u128::from(n as u64) * 0x9E37_79B9_7F4A_7C15;
        self.0 = (p as u64) ^ ((p >> 64) as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

thread_local! {
    static STEPS: RefCell<Option<Rc<Steps>>> = const { RefCell::new(None) };
    /// Work done on this thread finding a transaction's places (#2526): one per step
    /// map range and per segment the scope composes, per node indexed to find a block,
    /// per segment a block's lookup examines, and per block or inline node the
    /// top-level diff compares to tell whether a change shares one. Counted only
    /// under the `test-util` feature (this crate's own tests), so a release build
    /// pays nothing for it.
    #[cfg(feature = "test-util")]
    static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count `n` units of `WORK`; nothing without the `test-util` feature.
#[inline(always)]
pub(crate) fn count_work(n: usize) {
    #[cfg(feature = "test-util")]
    WORK.with(|c| c.set(c.get() + n as u64));
    #[cfg(not(feature = "test-util"))]
    let _ = n;
}

/// The running count behind `crate::testing::mapped_work`.
#[cfg(feature = "test-util")]
pub(crate) fn work() -> u64 {
    WORK.with(|c| c.get())
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
///
/// Nor does a mapping holding an inverted map ([`StepMap::is_inverted`], one from
/// `Mapping::invert`): [`map_segments`] reads a map's ranges as its own steps' are
/// read, so such a change is diffed the usual way. A transaction's own mapping holds
/// none; `record_local_mapped` is public, and a caller may hand it any mapping.
pub(crate) fn steps_scope(before: &Node, after: &Node, mapping: &Mapping) -> StepsScope {
    let maps = mapping.maps();
    let triples: usize = maps.iter().map(|m| m.ranges().len() / 3).sum();
    if triples < 2 || maps.iter().any(StepMap::is_inverted) {
        return StepsScope(None);
    }
    let segments = compose_all(maps);
    if segments.len() < 2 {
        return StepsScope(None);
    }
    let steps = Steps {
        before: before.clone(),
        after: after.clone(),
        segments,
        lookups: Cell::new(0),
        blocks: OnceCell::new(),
    };
    let previous = STEPS.with(|s| s.borrow_mut().replace(Rc::new(steps)));
    StepsScope(Some(previous))
}

/// The places of `maps`, applied in order, as one segment list: each half composed
/// on its own, then the two halves.
fn compose_all(maps: &[StepMap]) -> Vec<Segment> {
    match maps {
        [] => Vec::new(),
        [map] => map_segments(map),
        _ => {
            let (first, second) = maps.split_at(maps.len() / 2);
            compose(&compose_all(first), &compose_all(second))
        }
    }
}

/// One map's ranges as segments. The map is not inverted ([`steps_scope`] holds no
/// scope for a mapping with one), so each `[start, old_size, new_size]` triple's `start` is in the
/// document before the map, and the ranges before it move its new side by their
/// growth.
fn map_segments(map: &StepMap) -> Vec<Segment> {
    let mut out = Vec::with_capacity(map.ranges().len() / 3);
    let mut growth: isize = 0;
    for t in map.ranges().as_chunks::<3>().0 {
        count_work(1);
        let (start, old, new) = (t[0], t[1], t[2]);
        let a0 = (start as isize + growth) as usize;
        out.push(Segment {
            b0: start,
            b1: start + old,
            a0,
            a1: a0 + new,
        });
        growth += new as isize - old as isize;
    }
    // Ranges of one map that touch are one place.
    compose(&out, &[])
}

/// `first` (from documents X to Y) followed by `second` (Y to Z), as one segment
/// list from X to Z: every segment of either, as an interval of Y (`first`'s after
/// sides, `second`'s before sides), grouped where they overlap or touch, each group
/// one segment. A position of Y outside every group maps back to X by the growth of
/// `first`'s segments before it, and on to Z by the growth of `second`'s.
fn compose(first: &[Segment], second: &[Segment]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(first.len() + second.len());
    let (mut i, mut j) = (0, 0);
    // The growth of the segments of each list wholly before the current group.
    let (mut before_first, mut before_second): (isize, isize) = (0, 0);
    // The next interval of Y, from whichever list has the earlier one: (start, end,
    // growth, from `first`).
    let next = |i: usize, j: usize| -> Option<(usize, usize, isize, bool)> {
        match (first.get(i), second.get(j)) {
            (Some(f), Some(s)) if f.a0 <= s.b0 => Some((f.a0, f.a1, f.growth(), true)),
            (_, Some(s)) => Some((s.b0, s.b1, s.growth(), false)),
            (Some(f), None) => Some((f.a0, f.a1, f.growth(), true)),
            (None, None) => None,
        }
    };
    while let Some((m0, mut m1, _, _)) = next(i, j) {
        let (mut grow_first, mut grow_second) = (0isize, 0isize);
        while let Some((start, end, growth, from_first)) = next(i, j) {
            if start > m1 {
                break;
            }
            count_work(1);
            m1 = m1.max(end);
            if from_first {
                grow_first += growth;
                i += 1;
            } else {
                grow_second += growth;
                j += 1;
            }
        }
        let (m0, m1) = (m0 as isize, m1 as isize);
        out.push(Segment {
            b0: (m0 - before_first) as usize,
            b1: (m1 - before_first - grow_first) as usize,
            a0: (m0 + before_second) as usize,
            a1: (m1 + before_second + grow_second) as usize,
        });
        before_first += grow_first;
        before_second += grow_second;
    }
    out
}

/// The content start of the textblock `target` (by identity) in `doc`: a walk of
/// `doc` up to it.
fn find_block(doc: &Node, target: &Node) -> Option<usize> {
    fn walk(n: &Node, at: usize, target: &Node) -> Option<usize> {
        // `at` is the position of `n`'s content start.
        let mut pos = at;
        for i in 0..n.child_count() {
            let c = n.child(i);
            count_work(1);
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

/// The content start of every node of `doc` outside a textblock, by [`node_key`]:
/// the first, where one node occurs twice (the one [`find_block`] finds).
fn index_blocks(doc: &Node) -> Blocks {
    fn walk(n: &Node, at: usize, out: &mut Blocks) {
        // `at` is the position of `n`'s content start.
        let mut pos = at;
        for i in 0..n.child_count() {
            let c = n.child(i);
            count_work(1);
            out.entry(node_key(c)).or_insert(pos + 1);
            if !c.is_textblock() && !c.is_leaf() {
                walk(c, pos + 1, out);
            }
            pos += c.node_size();
        }
    }
    let mut out = Blocks::default();
    walk(doc, 0, &mut out);
    out
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
    let lookups = steps.lookups.get();
    steps.lookups.set(lookups + 1);
    let (a0, b0) = if lookups < WALKS_BEFORE_INDEX {
        (
            find_block(&steps.after, after)?,
            find_block(&steps.before, before)?,
        )
    } else {
        let [in_before, in_after] = steps
            .blocks
            .get_or_init(|| [index_blocks(&steps.before), index_blocks(&steps.after)]);
        (
            *in_after.get(&node_key(after))?,
            *in_before.get(&node_key(before))?,
        )
    };
    let (a1, b1) = (a0 + n_len, b0 + o_len);
    // The runs of the block's text no step touched, each as (old offset, new offset,
    // length): the gaps between the segments that meet the block, each mapped back by
    // the segment before it.
    let segments = &steps.segments;
    let first = segments.partition_point(|s| s.a1 < a0);
    count_work(1);
    let mut back: isize = match first {
        0 => 0,
        k => segments[k - 1].b1 as isize - segments[k - 1].a1 as isize,
    };
    let mut anchors: Vec<(usize, usize, usize)> = Vec::new();
    let mut cut = |u: usize, v: usize, back: isize| {
        if v <= u {
            return;
        }
        let bu = (u as isize + back) as usize;
        if bu < b0 || bu + (v - u) > b1 {
            return;
        }
        anchors.push((bu - b0, u - a0, v - u));
    };
    let mut cursor = a0;
    for s in &segments[first..] {
        count_work(1);
        if s.a0 > a1 {
            break;
        }
        cut(cursor, s.a0.clamp(a0, a1), back);
        cursor = cursor.max(s.a1.clamp(a0, a1));
        back = s.b1 as isize - s.a1 as isize;
    }
    cut(cursor, a1, back);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A small xorshift, for the property test below.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            (x % n as u64) as usize
        }
    }

    /// Where `pos` of the document after `segments` was before them, read as
    /// [`step_hunks`] reads it: a position in a gap between segments, where a gap's
    /// end (a segment's `a0`) is read toward the gap (`assoc` -1) and its start (a
    /// segment's `a1`) away from the segment (`assoc` 1). `None` for any other.
    fn back(segments: &[Segment], pos: usize, assoc: i32) -> Option<usize> {
        let mut shift: isize = 0;
        for s in segments {
            if pos < s.a0 {
                break;
            }
            if pos == s.a0 && assoc < 0 {
                break;
            }
            if pos < s.a1 || (pos == s.a1 && assoc < 0) {
                return None;
            }
            shift = s.b1 as isize - s.a1 as isize;
        }
        Some((pos as isize + shift) as usize)
    }

    /// Random step maps over a document of `len` positions: each a few disjoint
    /// ranges of the document as it stands when the map applies.
    /// With `touching`, a map's next range may start where the last one ends.
    fn random_maps(rng: &mut Rng, len: usize, steps: usize, touching: bool) -> Vec<StepMap> {
        let mut len = len;
        let mut maps = Vec::new();
        for _ in 0..steps {
            let mut ranges = Vec::new();
            let mut at = rng.below(4);
            let mut growth: isize = 0;
            for _ in 0..1 + rng.below(3) {
                if at > len {
                    break;
                }
                let old = rng.below(3).min(len - at);
                let new = rng.below(3);
                if old == 0 && new == 0 {
                    at += 1 + rng.below(4);
                    continue;
                }
                ranges.extend([at, old, new]);
                growth += new as isize - old as isize;
                // A touching range never starts where one that deletes nothing does.
                let gap = rng.below(4);
                at += old + if touching && old > 0 { gap } else { gap.max(1) };
            }
            len = (len as isize + growth) as usize;
            maps.push(StepMap::new(ranges));
        }
        maps
    }

    /// A mapping holding an inverted map sets no scope, so the change is diffed the
    /// usual way rather than read through ranges that mean something else; the same
    /// maps not inverted do set one (the positive control).
    #[test]
    fn a_mapping_with_an_inverted_map_sets_no_scope() {
        let schema = rinch_editor_core::Schema::starter_kit();
        let text = schema.text("abcdefghij").unwrap();
        let p = schema
            .branch("paragraph", rinch_editor_core::Fragment::from_node(text))
            .unwrap();
        let doc = schema
            .branch("doc", rinch_editor_core::Fragment::from_node(p))
            .unwrap();
        let held = || STEPS.with(|s| s.borrow().is_some());
        let mut forward = Mapping::new();
        forward.append_map(StepMap::new(vec![8, 1, 0]));
        forward.append_map(StepMap::new(vec![2, 1, 0]));
        {
            let _scope = steps_scope(&doc, &doc, &forward);
            assert!(held(), "the forward maps set a scope");
        }
        assert!(!held());
        let inverse = forward.invert();
        assert!(inverse.maps().iter().all(StepMap::is_inverted));
        let _scope = steps_scope(&doc, &doc, &inverse);
        assert!(!held(), "an inverted map sets no scope");
    }

    /// Every position the composed segments say no map touched maps back through the
    /// maps' inverses, last to first, to where the segments say, as the step hunks
    /// read it before #2526 (a run's start with `assoc` 1, its end with -1); and a
    /// position inside a run maps forward again onto itself.
    ///
    /// In half the seeds a map's ranges touch. `StepMap::map` stops at the first of
    /// two touching ranges (two deletions side by side map their meeting point to the
    /// first one's end), where the segments make them one place, so those seeds check
    /// positions inside a run only, not at its ends.
    #[test]
    fn composed_segments_map_as_the_maps_do() {
        let mut touching_checked = 0;
        for seed in 1..=2000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let len = 5 + rng.below(30);
            let steps = 1 + rng.below(8);
            let touching = seed % 2 == 0;
            let maps = random_maps(&mut rng, len, steps, touching);
            let segments = compose_all(&maps);
            for w in segments.windows(2) {
                assert!(
                    w[0].a1 < w[1].a0 && w[0].b1 < w[1].b0,
                    "{seed}: {segments:?}"
                );
            }
            let end: usize = segments.last().map_or(len, |s| {
                (len as isize + s.a1 as isize - s.b1 as isize) as usize
            });
            let mut checked = 0;
            for pos in 0..=end {
                for assoc in [-1, 1] {
                    let Some(old) = back(&segments, pos, assoc) else {
                        continue;
                    };
                    let inside = back(&segments, pos, -assoc) == Some(old);
                    if touching && !inside {
                        continue;
                    }
                    checked += 1;
                    let mapped = maps.iter().rev().fold(pos, |p, m| m.invert().map(p, assoc));
                    assert_eq!(
                        mapped, old,
                        "{seed}: back({pos}, {assoc}) {segments:?} {maps:?}"
                    );
                    // Forward from inside a gap only: at a gap's edge a map whose
                    // ranges touch (a deletion, then an insertion at its end) maps
                    // the position between them to the first range's end and stops,
                    // where the two are one place.
                    if assoc > 0 && inside {
                        let forward = maps.iter().fold(old, |p, m| m.map(p, 1));
                        assert_eq!(forward, pos, "{seed}: forward({old}) {segments:?} {maps:?}");
                    }
                }
            }
            assert!(touching || checked > 0, "{seed}: no position checked");
            touching_checked += usize::from(touching) * checked;
        }
        // Positive control for the touching seeds, which may each check nothing.
        assert!(
            touching_checked > 1000,
            "{touching_checked} touching checks"
        );
    }
}
