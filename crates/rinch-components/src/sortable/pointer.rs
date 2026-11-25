//! Pointer handlers.

use super::*;

/// A drag moved over `target` (an item or a list): offer the place under the
/// pointer.
pub(super) fn drag_over(target: &NodeHandle) {
    let Some((group, false)) = active_group() else {
        return;
    };
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.hovered = true;
        }
    });
    let keys = active_keys();
    let Some((cfg, item)) = resolve_target(target, &group, &keys) else {
        withdraw();
        return;
    };
    let ctx = get_click_context();
    follow_pointer(ctx.mouse_x, ctx.mouse_y);
    let pointer = match cfg.axis {
        SortAxis::Vertical => ctx.mouse_y,
        SortAxis::Horizontal => ctx.mouse_x,
    };
    // The resolved node's box: the handler's own from the click context, any
    // other (a delegated item) asked of the document.
    let bounds_of = |node: &NodeHandle| -> Option<ElementBounds> {
        if node.node_id() == target.node_id() && ctx.element_width + ctx.element_height > 0.0 {
            Some(ElementBounds {
                x: ctx.element_x,
                y: ctx.element_y,
                width: ctx.element_width,
                height: ctx.element_height,
            })
        } else {
            node.bounding_rect()
        }
    };
    // Over the source itself where it was moved to the place on offer (a
    // gap in its own list): that place stays. Read from where it was laid
    // out, it would be its own place, and the place would flip back and
    // forth under a still pointer.
    if let Some((node, key)) = &item
        && keys.contains(key)
        && cfg.indicator.gap()
    {
        let held = ACTIVE.with(|a| {
            a.borrow().as_ref().and_then(|a| {
                if a.from.source == SortSource::Collapse || a.from_list != cfg.id {
                    return None;
                }
                // Only the source drawn away from its home is drawn at a
                // place: one at home (a fresh node a co-edit drew) is over
                // its own place, and is read as any item is.
                let moved_now = a
                    .shifted
                    .iter()
                    .any(|(n, x, y)| n.node_id() == node.node_id() && (*x != 0.0 || *y != 0.0));
                let moved_then = a.left_moved.contains(&node.node_id());
                match (&a.over, &a.left_over) {
                    (Some((l, _)), _) if *l == cfg.id && moved_now => Some(None),
                    (None, Some((l, s))) if *l == cfg.id && moved_then => Some(Some(*s)),
                    _ => None,
                }
            })
        });
        match held {
            Some(None) => return,
            Some(Some(slot)) => {
                offer_slot(&cfg, slot);
                // Drawn as offered, even when the offer itself is unchanged.
                refresh();
                return;
            }
            None => {}
        }
    }
    let all = items_of(&cfg.node);
    let mut slot = 0;
    match item {
        Some((node, key)) => {
            let mut carried_before = false;
            for (_, k) in &all {
                if *k == key {
                    break;
                }
                if keys.contains(k) {
                    carried_before = true;
                } else {
                    slot += 1;
                }
            }
            if !keys.contains(&key) {
                let past_middle = match bounds_of(&node) {
                    Some(b) => {
                        let (start, len) = span(&b, cfg.axis);
                        pointer >= start + len / 2.0
                    }
                    // No box to measure by (a host that cannot answer
                    // `bounding_rect`): over an item the carried ones are
                    // before, after it — the drag is moving forward past it —
                    // else before it.
                    None => carried_before,
                };
                if past_middle {
                    slot += 1;
                }
            }
        }
        None => {
            for (node, k) in &all {
                if keys.contains(k) {
                    continue;
                }
                match node.bounding_rect() {
                    Some(b) => {
                        let (start, len) = span(&b, cfg.axis);
                        if pointer >= start + len / 2.0 {
                            slot += 1;
                        }
                    }
                    // No boxes to measure by: the end.
                    None => slot += 1,
                }
            }
        }
    }
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.pending_none = false;
        }
    });
    offer_slot(&cfg, slot);
}

pub(super) fn drag_leave() {
    let now = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let Some(a) = a.as_mut().filter(|a| a.lift.is_none()) else {
            return false;
        };
        a.pending_none = !a.source_gone;
        a.source_gone
    });
    if now {
        withdraw();
    }
}

/// After every move's targets: a left target nothing replaced withdraws, and
/// the preview follows the pointer.
pub(super) fn drag_move() {
    // A left target nothing replaced, or a move over no target at all with
    // a place still held (the drag starts over its own place).
    let ctx = get_click_context();
    // The runtime offers no drop on the dragged element itself: a move over
    // the source (drawn at the place on offer, in its own list) hears no
    // `data-ondragover` and is not a move over nothing.
    let over_source = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|a| a.source.bounding_rect())
            .is_some_and(|b| {
                (b.x..=b.x + b.width).contains(&ctx.mouse_x)
                    && (b.y..=b.y + b.height).contains(&ctx.mouse_y)
            })
    });
    let pending = ACTIVE.with(|a| {
        a.borrow_mut().as_mut().is_some_and(|a| {
            // A leave's place is taken back in the same move or not at all.
            a.left_over = None;
            let unheard = !std::mem::replace(&mut a.hovered, false) && a.over.is_some();
            (std::mem::replace(&mut a.pending_none, false) || unheard) && !over_source
        })
    });
    if pending {
        withdraw();
    }
    follow_pointer(ctx.mouse_x, ctx.mouse_y);
}

/// The release: settle the offer against the lists as they are, deliver it if
/// still accepted, then end.
pub(super) fn drop_here() {
    let pointer = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_none()));
    if !pointer {
        return;
    }
    let delivered = settle();
    end_active();
    if let Some((cfg, mv)) = delivered
        && let Some(cb) = cfg.ondrop.clone()
    {
        cb.invoke(mv);
    }
}
