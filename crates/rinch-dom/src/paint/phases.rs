//! CSS 2.1 Appendix E's phases for in-flow content (#2276).
//!
//! A stacking context paints its in-flow, non-positioned descendants in
//! phases, not one box at a time: every block background and border (step 4),
//! then every float, each whole (step 5), then every line's content — text,
//! inline backgrounds, atomic inlines, list markers, replaced content (step 7).
//! So a later block's background never covers an earlier block's text, and a
//! float overhanging the next block is drawn over that block's background.
//!
//! **Atomic units restart the phases.** A float, an atomic inline
//! (`inline-block`, painted from its IFC) and a flex or grid item
//! (css-flexbox-1 §5.4, css-grid-1 §6: "exactly the same as inline blocks")
//! are painted as if they created a stacking context, so a later flex item
//! covers an earlier one's text, as in Chrome. A stacking context, and every
//! positioned box, is an entry of a sequence and is painted whole there.
//!
//! **Painted whole unless it matters.** Splitting walks the in-flow run three
//! times. Painting each child whole, in tree order, gives the same pixels
//! whenever no later child reaches what an earlier child painted, which is
//! every block-flow run where nothing overlaps: blocks stacked one under the
//! next. [`splits`] answers that from each child's in-flow extent (its box,
//! its lines, and its descendants' extents, the way hit testing's flow extent
//! is built), memoised per layout ([`PhaseMemo`], keyed on the tree's
//! `layout_epoch`), so a run splits only where a negative margin, an overflowing
//! line, a float or an overhanging descendant makes one child reach into an
//! earlier one's extent. A wrong `true` costs two extra walks of that run and
//! changes no pixel; a `false` paints the run as before #2276.
//!
//! **How a phase reaches a box.** The phase is a property of the walk, not of
//! the box, so it is thread-local state rather than a parameter of every paint
//! function: the run's loop asks for a phase just before it enters a child
//! ([`request`]), and [`super::paint_node`] takes that request on entry
//! ([`enter`]), so every other caller — a stacking-context entry, an atomic
//! inline drawn by its IFC, a filter's raster, a held highlight — enters in
//! [`Phase::All`] without asking.

use std::cell::Cell;

use std::cell::RefCell;

use crate::node::{Node, NodeTree, RawNodeId};
use crate::stacking::paints_at_stacking_root;

/// Which part of a box's in-flow paint the walk is drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Everything, in the box's own order: the box is painted whole, and its
    /// own in-flow run decides for itself whether to split.
    All,
    /// Step 4: backgrounds, borders and box shadows of block-level boxes.
    Backgrounds,
    /// Step 5: the floats, each painted whole.
    Floats,
    /// Step 7: line content, replaced content, outlines and scrollbars, and
    /// every atomic child (a flex or grid item) whole.
    Content,
}

impl Phase {
    /// Whether the box's own background, border and shadows are drawn.
    pub(crate) fn draws_box(self) -> bool {
        matches!(self, Phase::All | Phase::Backgrounds)
    }

    /// Whether the box's own content — its lines, replaced content, form
    /// value, outline and scrollbars — is drawn.
    pub(crate) fn draws_content(self) -> bool {
        matches!(self, Phase::All | Phase::Content)
    }

    /// The three passes a split run is walked in.
    pub(crate) const SPLIT: [Phase; 3] = [Phase::Backgrounds, Phase::Floats, Phase::Content];
}

thread_local! {
    /// The phase the next [`super::paint_node`] enters in.
    static REQUEST: Cell<Phase> = const { Cell::new(Phase::All) };
    /// The phase of the box being painted.
    static CURRENT: Cell<Phase> = const { Cell::new(Phase::All) };
}

/// Ask for the next [`enter`] to be in `phase`.
pub(crate) fn request(phase: Phase) {
    REQUEST.with(|r| r.set(phase));
}

/// The phase of the box being painted.
pub(crate) fn current() -> Phase {
    CURRENT.with(Cell::get)
}

/// Restores the phase of the box around this one on drop.
pub(crate) struct Entered(Phase);

impl Drop for Entered {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0));
    }
}

/// Enter a box in the requested phase (and reset the request to
/// [`Phase::All`]), for as long as the guard lives.
pub(crate) fn enter() -> Entered {
    let phase = REQUEST.with(|r| r.replace(Phase::All));
    Entered(CURRENT.with(|c| c.replace(phase)))
}

/// The phase `child` is entered in when its parent's run is walked in
/// `phase`, or `None` when the child draws nothing in that pass.
pub(crate) fn child_phase(phase: Phase, child: &Node) -> Option<Phase> {
    match phase {
        Phase::All => Some(Phase::All),
        // An atomic unit: painted whole, with the line content.
        _ if child.is_flex_or_grid_item.get() => (phase == Phase::Content).then_some(Phase::All),
        _ if is_float(child) => (phase == Phase::Floats).then_some(Phase::All),
        p => Some(p),
    }
}

/// A float that floats: not a flex or grid item, whose `float` is ignored.
pub(crate) fn is_float(node: &Node) -> bool {
    node.computed_style.float != taffy::Float::None && !node.is_flex_or_grid_item.get()
}

/// `[x0, y0, x1, y1]`, relative to a box's border-box origin, in CSS px.
type Extent = [f32; 4];

const EMPTY: Extent = [
    f32::INFINITY,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NEG_INFINITY,
];

fn is_empty(e: &Extent) -> bool {
    !(e[0] < e[2] && e[1] < e[3])
}

/// Whether `child` is in its parent's in-flow run: not hoisted to a stacking
/// root, and not drawn by its IFC.
fn in_run(tree: &NodeTree, child: &Node) -> bool {
    !paints_at_stacking_root(child) && !super::drawn_by_its_ifc(tree, child)
}

/// Where `child` sits in its parent's run, relative to the parent's
/// border-box origin, before the parent's own scroll offset.
fn child_origin(tree: &NodeTree, child: &Node) -> (f32, f32) {
    let (dx, dy) = super::ifc_content_box_offset(tree, child);
    (child.layout.x + dx, child.layout.y + dy)
}

/// What `id`'s in-flow paint covers, relative to its border-box origin: its
/// own box, its lines, and — on an axis it does not clip — its run's
/// extents. Ink (shadows, outlines, glyph overhang) is left out; a missed
/// overlap paints as before #2276.
fn extent(tree: &NodeTree, id: RawNodeId) -> Extent {
    if let Some(e) = tree.phase_memo.extent(tree, id) {
        return e;
    }
    let Some(node) = tree.get_live(id) else {
        return EMPTY;
    };
    let (w, h) = (node.layout.width, node.layout.height);
    // A box with no area paints no background, and `display: contents`
    // generates none.
    let mut e = if w > 0.0 && h > 0.0 {
        [0.0, 0.0, w, h]
    } else {
        EMPTY
    };
    let (clip_x, clip_y) = node.clip_axes();
    if !(clip_x && clip_y) {
        let (sx, sy) = (node.scroll_offset.0 as f32, node.scroll_offset.1 as f32);
        let mut grow = |c: Extent| {
            if is_empty(&c) {
                return;
            }
            if !clip_x {
                e[0] = e[0].min(c[0]);
                e[2] = e[2].max(c[2]);
            }
            if !clip_y {
                e[1] = e[1].min(c[1]);
                e[3] = e[3].max(c[3]);
            }
        };
        if let Some(inline) = node.text_layout.as_ref() {
            let (ox, oy) = super::ifc_root_content_origin(node);
            let (lw, lh) = (inline.layout.width(), inline.layout.height());
            if lw > 0.0 && lh > 0.0 {
                grow([ox - sx, oy - sy, ox - sx + lw, oy - sy + lh]);
            }
        }
        for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, id).iter() {
            let Some(child) = tree.get_live(child_id).filter(|c| in_run(tree, c)) else {
                continue;
            };
            let (cx, cy) = child_origin(tree, child);
            let c = extent(tree, child.id);
            if !is_empty(&c) {
                grow([
                    cx - sx + c[0],
                    cy - sy + c[1],
                    cx - sx + c[2],
                    cy - sy + c[3],
                ]);
            }
        }
    }
    tree.phase_memo.store_extent(id, e);
    e
}

/// Whether `id`'s in-flow run must be painted in phases: one of its children
/// reaches into what an earlier one paints. A flex or grid container's run
/// never splits: its items are atomic, painted whole in order.
pub(crate) fn splits(tree: &NodeTree, id: RawNodeId) -> bool {
    if let Some(s) = tree.phase_memo.splits(tree, id) {
        return s;
    }
    let kids = crate::RinchDocument::box_tree_children(&tree.nodes, id);
    let run = || {
        kids.iter()
            .filter_map(|&c| tree.get_live(c))
            .filter(|c| in_run(tree, c))
    };
    // One child cannot reach into an earlier one: no extent is worked out
    // for a wrapper's only child, nor for a flex or grid container's items.
    let split = run().nth(1).is_some() && {
        let mut seen = EMPTY;
        let mut split = false;
        for child in run() {
            if child.is_flex_or_grid_item.get() {
                split = false;
                break;
            }
            let c = extent(tree, child.id);
            if is_empty(&c) {
                continue;
            }
            let (cx, cy) = child_origin(tree, child);
            let c = [cx + c[0], cy + c[1], cx + c[2], cy + c[3]];
            // An overlap of positive area: boxes stacked one under the next
            // share an edge, not an area.
            const EPS: f32 = 1e-3;
            if c[0].max(seen[0]) + EPS < c[2].min(seen[2])
                && c[1].max(seen[1]) + EPS < c[3].min(seen[3])
            {
                split = true;
                break;
            }
            seen = [
                seen[0].min(c[0]),
                seen[1].min(c[1]),
                seen[2].max(c[2]),
                seen[3].max(c[3]),
            ];
        }
        split
    };
    tree.phase_memo.store_splits(id, split);
    split
}

/// [`extent`] and [`splits`], memoised for one `layout_epoch`.
#[derive(Debug, Default)]
pub(crate) struct PhaseMemo {
    state: RefCell<MemoState>,
}

#[derive(Debug, Default)]
struct MemoState {
    epoch: Option<u64>,
    /// By node slot.
    extents: Vec<Option<Extent>>,
    /// By node slot.
    splits: Vec<Option<bool>>,
}

impl PhaseMemo {
    /// The state for `tree`'s current layout, cleared when it moved.
    fn current(&self, tree: &NodeTree) -> std::cell::RefMut<'_, MemoState> {
        let mut s = self.state.borrow_mut();
        if s.epoch != Some(tree.layout_epoch) {
            s.epoch = Some(tree.layout_epoch);
            s.extents.clear();
            s.splits.clear();
        }
        s
    }

    fn extent(&self, tree: &NodeTree, id: RawNodeId) -> Option<Extent> {
        let slot = crate::node_slab::slot_of(id);
        self.current(tree).extents.get(slot).copied().flatten()
    }

    /// Store after [`Self::extent`] (or [`Self::splits`]) brought the state
    /// to the current layout.
    fn store_extent(&self, id: RawNodeId, e: Extent) {
        let slot = crate::node_slab::slot_of(id);
        let mut s = self.state.borrow_mut();
        if s.extents.len() <= slot {
            s.extents.resize(slot + 1, None);
        }
        s.extents[slot] = Some(e);
    }

    fn splits(&self, tree: &NodeTree, id: RawNodeId) -> Option<bool> {
        let slot = crate::node_slab::slot_of(id);
        self.current(tree).splits.get(slot).copied().flatten()
    }

    fn store_splits(&self, id: RawNodeId, split: bool) {
        let slot = crate::node_slab::slot_of(id);
        let mut s = self.state.borrow_mut();
        if s.splits.len() <= slot {
            s.splits.resize(slot + 1, None);
        }
        s.splits[slot] = Some(split);
    }
}
