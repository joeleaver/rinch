//! Places, judging and drawing: the place on offer, settling a drop, and
//! starting and ending a drag.

use super::*;

/// The list's items with the carried ones left out, and every key in order.
pub(super) fn rest_of(
    list: &NodeHandle,
    carried: &[String],
) -> (Vec<(NodeHandle, String)>, Vec<String>) {
    let all = items_of(list);
    let keys: Vec<String> = all.iter().map(|(_, k)| k.clone()).collect();
    let rest = all
        .into_iter()
        .filter(|(_, k)| !carried.contains(k))
        .collect();
    (rest, keys)
}

/// Whether putting `carried` at `slot` of `rest` leaves the list as it is.
fn is_no_op(
    all: &[String],
    rest: &[(NodeHandle, String)],
    carried: &[String],
    slot: usize,
) -> bool {
    if !carried.iter().all(|k| all.contains(k)) {
        return false;
    }
    let mut after: Vec<&String> = rest.iter().map(|(_, k)| k).collect();
    let tail = after.split_off(slot.min(after.len()));
    after.extend(carried.iter());
    after.extend(tail);
    after.len() == all.len() && after.iter().zip(all).all(|(a, b)| *a == b)
}

/// The start and length of `b` along `axis`.
pub(super) fn span(b: &ElementBounds, axis: SortAxis) -> (f32, f32) {
    match axis {
        SortAxis::Vertical => (b.y, b.height),
        SortAxis::Horizontal => (b.x, b.width),
    }
}

/// A place worked out against a list as it is drawn now.
struct Placed {
    slot: usize,
    mv: SortMove,
    mark: Mark,
    reason: NodeHandle,
}

/// The move `slot` of `cfg` would be for the active drag; `None` when there is
/// no drag or the place would leave the list as it is.
fn place_in(cfg: &ListConfig, slot: usize) -> Option<Placed> {
    let (group, keys, from_list) = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.group.clone(), a.keys.clone(), a.from_list.clone()))
    })?;
    let (rest, all) = rest_of(&cfg.node, &keys);
    let slot = slot.min(rest.len());
    if is_no_op(&all, &rest, &keys, slot) {
        return None;
    }
    let (mark, reason) = if rest.is_empty() {
        (Mark::Into(cfg.node.clone()), cfg.reason.clone())
    } else if slot == 0 {
        (Mark::Before(rest[0].0.clone()), reason_slot(&rest[0].0))
    } else {
        let item = rest[slot - 1].0.clone();
        let reason = reason_slot(&item);
        (Mark::After(item), reason)
    };
    let place = SortPlace {
        after: slot.checked_sub(1).map(|i| rest[i].1.clone()),
        before: rest.get(slot).map(|(_, k)| k.clone()),
        index: slot,
    };
    Some(Placed {
        slot,
        mv: SortMove {
            group,
            keys,
            from_list,
            to_list: cfg.id.clone(),
            place,
        },
        mark,
        reason,
    })
}

pub(super) fn judge(cfg: &ListConfig, mv: &SortMove) -> SortVerdict {
    cfg.judge.as_ref().map_or(SortVerdict::Accept, |judge| {
        rinch_core::untracked_handler(|| judge(mv))
    })
}

/// Make `slot` of `cfg` the place on offer for the active drag, judging it
/// and drawing the caret.
pub(super) fn offer_slot(cfg: &ListConfig, slot: usize) {
    let len = rest_of(&cfg.node, &active_keys()).0.len();
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.over = Some((cfg.id.clone(), slot.min(len)));
            a.left_over = None;
        }
    });
    let Some(placed) = place_in(cfg, slot) else {
        set_offer(None);
        return;
    };
    // The same place drawn on the same node: nothing to judge or redraw. (A
    // re-render that replaced the node draws it again.)
    let unchanged = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|a| a.offer.as_ref())
            .is_some_and(|o| {
                o.list_id == cfg.id && o.slot == placed.slot && o.mark.same(&placed.mark)
            })
    });
    if unchanged {
        return;
    }
    let verdict = judge(cfg, &placed.mv);
    let caret = if verdict.is_accept() {
        cfg.indicator.caret()
    } else {
        cfg.indicator != SortIndicator::None
    };
    set_offer(Some(Offer {
        list_id: cfg.id.clone(),
        slot: placed.slot,
        mv: placed.mv,
        verdict,
        mark: placed.mark,
        reason: Some(placed.reason),
        horizontal: cfg.axis == SortAxis::Horizontal,
        caret,
    }));
}

/// The drop: the place on offer worked out again against the lists as they
/// are now — the target list looked up by id, the slot found again by the
/// place's keys (after `after`; first when there was none) — and judged
/// again. The offer was made against the lists as they were at the last
/// move; a re-render since (a collaborator's edit) can have moved, removed or
/// changed what it named. `None` when nothing is on offer, the place is gone
/// (its `after` item left the list), it is now a no-op, or it is refused.
pub(super) fn settle() -> Option<(Rc<ListConfig>, SortMove)> {
    let (group, doc, list_id, place, keys) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let o = a.offer.as_ref()?;
        Some((
            a.group.clone(),
            a.doc,
            o.list_id.clone(),
            o.mv.place.clone(),
            a.keys.clone(),
        ))
    })?;
    let cfg = list_by_id(&group, &list_id, doc).filter(|c| !c.disabled)?;
    let (rest, _) = rest_of(&cfg.node, &keys);
    let slot = match &place.after {
        None => 0,
        Some(after) => rest.iter().position(|(_, k)| k == after)? + 1,
    };
    let placed = place_in(&cfg, slot)?;
    judge(&cfg, &placed.mv)
        .is_accept()
        .then_some((cfg, placed.mv))
}

/// An item's reason slot: its last child carrying the class.
fn reason_slot(item: &NodeHandle) -> NodeHandle {
    item.children()
        .into_iter()
        .rev()
        .find(|c| {
            c.get_attribute("class")
                .is_some_and(|cls| cls.split_whitespace().any(|w| w == REASON_CLASS))
        })
        .unwrap_or_else(|| item.clone())
}

pub(super) fn draw(offer: &Offer) {
    let refused = !offer.verdict.is_accept();
    match &offer.mark {
        _ if !offer.caret => {}
        Mark::Before(item) | Mark::After(item) => {
            item.add_class(if matches!(offer.mark, Mark::Before(_)) {
                DROP_BEFORE_CLASS
            } else {
                DROP_AFTER_CLASS
            });
            if offer.horizontal {
                item.add_class(DROP_ACROSS_CLASS);
            }
            if refused {
                item.add_class(ITEM_REFUSED_CLASS);
            }
        }
        Mark::Into(list) => {
            list.add_class(DROP_INTO_CLASS);
            if refused {
                list.add_class(LIST_REFUSED_CLASS);
            }
        }
    }
    if let (SortVerdict::Refuse(reason), Some(slot)) = (&offer.verdict, &offer.reason)
        && !reason.is_empty()
        && slot.get_attribute(KEY_ATTR).is_none()
    {
        slot.set_text(reason);
        slot.add_class(REASON_SHOWN_CLASS);
        slot.set_style("display", "block");
        if matches!(offer.mark, Mark::Before(_)) {
            slot.add_class(REASON_ABOVE_CLASS);
        }
    }
}

fn erase(offer: &Offer) {
    match &offer.mark {
        Mark::Before(item) | Mark::After(item) => {
            for class in [
                DROP_BEFORE_CLASS,
                DROP_AFTER_CLASS,
                DROP_ACROSS_CLASS,
                ITEM_REFUSED_CLASS,
            ] {
                item.remove_class(class);
            }
        }
        Mark::Into(list) => {
            list.remove_class(DROP_INTO_CLASS);
            list.remove_class(LIST_REFUSED_CLASS);
        }
    }
    if let Some(slot) = &offer.reason
        && slot.get_attribute(KEY_ATTR).is_none()
    {
        slot.remove_class(REASON_SHOWN_CLASS);
        slot.remove_class(REASON_ABOVE_CLASS);
        slot.set_style("display", "none");
        slot.set_text("");
    }
}

/// Replace the offer, redraw, and tell the source list's `ondropchange`.
fn set_offer(new: Option<Offer>) {
    set_offer_inner(new);
    refresh();
}

/// A drag left every list: nothing is under it any more.
pub(super) fn withdraw() {
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut()
            && let Some(over) = a.over.take()
        {
            a.left_over = Some(over);
            a.left_moved = a
                .shifted
                .iter()
                .filter(|(_, x, y)| *x != 0.0 || *y != 0.0)
                .map(|(n, _, _)| n.node_id())
                .collect();
        }
    });
    set_offer(None);
}

fn set_offer_inner(new: Option<Offer>) {
    let Some((old, from, report)) = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut()?;
        a.pending_none = false;
        if new.is_none() && a.offer.is_none() && !a.told {
            return None;
        }
        let report = new.as_ref().map(|o| (o.mv.clone(), o.verdict.clone()));
        a.told = report.is_some();
        Some((a.offer.take(), a.from.clone(), report))
    }) else {
        return;
    };
    if let Some(old) = &old {
        erase(old);
    }
    if let Some(new) = &new {
        draw(new);
    }
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.offer = new;
        }
    });
    if let Some(cb) = from.ondropchange.clone() {
        cb.invoke(report);
    }
}

/// End the active drag: take the caret and the dimming down; `ondropchange`
/// hears `None` if it last heard a move.
pub(super) fn end_active() {
    set_offer_inner(None);
    let Some(active) = ACTIVE.with(|a| a.borrow_mut().take()) else {
        return;
    };
    for node in &active.carried {
        for class in [DRAGGING_CLASS, LIFTED_CLASS, HIDDEN_CLASS, COLLAPSED_CLASS] {
            node.remove_class(class);
        }
    }
    // Everything at once, transition classes included: the drop's re-render
    // lands in the same frame, and nothing slides back from a stale offset.
    for (node, _, _) in &active.shifted {
        node.remove_class(SHIFTED_CLASS);
        clear_props(node, &[SHIFT_X, SHIFT_Y]);
    }
    for list in &active.reflowing {
        list.remove_class(REFLOWING_CLASS);
    }
    if let Some((ghost, scope)) = active.ghost {
        if let Some(scope) = scope {
            scope.dispose();
        }
        // Freed, not only detached: `set_inner_html` is the verb every
        // backend reclaims a subtree on (desktop's `discard` keeps its slab
        // entries, #723). The wrapper itself waits, detached, for the next
        // drag in this document.
        ghost.set_inner_html("");
        ghost.remove();
    }
    publish();
}

/// The runtime ended an element drag (dropped or cancelled): a pointer drag of
/// ours ends with it, whatever became of the source node its `data-ondragend`
/// was sent to.
pub(super) fn on_drag_end() {
    let pointer = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_none()));
    if pointer {
        end_active();
    }
}

pub(super) fn hook_drag_end() {
    if !DRAG_END_HOOKED.with(|h| h.replace(true)) {
        rinch_core::on_element_drag_end(on_drag_end);
    }
}

/// The carried items (in any list of the group).
fn carried_nodes(group: &str, doc: u64, keys: &[String]) -> Vec<NodeHandle> {
    let mut out = Vec::new();
    for cfg in lists_of_group(group, doc) {
        for (node, key) in items_of(&cfg.node) {
            if keys.contains(&key) {
                out.push(node);
            }
        }
    }
    out
}

/// The keys a drag from `key` in `cfg` carries.
fn keys_for(cfg: &ListConfig, key: &str) -> Vec<String> {
    let keys = cfg
        .drag_keys
        .as_ref()
        .map(|f| rinch_core::untracked_handler(|| f(key)))
        .unwrap_or_default();
    if keys.iter().any(|k| k == key) {
        keys
    } else {
        vec![key.to_string()]
    }
}

/// Start a drag of `item` (pointer or keyboard). Refused for a disabled item
/// or list.
pub(super) fn begin(item: &NodeHandle, key: &str, keyboard: bool) -> bool {
    end_active();
    PENDING_FOCUS.with_mut(|p| p.0.take());
    if item.get_attribute(DISABLED_ATTR).is_some() {
        return false;
    }
    let Some(cfg) = list_of(item).filter(|c| !c.disabled) else {
        return false;
    };
    let keys = keys_for(&cfg, key);
    let doc = item.doc_key();
    let carried = carried_nodes(&cfg.group, doc, &keys);
    // Measured before anything is shifted, hidden or copied.
    let (extent, spacing) = carried_extent(&cfg, item, &carried);

    let grab = (!keyboard).then(|| grab_point(item)).flatten();
    let (ghost, ghost_fix) = match (!keyboard)
        .then(|| build_ghost(&cfg, item, key, grab.as_ref()))
        .flatten()
    {
        Some((node, scope, fix)) => (Some((node, scope)), fix),
        None => (None, (0.0, 0.0)),
    };
    for node in &carried {
        if let Some(class) = carried_class(keyboard, cfg.source) {
            node.add_class(class);
        }
    }
    ACTIVE.with(|a| {
        *a.borrow_mut() = Some(Active {
            group: cfg.group.clone(),
            keys,
            from: cfg.clone(),
            from_list: cfg.id.clone(),
            doc,
            source: item.clone(),
            source_key: key.to_string(),
            carried,
            offer: None,
            told: false,
            pending_none: false,
            lift: None,
            source_gone: false,
            over: None,
            hovered: false,
            left_over: None,
            left_moved: Vec::new(),
            extent,
            grab,
            ghost,
            ghost_fix,
            spacing,
            shifted: Vec::new(),
            reflowing: Vec::new(),
        })
    });
    // The drag starts over its own place (a collapsed item's space stays
    // open there until the pointer leaves it).
    let home = items_of(&cfg.node)
        .iter()
        .take_while(|(_, k)| k != key)
        .filter(|(_, k)| !active_keys().contains(k))
        .count();
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.over = Some((cfg.id.clone(), home));
        }
    });
    refresh();
    true
}

pub(super) fn active_group() -> Option<(String, bool)> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.group.clone(), a.lift.is_some()))
    })
}

pub(super) fn active_keys() -> Vec<String> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| a.keys.clone())
            .unwrap_or_default()
    })
}
