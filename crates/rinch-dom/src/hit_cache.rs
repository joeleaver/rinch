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
//! `DomDocument` method on `RinchDocument`, `resolve_styles`,
//! `resolve_layout`, the transition and animation ticks, every
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

use crate::stacking::PaintOrder;

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
