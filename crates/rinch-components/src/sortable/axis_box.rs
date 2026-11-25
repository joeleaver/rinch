//! Items' boxes along a list's axis: the spacing beside an item, and which
//! way a list runs as laid out (#3065).

use super::*;

/// A box in a list's padding-box frame, along its axis and across it.
#[derive(Clone, Copy)]
pub(super) struct AxisBox {
    pub(super) at: f32,
    pub(super) len: f32,
    pub(super) cross: f32,
    pub(super) cross_len: f32,
}

impl AxisBox {
    pub(super) fn of(b: &ElementBounds, axis: SortAxis) -> Self {
        match axis {
            SortAxis::Vertical => AxisBox {
                at: b.y,
                len: b.height,
                cross: b.x,
                cross_len: b.width,
            },
            SortAxis::Horizontal => AxisBox {
                at: b.x,
                len: b.width,
                cross: b.y,
                cross_len: b.height,
            },
        }
    }

    pub(super) fn place(self, axis: SortAxis) -> SlotPlace {
        match axis {
            SortAxis::Vertical => SlotPlace::At {
                x: self.cross,
                y: self.at,
                w: self.cross_len,
                h: self.len,
            },
            SortAxis::Horizontal => SlotPlace::At {
                x: self.at,
                y: self.cross,
                w: self.len,
                h: self.cross_len,
            },
        }
    }

    pub(super) fn same_line(&self, other: &AxisBox) -> bool {
        self.cross < other.cross + other.cross_len && other.cross < self.cross + self.cross_len
    }

    /// The room between this box's and `other`'s facing edges along the
    /// axis, whichever comes first: a right-to-left or reversed row has its
    /// next item at smaller coordinates, and measured one way only its
    /// spacing came out negative, so zero (#3065).
    pub(super) fn between(&self, other: &AxisBox) -> f32 {
        (other.at - (self.at + self.len)).max(self.at - (other.at + other.len))
    }
}

/// The list's spacing along its axis beside item `i` of `boxes` (`None` for
/// an item with no box): the room between its facing edges and a
/// neighbour's **on its line**, the next item's, else the previous one's. In
/// a wrapping row or a grid the next item in order can start the next line,
/// and measured against it the spacing took in the line's length (#3065).
/// An item alone on its line takes the first two neighbours that share one
/// anywhere in the list. Zero when none do, or `i` has no box.
pub(super) fn spacing_beside(boxes: &[Option<AxisBox>], i: usize) -> f32 {
    let Some(here) = boxes.get(i).copied().flatten() else {
        return 0.0;
    };
    let beside = |j: Option<usize>| {
        let other = boxes.get(j?).copied().flatten()?;
        here.same_line(&other).then(|| here.between(&other))
    };
    beside(Some(i + 1))
        .or_else(|| beside(i.checked_sub(1)))
        .or_else(|| {
            boxes.windows(2).find_map(|w| match (w[0], w[1]) {
                (Some(a), Some(b)) if a.same_line(&b) => Some(a.between(&b)),
                _ => None,
            })
        })
        .unwrap_or(0.0)
        .max(0.0)
}

/// Whether a list's items run toward larger coordinates along its axis, as
/// laid out: the first two neighbours on one line say (a right-to-left row
/// or a `*-reverse` flex direction runs the other way). Forward when no two
/// share a line.
pub(super) fn flows_forward(boxes: &[AxisBox]) -> bool {
    boxes
        .windows(2)
        .find(|w| w[0].same_line(&w[1]) && (w[1].at - w[0].at).abs() > 0.5)
        .is_none_or(|w| w[1].at > w[0].at)
}

/// [`flows_forward`] for `cfg`'s items as laid out in it
/// ([`NodeHandle::layout_box_in`]). Forward where the host has no layout to
/// tell by, or the list is not positioned.
pub(super) fn runs_forward(cfg: &ListConfig) -> bool {
    items_of(&cfg.node)
        .iter()
        .map(|(n, _)| Some(AxisBox::of(&n.layout_box_in(&cfg.node)?, cfg.axis)))
        .collect::<Option<Vec<_>>>()
        .is_none_or(|boxes| flows_forward(&boxes))
}
