//! The rasters paint keeps between frames for a `filter` and a
//! `backdrop-filter` (#2719): what they were drawn from, when they go stale,
//! and how much memory they may hold.
//!
//! **Kept.** Each box holds at most one filter raster and one backdrop
//! ([`RasterCache`] in `Node::raster_cache`), with the [`CacheKey`] it was
//! drawn under. A paint whose key matches draws the kept image (on the GPU,
//! the image Vello already holds: the pixels are a shared blob).
//!
//! **Stale.** [`invalidate`] clears what a change may have made stale. It runs
//! where the change lists are drained, at the start of
//! `NodeTree::consume_paint_dirty` — which every shell path calls before it
//! paints (`build_scene`, the desktop frame's damage) — and again at the
//! start of `paint_document`, for a caller that paints before it drains (the
//! second run finds the lists empty in the shell's order). A filter raster is
//! cleared when a paint-dirty node is the box or inside it, all of them when
//! fonts are registered or the system fonts refreshed ([`clear_all`]), a
//! backdrop when a change behind it reaches it (`paint::backdrop::invalidate`);
//! both are cleared on a whole-document restyle,
//! and when the box has no box any more (`display: none`): what it is shown
//! over again later is not known.
//!
//! **Bounded.** All kept images together may hold [`BUDGET_BYTES`]; past it the
//! least recently drawn are dropped first. And at the end of each
//! `paint_document` with a window, the rasters of boxes now more than one
//! window away from it are dropped ([`sweep`]).

use std::cell::Cell;

use crate::node::{Node, NodeTree, RawNodeId};

use super::painter::RasterImage;

/// What all kept rasters together may hold: 48 MiB. A 300x100 glass panel at
/// 2x is 0.5 MiB; a HUD of a few filtered elements a few MiB.
pub const BUDGET_BYTES: usize = 48 << 20;

/// Keep the whole visible raster when it is at most this many times the
/// part a partial repaint shows; past it, rasterise only that part.
pub(super) const KEEP_RATIO: f64 = 4.0;

/// Which raster of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Filter,
    /// A backdrop (`paint::backdrop`).
    Backdrop,
}

/// What a kept raster was drawn from, besides what [`invalidate`] watches.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CacheKey {
    /// The raster's device rect, on whole pixels (for a backdrop: the part of
    /// its box that shows).
    pub(super) canvas: [f64; 4],
    /// The box's painted origin on the device (for a backdrop, its root's).
    pub(super) origin: (f64, f64),
    /// The box's transform on the device.
    pub(super) transform: [f64; 6],
    pub(super) scale: f64,
    pub(super) size: (f32, f32),
    pub(super) list: Vec<crate::computed_style::FilterOp>,
    pub(super) shadows_only: bool,
    /// For a backdrop, the backdrop root painted again; `0` for a filter.
    pub(super) tag: RawNodeId,
}

/// One kept raster.
#[derive(Debug)]
pub(crate) struct CacheEntry {
    key: CacheKey,
    image: RasterImage,
    x: i32,
    y: i32,
    /// The paint that last drew it ([`NodeTree::raster_serial`]).
    used: Cell<u64>,
}

impl CacheEntry {
    fn bytes(&self) -> usize {
        self.image.width as usize * self.image.height as usize * 4
    }
}

/// The rasters kept for one box (`Node::raster_cache`).
#[derive(Debug, Default)]
pub(crate) struct RasterCache {
    filter: Option<CacheEntry>,
    backdrop: Option<CacheEntry>,
}

impl RasterCache {
    /// Whether a backdrop is kept.
    pub(super) fn has_backdrop(&self) -> bool {
        self.backdrop.is_some()
    }

    fn slot(&mut self, kind: Kind) -> &mut Option<CacheEntry> {
        match kind {
            Kind::Filter => &mut self.filter,
            Kind::Backdrop => &mut self.backdrop,
        }
    }
}

/// The kept raster of `kind` for `node` when it was drawn under `key`: its
/// image and where its top-left texel goes.
pub(super) fn lookup(
    tree: &NodeTree,
    node: &Node,
    kind: Kind,
    key: &CacheKey,
) -> Option<(RasterImage, i32, i32)> {
    let mut c = node.raster_cache.borrow_mut();
    let e = c.as_mut()?.slot(kind).as_ref()?;
    if e.key != *key {
        return None;
    }
    e.used.set(tree.raster_serial.get());
    Some((e.image.clone(), e.x, e.y))
}

/// Keep `image` as `node_id`'s raster of `kind`, then drop the least recently
/// drawn rasters until all of them fit [`BUDGET_BYTES`].
#[allow(clippy::too_many_arguments)]
pub(super) fn store(
    tree: &NodeTree,
    node_id: RawNodeId,
    node: &Node,
    kind: Kind,
    key: CacheKey,
    image: RasterImage,
    x: i32,
    y: i32,
) {
    let entry = CacheEntry {
        key,
        image,
        x,
        y,
        used: Cell::new(tree.raster_serial.get()),
    };
    let added = entry.bytes();
    if added > BUDGET_BYTES {
        return;
    }
    {
        let mut slot = node.raster_cache.borrow_mut();
        if slot.is_none() {
            tree.raster_cached.borrow_mut().push(node_id);
        }
        let old = slot
            .get_or_insert_with(Default::default)
            .slot(kind)
            .replace(entry);
        let freed = old.map_or(0, |o| o.bytes());
        tree.raster_cache_bytes
            .set(tree.raster_cache_bytes.get() + added - freed);
    }
    if tree.raster_cache_bytes.get() > BUDGET_BYTES {
        evict(tree, (node_id, kind));
    }
}

/// Drop the least recently drawn rasters, `keep` aside, until all fit.
fn evict(tree: &NodeTree, keep: (RawNodeId, Kind)) {
    let mut all: Vec<(u64, RawNodeId, Kind)> = Vec::new();
    for &id in tree.raster_cached.borrow().iter() {
        let Some(n) = tree.get(id) else { continue };
        let c = n.raster_cache.borrow();
        let Some(c) = c.as_ref() else { continue };
        for (kind, e) in [(Kind::Filter, &c.filter), (Kind::Backdrop, &c.backdrop)] {
            if let Some(e) = e
                && (id, kind) != keep
            {
                all.push((e.used.get(), id, kind));
            }
        }
    }
    all.sort_unstable_by_key(|&(used, ..)| used);
    for (_, id, kind) in all {
        if tree.raster_cache_bytes.get() <= BUDGET_BYTES {
            break;
        }
        clear(tree, id, kind);
    }
    prune(tree);
}

/// Forget `id`'s raster of `kind`, if it holds one.
pub(super) fn clear(tree: &NodeTree, id: RawNodeId, kind: Kind) {
    let Some(n) = tree.get(id) else { return };
    if let Some(e) = n
        .raster_cache
        .borrow_mut()
        .as_mut()
        .and_then(|c| c.slot(kind).take())
    {
        tree.raster_cache_bytes
            .set(tree.raster_cache_bytes.get().saturating_sub(e.bytes()));
    }
}

fn clear_both(tree: &NodeTree, id: RawNodeId) {
    clear(tree, id, Kind::Filter);
    clear(tree, id, Kind::Backdrop);
}

/// Drop the holders that hold nothing, and the ones whose node is gone (a
/// freed node's bytes were counted: forget them with it).
fn prune(tree: &NodeTree) {
    tree.raster_cached.borrow_mut().retain(|&id| {
        let Some(n) = tree.get(id) else {
            return false;
        };
        let mut slot = n.raster_cache.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|c| c.filter.is_none() && c.backdrop.is_none())
        {
            *slot = None;
        }
        slot.is_some()
    });
    // Recount: a node freed with its raster took its bytes with it.
    let mut total = 0;
    for &id in tree.raster_cached.borrow().iter() {
        if let Some(c) = tree.get(id).and_then(|n| {
            n.raster_cache.borrow().as_ref().map(|c| {
                [&c.filter, &c.backdrop]
                    .iter()
                    .flat_map(|e| e.iter())
                    .map(CacheEntry::bytes)
                    .sum::<usize>()
            })
        }) {
            total += c;
        }
    }
    tree.raster_cache_bytes.set(total);
}

/// Clear what the changes since the last drain may have made stale (the
/// module doc). Free while nothing is kept.
pub(crate) fn invalidate(tree: &NodeTree) {
    if tree.raster_cached.borrow().is_empty() {
        return;
    }
    let held: Vec<RawNodeId> = tree.raster_cached.borrow().clone();
    if tree.whole_document_damaged {
        held.iter().for_each(|&id| clear_both(tree, id));
    } else {
        // A filter raster: the box itself or anything inside it changed.
        for &d in &tree.paint_dirty_nodes {
            let mut cur = Some(d);
            while let Some(id) = cur {
                let Some(n) = tree.get(id) else { break };
                if n.raster_cache.borrow().is_some() {
                    clear(tree, id, Kind::Filter);
                }
                cur = n.parent;
            }
        }
        // A backdrop: what changed behind it (`paint::backdrop`).
        super::backdrop::invalidate(tree);
        // A box that no longer has a box keeps nothing.
        for &id in &held {
            if tree
                .get(id)
                .is_none_or(|n| n.layout.width <= 0.0 || n.layout.height <= 0.0)
            {
                clear_both(tree, id);
            }
        }
    }
    prune(tree);
}

/// Drop every kept raster: the fonts changed (`note_fonts_registered`, which
/// a late registration and a system-font refresh both call), and text inside
/// a filter or behind a backdrop may now shape with another face, with no
/// node pushed paint-dirty for it on every path (the embed overlay's).
pub(crate) fn clear_all(tree: &NodeTree) {
    let held: Vec<RawNodeId> = tree.raster_cached.borrow().clone();
    for id in held {
        clear_both(tree, id);
    }
    prune(tree);
}

/// Drop the rasters of boxes more than one window away from the window
/// (`target`, device px), at the end of a paint.
pub(super) fn sweep(tree: &NodeTree, scale: f64, target: peniko::kurbo::Rect) {
    if tree.raster_cached.borrow().is_empty() {
        return;
    }
    let near = target.inflate(target.width(), target.height());
    let held: Vec<RawNodeId> = tree.raster_cached.borrow().clone();
    for id in held {
        if tree.get(id).is_none() {
            continue;
        }
        let b = super::painted_border_box(tree, id, scale);
        if !(b.x0 < near.x1 && b.x1 > near.x0 && b.y0 < near.y1 && b.y1 > near.y0) {
            clear_both(tree, id);
        }
    }
    prune(tree);
}
