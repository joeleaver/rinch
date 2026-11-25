//! Presentation: the published drag state and the carried items' look.

use super::*;

thread_local! {
    static STATE: Cell<Option<Signal<Option<SortDragState>>>> = const { Cell::new(None) };
    static POINTER: Cell<Option<Signal<Option<SortPointer>>>> = const { Cell::new(None) };
}

/// A signal that lives as long as the thread's reactive runtime does: made
/// again if the one held was freed (a test that tore its runtime down).
fn live_signal<T: Clone + 'static>(
    slot: &'static std::thread::LocalKey<Cell<Option<Signal<Option<T>>>>>,
) -> Signal<Option<T>> {
    slot.with(|c| match c.get().filter(|s| s.is_alive()) {
        Some(s) => s,
        None => {
            let s = rinch_core::unowned(|| Signal::new(None).leak());
            c.set(Some(s));
            s
        }
    })
}

/// The sortable drag in progress, pointer or keyboard, read reactively: an
/// effect or a `{|| …}` that calls it runs again when the drag starts, moves
/// to another place, is judged or ends. With every presentation option at
/// its none value (`preview: SortPreview::None`, `indicator:
/// SortIndicator::None`, `source: SortSource::Unchanged`) an app draws the
/// whole drag from this and [`sortable_drag_pointer`].
pub fn sortable_drag() -> Option<SortDragState> {
    live_signal(&STATE).get()
}

/// The pointer of a pointer drag in progress, read reactively (it changes on
/// every move, so it is a signal of its own). `None` for a keyboard lift.
pub fn sortable_drag_pointer() -> Option<SortPointer> {
    live_signal(&POINTER).get()
}

/// Bring the drag-state signals up to date.
pub(super) fn publish() {
    let (state, pointer) = ACTIVE.with(|a| {
        let a = a.borrow();
        let Some(a) = a.as_ref() else {
            return (None, None);
        };
        let target = a.offer.as_ref().map(|o| SortTarget {
            list: o.list_id.clone(),
            index: o.slot,
        });
        let state = SortDragState {
            group: a.group.clone(),
            keys: a.keys.clone(),
            source_key: a.source_key.clone(),
            from_list: a.from_list.clone(),
            keyboard: a.lift.is_some(),
            over: a.over.as_ref().map(|(list, index)| SortTarget {
                list: list.clone(),
                index: *index,
            }),
            target,
            verdict: a.offer.as_ref().map(|o| o.verdict.clone()),
        };
        (Some(state), a.grab.filter(|_| a.lift.is_none()))
    });
    live_signal(&STATE).set_if_changed(state);
    live_signal(&POINTER).set_if_changed(pointer);
}

/// The class the carried items wear for the drag.
pub(super) fn carried_class(keyboard: bool, source: SortSource) -> Option<&'static str> {
    if keyboard {
        return Some(LIFTED_CLASS);
    }
    match source {
        SortSource::Dim => Some(DRAGGING_CLASS),
        SortSource::Collapse => Some(COLLAPSED_CLASS),
        SortSource::Hide => Some(HIDDEN_CLASS),
        SortSource::Unchanged => None,
    }
}

/// The carried items' extent along x and y, each with the list's spacing
/// after it (measured beside the pressed item on its line,
/// [`spacing_beside`]): what a gap opens by. Zero where the host has no
/// boxes.
pub(super) fn carried_extent(
    cfg: &ListConfig,
    item: &NodeHandle,
    carried: &[NodeHandle],
) -> ((f32, f32), f32) {
    let all = items_of(&cfg.node);
    let at = all.iter().position(|(n, _)| n.node_id() == item.node_id());
    // Painted, then brought to the item's own units (an ancestor's scale):
    // shifts are `translate`s in those units.
    let fit = item
        .bounding_rect()
        .zip(item.layout_size())
        .map(|(b, (lw, lh))| match cfg.axis {
            SortAxis::Vertical if lh > 0.0 => b.height / lh,
            SortAxis::Horizontal if lw > 0.0 => b.width / lw,
            _ => 1.0,
        })
        .filter(|f| *f > 0.0)
        .unwrap_or(1.0);
    let boxes: Vec<Option<AxisBox>> = all
        .iter()
        .map(|(n, _)| Some(AxisBox::of(&n.bounding_rect()?, cfg.axis)))
        .collect();
    let spacing = at.map_or(0.0, |i| spacing_beside(&boxes, i)) / fit;
    let (mut w, mut h) = (0.0, 0.0);
    for node in carried {
        if let Some((nw, nh)) = size_of(node) {
            w += nw;
            h += nh;
            match cfg.axis {
                SortAxis::Vertical => h += spacing,
                SortAxis::Horizontal => w += spacing,
            }
        }
    }
    ((w, h), spacing)
}

/// `node`'s size in its own units (as laid out, before an ancestor's
/// scale); its painted size where the host has no layout.
pub(super) fn size_of(node: &NodeHandle) -> Option<(f32, f32)> {
    node.layout_size()
        .or_else(|| node.bounding_rect().map(|b| (b.width, b.height)))
}

/// Where the press that started a pointer drag landed in `item`, from the
/// `data-ondragstart` click context (the press point); its middle when the
/// context names a point outside it (a host that does not set it).
pub(super) fn grab_point(item: &NodeHandle) -> Option<SortPointer> {
    let b = item.bounding_rect()?;
    let ctx = get_click_context();
    let (mut gx, mut gy) = (ctx.mouse_x - b.x, ctx.mouse_y - b.y);
    if !(0.0..=b.width).contains(&gx) || !(0.0..=b.height).contains(&gy) {
        gx = b.width / 2.0;
        gy = b.height / 2.0;
    }
    Some(SortPointer {
        x: b.x + gx,
        y: b.y + gy,
        grab_x: gx,
        grab_y: gy,
        width: b.width,
        height: b.height,
    })
}
