//! Presentation: the gap at the place on offer, laid out as item shifts.

use super::*;

/// Take `props` out of `node`'s inline style.
pub(super) fn clear_props(node: &NodeHandle, props: &[&str]) {
    let Some(style) = node.get_attribute("style") else {
        return;
    };
    let decls: Vec<(String, String)> = rinch_core::dom::split_declarations(&style)
        .into_iter()
        .filter(|(k, _)| !props.contains(&k.as_str()))
        .collect();
    let out = rinch_core::dom::serialize_declarations(&decls);
    if out.is_empty() {
        node.remove_attribute("style");
    } else {
        node.set_attribute("style", &out);
    }
}

/// Each item of `cfg` (the source list) and how far it moves when the
/// carried items in it are put at `slot`: its place in the list packed in the
/// new order less its place packed as it is, from the items' sizes as laid
/// out now — so an item added or removed mid-drag (a co-edit) is laid out
/// with the rest. Nothing where a size is unknown.
fn reordered(
    cfg: &ListConfig,
    keys: &[String],
    slot: usize,
    spacing: f32,
) -> Vec<(NodeHandle, f32)> {
    let all = items_of(&cfg.node);
    let len = |n: &NodeHandle| {
        size_of(n).map(|(w, h)| match cfg.axis {
            SortAxis::Vertical => h,
            SortAxis::Horizontal => w,
        })
    };
    let mut before = Vec::with_capacity(all.len());
    let mut at = 0.0;
    for (n, _) in &all {
        let Some(l) = len(n) else {
            return Vec::new();
        };
        before.push((n.node_id(), at));
        at += l + spacing;
    }
    let (mut order, mut carried): (Vec<_>, Vec<_>) =
        all.into_iter().partition(|(_, k)| !keys.contains(k));
    carried.sort_by_key(|(_, k)| keys.iter().position(|x| x == k));
    let at = slot.min(order.len());
    order.splice(at..at, carried);
    let mut cursor = 0.0;
    order
        .into_iter()
        .map(|(n, _)| {
            let was = before
                .iter()
                .find(|(id, _)| *id == n.node_id())
                .map_or(cursor, |b| b.1);
            let d = cursor - was;
            cursor += len(&n).unwrap_or(0.0) + spacing;
            (n, d)
        })
        .collect()
}

/// Lay the gap and the collapse out as shifts of the items, then publish.
pub(super) fn refresh() {
    reflow();
    publish();
}

/// The shift every item of the drag's lists should have now: the gap at the
/// place on offer (in a list with a gap indicator, never at a refused place),
/// and the carried items' space closed up in their list when they collapse.
pub(super) fn reflow() {
    struct Plan {
        group: String,
        doc: u64,
        keys: Vec<String>,
        from_list: String,
        collapse: bool,
        gap_at: Option<(String, usize)>,
        extents: (f32, f32),
        spacing: f32,
    }
    let Some(plan) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let collapse = a.lift.is_none() && a.from.source == SortSource::Collapse;
        let gap_at = match &a.offer {
            Some(o) if o.verdict.is_accept() => Some((o.list_id.clone(), o.slot)),
            Some(_) => None,
            // Over its own place, a collapsed item's space opens again.
            None if collapse => a.over.clone().filter(|(l, _)| *l == a.from_list),
            None => None,
        };
        Some(Plan {
            group: a.group.clone(),
            doc: a.doc,
            keys: a.keys.clone(),
            from_list: a.from_list.clone(),
            collapse,
            gap_at,
            extents: a.extent,
            spacing: a.spacing,
        })
    }) else {
        return;
    };
    let mut want: Vec<(NodeHandle, NodeHandle, f32, f32)> = Vec::new();
    let mut add = |cfg: &ListConfig, node: &NodeHandle, d: f32| {
        let (dx, dy) = match cfg.axis {
            SortAxis::Vertical => (0.0, d),
            SortAxis::Horizontal => (d, 0.0),
        };
        match want.iter_mut().find(|w| w.0.node_id() == node.node_id()) {
            Some(w) => {
                w.2 += dx;
                w.3 += dy;
            }
            None => want.push((node.clone(), cfg.node.clone(), dx, dy)),
        }
    };
    let along = |cfg: &ListConfig| match cfg.axis {
        SortAxis::Vertical => plan.extents.1,
        SortAxis::Horizontal => plan.extents.0,
    };
    if plan.collapse
        && let Some(cfg) = list_by_id(&plan.group, &plan.from_list, plan.doc)
    {
        // Each item moves up by the carried items before it: their share of
        // the extent, in proportion to how many there are.
        let all = items_of(&cfg.node);
        let carried = all.iter().filter(|(_, k)| plan.keys.contains(k)).count();
        let per = if carried > 0 {
            along(&cfg) / carried as f32
        } else {
            0.0
        };
        let mut before = 0;
        for (node, key) in &all {
            if plan.keys.contains(key) {
                before += 1;
            } else if before > 0 {
                add(&cfg, node, -per * before as f32);
            }
        }
    }
    if let Some((list, slot)) = &plan.gap_at
        && let Some(cfg) = list_by_id(&plan.group, list, plan.doc).filter(|c| c.indicator.gap())
    {
        if cfg.id == plan.from_list && !plan.collapse {
            // The source is drawn in its own list: the list is laid out again
            // in the order the drop would give, from where its items were, so
            // the ones between the place and the source make room and the
            // source sits at the place — nothing is painted over it.
            for (node, d) in reordered(&cfg, &plan.keys, *slot, plan.spacing) {
                add(&cfg, &node, d);
            }
        } else {
            let d = along(&cfg);
            for (node, _) in rest_of(&cfg.node, &plan.keys).0.iter().skip(*slot) {
                add(&cfg, node, d);
            }
        }
    }
    let (old, mut reflowing) = ACTIVE.with(|a| {
        a.borrow_mut()
            .as_mut()
            .map_or((Vec::new(), Vec::new()), |a| {
                (
                    std::mem::take(&mut a.shifted),
                    std::mem::take(&mut a.reflowing),
                )
            })
    });
    let mut shifted: Vec<(NodeHandle, f32, f32)> = Vec::new();
    for (node, list, dx, dy) in want {
        let prev = old.iter().find(|(n, _, _)| n.node_id() == node.node_id());
        if prev.is_none_or(|(_, x, y)| *x != dx || *y != dy) {
            if !reflowing.iter().any(|l| l.node_id() == list.node_id()) {
                list.add_class(REFLOWING_CLASS);
                reflowing.push(list);
            }
            node.add_class(SHIFTED_CLASS);
            node.set_styles(&[(SHIFT_X, &px(dx)), (SHIFT_Y, &px(dy))]);
        }
        shifted.push((node, dx, dy));
    }
    // What no longer shifts slides back, keeping its class until the end so
    // the way back is a transition too.
    for (node, x, y) in old {
        if shifted
            .iter()
            .any(|(n, _, _)| n.node_id() == node.node_id())
        {
            continue;
        }
        if x != 0.0 || y != 0.0 {
            node.set_styles(&[(SHIFT_X, "0px"), (SHIFT_Y, "0px")]);
        }
        shifted.push((node, 0.0, 0.0));
    }
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.shifted = shifted;
            a.reflowing = reflowing;
        }
    });
}
