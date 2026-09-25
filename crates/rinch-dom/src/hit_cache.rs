//! The hit tester's memo: per-node subtree extents and per-root stacking
//! sequences, reused across pointer events until the geometry changes.
//!
//! Hit testing (`crates/rinch/src/app/hit_testing.rs`) runs on every pointer
//! move. Two things made it O(document) per probe: it built every stacking
//! root's [`PaintOrder`] afresh, and it had no way to skip a subtree whose
//! content lay nowhere near the pointer — a box that does not clip may have
//! descendants anywhere, so its children were always probed. This cache holds
//! the answers to both, keyed by nothing but a **generation**:
//! [`HitCache::invalidate`] bumps it, and the next lookup finds everything
//! stale and starts over. The hit tester fills it lazily; this module knows
//! nothing about what an extent means beyond the one fact a scroll needs (an
//! extent is relative to its own node — [`HitCache::invalidate_scroll`]).
//!
//! **What bumps the generation** is everything that can move a box, change the
//! tree's shape, or change a style a hit test reads: every mutating
//! `DomDocument` method on `RinchDocument` that changes something (an
//! identical attribute or text write returns first), `resolve_styles`,
//! `resolve_layout` — unless it took the paint-only path, which moves no box
//! (a restyle on that path invalidates in `resolve_styles`) — a transition or
//! animation tick that changes a node's [`HitStyleKey`], and every
//! `NodeTree::push_dirty`.
//!
//! **A scroll is the one partial invalidation** (#911).
//! `NodeTree::mark_scrolled`, which every scroll-offset write in the shell and
//! `set_scroll_top`/`set_scroll_left` go through, calls
//! [`HitCache::invalidate_scroll`]: the generation moves, but every extent the
//! scroll cannot reach is kept, so the next notch of a long list does not
//! rebuild every row's.
//!
//! A write that reaches into `tree.nodes[..]` and changes `layout`,
//! `scroll_offset` or `computed_style` **without** going through one of those
//! must call [`HitCache::invalidate`] itself, or the next hit test answers from
//! the old geometry.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::computed_style::{
    ComputedStyle, DisplayValue, LengthPercentageValue, OverflowValue, PointerEventsValue,
    PositionValue, VisibilityValue,
};
use crate::stacking::PaintOrder;

/// Every `computed_style` input hit testing reads, and nothing else — so a
/// write that leaves it equal cannot change any hit-test answer.
///
/// The transition and animation ticks write `computed_style` directly, every
/// frame, with no `resolve_layout` around them. Invalidating the cache on
/// every tick made every pointer move cold whenever anything animated (a
/// `Loader`, a `Skeleton` shimmer) and, since the shell's `AboutToWait` ticks
/// after every batch, even when nothing did. A tick compares this key before
/// and after each node it writes and invalidates only on a difference.
///
/// The readers, which is how the list was drawn up:
/// - `stacking::paints_at_stacking_root` / `Node::creates_stacking_context`:
///   `position`, `z_index`, `opacity < 1`, whether `transform` is the identity;
/// - `Node::establishes_abs_containing_block` (the clip chain's truncation
///   point): `position`, whether `transform` is the identity;
/// - `Node::clips_overflow` and `paint::clip_shape`: `overflow_x`/`_y`
///   (the radii only shape paint; hit testing tests the rect);
/// - `hit_testing::descend` / `local_point`: `position` (the fixed hoist),
///   `display` (`contents`) — and the transform's *value*, but `descend`
///   reads that live on every probe, and nothing cached holds it (below);
/// - `paint::ifc_content_box_offset`: an IFC root's `padding` and
///   `border` left/top;
/// - `hit_test_node`: `visibility`, `pointer_events`;
/// - `RinchDocument::box_tree_children`: `display`.
///
/// **Only transform identity is keyed, not the value**, so a `Drawer` or
/// `Popover` slide keeps pointer moves warm: nothing the cache holds depends
/// on the value. A transformed box creates a stacking context, so it is never
/// inside any flow extent (`flow_extent` skips it and `flow_subtree_may_contain`
/// never prunes it); a stacking sequence's offsets are accumulated through
/// untransformed boxes only (the collector never crosses a stacking context);
/// and its clip rects live in the collecting root's own space. The value
/// matters only to `descend`'s inverse, which is recomputed per probe.
///
/// Of these, only `opacity`, `transform`, the padding and the border widths
/// can be written by a tick at all (`TransitionProperty` has no `display`,
/// `position`, `overflow`, `z-index`, `visibility` or `pointer-events`
/// variant); the rest are here so that a future animatable property is
/// covered the day it is added, not the day its bug is found.
///
/// Geometry itself (`layout`) is not here: it changes only inside
/// `resolve_layout`, which invalidates on entry. A tick that animates `width`
/// changes the box at the next layout, not at the tick.
#[derive(PartialEq)]
pub struct HitStyleKey {
    display: DisplayValue,
    position: PositionValue,
    overflow_x: OverflowValue,
    overflow_y: OverflowValue,
    translucent: bool,
    visibility: VisibilityValue,
    transformed: bool,
    z_index: Option<i32>,
    pointer_events: PointerEventsValue,
    padding_left: LengthPercentageValue,
    padding_top: LengthPercentageValue,
    border_left_width: LengthPercentageValue,
    border_top_width: LengthPercentageValue,
}

impl HitStyleKey {
    /// The key of `cs`.
    pub fn of(cs: &ComputedStyle) -> Self {
        Self {
            display: cs.display,
            position: cs.position,
            overflow_x: cs.overflow_x,
            overflow_y: cs.overflow_y,
            translucent: cs.opacity < 1.0,
            visibility: cs.visibility,
            transformed: !cs.transform.is_identity,
            z_index: cs.z_index,
            pointer_events: cs.pointer_events,
            padding_left: cs.padding_left,
            padding_top: cs.padding_top,
            border_left_width: cs.border_left_width,
            border_top_width: cs.border_top_width,
        }
    }
}

/// A subtree extent: `[x0, y0, x1, y1]` relative to the node's own border-box
/// origin, in layout px. Opaque to this module.
pub type Extent = [f32; 4];

/// Key of a cached stacking sequence: the root's id and the exact bits of the
/// base offsets it was built at.
type OrderKey = (usize, u64, u64);

/// `extent_parents` slot of a node no stored extent was computed through.
const NO_PARENT: usize = usize::MAX;

#[derive(Default)]
struct State {
    /// The generation `extents` and `orders` were filled at.
    filled_at: u64,
    extents: Vec<Option<Extent>>,
    /// For each node, the node whose extent was computed *through* it — the
    /// one extent that folded this node's in, and so the one a change to this
    /// node's extent makes stale. [`NO_PARENT`] where none did. Filled by
    /// [`HitCache::note_extent_parent`]; read by
    /// [`HitCache::invalidate_scroll`].
    extent_parents: Vec<usize>,
    /// A node was folded into two different nodes' extents. The hit tester's
    /// box-tree walk never does that, but if it ever did, one parent slot
    /// could not name both, so a scroll falls back to dropping everything.
    extent_parents_ambiguous: bool,
    orders: HashMap<OrderKey, Rc<PaintOrder>>,
}

/// See the [module docs](self).
#[derive(Default)]
pub struct HitCache {
    generation: Cell<u64>,
    state: RefCell<State>,
}

impl fmt::Debug for HitCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HitCache")
            .field("generation", &self.generation.get())
            .finish_non_exhaustive()
    }
}

impl HitCache {
    /// Everything cached so far describes geometry that may no longer hold.
    #[inline]
    pub fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    /// The current generation. Two reads that agree bracket a span in which
    /// nothing that invalidates the cache ran — the shell uses that to reuse
    /// one hit-test result across the handlers of a single pointer move.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    // `#[inline]`: this sits under every `extent` lookup of a pointer move's
    // hit test, and whether rustc inlines it on its own flips with unrelated
    // changes elsewhere in the crate (+6% on `shell::pointer_move_warm`
    // when it did not, #894).
    #[inline]
    fn state(&self) -> std::cell::RefMut<'_, State> {
        let mut s = self.state.borrow_mut();
        // `generation` starts at 0 and `filled_at` at 0 too, but a fresh state
        // has nothing in it, so agreeing there is harmless.
        if s.filled_at != self.generation.get() {
            s.filled_at = self.generation.get();
            s.extents.clear();
            s.extent_parents.clear();
            s.extent_parents_ambiguous = false;
            s.orders.clear();
        }
        s
    }

    /// `id`'s scroll offset changed, and nothing else did.
    ///
    /// Bumps the generation, like [`Self::invalidate`] — so a hit-test result
    /// held across it (the shell's `move_hit`) is not reused — but keeps
    /// every extent the scroll cannot reach (#911). An extent is relative to
    /// its own node's origin, and a node's scroll offset is applied by that
    /// node when it places its children; so a scroll changes `id`'s own extent
    /// and every extent that was folded out of it — the chain
    /// [`Self::note_extent_parent`] recorded — and no other. Every extent in
    /// `id`'s subtree, which is where the rows of a long list are, survives.
    ///
    /// Stacking sequences are all dropped: an entry's offset is accumulated
    /// through every scroller between it and its root, and the cheap question
    /// is not "which roots is `id` under" but "was anything built".
    ///
    /// A caller that changed anything else as well — a layout, a style, the
    /// tree's shape — calls [`Self::invalidate`] instead.
    pub fn invalidate_scroll(&self, id: usize) {
        let mut s = self.state.borrow_mut();
        let was_current = s.filled_at == self.generation.get();
        self.invalidate();
        if !was_current || s.extent_parents_ambiguous {
            // Already stale, or not safely patchable: the next lookup clears.
            return;
        }
        s.filled_at = self.generation.get();
        s.orders.clear();
        let mut cur = id;
        // Bounded by the node count: a parent chain is a path up the box tree.
        for _ in 0..=s.extent_parents.len() {
            if let Some(e) = s.extents.get_mut(cur) {
                *e = None;
            }
            match s.extent_parents.get(cur) {
                Some(&p) if p != NO_PARENT => cur = p,
                _ => return,
            }
        }
        // A cycle cannot come out of a tree walk; if one ever did, stop
        // trusting the memo rather than loop.
        s.extents.clear();
        s.extent_parents.clear();
    }

    /// The extent stored for `id` in the current generation.
    #[inline]
    pub fn extent(&self, id: usize) -> Option<Extent> {
        self.state().extents.get(id).copied().flatten()
    }

    /// Store `id`'s extent for the current generation.
    pub fn store_extent(&self, id: usize, extent: Extent) {
        let mut s = self.state();
        if s.extents.len() <= id {
            s.extents.resize(id + 1, None);
        }
        s.extents[id] = Some(extent);
    }

    /// Record that `parent`'s extent is being computed through `child`'s:
    /// a later change to `child`'s extent makes `parent`'s stale too. Called
    /// once per child each time a parent's extent is computed, whether or not
    /// the child's own extent was already stored.
    pub fn note_extent_parent(&self, child: usize, parent: usize) {
        let mut s = self.state();
        if s.extent_parents.len() <= child {
            s.extent_parents.resize(child + 1, NO_PARENT);
        }
        let slot = &mut s.extent_parents[child];
        if *slot == NO_PARENT {
            *slot = parent;
        } else if *slot != parent {
            s.extent_parents_ambiguous = true;
        }
    }

    /// The stacking sequence cached for `root` at base `(ox, oy)`.
    pub fn order(&self, root: usize, ox: f64, oy: f64) -> Option<Rc<PaintOrder>> {
        self.state()
            .orders
            .get(&(root, ox.to_bits(), oy.to_bits()))
            .cloned()
    }

    /// Cache `order` for `root` at base `(ox, oy)`.
    pub fn store_order(&self, root: usize, ox: f64, oy: f64, order: Rc<PaintOrder>) {
        self.state()
            .orders
            .insert((root, ox.to_bits(), oy.to_bits()), order);
    }
}
