//! The history plugin — a single, Step-based, grouped, typing-merged undo/redo.
//!
//! This is the **only** undo system (design §5; it replaces the old engine's two
//! competing interpreters). History stores **inverted Steps**, not byte positions:
//! a recorded event is "the steps that, applied in reverse, revert this edit",
//! plus the selection to restore. Undo applies an event's inverse steps as a
//! transaction (`addToHistory=false`); redo re-applies the forward steps. The same
//! event machinery serves both branches.
//!
//! **Changes the history does not record** (a transaction with `addToHistory=false`
//! that changes the document: a collaboration peer's edit, a cure) are kept as
//! their step maps, and an undo maps each inverse step through every change made
//! after it before applying it. This is ProseMirror's history (`Branch.addMaps`,
//! `Branch.popEvent`): a branch is a list of items, each the step map of one step
//! plus, for a recorded step, its inverse, and the undo walks the popped event's
//! items newest first, building the mapping as it goes. A step that no longer maps
//! (a peer deleted all of its range) or no longer applies is dropped, and the rest
//! of the event still undoes. Before #1249 such a change was ignored, so an undo
//! replayed its steps at their old positions — in a peer's text.
//!
//! **Cost.** A branch is a persistent list (newest item first), so the state
//! copy every transaction makes is a pointer copy, and recording a step or a
//! peer's change is O(1). A run of a peer's changes with no recorded step between
//! them is folded into one item ([`StepMap::compose`]), so the list holds about as
//! many items as the retained events have steps, however long the peers type.
//! Events past the depth cap are hidden at once and dropped from the list in a
//! batch. [`__history_work`] counts the items each operation touches; the pins in
//! `tests/history_remote_1249.rs` hold it flat.
//!
//! Grouping: consecutive **typing** transactions within a time window merge into a
//! single undo step (so a word is one undo, not one-per-keystroke); any non-typing
//! transaction (a mark, a block change, a split) is its own group, and an explicit
//! caret move breaks the current group. The time window is driven by
//! [`Transaction::set_time`]: the core reads no clock, and `rinch-editor-view`'s
//! `EditorHandle` stamps every transaction it dispatches (#940). With times unset
//! (`0`, a headless state driven directly), consecutive typing always merges,
//! which tests pin by spacing timestamps apart.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use crate::command::Command;
use crate::keymap::KeyBinding;
use crate::model::{Fragment, Node, Slice};
use crate::plugin::{Plugin, PluginKey};
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::step::Step;
use crate::transform::step_map::{Mapping, StepMap};
use crate::transform::steps::ReplaceStep;

/// The history plugin's key.
pub const HISTORY_KEY: PluginKey = PluginKey("history");
/// Transaction meta key marking an undo/redo (so the plugin shifts branches
/// instead of recording a new event).
const HISTORY_META: &str = "history";
/// How many events past the depth cap a branch hides before it drops them from
/// its list in one pass (ProseMirror's `DEPTH_OVERFLOW`).
const DEPTH_OVERFLOW: usize = 20;

thread_local! {
    static HISTORY_WORK: Cell<u64> = const { Cell::new(0) };
}

fn work(n: usize) {
    HISTORY_WORK.with(|w| w.set(w.get() + n as u64));
}

/// How many history items this thread's history has created or walked. A work
/// counter for the cost pins (#1249): recording a keystroke or a peer's change
/// touches a fixed number of items, whatever the history holds. Not part of the
/// API.
#[doc(hidden)]
pub fn __history_work() -> u64 {
    HISTORY_WORK.with(|w| w.get())
}

/// Marks a transaction as an undo or redo for the history plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryAction {
    Undo,
    Redo,
}

/// An undo/redo transaction's meta: which one it is, the branch it popped with
/// that event removed (and the maps of what the undo did appended, so older
/// events still map through it), and the popped event's classification.
#[derive(Clone)]
struct HistoryMeta {
    action: HistoryAction,
    remaining: Branch,
    typing: bool,
}

/// What an event's first (oldest) item carries: the selection to restore and
/// whether it is a typing group.
#[derive(Clone)]
struct EventInfo {
    selection: Selection,
    typing: bool,
}

/// One entry of a branch: the forward step map of a step, and for a recorded
/// step its inverse. A map-only item (`step: None`) is a change the history did
/// not record (`peer`), or the map of a step an undo applied.
#[derive(Clone)]
struct Item {
    map: StepMap,
    step: Option<Box<dyn Step>>,
    /// `Some` on the first item of an event.
    event: Option<EventInfo>,
    /// How many items back the item whose map this one inverts sits (PM's
    /// `mirrorOffset`), so mapping through both recovers a deleted position.
    mirror_offset: Option<usize>,
    /// A change the history did not record: the next one folds into it.
    peer: bool,
}

impl Item {
    fn map_only(map: StepMap, mirror_offset: Option<usize>) -> Item {
        Item {
            map,
            step: None,
            event: None,
            mirror_offset,
            peer: false,
        }
    }
}

struct Cons {
    item: Item,
    next: Items,
}

/// A persistent list of items, newest first: cloning it is a pointer copy, and
/// pushing shares everything older.
#[derive(Clone, Default)]
struct Items(Option<Rc<Cons>>);

impl Items {
    fn push(&self, item: Item) -> Items {
        work(1);
        Items(Some(Rc::new(Cons {
            item,
            next: self.clone(),
        })))
    }

    fn head(&self) -> Option<&Item> {
        self.0.as_ref().map(|c| &c.item)
    }

    fn tail(&self) -> Items {
        self.0.as_ref().map(|c| c.next.clone()).unwrap_or_default()
    }

    /// Newest first.
    fn iter(&self) -> impl Iterator<Item = &Item> {
        let mut cur = self.0.as_deref();
        std::iter::from_fn(move || {
            let c = cur?;
            work(1);
            cur = c.next.0.as_deref();
            Some(&c.item)
        })
    }

    /// The list of `items`, oldest first, pushed onto `self`.
    fn extend(mut self, items: impl IntoIterator<Item = Item>) -> Items {
        for item in items {
            self = self.push(item);
        }
        self
    }
}

impl Drop for Items {
    // Iteratively, so a long list does not overflow the stack.
    fn drop(&mut self) {
        let mut cur = self.0.take();
        while let Some(rc) = cur {
            match Rc::try_unwrap(rc) {
                Ok(mut cons) => cur = cons.next.0.take(),
                Err(_) => break,
            }
        }
    }
}

/// One side of the history (the undo stack or the redo stack). Port of
/// prosemirror-history's `Branch` on a persistent list.
#[derive(Clone, Default)]
struct Branch {
    items: Items,
    /// Events an undo (or redo) can reach.
    event_count: usize,
    /// Older events past the depth cap, still in `items` until a batch drops them.
    hidden: usize,
    /// The newest event's time and typing flag while it may take more typing
    /// (set by recording, cleared by an undo or redo).
    last: Option<(u64, bool)>,
}

impl Branch {
    /// Record `tr`'s steps (inverted against their pre-step documents). `event`
    /// starts a new event; `None` appends them to the newest one.
    fn add_transform(&mut self, tr: &Transaction, mut event: Option<EventInfo>) {
        let time = tr.time();
        self.last = match (&event, self.last) {
            (Some(e), _) => Some((time, e.typing)),
            (None, Some((_, typing))) => Some((time, typing)),
            (None, None) => None,
        };
        let mut items = std::mem::take(&mut self.items);
        for (i, step) in tr.steps().iter().enumerate() {
            let info = event.take();
            if info.is_some() {
                self.event_count += 1;
            }
            items = items.push(Item {
                map: step.get_map(),
                step: Some(step.invert(&tr.docs()[i])),
                event: info,
                mirror_offset: None,
                peer: false,
            });
        }
        self.items = items;
    }

    /// Record a change this branch's events must be mapped through (PM's
    /// `addMaps`), folded into the previous item when that is a change of the same
    /// kind. Nothing to map when the branch holds no event.
    fn add_maps(&mut self, maps: &[StepMap]) {
        if self.event_count == 0 {
            *self = Branch::default();
            return;
        }
        for map in maps {
            let folded = match self.items.head() {
                Some(head) if head.peer => head.map.compose(map),
                _ => None,
            };
            self.items = match folded {
                Some(map) => self.items.tail().push(Item {
                    peer: true,
                    ..Item::map_only(map, None)
                }),
                None => self.items.push(Item {
                    peer: true,
                    ..Item::map_only(map.clone(), None)
                }),
            };
        }
    }

    /// Hide the oldest events past `depth`, and drop hidden ones from the list
    /// once there are enough of them.
    fn cut_to_depth(&mut self, depth: usize) {
        if self.event_count > depth {
            self.hidden += self.event_count - depth;
            self.event_count = depth;
        }
        if self.hidden <= DEPTH_OVERFLOW {
            return;
        }
        let mut kept: Vec<Item> = Vec::new();
        let mut seen = 0;
        for item in self.items.iter() {
            if seen == self.event_count {
                break;
            }
            if item.event.is_some() {
                seen += 1;
            }
            kept.push(item.clone());
        }
        self.items = Items::default().extend(kept.into_iter().rev());
        self.hidden = 0;
    }

    /// Build the transaction that reverts the newest event (PM's `popEvent`).
    /// Each inverse step is mapped through every map recorded after it — a
    /// change the history did not record, and the steps this undo has already
    /// applied (mirrored, so a position the event's own step deleted and its
    /// inverse re-inserted is recovered). A step that no longer maps or applies
    /// is skipped. Returns the transaction (selection not set yet), the
    /// selection to restore, the branch without the event, and the event.
    fn pop_event(
        &self,
        state: &EditorState,
    ) -> Option<(Transaction, Selection, Branch, EventInfo)> {
        if self.event_count == 0 {
            return None;
        }
        // The newest event's items and everything after them, oldest first.
        let mut seg: Vec<&Item> = Vec::new();
        let mut rest = Items::default();
        let mut cur = &self.items;
        while let Some(c) = cur.0.as_deref() {
            work(1);
            seg.push(&c.item);
            if c.item.event.is_some() {
                rest = c.next.clone();
                break;
            }
            cur = &c.next;
        }
        seg.reverse();
        let len = seg.len();
        let mut tr = state.tr();
        let mut remap: Option<Mapping> = None;
        let mut map_from = 0usize;
        let mut add_before: Vec<Item> = Vec::new();
        let mut add_after: Vec<Item> = Vec::new();
        for i in (0..len).rev() {
            let item = seg[i];
            let Some(step) = &item.step else {
                if remap.is_none() {
                    let m = remapping(&seg, 0, i + 1);
                    map_from = m.len();
                    remap = Some(m);
                }
                map_from -= 1;
                add_before.push(item.clone());
                continue;
            };
            if let Some(remap) = remap.as_mut() {
                add_before.push(Item::map_only(item.map.clone(), None));
                let after = remap.slice(map_from, remap.len());
                if let Some(pieces) = split_delete(step.as_ref(), &after, tr.doc()) {
                    // A delete of text a peer has since typed inside: delete my
                    // pieces around theirs, back to front.
                    let mut maps = Vec::new();
                    for &(from, to) in pieces.iter().rev() {
                        let piece = ReplaceStep::new(from, to, Slice::empty());
                        if tr.step(Box::new(piece)).is_ok() {
                            let map = tr
                                .mapping()
                                .maps()
                                .last()
                                .cloned()
                                .unwrap_or_else(StepMap::empty);
                            add_after.push(Item::map_only(map.clone(), None));
                            maps.push(map);
                        }
                    }
                    drop(after);
                    map_from -= 1;
                    for map in maps {
                        remap.append_map(map);
                    }
                    if let Some(info) = &item.event {
                        let selection = info
                            .selection
                            .map(tr.doc(), &remap.slice(map_from, remap.len()));
                        return Some(self.finish(tr, selection, rest, add_before, add_after, info));
                    }
                    continue;
                }
                drop(after);
                let mut applied = None;
                if let Some(mapped) = step.map(&remap.slice(map_from, remap.len()))
                    && tr.step(mapped).is_ok()
                {
                    let map = tr
                        .mapping()
                        .maps()
                        .last()
                        .cloned()
                        .unwrap_or_else(StepMap::empty);
                    let offset = add_after.len() + add_before.len();
                    add_after.push(Item::map_only(map.clone(), Some(offset)));
                    applied = Some(map);
                }
                map_from -= 1;
                if let Some(map) = applied {
                    remap.append_map_mirrored(map, map_from);
                }
            } else {
                // Nothing recorded after this step: it applies where it was made.
                // A step that fails is skipped, as PM's `maybeStep` does.
                let _ = tr.step(step.clone());
            }
            if let Some(info) = &item.event {
                let selection = match &remap {
                    Some(r) => info.selection.map(tr.doc(), &r.slice(map_from, r.len())),
                    None => info.selection.clone(),
                };
                return Some(self.finish(tr, selection, rest, add_before, add_after, info));
            }
        }
        None
    }

    /// `pop_event`'s result: the branch left behind is the items before the event,
    /// then the maps it walked past (oldest first), then the maps of what the undo
    /// applied.
    fn finish(
        &self,
        tr: Transaction,
        selection: Selection,
        rest: Items,
        mut add_before: Vec<Item>,
        add_after: Vec<Item>,
        info: &EventInfo,
    ) -> (Transaction, Selection, Branch, EventInfo) {
        let event_count = self.event_count - 1;
        let remaining = if event_count == 0 {
            Branch::default()
        } else {
            add_before.reverse();
            Branch {
                items: rest.extend(add_before.into_iter().chain(add_after)),
                event_count,
                hidden: self.hidden,
                last: None,
            }
        };
        (tr, selection, remaining, info.clone())
    }
}

/// An inverse step that deletes text (the undo of my typing), split around the
/// text a later change put inside it, when it did: the ranges, in `doc`'s
/// positions, of the chars it deletes that are still there, each mapped on its
/// own (its start after what was inserted before it, its end before what was
/// inserted after it). ProseMirror maps the delete as one range, which grows over
/// a peer's text typed inside mine, so the undo deleted their text too (review of
/// #1889). `None` when the delete maps as one range, or is not a delete inside one
/// textblock.
fn split_delete(step: &dyn Step, mapping: &Mapping, doc: &Node) -> Option<Vec<(usize, usize)>> {
    let r = step.as_any().downcast_ref::<ReplaceStep>()?;
    if r.slice.size() > 0 || r.to <= r.from || r.to - r.from > 4096 {
        return None;
    }
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    for p in r.from..r.to {
        let (a, b) = (mapping.map(p, 1), mapping.map(p + 1, -1));
        if b <= a {
            continue;
        }
        match pieces.last_mut() {
            Some(last) if last.1 == a => last.1 = b,
            _ => pieces.push((a, b)),
        }
    }
    if pieces.len() < 2 {
        return None;
    }
    let (first, last) = (pieces[0].0, pieces[pieces.len() - 1].1);
    let (ra, rb) = (
        doc.resolve(crate::pos::Pos(first)).ok()?,
        doc.resolve(crate::pos::Pos(last)).ok()?,
    );
    (ra.parent().is_textblock() && ra.parent().same_ref(rb.parent())).then_some(pieces)
}

/// The maps of `seg[from..to]` (oldest first), with their mirror relations (PM's
/// `remapping`).
fn remapping(seg: &[&Item], from: usize, to: usize) -> Mapping {
    let mut maps = Mapping::new();
    for (i, item) in seg.iter().enumerate().take(to).skip(from) {
        let mirror = item
            .mirror_offset
            .filter(|&off| i >= from + off)
            .map(|off| maps.len() - off);
        match mirror {
            Some(m) => maps.append_map_mirrored(item.map.clone(), m),
            None => maps.append_map(item.map.clone()),
        }
    }
    maps
}

/// The history plugin's folded state.
#[derive(Clone, Default)]
pub struct HistoryState {
    done: Branch,
    undone: Branch,
    /// Set by a caret-only transaction; suppresses the next typing merge.
    break_group: bool,
}

impl HistoryState {
    /// Number of available undo steps.
    pub fn undo_depth(&self) -> usize {
        self.done.event_count
    }
    /// Number of available redo steps.
    pub fn redo_depth(&self) -> usize {
        self.undone.event_count
    }
}

/// A transaction is "typing" if all its steps are non-structural replaces (text
/// insert/delete) — the class that should coalesce into one undo group.
fn is_typing(tr: &Transaction) -> bool {
    !tr.steps().is_empty()
        && tr.steps().iter().all(|s| {
            s.as_any()
                .downcast_ref::<ReplaceStep>()
                .is_some_and(|r| !r.structure)
        })
}

/// Whether a new typing transaction at `time` should merge into the last
/// recorded done event.
fn should_merge(hist: &HistoryState, typing: bool, time: u64, delay: u64) -> bool {
    if hist.break_group || !typing || hist.done.event_count == 0 {
        return false;
    }
    match hist.done.last {
        Some((last_time, last_typing)) => {
            last_typing && time >= last_time && time - last_time < delay
        }
        None => false,
    }
}

/// One position's worth of a document, for [`unrecorded_maps`]: what a step map
/// cares about, and nothing it does not (marks, a block's type or attributes).
#[derive(PartialEq)]
enum Tok<'a> {
    Open,
    Close,
    Char(char),
    Leaf(&'a str),
}

/// The tokens of positions `from..to` of `content` (which starts at `base`).
fn tokens<'a>(content: &'a Fragment, base: usize, from: usize, to: usize, out: &mut Vec<Tok<'a>>) {
    let mut pos = base;
    for child in content.children() {
        let end = pos + child.node_size();
        if end <= from {
            pos = end;
            continue;
        }
        if pos >= to {
            break;
        }
        if let Some(text) = child.text() {
            let lo = from.saturating_sub(pos);
            let hi = (to - pos).min(end - pos);
            out.extend(text.chars().skip(lo).take(hi - lo).map(Tok::Char));
        } else if child.is_leaf() {
            out.push(Tok::Leaf(child.type_name()));
        } else {
            if pos >= from {
                out.push(Tok::Open);
            }
            tokens(child.content(), pos + 1, from, to, out);
            if end - 1 < to {
                out.push(Tok::Close);
            }
        }
        pos = end;
    }
}

/// The maps the history keeps for a change it does not record: each step's map,
/// narrowed for a replace to the positions that actually changed, and none for a
/// step that moves nothing (a mark, a block's type or attributes).
///
/// A collaboration peer's edit arrives as a replace of every block it changed
/// (`rinch-editor-collab`'s remote integration), and a position inside a replaced
/// range has nowhere to map: an undo step there is dropped. Narrowed to what
/// changed — the char a peer typed — a step of mine elsewhere in that paragraph
/// maps past it and still undoes. The narrowing compares positions, not nodes: a
/// char is the same char whatever its marks, and a block's open and close tokens
/// are the same whatever its type, so a peer's bold, heading or alignment moves
/// no position and keeps my edit undoable (review of #1889). Everything outside
/// the narrowed range is the same in both documents, so the map is exact for it.
fn unrecorded_maps(tr: &Transaction) -> Vec<StepMap> {
    let docs = tr.docs();
    let steps = tr.steps();
    // Several replaces in one change (the collab integration replaces each changed
    // block, back to front): diffed as one range of the document before and after
    // all of them, so a block one step deletes and another inserts elsewhere (a
    // peer's move) is found as the move it is, not as blocks paired by place.
    if steps.len() > 1
        && steps
            .iter()
            .all(|s| s.as_any().downcast_ref::<ReplaceStep>().is_some())
    {
        let maps = tr.mapping().maps();
        let back = |i: usize, pos: usize, assoc: i32| {
            maps[..i]
                .iter()
                .rev()
                .fold(pos, |p, m| m.invert().map(p, assoc))
        };
        let (mut lo, mut hi) = (usize::MAX, 0);
        for (i, step) in steps.iter().enumerate() {
            let r = step.as_any().downcast_ref::<ReplaceStep>().unwrap();
            lo = lo.min(back(i, r.from, -1));
            hi = hi.max(back(i, r.to, 1));
        }
        let (new_lo, new_hi) = (tr.mapping().map(lo, -1), tr.mapping().map(hi, 1));
        if let Some(found) = narrowed(&docs[0], tr.doc(), lo, hi, new_lo.min(new_hi), new_hi, true)
        {
            return found.into_iter().collect();
        }
    }
    let mut out = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let Some(r) = step.as_any().downcast_ref::<ReplaceStep>() else {
            let map = step.get_map();
            if !map.is_empty() {
                out.push(map);
            }
            continue;
        };
        let after = docs.get(i + 1).unwrap_or(tr.doc());
        let to_new = r.from + r.slice.size();
        if let Some(Some(map)) = narrowed(&docs[i], after, r.from, r.to, r.from, to_new, false) {
            out.push(map);
        }
    }
    out
}

/// The map of a change that turned positions `from..to` of `before` into
/// `new_from..new_to` of `after` (everything outside them the same), narrowed to
/// the tokens that differ: `Some(None)` when none do. With `exact`, `None` when
/// the difference is too large to diff run by run (the caller then maps step by
/// step); otherwise one range covers it.
fn narrowed(
    before: &Node,
    after: &Node,
    from: usize,
    to: usize,
    new_from: usize,
    new_to: usize,
    exact: bool,
) -> Option<Option<StepMap>> {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    tokens(before.content(), 0, from, to, &mut a);
    tokens(after.content(), 0, new_from, new_to, &mut b);
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let (old, new) = (a.len() - prefix - suffix, b.len() - prefix - suffix);
    if old == 0 && new == 0 {
        return Some(None);
    }
    let start = from + prefix;
    let ranges = if old == 0 || new == 0 {
        vec![start, old, new]
    } else {
        match token_diff(&a[prefix..prefix + old], &b[prefix..prefix + new]) {
            Some(edits) => edits
                .into_iter()
                .flat_map(|(at, o, n)| [start + at, o, n])
                .collect(),
            None if exact => return None,
            None => vec![start, old, new],
        }
    };
    Some(Some(StepMap::new(ranges)))
}

/// The changes that turn `a` into `b`, as `(position in a, deleted, inserted)`
/// runs in order: a shortest edit script (Myers). Separate changes in one replace
/// (a peer's keystrokes at both ends of a paragraph, a block wrapped in a quote:
/// an open token before it and a close token after) stay separate, so a position
/// between them maps exactly. `None` when the two differ too much for that to be
/// worth it, and the caller keeps one range.
fn token_diff(a: &[Tok<'_>], b: &[Tok<'_>]) -> Option<Vec<(usize, usize, usize)>> {
    const MAX_EDITS: usize = 64;
    const MAX_LEN: usize = 8192;
    let (n, m) = (a.len() as isize, b.len() as isize);
    if a.len() + b.len() > MAX_LEN {
        return None;
    }
    let max = (a.len() + b.len()).min(MAX_EDITS) as isize;
    let off = max + 1;
    let idx = |k: isize| (k + off) as usize;
    let mut v = vec![0isize; 2 * off as usize + 1];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
                v[idx(k + 1)]
            } else {
                v[idx(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx(k)] = x;
            if x >= n && y >= m {
                found = true;
                break 'outer;
            }
            k += 2;
        }
    }
    if !found {
        return None;
    }
    // Walk back: `true` for an op that consumes `a` (a delete), `false` for an
    // insert; equal runs are skipped.
    let (mut x, mut y) = (n, m);
    let mut ops: Vec<(isize, bool)> = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let prev_k = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[idx(prev_k)];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x {
                ops.push((x, false));
            } else {
                ops.push((prev_x, true));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    // Group the ops that sit at one place in `a` (a delete run and the inserts
    // beside it) into one range.
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    let mut del_end = 0usize;
    for (at, del) in ops {
        let at = at as usize;
        match out.last_mut() {
            Some(last) if at <= del_end => {
                if del {
                    last.1 += 1;
                    del_end += 1;
                } else {
                    last.2 += 1;
                }
            }
            _ => {
                out.push((at, usize::from(del), usize::from(!del)));
                del_end = at + usize::from(del);
            }
        }
    }
    Some(out)
}

/// The history plugin. `new_group_delay` is the typing-merge window in
/// milliseconds; `depth` caps the number of retained undo events.
#[derive(Debug)]
pub struct HistoryPlugin {
    new_group_delay: u64,
    depth: usize,
}

impl Default for HistoryPlugin {
    fn default() -> Self {
        HistoryPlugin {
            new_group_delay: 500,
            depth: 200,
        }
    }
}

impl HistoryPlugin {
    /// A history plugin with default grouping (500ms window, depth 200).
    pub fn new() -> HistoryPlugin {
        HistoryPlugin::default()
    }

    /// Override the typing-merge window (ms) and the retained-event cap.
    pub fn with(new_group_delay: u64, depth: usize) -> HistoryPlugin {
        HistoryPlugin {
            new_group_delay,
            depth,
        }
    }
}

impl Plugin for HistoryPlugin {
    fn key(&self) -> PluginKey {
        HISTORY_KEY
    }

    fn commands(&self) -> Vec<(&'static str, Command)> {
        vec![("undo", undo()), ("redo", redo())]
    }

    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
        [
            ("Mod-z", "undo"),
            ("Mod-y", "redo"),
            ("Mod-Shift-z", "redo"),
        ]
        .into_iter()
        .filter_map(|(b, c)| KeyBinding::parse(b).map(|kb| (kb, c)))
        .collect()
    }

    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        Some(Rc::new(HistoryState::default()))
    }

    fn apply(
        &self,
        tr: &Transaction,
        old: &EditorState,
        _new_doc: &Node,
        prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        let mut hist = prev
            .and_then(|p| p.downcast_ref::<HistoryState>())
            .cloned()
            .unwrap_or_default();

        let meta = tr
            .get_meta(HISTORY_META)
            .and_then(|m| m.downcast_ref::<HistoryMeta>());
        match meta {
            Some(meta) => {
                // The command reverted the newest event of one branch; record what
                // it did as an event on the other. The new event inherits the
                // reverted event's `typing` classification rather than recomputing
                // it: an event's inverse steps are always non-structural (PM's
                // `ReplaceStep.invert` drops the structure flag), so a redone
                // structural edit (e.g. a split) must NOT be reclassified as typing
                // and merged with the next keystroke.
                let event = EventInfo {
                    selection: old.selection.clone(),
                    typing: meta.typing,
                };
                match meta.action {
                    HistoryAction::Undo => {
                        hist.done = meta.remaining.clone();
                        hist.undone.add_transform(tr, Some(event));
                    }
                    HistoryAction::Redo => {
                        hist.undone = meta.remaining.clone();
                        hist.done.add_transform(tr, Some(event));
                    }
                }
                // The next edit starts a step of its own, as ProseMirror's
                // `prevTime = 0` makes it: otherwise typing within the window of
                // the group now on top of `done` joins it. (After a redo this was
                // already true — a redo needs an undo before it, which set it, and
                // only a recorded edit, which empties `undone`, clears it — and is
                // set here so the rule does not rest on that.)
                hist.break_group = true;
            }
            None => {
                if tr.add_to_history() && tr.doc_changed() {
                    let typing = is_typing(tr);
                    if should_merge(&hist, typing, tr.time(), self.new_group_delay) {
                        hist.done.add_transform(tr, None);
                    } else {
                        let event = EventInfo {
                            selection: old.selection.clone(),
                            typing,
                        };
                        hist.done.add_transform(tr, Some(event));
                    }
                    hist.break_group = false;
                    hist.undone = Branch::default(); // a fresh edit invalidates redo
                } else if tr.doc_changed() {
                    // A change the history does not record (a peer's edit): both
                    // branches' events are mapped through it when they are undone.
                    if hist.done.event_count > 0 || hist.undone.event_count > 0 {
                        let maps = unrecorded_maps(tr);
                        hist.done.add_maps(&maps);
                        hist.undone.add_maps(&maps);
                    } else {
                        hist.done = Branch::default();
                        hist.undone = Branch::default();
                    }
                } else if tr.selection_set() || tr.stored_marks_set() {
                    // A caret-only move or a stored-mark toggle (e.g. cursor-bold)
                    // ends the current typing group, matching PM's storedMarksSet
                    // group break.
                    hist.break_group = true;
                }
            }
        }

        // Cap retained undo history.
        hist.done.cut_to_depth(self.depth);
        Some(Rc::new(hist))
    }
}

/// Revert the newest event of `done` (undo) or `undone` (redo) as a transaction,
/// restoring the event's selection and tagging it so the plugin shifts branches.
fn hist_transaction(state: &EditorState, action: HistoryAction) -> Option<Transaction> {
    let hist = state.plugin_state::<HistoryState>(HISTORY_KEY)?;
    let branch = match action {
        HistoryAction::Undo => &hist.done,
        HistoryAction::Redo => &hist.undone,
    };
    let (mut tr, selection, remaining, event) = branch.pop_event(state)?;
    tr.set_selection(selection);
    tr.set_add_to_history(false);
    tr.set_meta(
        HISTORY_META,
        HistoryMeta {
            action,
            remaining,
            typing: event.typing,
        },
    );
    Some(tr)
}

/// A history command. The transaction is built eagerly so the applicability
/// query (`dispatch == None`) and the dispatch path share one code path (no
/// "reports applicable but then fails to dispatch" divergence — matching the
/// `command_tr` discipline elsewhere in the catalogue). An event whose steps all
/// failed to map (a peer deleted everything it touched) still pops: the command
/// applies and changes nothing, as ProseMirror's does.
fn history_command(action: HistoryAction) -> Command {
    Rc::new(move |state: &EditorState, dispatch| {
        let Some(tr) = hist_transaction(state, action) else {
            return false;
        };
        if let Some(dispatch) = dispatch {
            dispatch(tr);
        }
        true
    })
}

/// The undo command: revert the most recent recorded event.
pub fn undo() -> Command {
    history_command(HistoryAction::Undo)
}

/// The redo command: re-apply the most recently undone event.
pub fn redo() -> Command {
    history_command(HistoryAction::Redo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::commands::BaseCommandsPlugin;
    use crate::model::Fragment;
    use crate::pos::Pos;

    fn editor(text: &str) -> EditorState {
        let s = Rc::new(Schema::starter_kit());
        let para = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        EditorState::create(
            s,
            doc,
            vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
        )
    }

    fn all_text(node: &Node) -> String {
        if let Some(t) = node.text() {
            return t.to_string();
        }
        node.content().iter().map(all_text).collect()
    }

    fn type_char(state: &EditorState, text: &str, time: u64) -> EditorState {
        let mut tr = state.tr();
        tr.set_time(time);
        tr.insert_text(text).unwrap();
        state.apply(tr)
    }

    #[test]
    fn undo_and_redo_a_single_edit() {
        let state = editor("ab");
        let typed = type_char(&state, "X", 0); // cursor at start (pos 1) → "Xab"
        assert_eq!(all_text(&typed.doc), "Xab");
        let undone = typed.run("undo").expect("undo applies");
        assert_eq!(all_text(&undone.doc), "ab");
        let redone = undone.run("redo").expect("redo applies");
        assert_eq!(all_text(&redone.doc), "Xab");
    }

    #[test]
    fn typing_coalesces_into_one_undo_group() {
        // three chars typed within the window merge into a single undo
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let s2 = type_char(&s1, "b", 10);
        let s3 = type_char(&s2, "c", 20);
        assert_eq!(all_text(&s3.doc), "abc");
        let undone = s3.run("undo").expect("undo applies");
        assert_eq!(
            all_text(&undone.doc),
            "",
            "one undo reverts all coalesced typing"
        );
        // and a single redo restores all of it
        let redone = undone.run("redo").unwrap();
        assert_eq!(all_text(&redone.doc), "abc");
    }

    #[test]
    fn time_gap_breaks_typing_group() {
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let s2 = type_char(&s1, "b", 1000); // beyond the 500ms window
        assert_eq!(all_text(&s2.doc), "ab");
        let u1 = s2.run("undo").unwrap();
        assert_eq!(all_text(&u1.doc), "a", "second char is its own group");
        let u2 = u1.run("undo").unwrap();
        assert_eq!(all_text(&u2.doc), "");
    }

    #[test]
    fn type_then_bold_undo_reverts_bold_only() {
        // design M4 test: typing then a mark are separate groups
        let state = editor("ab");
        let typed = type_char(&state, "X", 0); // "Xab", cursor at 2
        let mut bolded_state = typed.clone();
        bolded_state.selection = Selection::text(Pos(1), Pos(4)); // select "Xab"
        let bolded = bolded_state.run("toggleBold").expect("bold applies");
        assert!(
            bolded
                .doc
                .child(0)
                .content()
                .iter()
                .all(|n| n.marks().iter().any(|m| m.type_name() == "bold"))
        );
        // first undo removes the bold (its own group), text remains
        let u1 = bolded.run("undo").expect("undo bold");
        assert_eq!(all_text(&u1.doc), "Xab");
        assert!(!range_has_bold(&u1));
        // second undo removes the typed X
        let u2 = u1.run("undo").expect("undo typing");
        assert_eq!(all_text(&u2.doc), "ab");
    }

    fn range_has_bold(state: &EditorState) -> bool {
        let bold = state.schema().mark_type("bold").unwrap();
        crate::commands::range_has_mark(&state.doc, 0, state.doc.content_size(), bold)
    }

    #[test]
    fn new_edit_clears_redo() {
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let undone = s1.run("undo").unwrap();
        assert!(undone.run("redo").is_some());
        // typing again should drop the redo stack
        let s2 = type_char(&undone, "b", 0);
        assert!(s2.run("redo").is_none(), "a fresh edit invalidates redo");
        assert_eq!(all_text(&s2.doc), "b");
    }

    #[test]
    fn undo_restores_selection() {
        let state = editor("ab");
        let mut s0 = state.clone();
        s0.selection = Selection::cursor(Pos(3)); // end of "ab"
        let typed = {
            let mut tr = s0.tr();
            tr.insert_text("Z").unwrap();
            s0.apply(tr)
        };
        assert_eq!(typed.selection, Selection::cursor(Pos(4)));
        let undone = typed.run("undo").unwrap();
        // selection restored to where it was before the edit (pos 3)
        assert_eq!(undone.selection, Selection::cursor(Pos(3)));
    }

    #[test]
    fn undo_unavailable_on_empty_history() {
        let state = editor("ab");
        assert!(!state.can_run("undo"));
        assert!(!state.can_run("redo"));
    }

    #[test]
    fn redone_split_does_not_merge_with_later_typing() {
        // verification finding #1: a redone structural edit (split) must keep its
        // non-typing classification so a following keystroke is its own undo group.
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(2)); // a|b
        let split = state.run("splitBlock").expect("split");
        assert_eq!(split.doc.child_count(), 2);
        let undone = split.run("undo").expect("undo split");
        assert_eq!(undone.doc.child_count(), 1);
        let redone = undone.run("redo").expect("redo split");
        assert_eq!(redone.doc.child_count(), 2);
        // type after the redone split, within the merge window
        let typed = type_char(&redone, "Z", 0);
        let after = typed.run("undo").expect("undo typing");
        assert_eq!(
            after.doc.child_count(),
            2,
            "undo reverts only the typing, not the redone split"
        );
    }

    #[test]
    fn stored_mark_toggle_breaks_typing_group() {
        // verification finding #2: a cursor mark toggle between keystrokes ends the
        // typing group (PM storedMarksSet group break).
        let mut state = editor("");
        state.selection = Selection::cursor(Pos(1));
        let s1 = type_char(&state, "a", 0);
        let s2 = s1.run("toggleBold").expect("cursor bold (stored marks)");
        let s3 = type_char(&s2, "b", 10); // within the window
        assert_eq!(all_text(&s3.doc), "ab");
        let undone = s3.run("undo").expect("undo");
        assert_eq!(
            all_text(&undone.doc),
            "a",
            "only 'b' reverted, not the whole word"
        );
    }

    #[test]
    fn block_join_starts_new_undo_group() {
        // verification finding #4: a structural block join is its own undo group and
        // does not coalesce into preceding typing.
        let s = Rc::new(Schema::starter_kit());
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let doc = s
            .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
            .unwrap();
        let mut state = EditorState::create(
            s,
            doc,
            vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
        );
        state.selection = Selection::cursor(Pos(3)); // end of first paragraph
        let typed = type_char(&state, "Z", 0); // "abZ" / "cd"
        assert_eq!(typed.doc.child_count(), 2);
        let joined = typed.run("deleteCharForward").expect("forward join");
        assert_eq!(joined.doc.child_count(), 1);
        assert_eq!(all_text(&joined.doc), "abZcd");
        // undo reverts ONLY the join, keeping the typed Z
        let after = joined.run("undo").expect("undo join");
        assert_eq!(after.doc.child_count(), 2, "join is its own undo group");
        assert_eq!(
            all_text(&after.doc),
            "abZcd",
            "the typed Z survives the undo"
        );
    }
}
