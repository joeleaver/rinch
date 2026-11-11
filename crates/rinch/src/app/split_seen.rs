//! Where a window's GPU surfaces were last found by a sliced paint (#2511).
//!
//! A partial frame repaints the whole region of each kept surface (split in
//! the previous frame) that its damage meets, so the surface is split again
//! from a region painted whole. A surface the frame splits that was not kept
//! had no region to add: its slice came out short, and the frame was painted
//! a second time with the region added. These records give such a surface a
//! region before the paint: where a sliced paint last split it, or last left
//! it in the scene for overlapping another split (D7a). The frame then
//! paints once what it used to paint twice. Added damage only widens what a
//! partial frame repaints, so the picture does not change.

use peniko::kurbo::Rect;
use rinch_dom::paint::slicing::{BlockedSplit, SurfaceSlice};

/// A surface where a sliced paint found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Seen {
    pub(super) id: usize,
    /// R in device pixels: where it was split, or would have been.
    pub(super) region: Rect,
    /// The element's ink: damage that meets it paints the surface.
    pub(super) ink: Rect,
    /// The split surface it overlapped, when it was left in the scene.
    blocked_by: Option<usize>,
}

/// The surfaces a window's sliced paints found, split or blocked.
#[derive(Default)]
pub(super) struct SeenSurfaces(Vec<Seen>);

/// `(x, y, width, height)` in device pixels as a rect.
fn rect_of((x, y, w, h): (u32, u32, u32, u32)) -> Rect {
    Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64)
}

/// Whether disjoint `rects` hold all of `r`.
pub(super) fn covers(rects: &[Rect], r: &Rect) -> bool {
    let held: f64 = rects.iter().map(|d| d.intersect(*r).area()).sum();
    held >= r.area() - 0.5
}

impl SeenSurfaces {
    /// After a sliced paint of `painted` (`None`: the whole window): each
    /// surface it split or blocked is where it found it. One it did not find
    /// although it painted all of its region is not split there any more
    /// (gone, or refused for a group effect or a transform) and is dropped,
    /// unless it is refused for the memory cap (`capped`): a frame painted
    /// again for the cap finds it nowhere, and its record is where it goes
    /// when the cap is retried.
    pub(super) fn note_pass(
        &mut self,
        painted: Option<&[Rect]>,
        slices: &[SurfaceSlice],
        blocked: &[BlockedSplit],
        capped: &[usize],
    ) {
        let found = |id: usize| {
            slices.iter().any(|s| s.surface_id == id) || blocked.iter().any(|b| b.surface_id == id)
        };
        self.0.retain(|s| {
            found(s.id)
                || capped.contains(&s.id)
                || painted.is_some_and(|rects| !covers(rects, &s.region))
        });
        let now = slices
            .iter()
            .map(|s| Seen {
                id: s.surface_id,
                region: rect_of(s.region),
                ink: s.ink,
                blocked_by: None,
            })
            .chain(blocked.iter().map(|b| Seen {
                id: b.surface_id,
                region: rect_of(b.region),
                ink: b.ink,
                blocked_by: Some(b.by),
            }));
        for seen in now {
            match self.0.iter_mut().find(|s| s.id == seen.id) {
                Some(s) => *s = seen,
                None => self.0.push(seen),
            }
        }
    }

    /// The surfaces a sliced frame may split that are not `kept`: not
    /// refused for the memory cap (`capped`), and not still blocked by the
    /// kept surface they overlapped, unless the frame repaints that one
    /// (`hit`), which may move it away.
    pub(super) fn may_split(
        &self,
        kept: &[(usize, Rect)],
        capped: &[usize],
        hit: &[usize],
    ) -> Vec<Seen> {
        let is_kept = |id: usize| kept.iter().any(|(k, _)| *k == id);
        self.0
            .iter()
            .filter(|s| !is_kept(s.id) && !capped.contains(&s.id) && !s.region.is_zero_area())
            .filter(|s| {
                s.blocked_by
                    .is_none_or(|by| !is_kept(by) || hit.contains(&by))
            })
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(id: usize, region: (u32, u32, u32, u32), by: usize) -> BlockedSplit {
        BlockedSplit {
            surface_id: id,
            region,
            ink: rect_of(region),
            by,
        }
    }

    fn ids(seen: &[Seen]) -> Vec<usize> {
        seen.iter().map(|s| s.id).collect()
    }

    #[test]
    fn covers_counts_disjoint_rects() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let halves = [
            Rect::new(0.0, 0.0, 5.0, 10.0),
            Rect::new(5.0, 0.0, 10.0, 10.0),
        ];
        assert!(covers(&halves, &r));
        assert!(!covers(&halves[..1], &r));
    }

    #[test]
    fn a_blocked_surface_is_offered_once_its_blocker_is_repainted_or_gone() {
        let mut seen = SeenSurfaces::default();
        seen.note_pass(None, &[], &[blocked(2, (10, 10, 20, 20), 1)], &[]);
        let kept = [(1, Rect::new(0.0, 0.0, 15.0, 15.0))];
        assert!(seen.may_split(&kept, &[], &[]).is_empty(), "still blocked");
        assert_eq!(ids(&seen.may_split(&kept, &[], &[1])), [2], "blocker hit");
        assert_eq!(ids(&seen.may_split(&[], &[], &[])), [2], "blocker gone");
        assert!(seen.may_split(&[], &[2], &[]).is_empty(), "capped");
    }

    #[test]
    fn a_surface_not_found_where_its_region_was_painted_is_dropped() {
        let mut seen = SeenSurfaces::default();
        seen.note_pass(None, &[], &[blocked(2, (10, 10, 20, 20), 1)], &[]);
        // A partial paint away from it keeps it.
        seen.note_pass(
            Some(&[Rect::new(100.0, 100.0, 120.0, 120.0)]),
            &[],
            &[],
            &[],
        );
        assert_eq!(ids(&seen.may_split(&[], &[], &[])), [2]);
        // A partial paint over all of it that did not find it drops it.
        seen.note_pass(Some(&[Rect::new(0.0, 0.0, 50.0, 50.0)]), &[], &[], &[]);
        assert!(seen.may_split(&[], &[], &[]).is_empty());
        // So does a full paint.
        seen.note_pass(None, &[], &[blocked(3, (10, 10, 20, 20), 1)], &[]);
        seen.note_pass(None, &[], &[], &[]);
        assert!(seen.may_split(&[], &[], &[]).is_empty());
    }

    #[test]
    fn a_surface_refused_for_the_cap_keeps_its_record() {
        let mut seen = SeenSurfaces::default();
        seen.note_pass(None, &[], &[blocked(2, (10, 10, 20, 20), 1)], &[]);
        // The cap's second paint, the whole window: refused, found nowhere.
        seen.note_pass(None, &[], &[], &[2]);
        assert!(seen.may_split(&[], &[2], &[]).is_empty(), "while capped");
        assert_eq!(ids(&seen.may_split(&[], &[], &[])), [2], "the cap retried");
    }

    #[test]
    fn a_later_find_replaces_the_record() {
        let mut seen = SeenSurfaces::default();
        seen.note_pass(None, &[], &[blocked(2, (10, 10, 20, 20), 1)], &[]);
        seen.note_pass(None, &[], &[blocked(2, (40, 40, 20, 20), 5)], &[]);
        let offered = seen.may_split(&[], &[], &[]);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].region, Rect::new(40.0, 40.0, 60.0, 60.0));
    }
}
