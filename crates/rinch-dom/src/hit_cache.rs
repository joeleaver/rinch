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
//! nothing about what an extent means.
//!
//! **What bumps the generation** is everything that can move a box, change the
//! tree's shape, or change a style a hit test reads: every mutating
//! `DomDocument` method on `RinchDocument` that changes something (an
//! identical attribute or text write returns first), `resolve_styles`,
//! `resolve_layout`, a transition or animation tick that changes a node's
//! [`HitStyleKey`], every
//! `NodeTree::push_dirty` (which every scroll write in the shell makes), and
//! the shell's scroll writes explicitly. A write that reaches into
//! `tree.nodes[..]` and changes `layout`, `scroll_offset` or `computed_style`
//! **without** going through one of those must call
//! [`HitCache::invalidate`] itself, or the next hit test answers from the old
//! geometry.

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

#[derive(Default)]
struct State {
    /// The generation `extents` and `orders` were filled at.
    filled_at: u64,
    extents: Vec<Option<Extent>>,
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

    fn state(&self) -> std::cell::RefMut<'_, State> {
        let mut s = self.state.borrow_mut();
        // `generation` starts at 0 and `filled_at` at 0 too, but a fresh state
        // has nothing in it, so agreeing there is harmless.
        if s.filled_at != self.generation.get() {
            s.filled_at = self.generation.get();
            s.extents.clear();
            s.orders.clear();
        }
        s
    }

    /// The extent stored for `id` in the current generation.
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
