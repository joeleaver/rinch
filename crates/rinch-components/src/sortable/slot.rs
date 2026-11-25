//! The open gap's slot ([`SortableList::slot`], #3065): where it is
//! placed, and drawing it as the list's own pseudo-element.

use super::*;

/// The open gap's slot ([`SortableList::slot`]): a box the size of what is
/// carried, where the drop lands, painted only as the app styles it
/// (`--rinch-sortable-slot-*`). Always a pseudo-element of the list holding
/// the gap, never of an item, so no item's `overflow` clips it.
#[derive(Clone)]
pub(super) struct Slot {
    pub(super) list: NodeHandle,
    pub(super) place: SlotPlace,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SlotPlace {
    /// Between items, or in the room a collapsed item left at its own
    /// list's end: the list's absolutely positioned `::before` over this box,
    /// in the list's padding-box frame, so it moves nothing.
    At { x: f32, y: f32, w: f32, h: f32 },
    /// At the end of a list the carried items are not in (an empty one
    /// included): the list's in-flow `::after`, `size` along its axis, so
    /// the list grows to hold it, and a wrapping list or a grid puts it
    /// where an item added at the end would go.
    End { size: f32 },
}

impl Slot {
    pub(super) fn same(&self, other: &Slot) -> bool {
        self.list.node_id() == other.list.node_id() && self.place == other.place
    }

    pub(super) fn draw(&self) {
        match self.place {
            SlotPlace::At { x, y, w, h } => {
                self.list.add_class(SLOT_CLASS);
                self.list.set_styles(&[
                    (SLOT_X, &px(x)),
                    (SLOT_Y, &px(y)),
                    (SLOT_WIDTH, &px(w)),
                    (SLOT_HEIGHT, &px(h)),
                ]);
            }
            SlotPlace::End { size } => {
                self.list.add_class(SLOT_INTO_CLASS);
                self.list.set_styles(&[(SLOT_SIZE, &px(size))]);
            }
        }
    }

    pub(super) fn erase(&self) {
        self.list.remove_class(SLOT_CLASS);
        self.list.remove_class(SLOT_INTO_CLASS);
        clear_props(
            &self.list,
            &[SLOT_X, SLOT_Y, SLOT_WIDTH, SLOT_HEIGHT, SLOT_SIZE],
        );
    }
}

/// The slot of a gap opened at `slot` of `rest` (the list's items less the
/// carried ones), `size` long along the axis, `spacing` between items;
/// `shift_of` is an item's shift before the gap's own (its collapse).
///
/// Placed from the items' boxes as laid out (untransformed, in the list's
/// frame, [`NodeHandle::layout_box_in`]), so a right-to-left row, a reversed
/// one, a wrapping one and a grid each put it where the drop lands:
///
/// - Between items, where the item after the gap is (it slides on by the
///   gap), on its line: at its start edge in the list's flow. When that is
///   before the start of every item's line (a gap at a line break in the
///   source's own collapsed list, where the items before the gap slid back
///   onto the line before), after the item before the gap instead, on its
///   line.
/// - At the end of the source's own collapsed list, where the carried items
///   still take their space: in the room that leaves, which ends where the
///   list's last item is laid out.
/// - At the end of any other list (an empty one included): the list's
///   in-flow `::after`, so the list grows to hold it.
///
/// A list holding only what is carried draws none, and none is drawn where
/// the host has no layout to place it by.
pub(super) fn slot_at(
    cfg: &ListConfig,
    rest: &[(NodeHandle, String)],
    slot: usize,
    size: f32,
    spacing: f32,
    shift_of: impl Fn(&NodeHandle) -> (f32, f32),
) -> Option<Slot> {
    let size = size.max(0.0);
    let held = items_of(&cfg.node);
    let at = |place| {
        Some(Slot {
            list: cfg.node.clone(),
            place,
        })
    };
    if rest.is_empty() {
        return if held.is_empty() {
            at(SlotPlace::End { size })
        } else {
            None
        };
    }
    let own_room = held.len() > rest.len();
    if slot >= rest.len() && !own_room {
        return at(SlotPlace::End { size });
    }
    let axis = cfg.axis;
    let boxes = held
        .iter()
        .map(|(n, _)| Some(AxisBox::of(&n.layout_box_in(&cfg.node)?, axis)))
        .collect::<Option<Vec<_>>>()?;
    let forward = flows_forward(&boxes);
    // Where `node` would be without the gap: its box moved by its collapse
    // shift, a translate already drawn the way the list runs.
    let shifted = |node: &NodeHandle| {
        let i = held
            .iter()
            .position(|(n, _)| n.node_id() == node.node_id())?;
        let (dx, dy) = shift_of(node);
        let mut b = boxes[i];
        let along = match axis {
            SortAxis::Vertical => dy,
            SortAxis::Horizontal => dx,
        };
        b.at += along;
        Some(b)
    };
    let sized = |b: AxisBox, at: f32| AxisBox { at, len: size, ..b };
    let Some((after, _)) = rest.get(slot) else {
        let last = *boxes.last()?;
        let start = if forward {
            last.at + last.len - size
        } else {
            last.at
        };
        return at(sized(last, start).place(axis));
    };
    let a = shifted(after)?;
    let before_after = sized(a, if forward { a.at } else { a.at + a.len - size });
    let on_a_line = if forward {
        let line_start = boxes.iter().map(|b| b.at).fold(f32::INFINITY, f32::min);
        before_after.at >= line_start - 0.5
    } else {
        let line_end = boxes
            .iter()
            .map(|b| b.at + b.len)
            .fold(f32::NEG_INFINITY, f32::max);
        before_after.at + size <= line_end + 0.5
    };
    let chosen = match slot.checked_sub(1).map(|i| &rest[i].0) {
        Some(prev) if !on_a_line => {
            let p = shifted(prev)?;
            let start = if forward {
                p.at + p.len + spacing
            } else {
                p.at - spacing - size
            };
            sized(p, start)
        }
        _ => before_after,
    };
    at(chosen.place(axis))
}
