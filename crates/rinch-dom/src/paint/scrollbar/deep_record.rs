//! What a kept walk records of each box it met ([`Rec`]), the walk that
//! records it, and the re-measure that brings a record up to date in place
//! ([`Walker`]) — the two share every rule, so a kept range and a fresh one
//! are the same sum taken the same way (#2054).
//!
//! Every record is in its box's own frame: what lies below a box is kept
//! from where that box places its children ([`WalkSnap::offset`]), not from
//! the container. A box that moves carries what is below it unchanged, so
//! a layout that shifts the rows after a removed one re-measures each row
//! where it is now, and nothing below any of them.

use super::deep_walk::{Span, WalkSnap, walk_out};
use super::extents::Reach;
use crate::NodeTree;
use crate::computed_style::{DisplayValue, PositionValue};
use crate::node::RawNodeId;

/// The box itself was pushed paint-dirty since the last check.
pub(super) const SELF: u8 = 1;
/// Something below the box was: a box pushed, added or taken out.
pub(super) const BELOW: u8 = 2;
/// Something the records cannot follow (an element outside the box tree
/// changed): walk the container again.
pub(super) const AGAIN: u8 = 4;
/// Its children changed as a list: one was added, taken out or moved away.
/// Its children's contributions are summed again from all of them, not
/// brought up to date from the ones marked ([`Rec::first`]).
pub(super) const STRUCT: u8 = 8;

/// No box: the end of a list of marked children ([`Rec::first`]).
pub(super) const NO_ID: RawNodeId = RawNodeId::MAX;

/// One box a kept walk met: what it read of it ([`WalkSnap`]), the walk it
/// belongs to and where in it, and what it found below it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Rec {
    pub(super) snap: WalkSnap,
    /// The scroll container whose walk met it. A box is met by one walk at
    /// most: a walk stops at a scroll container below it, which clips.
    pub(super) owner: RawNodeId,
    /// Its box-tree parent when it was met.
    pub(super) parent: RawNodeId,
    /// Its children's contributions ([`Rec::contribution`]), in its own
    /// children frame; empty for a box the walk does not go into.
    pub(super) kids: Span,
    /// Its own contribution, kept with the rest ([`Rec::contribution`]): a
    /// box nothing was pushed of answers it as it is.
    pub(super) contrib: Span,
    /// What was pushed of it or below it in check [`Rec::epoch`]
    /// ([`SELF`], [`BELOW`], [`STRUCT`]); nothing in any other check.
    pub(super) touch: u8,
    /// The check (`DeepOverflowState::epoch`) that last marked it: its
    /// `touch`, `first` and `next` are that check's, and stale in any other.
    /// Every mark a check makes is brought up to date within it, so a mark
    /// left on a box nothing visited (below a box the walk does not go
    /// into) never reaches a later check.
    pub(super) epoch: u64,
    /// The first of its children marked in this check, each listing the
    /// next (`next`), or [`NO_ID`]: what brings its children's span up to
    /// date without visiting the children nothing was pushed of.
    pub(super) first: RawNodeId,
    /// The next child of its parent marked in this check, or [`NO_ID`].
    pub(super) next: RawNodeId,
}

/// The records of every kept walk, by box. A walk inserts one per box of
/// its container's subtree, so the map is keyed with FxHash: with the std
/// `SipHash` the inserts were most of a walk's cost (`row_shift_damage` in
/// `rinch-bench`). Node ids come from the tree's slab, not from outside, so
/// there is nothing to guard against collisions for.
///
/// A record lives as long as its box is where the walk met it: a box taken
/// out of the tree (`DeepOverflow::forget_detached`) or freed
/// (`DeepOverflow::forget_freed`) has its records dropped then, and a walk
/// dropped to be taken again drops the records it made. So the map holds at
/// most one record per box in the kept walks, and a freed slot's new node
/// (whose id differs only in its generation, which wraps) finds none.
pub(super) type Records = rustc_hash::FxHashMap<RawNodeId, Rec>;

impl Rec {
    /// A record of a box met by `owner`'s walk under `parent`, nothing
    /// marked.
    pub(super) fn new(snap: WalkSnap, owner: RawNodeId, parent: RawNodeId, kids: Span) -> Rec {
        let mut rec = Rec {
            snap,
            owner,
            parent,
            kids,
            contrib: Span::EMPTY,
            touch: 0,
            epoch: 0,
            first: NO_ID,
            next: NO_ID,
        };
        rec.contrib = rec.contribution();
        rec
    }

    /// What check `epoch` marked of it.
    pub(super) fn marks(&self, epoch: u64) -> u8 {
        if self.epoch == epoch { self.touch } else { 0 }
    }

    /// Mark it with `flag` in check `epoch`; whether nothing was marked of
    /// it in that check before (it is then in no list of marked children).
    pub(super) fn mark(&mut self, epoch: u64, flag: u8) -> bool {
        if self.epoch != epoch {
            self.epoch = epoch;
            self.touch = flag;
            self.first = NO_ID;
            self.next = NO_ID;
            return true;
        }
        let was = self.touch;
        self.touch |= flag;
        was == 0
    }

    /// What the box adds to its parent's scrollable overflow, in its
    /// parent's children frame: its border box (no margins) and, unless the
    /// walk stops at it, its lines and what is below it; a `display:
    /// contents` wrapper's children's, in that same frame; nothing of a box
    /// that is not the container's overflow, not displayed, or transformed
    /// (measured where it is drawn on each read instead).
    pub(super) fn contribution(&self) -> Span {
        let s = &self.snap;
        if s.out || s.display == DisplayValue::None {
            return Span::EMPTY;
        }
        if s.contents {
            return if s.entered() { self.kids } else { Span::EMPTY };
        }
        if s.transformed {
            return Span::EMPTY;
        }
        let (x, y) = s.offset();
        let (w, h) = (s.layout.width as f64, s.layout.height as f64);
        let mut span = Span::EMPTY;
        if w > 0.0 || h > 0.0 {
            span.grow(x, x + w, y + h);
        }
        if s.entered() {
            let mut below = self.kids;
            below.join(s.lines_span());
            span.join(below.moved((x, y)));
        }
        span
    }
}

/// Which rule decides whether a box is the container's overflow: a child of
/// the container is entered unless it is out of flow above it; a box below
/// that also counts nothing when it is an empty absolute box ([`walk_out`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Level {
    Top,
    Below,
}

/// One container's walk over the records: [`Walker::record`] records a
/// subtree fresh, [`Walker::visit`] brings a recorded one up to date.
pub(super) struct Walker<'a> {
    pub(super) tree: &'a NodeTree,
    pub(super) owner: RawNodeId,
    pub(super) recs: &'a mut Records,
    /// The transformed boxes met below a top box, not entered: measured on
    /// each read, where they are drawn.
    pub(super) live: &'a mut Vec<RawNodeId>,
    /// Boxes measured ([`WalkSnap::of`]), for the tests that pin the cost.
    pub(super) measured: u64,
    /// The records cannot follow what changed (a box's kind, a box met
    /// under another parent or in another walk): walk the container again.
    pub(super) again: bool,
    /// The check whose marks it brings up to date ([`Rec::marks`]).
    pub(super) epoch: u64,
}

/// Whether `node`, at `level` below scroll container `owner`, is not its
/// overflow ([`Level`]).
fn out_of(tree: &NodeTree, owner: RawNodeId, node: &crate::node::Node, level: Level) -> bool {
    match level {
        Level::Top => {
            !(node.taffy_style_owned_by_contents_splice()
                || crate::out_of_flow::contributes_to_scrollable_overflow(tree, owner, node))
        }
        Level::Below => walk_out(tree, owner, node),
    }
}

impl Walker<'_> {
    /// The contributions of `id`'s box-tree children, in its children frame
    /// — each recorded fresh (`fresh`), or brought up to date.
    pub(super) fn children(&mut self, id: RawNodeId, level: Level, fresh: bool) -> Span {
        let tree = self.tree;
        let mut kids = Span::EMPTY;
        for &child in crate::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            let span = if fresh {
                self.record(child, id, level)
            } else {
                self.visit(child, id, level)
            };
            if self.again {
                return Span::EMPTY;
            }
            kids.join(span);
        }
        kids
    }

    /// Record box `id`, a box-tree child of `parent`, and everything below
    /// it the walk goes into; its contribution.
    pub(super) fn record(&mut self, id: RawNodeId, parent: RawNodeId, level: Level) -> Span {
        let Some(node) = self.tree.get_live(id) else {
            return Span::EMPTY;
        };
        if node.is_text() {
            return Span::EMPTY;
        }
        let snap = WalkSnap::of(self.tree, node, out_of(self.tree, self.owner, node, level));
        self.measured += 1;
        if level == Level::Below
            && snap.transformed
            && !snap.out
            && !snap.contents
            && snap.display != DisplayValue::None
            && !self.live.contains(&id)
        {
            self.live.push(id);
        }
        let kids = if snap.entered() {
            self.children(id, Level::Below, true)
        } else {
            Span::EMPTY
        };
        let rec = Rec::new(snap, self.owner, parent, kids);
        self.recs.insert(id, rec);
        rec.contrib
    }

    /// Box `id`'s contribution, its record brought up to date: as recorded
    /// when nothing was pushed of it or below it; re-measured where it is
    /// when it was — only its box, its lines and what is below it may have
    /// changed, and only what was pushed below it is visited. A box with no
    /// record is new here, and recorded. Anything else (a box whose kind
    /// changed, met under another parent) sets [`Self::again`].
    pub(super) fn visit(&mut self, id: RawNodeId, parent: RawNodeId, level: Level) -> Span {
        let Some(node) = self.tree.get_live(id) else {
            return Span::EMPTY;
        };
        if node.is_text() {
            return Span::EMPTY;
        }
        let (tree, owner) = (self.tree, self.owner);
        let Some(rec) = self.recs.get_mut(&id) else {
            return self.record(id, parent, level);
        };
        if rec.owner != owner || rec.parent != parent {
            self.again = true;
            return Span::EMPTY;
        }
        let touch = rec.marks(self.epoch);
        if touch == 0 {
            return rec.contrib;
        }
        // An absolute box is out when it has no area: its size decides.
        let out = if rec.snap.position == PositionValue::Absolute {
            out_of(tree, owner, node, level)
        } else {
            rec.snap.out
        };
        let now = WalkSnap::of(tree, node, out);
        self.measured += 1;
        if !now.same_kind(&rec.snap) || now.out != rec.snap.out {
            self.again = true;
            return Span::EMPTY;
        }
        rec.snap = now;
        rec.touch = 0;
        if !(now.entered() && touch & BELOW != 0) {
            rec.contrib = rec.contribution();
            return rec.contrib;
        }
        let (kids, first) = (rec.kids, rec.first);
        let kids = if touch & STRUCT != 0 {
            self.children(id, Level::Below, false)
        } else {
            self.marked_children(id, kids, first)
        };
        if self.again {
            return Span::EMPTY;
        }
        let Some(rec) = self.recs.get_mut(&id) else {
            return Span::EMPTY;
        };
        rec.kids = kids;
        rec.contrib = rec.contribution();
        rec.contrib
    }

    /// Box `id`'s children's contributions, from `kids` (their span before)
    /// and its children marked in this check, from `first` on: each is
    /// visited, and its new contribution joined to the span — unless its old
    /// one held an edge of it that the new one does not reach, which only
    /// the sum over all the children can say. So a change below one child of
    /// a box with many visits that child alone.
    ///
    /// The list names only children recorded under `id` (a box moved away
    /// or taken out marks its parent [`STRUCT`], which sums them all); a
    /// name that is not one sums them all too.
    fn marked_children(&mut self, id: RawNodeId, mut kids: Span, first: RawNodeId) -> Span {
        let mut whole = false;
        let mut child = first;
        while child != NO_ID {
            let Some(rec) = self.recs.get(&child).filter(|r| r.parent == id) else {
                whole = true;
                break;
            };
            let (old, next) = (rec.contrib, rec.next);
            let new = self.visit(child, id, Level::Below);
            if self.again {
                return Span::EMPTY;
            }
            if !whole && new != old {
                if old.holds_an_edge_of(&kids, &new) {
                    whole = true;
                } else {
                    kids.join(new);
                }
            }
            child = next;
        }
        if whole {
            // Every marked child visited (a visit clears its mark): the rest
            // answer their kept contributions.
            return self.children(id, Level::Below, false);
        }
        kids
    }

    /// The container's reach from the records of its children, which are up
    /// to date: for each top box, everything below it — unless all of it is
    /// inside the top box's border box, which the container's own measure
    /// counts at least as far ([`WalkSnap::containing_rect`]).
    pub(super) fn top_reach(&self, container: RawNodeId) -> Reach {
        let mut reach = Reach::NONE;
        for &child in crate::RinchDocument::box_tree_children(&self.tree.nodes, container).iter() {
            self.top(child, true, &mut reach);
        }
        reach
    }

    /// Grow `reach` by top box `id` — a child of the container (`direct`),
    /// or one below `display: contents` wrappers that are — or, for a
    /// wrapper, by the top boxes below it. A child of the container counts
    /// what is below it alone (the container's measure counts the child);
    /// one below a wrapper counts itself too, as the walk below a child does.
    fn top(&self, id: RawNodeId, direct: bool, reach: &mut Reach) {
        let Some(rec) = self.recs.get(&id) else {
            return;
        };
        let s = &rec.snap;
        if s.out || s.display == DisplayValue::None {
            return;
        }
        if s.contents {
            if s.entered() {
                for &child in crate::RinchDocument::box_tree_children(&self.tree.nodes, id).iter() {
                    self.top(child, false, reach);
                }
            }
            return;
        }
        let span = if direct {
            if !s.entered() {
                return;
            }
            rec.kids.moved(s.offset())
        } else {
            rec.contrib
        };
        if span == Span::EMPTY
            || s.containing_rect(s.offset())
                .is_some_and(|rect| span.inside(&rect))
        {
            return;
        }
        reach.grow(span.left, span.right, span.bottom);
    }
}
