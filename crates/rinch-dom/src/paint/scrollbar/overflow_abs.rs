//! The absolute boxes listed for a container from below its children (#770,
//! `NodeTree::overflow_absolutes`): where each is placed, and the memo of
//! their extent.

use super::measure::is_empty_box;
use crate::NodeTree;
use crate::computed_style::PositionValue;
use crate::node::RawNodeId;

/// `NodeTree::overflow_absolutes`: every `absolute` box the layout read back
/// (`listed`, #770), and a memo of them, each assigned to the scroll
/// container whose range it is (`out_of_flow::overflow_scroller`) and
/// measured in that container's border-box frame — filled by the first scroll
/// range asked for after the geometry changed and kept until it changes
/// again.
///
/// **Keyed on `HitCache::shape_generation`**, which everything that can move
/// a box moves: a layout, a mutation, a restyle, a tick that changes a
/// box's `HitStyleKey` — and `out_of_flow::replace_all`, which moves boxes
/// after the layout's clamp may already have measured them. A scroll of a box
/// that clips does not: a listed box's place in its container is measured
/// with the scroll of every box between them taken off, and the ones that
/// scroll away from it are written back by `out_of_flow::replace_after_scroll`
/// with their scroll added, so it is the same at every offset.
///
/// **A box with a transform on its way** — its own, or any box's between it
/// and its container, identity included — is not measured here but listed
/// in `live`, and measured by every call: a tick writes a transform's value
/// without moving the generation (only whether there is one is in the key).
/// So is a box whose way up the walk below does not follow (an absolute box
/// between it and its containing block's chain), measured as
/// `out_of_flow::overflow_scroller` and [`border_box_in`](super::measure::border_box_in) say.
#[derive(Debug, Default)]
pub(crate) struct OverflowAbsolutes {
    /// `(absolute box, its out_of_flow_kind)`, in read order. Cleared by
    /// `out_of_flow::begin_read`.
    pub(crate) listed: Vec<(RawNodeId, Option<crate::out_of_flow::OutOfFlowKind>)>,
    state: std::cell::RefCell<OverflowAbsState>,
}

#[derive(Debug, Default)]
pub(crate) struct OverflowAbsState {
    /// The generation `measured` and `live` were filled at.
    filled_at: Option<u64>,
    /// `(scroll container, left, right, bottom)`: the furthest border-box
    /// edges of its listed boxes with nothing transformed on their way, in
    /// its border-box frame. One per container, sorted by container.
    pub(super) measured: Vec<(RawNodeId, f64, f64, f64)>,
    /// `(scroll container, absolute box)` for the boxes measured per call,
    /// sorted by container.
    pub(super) live: Vec<(RawNodeId, RawNodeId)>,
}

impl OverflowAbsolutes {
    /// The memo, filled for `tree`'s current geometry.
    pub(super) fn current<'a>(&'a self, tree: &NodeTree) -> std::cell::Ref<'a, OverflowAbsState> {
        let generation = tree.hit_cache.shape_generation();
        if self.state.borrow().filled_at != Some(generation) {
            let mut state = self.state.borrow_mut();
            state.fill(tree);
            state.filled_at = Some(generation);
        }
        self.state.borrow()
    }
}

/// Where one listed box's walk up ended.
pub(super) enum Placed {
    /// It counts nowhere: no scroll container takes it (a `clip` box clips
    /// it, the viewport holds a `fixed` box), or the container's own walk
    /// of its children measures it.
    Nowhere,
    /// Its container, and its border box's left, right and bottom edges
    /// there, with nothing transformed on the way.
    Measured(RawNodeId, f64, f64, f64),
    /// Measured per call, in this container.
    Live(RawNodeId),
}

impl OverflowAbsState {
    fn fill(&mut self, tree: &NodeTree) {
        self.measured.clear();
        self.live.clear();
        let body_scrolls = tree.get(tree.body_id).is_some_and(|body| {
            let (clip_x, clip_y) = body.clip_axes();
            (clip_x || clip_y) && crate::out_of_flow::is_scroll_container(body)
        });
        let mut sorted = true;
        for &(abs_id, kind) in &tree.overflow_absolutes.listed {
            match place_listed(tree, abs_id, kind, body_scrolls) {
                Placed::Nowhere => {}
                Placed::Measured(scroller, x0, x1, y1) => match self.measured.last_mut() {
                    Some(last) if last.0 == scroller => {
                        last.1 = last.1.min(x0);
                        last.2 = last.2.max(x1);
                        last.3 = last.3.max(y1);
                    }
                    last => {
                        sorted &= last.is_none_or(|l| l.0 < scroller);
                        self.measured.push((scroller, x0, x1, y1));
                    }
                },
                Placed::Live(scroller) => self.live.push((scroller, abs_id)),
            }
        }
        if !sorted {
            self.measured.sort_unstable_by_key(|m| m.0);
            self.measured.dedup_by(|later, kept| {
                let same = later.0 == kept.0;
                if same {
                    kept.1 = kept.1.min(later.1);
                    kept.2 = kept.2.max(later.2);
                    kept.3 = kept.3.max(later.3);
                }
                same
            });
        }
        self.live.sort_unstable();
    }
}

/// Which scroll container the listed absolute box `abs_id` (of
/// `out_of_flow_kind` `kind`) is part of the scrollable overflow of, and
/// where it is there: one walk up its box-tree ancestors that finds the
/// container as `out_of_flow::overflow_scroller` does and sums the offsets as
/// [`border_box_in`](super::measure::border_box_in) does.
///
/// The walk first looks for the box's containing block — passing every box
/// on the way, scroll containers included (the box escapes them), with its
/// scroll taken off — and from there on stops at the first box that clips,
/// which is the container when it scrolls. The initial containing block's
/// scroll is the `<body>`'s when the body scrolls and the `<html>` box's
/// otherwise. An `absolute` or `fixed` box on the way, or a containing block
/// the walk does not pass, hands the box to the general answer, measured per
/// call ([`Placed::Live`]).
pub(super) fn place_listed(
    tree: &NodeTree,
    abs_id: RawNodeId,
    kind: Option<crate::out_of_flow::OutOfFlowKind>,
    body_scrolls: bool,
) -> Placed {
    use crate::out_of_flow::OutOfFlowKind;
    let Some(node) = tree.get(abs_id) else {
        return Placed::Nowhere;
    };
    if node.box_position() != PositionValue::Absolute || is_empty_box(node) {
        return Placed::Nowhere;
    }
    // What the walk looks for before it climbs: the containing block, the
    // page (an initial-containing-block box), or nothing (the box's layout
    // parent is its containing block, so it climbs from the first box).
    enum Seek {
        Climb,
        Block(RawNodeId),
        Page,
    }
    let mut seek = match kind {
        Some(OutOfFlowKind::Fixed) => return Placed::Nowhere,
        Some(OutOfFlowKind::IcbAbsolute) => Seek::Page,
        Some(OutOfFlowKind::AncestorAbsolute(cb)) => {
            match crate::out_of_flow::chain_end(tree, cb) {
                Some(block) => Seek::Block(block),
                None => return general(tree, abs_id, kind),
            }
        }
        None => Seek::Climb,
    };
    let (dx, dy) = crate::paint::ifc_content_box_offset(tree, node);
    let mut x = (node.layout.x + dx) as f64;
    let mut y = (node.layout.y + dy) as f64;
    let mut transformed = !node.computed_style.transform.is_none;
    // The first box above it that is not `display: contents` — its layout
    // parent, whose own walk of its children may measure it already.
    let mut layout_parent = None;
    let mut current = crate::RinchDocument::box_tree_parent(&tree.nodes, abs_id);
    let scroller = loop {
        let Some(id) = current else {
            return general(tree, abs_id, kind);
        };
        if id == tree.html_id {
            match seek {
                Seek::Block(_) => return general(tree, abs_id, kind),
                Seek::Climb | Seek::Page => break id,
            }
        }
        if id == tree.root_id {
            return general(tree, abs_id, kind);
        }
        let Some(n) = tree.get_live(id) else {
            return Placed::Nowhere;
        };
        let climbing = match seek {
            Seek::Climb => true,
            Seek::Block(block) if block == id => {
                seek = Seek::Climb;
                true
            }
            Seek::Block(_) => false,
            Seek::Page => {
                if id == tree.body_id && body_scrolls {
                    break id;
                }
                false
            }
        };
        if climbing {
            let (clip_x, clip_y) = n.clip_axes();
            if clip_x || clip_y {
                if crate::out_of_flow::is_scroll_container(n) {
                    break id;
                }
                return Placed::Nowhere;
            }
            if matches!(
                n.box_position(),
                PositionValue::Absolute | PositionValue::Fixed
            ) {
                return general(tree, abs_id, kind);
            }
        }
        if layout_parent.is_none()
            && n.computed_style.display != crate::computed_style::DisplayValue::Contents
        {
            layout_parent = Some(id);
        }
        transformed |= !n.computed_style.transform.is_none;
        let (dx, dy) = crate::paint::ifc_content_box_offset(tree, n);
        x += (n.layout.x + dx) as f64 - n.scroll_offset.0;
        y += (n.layout.y + dy) as f64 - n.scroll_offset.1;
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    };
    let layout_parent = layout_parent.or(Some(scroller));
    if layout_parent == Some(scroller)
        && crate::out_of_flow::contributes_to_scrollable_overflow(tree, scroller, node)
    {
        return Placed::Nowhere;
    }
    if transformed {
        return Placed::Live(scroller);
    }
    Placed::Measured(
        scroller,
        x,
        x + node.layout.width as f64,
        y + node.layout.height as f64,
    )
}

/// [`place_listed`]'s answer for a box whose way up its walk does not follow:
/// `out_of_flow::overflow_scroller`'s container, measured per call.
pub(super) fn general(
    tree: &NodeTree,
    abs_id: RawNodeId,
    kind: Option<crate::out_of_flow::OutOfFlowKind>,
) -> Placed {
    let Some(scroller) = crate::out_of_flow::overflow_scroller(tree, abs_id, kind) else {
        return Placed::Nowhere;
    };
    if crate::out_of_flow::layout_parent(tree, abs_id) == Some(scroller)
        && crate::out_of_flow::contributes_to_scrollable_overflow(
            tree,
            scroller,
            &tree.nodes[abs_id],
        )
    {
        return Placed::Nowhere;
    }
    Placed::Live(scroller)
}
