//! The pruned, cached hit test answers exactly what the unpruned one did.
//!
//! `hit_test` skips subtrees whose flow extent misses the point and reuses
//! stacking sequences across probes (`rinch_dom::hit_cache`). Both are pure
//! optimisations, so the oracle here is the walk they replaced, kept verbatim
//! as [`oracle::reference_hit_test_node`] — no extent, no cache, a fresh
//! `stacking_paint_order` per stacking root per probe. Every fixture compares
//! the two over a grid of points that reaches past the window on every side.
//!
//! The documents are generated from a fixed seed and mix everything the walk
//! special-cases: `relative`/`absolute`/`fixed`/`sticky` boxes with and
//! without `z-index`, negative offsets that overflow a parent, transforms,
//! `opacity`, `overflow: hidden`/`auto`/`clip` scrolled to a non-zero offset,
//! `pointer-events: none`, `visibility: hidden`, `display: contents`/`none`,
//! inline-blocks in text flow, and flex containers. The cache half is pinned
//! by probing, mutating (a style write, a scroll, a structural change, each
//! followed by a layout), and probing again: a stale memo would answer from
//! the old geometry.
//!
//! - `oracle` — the unpruned walk, verbatim from before the prune, and the
//!   test that keeps it per-axis like the real one.
//! - `random` — the seeded document generator and the fuzzed agreement test.
//! - `fixtures` — hand-built documents: scrolled lists, stacking-context rows,
//!   nested scrollers, a viewport resize, a row that starts hoisting.
//! - `ticks` — transition and animation ticks that write the `HitStyleKey`
//!   fields without a layout.

mod fixtures;
mod oracle;
mod random;
mod ticks;

use super::hit_test;
use rinch_dom::RinchDocument;

/// The oracle: `hit_test` as it was before the prune and the cache.
fn reference_hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<u64> {
    oracle::reference_hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y, x, y, None)
}

/// Compare the two over a grid; returns how many points were probed.
#[track_caller]
fn assert_agree(doc: &RinchDocument, label: &str) -> usize {
    let mut probed = 0;
    let mut y = -23.0f32;
    while y < 640.0 {
        let mut x = -23.0f32;
        while x < 840.0 {
            let want = reference_hit_test(&doc.tree, x, y);
            let got = hit_test(&doc.tree, x, y);
            assert_eq!(
                got, want,
                "{label}: the pruned hit test disagrees at ({x}, {y})"
            );
            probed += 1;
            x += 7.0;
        }
        y += 7.0;
    }
    probed
}
