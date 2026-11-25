//! Keyboard: lifting, moving and dropping an item by key.

use super::*;

/// An arrow, as a direction on screen: (dx, dy).
pub(super) fn arrow(key: &str) -> Option<(i32, i32)> {
    match key {
        "ArrowUp" => Some((0, -1)),
        "ArrowDown" => Some((0, 1)),
        "ArrowLeft" => Some((-1, 0)),
        "ArrowRight" => Some((1, 0)),
        _ => None,
    }
}

fn center(b: &ElementBounds) -> (f32, f32) {
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

/// The nearest list of the group from `from` in direction `dir`, by on-screen
/// boxes; without boxes, the previous or next in document order. Disabled
/// lists and lists inside a carried item are not places.
pub(super) fn neighbour(
    from: &ListConfig,
    dir: (i32, i32),
    keys: &[String],
) -> Option<Rc<ListConfig>> {
    let lists: Vec<Rc<ListConfig>> = lists_of_group(&from.group, from.node.doc_key())
        .into_iter()
        .filter(|c| c.id == from.id || (!c.disabled && !inside_carried(&c.node, &from.group, keys)))
        .collect();
    let at = lists.iter().position(|c| c.id == from.id)?;
    let Some(here) = from.node.bounding_rect() else {
        let forward = dir.0 + dir.1 > 0;
        return if forward {
            lists.get(at + 1).cloned()
        } else {
            at.checked_sub(1).map(|i| lists[i].clone())
        };
    };
    let (hx, hy) = center(&here);
    let mut best: Option<(f32, Rc<ListConfig>)> = None;
    for cfg in &lists {
        if cfg.id == from.id {
            continue;
        }
        let Some(b) = cfg.node.bounding_rect() else {
            continue;
        };
        if b.width <= 0.0 && b.height <= 0.0 {
            continue;
        }
        let (cx, cy) = center(&b);
        let (along, across) = if dir.0 != 0 {
            ((cx - hx) * dir.0 as f32, (cy - hy).abs())
        } else {
            ((cy - hy) * dir.1 as f32, (cx - hx).abs())
        };
        if along <= 1.0 {
            continue;
        }
        let score = along + 2.0 * across;
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, cfg.clone()));
        }
    }
    best.map(|(_, c)| c)
}

/// Lift `item` for a keyboard move.
pub(super) fn lift(item: &NodeHandle, key: &str) {
    if !begin(item, key, true) {
        return;
    }
    let Some(cfg) = list_of(item) else {
        return;
    };
    let keys = active_keys();
    let home = items_of(&cfg.node)
        .iter()
        .take_while(|(_, k)| *k != key)
        .filter(|(_, k)| !keys.contains(k))
        .count();
    let entry = focus_leave_entry(item);
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.lift = Some(Lift {
                list_id: cfg.id.clone(),
                slot: home,
                _keys: Some(entry),
            });
        }
    });
    publish();
    let label = label_of(item, key);
    announce(&format!(
        "Picked up {label}. Use the arrow keys to move it, the same key to drop it, \
         Escape to cancel."
    ));
}

/// A key entry (taking no key) whose focus-leave cancels the lift of `item`:
/// when the focus is now on another element. A focus that went nowhere known
/// (the lifted node was re-rendered away) is not a leave.
pub(super) fn focus_leave_entry(item: &NodeHandle) -> DismissHandle {
    let owner = item.clone();
    rinch_core::push_key_handler(
        item,
        |_| false,
        move || {
            let lifted = ACTIVE.with(|a| {
                a.borrow()
                    .as_ref()
                    .filter(|a| a.lift.is_some())
                    .map(|a| a.source.clone())
            });
            let Some(source) = lifted else { return };
            let Some(focus) = owner.active_element() else {
                return;
            };
            let mut at = Some(focus);
            while let Some(n) = at {
                if n.node_id() == source.node_id() {
                    return;
                }
                at = n.parent_node();
            }
            cancel_lift(true);
        },
    )
}

fn label_of(item: &NodeHandle, key: &str) -> String {
    item.get_attribute("data-sortable-label")
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| key.to_string())
}

/// Say `text` in the live region of the drag's source list, as it is drawn
/// now (a re-render of its lane gives it a new region), and say it even when
/// it repeats the last (an alternating trailing no-break space changes it).
fn announce(text: &str) {
    let region = ACTIVE.with(|a| {
        a.borrow().as_ref().map(|a| {
            list_by_id(&a.group, &a.from_list, a.doc)
                .map_or_else(|| a.from.live.clone(), |c| c.live.clone())
        })
    });
    if let Some(region) = region {
        announce_in(&region, text);
    }
}

fn announce_in(region: &NodeHandle, text: &str) {
    // The same sentence twice in a row (an arrow at the end of the lists) is
    // a change a screen reader still hears: a trailing no-break space, on
    // every other repeat.
    let current = region.text_content().unwrap_or_default();
    let text = if current.trim_end_matches('\u{a0}') == text && !current.ends_with('\u{a0}') {
        format!("{text}\u{a0}")
    } else {
        text.to_string()
    };
    region.set_text(&text);
}

/// The list a keyboard lift's caret is in, as drawn now.
fn lift_list() -> Option<(Rc<ListConfig>, usize)> {
    let (group, doc, list_id, slot) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let lift = a.lift.as_ref()?;
        Some((a.group.clone(), a.doc, lift.list_id.clone(), lift.slot))
    })?;
    Some((list_by_id(&group, &list_id, doc)?, slot))
}

/// Move a keyboard lift one step in `dir`.
fn move_lift(dir: (i32, i32)) {
    let Some((cfg, slot)) = lift_list() else {
        return;
    };
    let keys = active_keys();
    let len = rest_of(&cfg.node, &keys).0.len();
    let slot = slot.min(len);
    let (forward, along) = match cfg.axis {
        SortAxis::Vertical => (dir.1 > 0, dir.0 == 0),
        SortAxis::Horizontal => (dir.0 > 0, dir.1 == 0),
    };
    let (cfg, slot) = if along && forward && slot < len {
        (cfg, slot + 1)
    } else if along && !forward && slot > 0 {
        (cfg, slot - 1)
    } else if let Some(next) = neighbour(&cfg, dir, &keys) {
        let next_len = rest_of(&next.node, &keys).0.len();
        let slot = if along {
            if forward { 0 } else { next_len }
        } else {
            slot.min(next_len)
        };
        (next, slot)
    } else {
        (cfg, slot)
    };
    ACTIVE.with(|a| {
        if let Some(lift) = a.borrow_mut().as_mut().and_then(|a| a.lift.as_mut()) {
            lift.list_id = cfg.id.clone();
            lift.slot = slot;
        }
    });
    offer_slot(&cfg, slot);
    describe_lift(&cfg, slot);
}

/// Scroll the caret into view and announce where the lift is.
fn describe_lift(cfg: &ListConfig, slot: usize) {
    let (label, verdict, mark) = ACTIVE.with(|a| {
        let a = a.borrow();
        let Some(a) = a.as_ref() else {
            return (String::new(), None, None);
        };
        let label = label_of(&a.source, &a.source_key);
        let label = if a.keys.len() > 1 {
            format!("{label} and {} more", a.keys.len() - 1)
        } else {
            label
        };
        let offer = a.offer.as_ref();
        (
            label,
            offer.map(|o| o.verdict.clone()),
            offer.map(|o| o.mark.clone()),
        )
    });
    if let Some(mark) = &mark {
        mark.node().scroll_into_view();
    }
    let list = cfg.name();
    let text = match verdict {
        None => format!("{label}: back where it was."),
        Some(SortVerdict::Accept) => format!("{label}: {list}, position {}.", slot + 1),
        Some(SortVerdict::Refuse(reason)) => {
            format!(
                "{label}: {list}, position {}. Cannot drop here. {reason}",
                slot + 1
            )
        }
    };
    announce(&text);
}

/// Drop a keyboard lift where its caret is: settled and judged again first.
fn drop_lift() {
    let Some((group, doc, first)) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        a.lift.as_ref()?;
        Some((a.group.clone(), a.doc, a.keys.first().cloned()))
    }) else {
        return;
    };
    let delivered = settle();
    match &delivered {
        Some((cfg, mv)) => announce(&format!(
            "Dropped in {}, position {}.",
            cfg.name(),
            mv.place.index + 1
        )),
        None => announce("Not moved."),
    }
    end_active();
    let Some((cfg, mv)) = delivered else {
        return;
    };
    let cfg_id = Some(cfg.id.clone());
    if let Some(cb) = cfg.ondrop.clone() {
        cb.invoke(mv);
    }
    // The app re-renders from its data — in this handler's batch, or later —
    // and the moved item (a new node, when it changed lists) takes the focus
    // back once it is drawn.
    if let Some(first) = first {
        request_focus(doc, group, first, cfg_id);
    }
}

/// Focus the item `key` of `group` once it is drawn in `list`: at every
/// render of it, and on the next turn of the event loop (timers permitting)
/// if it is already there. Forgotten at the next drag.
fn request_focus(doc: u64, group: String, key: String, list: Option<String>) {
    PENDING_FOCUS.with_mut(|p| p.0 = Some((doc, group, key, list)));
    // Unowned: the item that parked it is usually the node the re-render
    // replaced, and a parked callback of a disposed owner is dropped. Where
    // no timer runs, the render of the item is the only place it is focused.
    rinch_core::unowned(|| set_timeout(0, try_pending_focus));
}

pub(super) fn try_pending_focus() {
    let Some((doc, group, key, list)) = PENDING_FOCUS.with(|p| p.0.clone()) else {
        return;
    };
    // Not yet where it was dropped (the app re-renders later): its render
    // there focuses it.
    for cfg in lists_of_group(&group, doc)
        .into_iter()
        .filter(|c| list.as_ref().is_none_or(|l| *l == c.id))
    {
        if let Some((node, _)) = items_of(&cfg.node).into_iter().find(|(_, k)| *k == key) {
            PENDING_FOCUS.with_mut(|p| p.0.take());
            focus_item(&node);
            return;
        }
    }
}

/// Give a keyboard lift's item the focus. An item is a Tab stop only in a
/// [`SortableList::keyboard`] list, and only once the list has seen it
/// (`offer_keyboard`): the item drawn again in the middle of a lift (its
/// render comes before its list's), and one dropped into a list without
/// `keyboard`, are made focusable first, as the item that holds the
/// keyboard has to be.
pub(super) fn focus_item(item: &NodeHandle) {
    if item.get_attribute("tabindex").is_none() {
        item.set_attribute("tabindex", "0");
    }
    item.focus();
}

/// Cancel a keyboard lift (`focus_left`: the focus moved away).
fn cancel_lift(focus_left: bool) {
    let lifted = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_some()));
    if !lifted {
        return;
    }
    if !focus_left {
        announce("Cancelled. Not moved.");
    }
    end_active();
}

/// An item's `onkeydown`.
pub(super) fn item_key(item: &NodeHandle, key: &str, ev: &KeyEventData) -> bool {
    if ev.kind != rinch_core::events::KeyEventKind::Down {
        return false;
    }
    // Only for the item itself: a key in a field or a button inside it is
    // that control's. The event names its target where the backend knows it;
    // else the document's focus.
    let elsewhere = match ev.target {
        Some(target) => target != item.node_id(),
        None => item
            .active_element()
            .is_some_and(|f| f.node_id() != item.node_id()),
    };
    if elsewhere {
        return false;
    }
    let Some(cfg) = list_of(item) else {
        return false;
    };
    let lifted_here = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.lift.is_some() && a.source.node_id() == item.node_id())
    });
    let modified = ev.ctrl || ev.alt || ev.meta;
    let activate = cfg.is_lift_key(ev) && !modified;
    if lifted_here {
        if let Some(dir) = arrow(&ev.key) {
            move_lift(dir);
            return true;
        }
        if activate {
            drop_lift();
            return true;
        }
        if ev.key == "Escape" {
            cancel_lift(false);
            return true;
        }
        if ev.key == "Tab" {
            cancel_lift(false);
        }
        return false;
    }
    let pointer_drag = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.lift.is_none() && !a.source_gone)
    });
    // Only a `keyboard` list's lift keys lift (#3062): elsewhere they are the
    // app's. A lift under way goes on above whatever the list says now.
    if activate && cfg.keyboard && !pointer_drag && !ev.shift && !cfg.disabled {
        lift(item, key);
        return ACTIVE.with(|a| a.borrow().is_some());
    }
    false
}
