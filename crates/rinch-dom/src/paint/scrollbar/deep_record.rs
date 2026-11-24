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
    /// What was pushed of it or below it since its last measure
    /// ([`SELF`], [`BELOW`]).
    pub(super) touch: u8,
}

/// The records of every kept walk, by box. A walk inserts one per box of
/// its container's subtree, so the map is keyed with FxHash: with the std
/// `SipHash` the inserts were most of a walk's cost (`row_shift_damage` in
/// `rinch-bench`). Node ids come from the tree's slab, not from outside, so
/// there is nothing to guard against collisions for; they carry a
/// generation, so a freed slot's new node never matches an old record.
pub(super) type Records = rustc_hash::FxHashMap<RawNodeId, Rec>;

impl Rec {
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
        let mut rec = Rec {
            snap,
            owner: self.owner,
            parent,
            kids,
            contrib: Span::EMPTY,
            touch: 0,
        };
        rec.contrib = rec.contribution();
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
        let touch = rec.touch;
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
        let kids = self.children(id, Level::Below, false);
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
