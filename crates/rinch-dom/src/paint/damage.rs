//! The software renderer's damage: a short list of rects, not one bounding box.
//!
//! One bounding rect turned two small, distant changes — a caret at the top and
//! a ticking clock at the bottom, a hover leaving one card and entering
//! another — into a repaint of everything between them, and often into a full
//! repaint once that span crossed [`FULL_REPAINT_FRACTION`]. A
//! [`DamageRegion`] keeps up to [`MAX_DAMAGE_RECTS`] rects instead, merging
//! two only when they overlap or when the union wastes little
//! ([`merge_is_cheap`]), and decides "repaint everything" on the **sum** of
//! their areas.
//!
//! Every rect is snapped out to whole pixels and clamped to the surface as it is
//! added, so a change entirely
//! off-screen contributes nothing, and the rects a region hands out are pairwise
//! disjoint: overlapping rects are always merged. That is what lets the area be
//! a plain sum and the clip be the rects' union under `NonZero`.

use peniko::kurbo::{BezPath, Rect, Shape};

use super::FULL_REPAINT_FRACTION;

/// The most rects a [`DamageRegion`] keeps. Past this the two whose union
/// wastes least are merged. Each rect is one pruning test per painted node and
/// one sub-path of the clip, so a handful is cheap; eight covers "a few
/// unrelated spots" without letting a reflow's hundreds of rows through.
pub const MAX_DAMAGE_RECTS: usize = 8;

/// Area (physical px²) of waste a merge may always add. Two small rects close
/// together are cheaper painted as one than tested and clipped as two.
const CHEAP_WASTE_PX: f64 = 64.0 * 64.0;

/// What changed on the surface since the frame on screen: a short list of
/// disjoint rects in physical pixels, or "everything".
#[derive(Clone, Debug)]
pub struct DamageRegion {
    rects: Vec<Rect>,
    whole: Rect,
    limit: f64,
    full: bool,
}

impl DamageRegion {
    /// An empty region over a `viewport_w` x `viewport_h` surface.
    pub fn new(viewport_w: f64, viewport_h: f64) -> Self {
        Self {
            rects: Vec::new(),
            whole: Rect::new(0.0, 0.0, viewport_w, viewport_h),
            limit: viewport_w * viewport_h * FULL_REPAINT_FRACTION,
            full: false,
        }
    }

    /// The whole surface.
    pub fn full(viewport_w: f64, viewport_h: f64) -> Self {
        let mut r = Self::new(viewport_w, viewport_h);
        r.set_full();
        r
    }

    /// Mark the whole surface damaged.
    pub fn set_full(&mut self) {
        self.full = true;
        self.rects.clear();
        self.rects.push(self.whole);
    }

    /// Whether the whole surface is damaged — either asked for outright or
    /// because the rects reached [`FULL_REPAINT_FRACTION`] of it.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Whether nothing on the surface is damaged.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// The damaged rects: pairwise disjoint, inside the surface, never empty
    /// rects. The whole surface alone when [`Self::is_full`].
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Total damaged area in physical px² (the rects are disjoint, so a sum).
    pub fn area(&self) -> f64 {
        self.rects.iter().map(|r| r.area()).sum()
    }

    /// The bounding box of every rect, or `None` when empty.
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(|a, b| a.union(b))
    }

    /// The union of the rects as one path, for a `NonZero` clip. Every
    /// sub-path winds the same way, so overlaps — there are none, but a
    /// caller's own added rects need not be merged — still fill once.
    pub fn clip_path(&self) -> BezPath {
        let mut path = BezPath::new();
        for r in &self.rects {
            path.extend(r.path_elements(0.1));
        }
        path
    }

    /// Add `r` (physical px). Returns `true` once the region is full, so a
    /// caller measuring many rects can stop early.
    pub fn add(&mut self, r: Rect) -> bool {
        if self.full {
            return true;
        }
        // Snapped out to whole pixels, so the rect cleared, the rect clipped to
        // and the area counted are the same pixels.
        let r =
            Rect::new(r.x0.floor(), r.y0.floor(), r.x1.ceil(), r.y1.ceil()).intersect(self.whole);
        if !(r.width() > 0.0 && r.height() > 0.0) {
            return false;
        }
        self.insert_merged(r);
        while self.rects.len() > MAX_DAMAGE_RECTS {
            self.merge_cheapest_pair();
        }
        if self.area() >= self.limit {
            self.set_full();
        }
        self.full
    }

    /// Merge the two rects whose union wastes least, then re-merge anything
    /// the result overlaps.
    fn merge_cheapest_pair(&mut self) {
        let mut best = (0, 1, f64::INFINITY);
        for i in 0..self.rects.len() {
            for j in i + 1..self.rects.len() {
                let w = waste(self.rects[i], self.rects[j]);
                if w < best.2 {
                    best = (i, j, w);
                }
            }
        }
        let (i, j, _) = best;
        let b = self.rects.swap_remove(j);
        let a = self.rects.swap_remove(i);
        self.insert_merged(a.union(b));
    }

    /// Push `r`, first folding into it every rect it should merge with —
    /// repeatedly, since a merge grows it and the grown rect may now overlap
    /// another. Keeps the rects pairwise disjoint.
    fn insert_merged(&mut self, r: Rect) {
        let mut pending = r;
        while let Some(i) = self
            .rects
            .iter()
            .position(|&existing| merge_is_cheap(existing, pending))
        {
            pending = pending.union(self.rects.swap_remove(i));
        }
        self.rects.push(pending);
    }
}

/// Area the union of `a` and `b` covers that neither does.
fn waste(a: Rect, b: Rect) -> f64 {
    let overlap = a.intersect(b);
    let overlap = if overlap.width() > 0.0 && overlap.height() > 0.0 {
        overlap.area()
    } else {
        0.0
    };
    (a.union(b).area() - a.area() - b.area() + overlap).max(0.0)
}

/// Whether `a` and `b` should be one rect: always when they overlap (which
/// keeps the region disjoint), and otherwise when the union's waste is small —
/// under [`CHEAP_WASTE_PX`] or under half the area the two already cover.
fn merge_is_cheap(a: Rect, b: Rect) -> bool {
    let overlap = a.intersect(b);
    if overlap.width() > 0.0 && overlap.height() > 0.0 {
        return true;
    }
    let w = waste(a, b);
    w <= CHEAP_WASTE_PX || w <= 0.5 * (a.area() + b.area())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_distant_small_rects_stay_two() {
        let mut d = DamageRegion::new(1000.0, 1000.0);
        d.add(Rect::new(0.0, 0.0, 10.0, 10.0));
        d.add(Rect::new(900.0, 900.0, 910.0, 910.0));
        assert_eq!(d.rects().len(), 2);
        assert_eq!(d.area(), 200.0);
        assert!(!d.is_full());
    }

    #[test]
    fn overlapping_rects_merge_and_stay_disjoint() {
        let mut d = DamageRegion::new(1000.0, 1000.0);
        d.add(Rect::new(0.0, 0.0, 100.0, 100.0));
        d.add(Rect::new(500.0, 0.0, 600.0, 100.0));
        // Bridges both.
        d.add(Rect::new(50.0, 40.0, 550.0, 60.0));
        assert_eq!(d.rects().len(), 1);
        assert_eq!(d.rects()[0], Rect::new(0.0, 0.0, 600.0, 100.0));
    }

    #[test]
    fn adjacent_rows_merge() {
        let mut d = DamageRegion::new(1000.0, 1000.0);
        d.add(Rect::new(0.0, 0.0, 800.0, 20.0));
        d.add(Rect::new(0.0, 20.0, 800.0, 40.0));
        assert_eq!(d.rects().len(), 1);
    }

    #[test]
    fn the_cap_merges_the_cheapest_pair() {
        let mut d = DamageRegion::new(10_000.0, 10_000.0);
        for i in 0..(MAX_DAMAGE_RECTS + 3) {
            let x = i as f64 * 1000.0;
            d.add(Rect::new(x, 0.0, x + 10.0, 10.0));
        }
        assert!(d.rects().len() <= MAX_DAMAGE_RECTS);
        for (i, a) in d.rects().iter().enumerate() {
            for b in &d.rects()[i + 1..] {
                let o = a.intersect(*b);
                assert!(o.width() <= 0.0 || o.height() <= 0.0, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn the_sum_not_the_span_decides_full() {
        let mut d = DamageRegion::new(1000.0, 1000.0);
        // Opposite corners: their span is the whole surface.
        assert!(!d.add(Rect::new(0.0, 0.0, 50.0, 50.0)));
        assert!(!d.add(Rect::new(950.0, 950.0, 1000.0, 1000.0)));
        // Half the surface in total is full.
        assert!(d.add(Rect::new(0.0, 200.0, 1000.0, 700.0)));
        assert!(d.is_full());
        assert_eq!(d.rects(), &[Rect::new(0.0, 0.0, 1000.0, 1000.0)]);
    }

    #[test]
    fn off_surface_rects_add_nothing() {
        let mut d = DamageRegion::new(100.0, 100.0);
        d.add(Rect::new(200.0, 200.0, 300.0, 300.0));
        assert!(d.is_empty());
    }
}
