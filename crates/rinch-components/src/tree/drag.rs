//! Wiring a row as a drag source and drop target, and judging a drop.

use super::*;

/// Make `content` (the row of `value`) a drag source and a drop target.
pub(super) fn wire_drag(
    __scope: &mut RenderScope,
    content: &NodeHandle,
    value: &str,
    tree_state: UseTreeReturn,
    config: &Rc<TreeConfig>,
) {
    content.set_attribute("draggable", "true");
    // A row redrawn while it is being dragged (its children arrived, say)
    // is still being dragged, and still shows the drop on offer.
    let carried = config.drag.dragged.borrow().iter().any(|d| d == value);
    if carried {
        content.add_class(DRAGGING_CLASS);
    }
    let drawn_here = config
        .drag
        .candidate
        .borrow()
        .as_ref()
        .filter(|(drop, _, _)| drop.over == value)
        .map(|(drop, placement, verdict)| (drop.zone, placement.line_level, *verdict));
    if let Some((zone, level, verdict)) = drawn_here {
        draw(content, zone, level, verdict);
    }

    let cfg = config.clone();
    let v = value.to_string();
    let start = __scope.register_handler(move || {
        let (_, carried) = carried_from(&cfg, tree_state, &v);
        // The tree shows the drag itself (the carried rows fade, the row
        // under the pointer draws the drop); the built-in snapshot of the
        // source row would sit over the very row the drop is drawn on.
        rinch_core::suppress_drag_ghost();
        mark_dragging(&cfg, &carried, true);
        cfg.drag.level_px.set(None);
        *cfg.drag.dragged.borrow_mut() = carried.clone();
        if let Some(cb) = &cfg.ondragstart {
            cb.invoke(carried);
        }
    });
    content.set_attribute("data-ondragstart", &start.0.to_string());

    let cfg = config.clone();
    let end = __scope.register_handler(move || end_drag(&cfg));
    content.set_attribute("data-ondragend", &end.0.to_string());

    let cfg = config.clone();
    let v = value.to_string();
    let over = __scope.register_handler(move || drag_over(&cfg, &v, tree_state));
    content.set_attribute("data-ondragover", &over.0.to_string());

    let cfg = config.clone();
    let v = value.to_string();
    let leave = __scope.register_handler(move || {
        let here = cfg
            .drag
            .candidate
            .borrow()
            .as_ref()
            .is_some_and(|(drop, _, _)| drop.over == v);
        if here {
            withdraw(&cfg);
        }
    });
    content.set_attribute("data-ondragleave", &leave.0.to_string());

    // Sent to the source row after every move, once the targets have heard
    // it: the moment to tell the app a withdrawn drop was not replaced.
    let cfg = config.clone();
    let moved = __scope.register_handler(move || report_withdrawn(&cfg));
    content.set_attribute("data-ondragmove", &moved.0.to_string());

    let cfg = config.clone();
    let drop = __scope.register_handler(move || {
        let candidate = cfg.drag.candidate.borrow().clone();
        offer(&cfg, None);
        if let Some((drop, _, DropVerdict::Allowed)) = candidate {
            if let Some(ondrop) = &cfg.ondrop {
                ondrop.invoke(drop);
            }
        }
        // The drop usually moves the dragged rows, and the source row a
        // drag's end is sent to may be gone by then: the drag ends here.
        end_drag(&cfg);
    });
    content.set_attribute("data-ondrop", &drop.0.to_string());
}

/// The drag is over, dropped or not: undim its rows, take down the drop on
/// offer, tell the app. Once: a drop ends it, and the source row's own end
/// (when the row is still there) finds nothing left to do.
fn end_drag(config: &Rc<TreeConfig>) {
    let carried = std::mem::take(&mut *config.drag.dragged.borrow_mut());
    if carried.is_empty() {
        return;
    }
    mark_dragging(config, &carried, false);
    offer(config, None);
    if let Some(cb) = &config.ondragend {
        cb.invoke(());
    }
}

/// Dim (or stop dimming) the rows a drag carries.
fn mark_dragging(config: &Rc<TreeConfig>, values: &[String], on: bool) {
    let rows: Vec<NodeHandle> = {
        let rows = config.drag.rows.borrow();
        values.iter().filter_map(|v| rows.get(v).cloned()).collect()
    };
    for row in &rows {
        if on {
            row.add_class(DRAGGING_CLASS);
        } else {
            row.remove_class(DRAGGING_CLASS);
        }
    }
}

/// A drag moved over the row of `value`: work out the drop on offer, show it,
/// and arm a hover-to-expand.
fn drag_over(config: &Rc<TreeConfig>, value: &str, tree_state: UseTreeReturn) {
    let dragged = config.drag.dragged.borrow().clone();
    if dragged.is_empty() {
        // Not a drag that started in this tree.
        return;
    }
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(over) = shape.index(value) else {
        return;
    };
    let dragged_rows: Vec<usize> = dragged.iter().filter_map(|d| shape.index(d)).collect();

    let ctx = get_click_context();
    let height = ctx.element_height.max(1.0);
    let zone = zone_at(
        (ctx.mouse_y - ctx.element_y) / height,
        shape.rows[over].accepts,
    );
    // Nothing is offered under a dragged row, or inside one. A dragged row's
    // own edges are places, which is how the last row of a group is dragged
    // out of it: left, over itself.
    let on_dragged_row = dragged_rows.contains(&over);
    let under_dragged = dragged_rows
        .iter()
        .any(|&d| d != over && shape.is_within(over, d));
    if under_dragged || (on_dragged_row && zone == DropZone::Inside) {
        offer(config, None);
        return;
    }
    let pointer_level = level_px(config, &shape, over)
        .filter(|px| *px > 0.0)
        .map(|px| ((ctx.mouse_x - ctx.element_x).max(0.0) / px) as usize);
    let expanded = untracked(|| tree_state.expanded.get());
    // The dragged rows are placed as if collapsed: their children travel
    // with them, so the row drawn below an expanded dragged row is the one
    // after its subtree, and its own bottom edge can take it out of its
    // group.
    let mut placing = expanded.clone();
    for d in &dragged {
        placing.remove(d);
    }
    let placement = shape.placement(over, zone, pointer_level, &placing, config.after_open_row);

    // Hover to expand: resting inside a collapsed row with children.
    let wants_hover = zone == DropZone::Inside
        && config.hover_expand_ms > 0
        && !shape.rows[over].children.is_empty()
        && !expanded.contains(value);
    let waiting_here = config
        .drag
        .hover
        .borrow()
        .as_ref()
        .is_some_and(|(v, _)| v == value);
    if !wants_hover {
        config.drag.cancel_hover();
    } else if !waiting_here {
        config.drag.cancel_hover();
        let v = value.to_string();
        let onexpand = config.onexpand.clone();
        let cfg = Rc::downgrade(config);
        let handle = set_timeout(config.hover_expand_ms, move || {
            let Some(cfg) = cfg.upgrade() else { return };
            cfg.drag.hover.borrow_mut().take();
            if cfg.drag.dragged.borrow().is_empty() {
                return;
            }
            tree_state.controller.expand(&v);
            if let Some(cb) = &onexpand {
                cb.invoke(v.clone());
            }
        });
        *config.drag.hover.borrow_mut() = Some((value.to_string(), handle));
    }

    let unchanged = config
        .drag
        .candidate
        .borrow()
        .as_ref()
        .is_some_and(|(drop, p, _)| drop.over == value && drop.zone == zone && *p == placement);
    if unchanged {
        return;
    }
    let row_value = |row: usize| shape.rows[row].value.clone();
    let drop = TreeDrop {
        dragged,
        parent: placement.parent.map(row_value),
        slot: match placement.slot {
            Slot::First => TreeSlot::First,
            Slot::Last => TreeSlot::Last,
            Slot::After(row) => TreeSlot::After(row_value(row)),
        },
        over: value.to_string(),
        zone,
    };
    let verdict = judge(
        &shape,
        &placement,
        &dragged_rows,
        &drop,
        config.judge_drop.as_ref(),
    );
    offer(config, Some((drop, placement, verdict)));
}

/// The verdict on a drop, for a drag and a keyboard move alike
/// ([`TreeKey::drop_for`]): refused when it would put a row into its own
/// subtree, or into a row that takes no children ([`Tree::accepts_children`]),
/// whatever the app says; otherwise the app's ([`Tree::judge_drop`]). The
/// places a drag offers never reach the first two cases (rows under a dragged
/// row offer nothing, a dragged row is placed as if collapsed, and a row that
/// takes no children has no inside zone); the keyboard's can.
pub(super) fn judge(
    shape: &Shape,
    placement: &Placement,
    dragged_rows: &[usize],
    drop: &TreeDrop,
    judge_drop: Option<&JudgeDrop>,
) -> DropVerdict {
    let cycle = placement
        .parent
        .is_some_and(|parent| dragged_rows.iter().any(|&d| shape.is_within(parent, d)));
    let refused_inside = drop.zone == DropZone::Inside
        && placement
            .parent
            .is_some_and(|parent| !shape.rows[parent].accepts);
    if cycle || refused_inside {
        DropVerdict::Refused
    } else {
        judge_drop.map_or(DropVerdict::Allowed, |judge| judge(drop))
    }
}

/// Pixels per level, for picking a depth by the pointer's column: the
/// tree's [`Tree::indent`], else measured from the rendered rows once per
/// drag. A row's first child (its chevron or spacer) starts its level's
/// indent into the row, so the difference between `over`'s and a top-level
/// row's, over `over`'s level, is one level's width. `None` over a top-level
/// row before anything was measured (a depth is then never asked for).
fn level_px(config: &Rc<TreeConfig>, shape: &Shape, over: usize) -> Option<f32> {
    if config.indent.is_some() {
        return config.indent;
    }
    if let Some(px) = config.drag.level_px.get() {
        return Some(px);
    }
    let level = shape.rows[over].level;
    let root = *shape.roots.first()?;
    if level == 0 {
        return None;
    }
    let inset = |row: usize| -> Option<f32> {
        let handle = config
            .drag
            .rows
            .borrow()
            .get(&shape.rows[row].value)
            .cloned()?;
        let first = handle.children().into_iter().next()?;
        first.get_layout_bounds().map(|(x, _, _, _)| x)
    };
    let px = (inset(over)? - inset(root)?) / level as f32;
    config.drag.level_px.set(Some(px));
    Some(px)
}

/// The indicator's classes on a row: a line (`before`, `after`) or an
/// outline (`inside`), drawn by the stylesheet's `::after` at the indent of
/// `--tree-drop-level`. On the row itself, so a row redrawn mid-drag (its
/// children arrived) is simply drawn again, never stranded.
const DROP_CLASSES: [&str; 4] = [
    "rinch-tree__node-content--drop-before",
    "rinch-tree__node-content--drop-inside",
    "rinch-tree__node-content--drop-after",
    "rinch-tree__node-content--drop-refused",
];

fn draw(row: &NodeHandle, zone: DropZone, line_level: usize, verdict: DropVerdict) {
    clear_drawn(row);
    row.add_class(match zone {
        DropZone::Before => DROP_CLASSES[0],
        DropZone::Inside => DROP_CLASSES[1],
        DropZone::After => DROP_CLASSES[2],
    });
    if verdict == DropVerdict::Refused {
        row.add_class(DROP_CLASSES[3]);
    }
    row.set_style("--tree-drop-level", &line_level.to_string());
}

pub(super) fn clear_drawn(row: &NodeHandle) {
    for class in DROP_CLASSES {
        row.remove_class(class);
    }
}

/// Make `candidate` the drop on offer: draw it, tell the app.
fn offer(config: &Rc<TreeConfig>, candidate: Option<(TreeDrop, Placement, DropVerdict)>) {
    let drag = &config.drag;
    drag.pending_none.set(false);
    if candidate.is_none() {
        drag.cancel_hover();
        if drag.candidate.borrow().is_none() && !drag.told.get() {
            return;
        }
    }
    take_down(drag);
    if let Some((drop, placement, verdict)) = &candidate {
        let row = drag.rows.borrow().get(&drop.over).cloned();
        if let Some(row) = row {
            *drag.drawn.borrow_mut() = Some(drop.over.clone());
            draw(&row, drop.zone, placement.line_level, *verdict);
        }
    }
    let told = candidate
        .as_ref()
        .map(|(drop, _, verdict)| (drop.clone(), *verdict));
    *drag.candidate.borrow_mut() = candidate;
    drag.told.set(told.is_some());
    if let Some(cb) = &config.ondropchange {
        cb.invoke(told);
    }
}

/// Clear the indicator from the row it is drawn on.
fn take_down(drag: &DragState) {
    let previous = drag.drawn.borrow_mut().take();
    let previous = previous.and_then(|value| drag.rows.borrow().get(&value).cloned());
    if let Some(row) = previous {
        clear_drawn(&row);
    }
}

/// The pointer left the row on offer. Its drop is no longer on offer, but
/// the app is not told yet: both backends send the old row's leave before
/// the new row's over, so a move from one row onto the next would report
/// `None` between the two. The source row's drag move, which follows every
/// move's targets, reports it if nothing replaced it ([`report_withdrawn`]).
fn withdraw(config: &Rc<TreeConfig>) {
    let drag = &config.drag;
    drag.cancel_hover();
    take_down(drag);
    drag.candidate.borrow_mut().take();
    drag.pending_none.set(true);
}

/// Tell the app a withdrawn drop was not replaced by another.
fn report_withdrawn(config: &Rc<TreeConfig>) {
    let drag = &config.drag;
    if !drag.pending_none.replace(false) || !drag.told.replace(false) {
        return;
    }
    if let Some(cb) = &config.ondropchange {
        cb.invoke(None);
    }
}
