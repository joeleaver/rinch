//! A history branch: the persistent list of items one direction of the
//! history holds, and how it records, maps and pops events.

use std::rc::Rc;

use crate::model::{Node, Slice};
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::step::Step;
use crate::transform::step_map::{Mapping, StepMap};
use crate::transform::steps::{BatchStep, ReplaceStep};

use super::{DEPTH_OVERFLOW, EventInfo, Popped, work};

/// One entry of a branch: the forward step map of a step, and for a recorded
/// step its inverse. A map-only item (`step: None`) is a change the history did
/// not record (`peer`), or the map of a step an undo applied.
#[derive(Clone)]
pub(super) struct Item {
    pub(super) map: StepMap,
    pub(super) step: Option<Box<dyn Step>>,
    /// `Some` on the first item of an event.
    pub(super) event: Option<EventInfo>,
    /// How many items back the item whose map this one inverts sits (PM's
    /// `mirrorOffset`), so mapping through both recovers a deleted position.
    pub(super) mirror_offset: Option<usize>,
    /// A change the history did not record: the next one folds into it.
    pub(super) peer: bool,
    /// `Some((at, n))`: this peer item is `n` one-char deletions at `at` (a run of
    /// a peer's Backspace or Delete). Mapped as those `n` maps in turn, which is
    /// exact; stored as two numbers, so a long run costs one item. `map` is then
    /// the run as one range, for nothing but display.
    pub(super) unit_run: Option<(usize, usize)>,
    /// A change the history did not record came after this item and moved no
    /// position (`unrecorded_maps` gave it no map: a peer's attribute change, a
    /// cell's `colspan` among them), so it has no item of its own. An undo that
    /// walks past it is not the plain inverse of its event (#1937).
    pub(super) unmapped_after: bool,
}

impl Item {
    fn map_only(map: StepMap, mirror_offset: Option<usize>) -> Item {
        Item {
            map,
            step: None,
            event: None,
            mirror_offset,
            peer: false,
            unit_run: None,
            unmapped_after: false,
        }
    }
}

pub(super) struct Cons {
    item: Item,
    pub(super) next: Items,
}

/// A persistent list of items, newest first: cloning it is a pointer copy, and
/// pushing shares everything older.
#[derive(Clone, Default)]
pub(super) struct Items(pub(super) Option<Rc<Cons>>);

impl Items {
    pub(super) fn push(&self, item: Item) -> Items {
        work(1);
        Items(Some(Rc::new(Cons {
            item,
            next: self.clone(),
        })))
    }

    pub(super) fn head(&self) -> Option<&Item> {
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
pub(super) struct Branch {
    pub(super) items: Items,
    /// Events an undo (or redo) can reach.
    pub(super) event_count: usize,
    /// Older events past the depth cap, still in `items` until a batch drops them.
    pub(super) hidden: usize,
    /// The newest event's time and typing flag while it may take more typing
    /// (set by recording, cleared by an undo or redo).
    pub(super) last: Option<(u64, bool)>,
}

impl Branch {
    /// Record `tr`'s steps (inverted against their pre-step documents). `event`
    /// starts a new event; `None` appends them to the newest one.
    pub(super) fn add_transform(&mut self, tr: &Transaction, mut event: Option<EventInfo>) {
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
                unit_run: None,
                unmapped_after: false,
            });
        }
        self.items = items;
    }

    /// Note a change the history did not record that moved no position, on the
    /// newest item ([`Item::unmapped_after`]).
    pub(super) fn note_unmapped(&mut self) {
        let Some(head) = self.items.head() else {
            return;
        };
        if head.unmapped_after {
            return;
        }
        let mut item = head.clone();
        item.unmapped_after = true;
        self.items = self.items.tail().push(item);
    }

    /// Record a change this branch's events must be mapped through (PM's
    /// `addMaps`), folded into the previous item when that is a change of the same
    /// kind. Nothing to map when the branch holds no event.
    pub(super) fn add_maps(&mut self, maps: &[StepMap]) {
        if self.event_count == 0 {
            *self = Branch::default();
            return;
        }
        for map in maps {
            let folded = match self.items.head() {
                Some(head) if head.peer => fold(head, map),
                _ => None,
            };
            let item = match folded {
                Some(item) => {
                    self.items = self.items.tail();
                    item
                }
                None => peer_item(map.clone()),
            };
            self.items = self.items.push(item);
        }
    }

    /// Record the maps of the table repair an undo or redo of this branch's
    /// events made (#1940): as a peer's change, and one that moved nothing (a
    /// span set) as [`Branch::note_unmapped`] records a peer's attribute change.
    pub(super) fn add_repair_maps(&mut self, repair: &[StepMap]) {
        let maps: Vec<StepMap> = repair.iter().filter(|m| !m.is_empty()).cloned().collect();
        if maps.len() < repair.len() {
            self.note_unmapped();
        }
        self.add_maps(&maps);
    }

    /// Hide the oldest events past `depth`, and drop hidden ones from the list
    /// once there are enough of them.
    pub(super) fn cut_to_depth(&mut self, depth: usize) {
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
    /// selection to restore, the branch without the event, the event, and
    /// whether a step was mapped through a change the history did not record.
    pub(super) fn pop_event(&self, state: &EditorState) -> Option<Popped> {
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
        // A change the history did not record and that moved no position (a
        // peer's `colspan`) came after the event: the undo is not its plain
        // inverse either (#1937).
        let unmapped = seg.iter().any(|item| item.unmapped_after);
        let mut tr = state.tr();
        let mut remap: Option<(Mapping, Vec<usize>)> = None;
        let mut add_before: Vec<Item> = Vec::new();
        let mut add_after: Vec<Item> = Vec::new();
        // Whether a step that was not mapped failed to apply.
        let mut skipped = false;
        for i in (0..len).rev() {
            let item = seg[i];
            let Some(step) = &item.step else {
                if remap.is_none() {
                    remap = Some(remapping(&seg, i + 1));
                }
                add_before.push(item.clone());
                continue;
            };
            if let Some((remap, starts)) = remap.as_mut() {
                add_before.push(Item::map_only(item.map.clone(), None));
                // Everything after this step: the maps of the items after it, and
                // of what this undo has applied so far.
                let after = remap.slice(starts[i + 1], remap.len());
                let pieces = split_delete(step.as_ref(), &after, tr.doc());
                let mapped = if pieces.is_none() {
                    step.map(&after)
                } else {
                    None
                };
                // Which range of the mapped step's map each of the step's own
                // became, for a step that lost or joined some (#1940).
                let range_index = step
                    .as_any()
                    .downcast_ref::<BatchStep>()
                    .filter(|_| {
                        mapped
                            .as_ref()
                            .is_some_and(|m| m.get_map().ranges().len() != item.map.ranges().len())
                    })
                    .map(|b| b.mapped_range_index(&after));
                drop(after);
                if let Some(pieces) = pieces {
                    // A delete of text a peer has since typed inside: delete my
                    // pieces around theirs, back to front.
                    for &(from, to) in pieces.iter().rev() {
                        let piece = ReplaceStep::new(from, to, Slice::empty());
                        if tr.step(Box::new(piece)).is_ok() {
                            let map = last_map(&tr);
                            add_after.push(Item::map_only(map.clone(), None));
                            remap.append_map(map);
                        }
                    }
                } else if let Some(mapped) = mapped
                    && tr.step(mapped).is_ok()
                {
                    let mut map = last_map(&tr);
                    // Mirrored with the step's own map, so a position a map deleted
                    // is recovered by its mirror's range of the same index. A mapped
                    // step can lose ranges — a `BatchStep` drops each replace a later
                    // change deleted across (a `mergeCells` undone after a peer
                    // replaced the table), and may still apply, its attributes kept.
                    // Mirrored range for range, recovering into it indexed past its
                    // end (#1935); left unmirrored, a position the ranges it kept
                    // restore was lost, and an undo of typing in a cell the merge
                    // emptied did nothing (#1940). So its map carries which of its
                    // ranges holds each of the step's, and where: the ones it kept
                    // (alone or joined) recover, the ones it lost map a position
                    // through both.
                    let mut mirrored = map.ranges().len() == item.map.ranges().len();
                    if !mirrored
                        && let Some(index) = range_index
                        && index.len() * 3 == item.map.ranges().len()
                        && index
                            .iter()
                            .flatten()
                            .all(|&(k, _)| k * 3 < map.ranges().len())
                    {
                        map = map.with_mirror_index(index);
                        mirrored = true;
                    }
                    let offset = add_after.len() + add_before.len();
                    add_after.push(Item::map_only(map.clone(), mirrored.then_some(offset)));
                    if mirrored {
                        remap.append_map_mirrored(map, starts[i]);
                    } else {
                        remap.append_map(map);
                    }
                }
            } else {
                // Nothing recorded after this step: it applies where it was made.
                // A step that fails is skipped, as PM's `maybeStep` does.
                skipped |= tr.step(step.clone()).is_err();
            }
            if let Some(info) = &item.event {
                let selection = match &remap {
                    Some((r, starts)) => info.selection.map(tr.doc(), &r.slice(starts[i], r.len())),
                    // A change the history did not record that moved no position
                    // (a peer's retype, #1942) can make an inverse no longer fit,
                    // and the document is then not the one the selection was
                    // made in: resolve it in this one (clamped, and moved into
                    // text), as a mapped one is.
                    None if unmapped || skipped => info.selection.map(tr.doc(), &Mapping::new()),
                    None => info.selection.clone(),
                };
                // Whether a change the history did not record (a peer's) came
                // after the event: only then can the result differ from the
                // document before the event (#1937).
                let remapped = remap.is_some() || unmapped;
                let (tr, selection, branch, info) =
                    self.finish(tr, selection, rest, add_before, add_after, info);
                return Some((tr, selection, branch, info, remapped));
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

/// The map of the step `tr` applied last.
fn last_map(tr: &Transaction) -> StepMap {
    tr.mapping()
        .maps()
        .last()
        .cloned()
        .unwrap_or_else(StepMap::empty)
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

/// A peer item for `map`: a one-char deletion starts a run of them.
pub(super) fn peer_item(map: StepMap) -> Item {
    let unit_run = unit_deletion(&map).map(|at| (at, 1));
    Item {
        peer: true,
        unit_run,
        ..Item::map_only(map, None)
    }
}

/// `Some(at)` when `map` deletes exactly one position at `at` and inserts nothing.
fn unit_deletion(map: &StepMap) -> Option<usize> {
    let r = map.ranges();
    (r.len() == 3 && r[1] == 1 && r[2] == 0).then_some(r[0])
}

/// `head` (a peer item) followed by `next` as one item, when that maps every
/// position as the two in turn do — `None` otherwise, and `next` gets an item of
/// its own. A one-char deletion beside a run of them (Backspace before it, Delete
/// after it) extends the run; anything else folds only where
/// [`StepMap::compose_exact`] finds the result exact, which a run of typing is and
/// an insertion beside a deletion is not (review of #1889: folding those lost the
/// position between them, so an undo deleted a peer's char or dropped my restore).
pub(super) fn fold(head: &Item, next: &StepMap) -> Option<Item> {
    if let Some((at, n)) = head.unit_run {
        // After the run, its `n` chars are gone and position `at` stands where they
        // were: a deletion of the char before (`at - 1`) or after (`at`) makes it
        // `n + 1` one-char deletions in a row, which map exactly as the same
        // number of deletions at the run's new start.
        let start = match unit_deletion(next)? {
            d if d == at => at,
            d if d + 1 == at => d,
            _ => return None,
        };
        return Some(Item {
            unit_run: Some((start, n + 1)),
            ..peer_item(StepMap::new(vec![start, n + 1, 0]))
        });
    }
    let map = head.map.compose_exact(next)?;
    Some(Item {
        unit_run: None,
        ..peer_item(map)
    })
}

/// The maps of `seg[..to]` (oldest first), with their mirror relations (PM's
/// `remapping`), and where each item's maps start in it (`starts[i]`, with
/// `starts[to]` the end): a unit run is several maps.
pub(super) fn remapping(seg: &[&Item], to: usize) -> (Mapping, Vec<usize>) {
    let mut maps = Mapping::new();
    let mut starts = Vec::with_capacity(to + 1);
    for (i, item) in seg.iter().enumerate().take(to) {
        starts.push(maps.len());
        if let Some((at, n)) = item.unit_run {
            for _ in 0..n {
                maps.append_map(StepMap::new(vec![at, 1, 0]));
            }
            continue;
        }
        let mirror = item
            .mirror_offset
            .filter(|&off| i >= off)
            .map(|off| starts[i - off]);
        match mirror {
            Some(m) => maps.append_map_mirrored(item.map.clone(), m),
            None => maps.append_map(item.map.clone()),
        }
    }
    starts.push(maps.len());
    (maps, starts)
}
