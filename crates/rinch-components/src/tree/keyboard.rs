//! The keyboard cursor, the roving tab stop, focus tracking and the key handler.

use super::*;

/// Draw the cursor on the row of `value` (and off the one it was on).
fn draw_cursor(config: &Rc<TreeConfig>, value: Option<&str>) {
    if config.keys.drawn.borrow().as_deref() == value {
        // A row drawn again draws its own cursor (`render_tree_node`).
        return;
    }
    let previous = config.keys.drawn.borrow_mut().take();
    let previous = previous.and_then(|v| config.drag.rows.borrow().get(&v).cloned());
    if let Some(row) = previous {
        row.remove_class(CURSOR_CLASS);
    }
    if let Some(value) = value {
        let row = config.drag.rows.borrow().get(value).cloned();
        if let Some(row) = row {
            *config.keys.drawn.borrow_mut() = Some(value.to_string());
            row.add_class(CURSOR_CLASS);
        }
    }
}

/// Make the row of `value` the tree's one Tab stop (roving tabindex): it
/// gets `tabindex="0"`, the one before it `-1`. A row not drawn yet takes it
/// when it is (`render_tree_node`).
fn set_stop(config: &Rc<TreeConfig>, value: Option<String>) {
    let old = config.keys.stop.replace(value.clone());
    if old == value {
        return;
    }
    let (old, new) = {
        let rows = config.drag.rows.borrow();
        (
            old.and_then(|v| rows.get(&v).cloned()),
            value.and_then(|v| rows.get(&v).cloned()),
        )
    };
    if let Some(old) = old {
        old.set_attribute("tabindex", "-1");
    }
    if let Some(new) = new {
        new.set_attribute("tabindex", "0");
    }
}

/// The Tab stop is the cursor row while it is shown, else the first row.
fn refresh_stop(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    let cursor = untracked(|| tree_state.cursor.get());
    let expanded = untracked(|| tree_state.expanded.get());
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let stop = cursor
        .and_then(|c| shape.index(&c))
        .filter(|&row| shape.is_shown(row, &expanded))
        .or_else(|| shape.roots.first().copied())
        .map(|row| shape.rows[row].value.clone());
    set_stop(config, stop);
}

/// The tree's row the keyboard is on, if it is on one of them, by value.
fn focused_row(config: &Rc<TreeConfig>) -> Option<String> {
    let container = config.keys.container.borrow().clone()?;
    let active = container.active_element()?;
    let on_a_row = active.get_attribute("class").is_some_and(|c| {
        c.split_whitespace()
            .any(|one| one == "rinch-tree__node-content")
    });
    if !on_a_row || !focus_inside(config) {
        return None;
    }
    active
        .parent_node()
        .and_then(|li| li.get_attribute("data-value"))
}

/// The cursor moved while a row has the keyboard (the app put it somewhere,
/// [`TreeController::set_cursor`]): the keyboard goes with it.
fn follow_focus(config: &Rc<TreeConfig>, cursor: Option<&str>) {
    let Some(cursor) = cursor else { return };
    if !config.keys.focused.get() {
        return;
    }
    let Some(focused) = focused_row(config) else {
        return;
    };
    if focused == cursor {
        return;
    }
    let row = config.drag.rows.borrow().get(cursor).cloned();
    if let Some(row) = row.filter(|row| row.is_rendered()) {
        row.focus();
        row.scroll_into_view();
    }
}

/// The tree's data changed (`old` is what it was). A cursor whose row is
/// gone moves to the row drawn below it (above it at the end) that is still
/// there, taking the keyboard with it if it had it; and the Tab stop follows.
pub(super) fn data_changed(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    old: &[TreeNodeData],
) {
    let cursor = tree_state.cursor.get();
    let shape = config.drag.shape(config.accepts_children.as_ref());
    if let Some(cursor) = cursor.filter(|c| shape.index(c).is_none()) {
        let expanded = tree_state.expanded.get();
        let before = Shape::of(old, None);
        let next = before.index(&cursor).and_then(|row| {
            let visible = before.visible(&expanded);
            let at = visible.iter().position(|&r| r == row)?;
            visible[at + 1..]
                .iter()
                .chain(visible[..at].iter().rev())
                .map(|&r| &before.rows[r].value)
                .find(|v| shape.index(v).is_some())
                .cloned()
        });
        if config.keys.focused.get() && next.is_none() {
            set_focused(config, false);
        }
        if config.keys.focused.get() && next.is_some() {
            config.keys.lost_row.set(true);
            let weak = Rc::downgrade(config);
            let _ = set_timeout(0, move || {
                if let Some(cfg) = weak.upgrade() {
                    restore_keyboard(&cfg, tree_state);
                }
            });
        }
        tree_state.cursor.set(next);
    }
    refresh_stop(config, tree_state);
}

/// Whether the keyboard is on one of the tree's elements (a row, or a field
/// the app put in one).
fn focus_inside(config: &Rc<TreeConfig>) -> bool {
    let container = config.keys.container.borrow().clone();
    let Some(container) = container else {
        return false;
    };
    let mut at = container.active_element();
    while let Some(node) = at {
        if node.node_id() == container.node_id() {
            return true;
        }
        at = node.parent_node();
    }
    false
}

/// Whether something outside the tree has the keyboard now (a field the
/// user went to): the tree must not take it back.
fn focus_elsewhere(config: &Rc<TreeConfig>) -> bool {
    let container = config.keys.container.borrow().clone();
    let Some(active) = container.and_then(|c| c.active_element()) else {
        return false;
    };
    // The removed row the focus fell from is in no document.
    active.is_rendered() && !focus_inside(config)
}

/// Give the keyboard back to the cursor row as drawn now, after the box that
/// had it was removed: once, right after. Not when the keyboard is somewhere
/// in the tree already (a rename box the app opened) or has gone somewhere
/// else, and not to a row that is not drawn.
pub(super) fn restore_keyboard(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    if !config.keys.lost_row.replace(false) {
        return;
    }
    if focus_inside(config) {
        return;
    }
    if focus_elsewhere(config) {
        set_focused(config, false);
        return;
    }
    let cursor = untracked(|| tree_state.cursor.get());
    let row = cursor.and_then(|c| config.drag.rows.borrow().get(&c).cloned());
    match row.filter(|row| row.is_rendered()) {
        Some(row) => row.focus(),
        None => set_focused(config, false),
    }
}

pub(super) fn set_focused(config: &Rc<TreeConfig>, focused: bool) {
    config.keys.focused.set(focused);
    let container = config.keys.container.borrow().clone();
    if let Some(container) = container {
        if focused {
            container.add_class(FOCUSED_CLASS);
        } else {
            container.remove_class(FOCUSED_CLASS);
        }
    }
}

/// Select the row of `value` from the keyboard, as a click on it would:
/// only in a tree that selects on click, never a disabled row, and the app
/// is told ([`Tree::onselect`]). In a `multiple` tree the row is toggled.
fn select_from_keyboard(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    shape: &Shape,
    value: &str,
) -> bool {
    let selectable = config.select_on_click
        && shape
            .index(value)
            .is_some_and(|row| !shape.rows[row].disabled);
    if !selectable {
        return false;
    }
    let controller = tree_state.controller;
    if controller.multiple {
        // Space toggles the row and starts the next Shift run from it; a row
        // it selects is told to `onselect`, as a click on it is.
        tree_state.controller.set_anchor(Some(value));
        if untracked(|| controller.is_selected(value)) {
            controller.deselect(value);
            selection_changed(config, tree_state);
            return true;
        }
        controller.select(value);
        selection_changed(config, tree_state);
    } else {
        controller.select(value);
    }
    if let Some(cb) = &config.onselect {
        cb.invoke(value.to_string());
    }
    true
}

/// Move the cursor to `value` from the keyboard: give its row the keyboard,
/// show it, and tell the app once it rests. A single-select tree's
/// selection follows the cursor (as a click would select); a `multiple`
/// tree's does not: Space selects there. (Nor does a plain move change where
/// the next Shift run starts: that is the row last clicked or toggled.)
fn move_cursor(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, shape: &Shape, value: &str) {
    tree_state.cursor.set(Some(value.to_string()));
    if !tree_state.controller.multiple {
        select_from_keyboard(config, tree_state, shape, value);
    }
    show_cursor(config, value);
}

/// The cursor has moved to `value`: its row takes the keyboard and is
/// shown, and [`Tree::oncursor`] hears of it once it rests.
fn show_cursor(config: &Rc<TreeConfig>, value: &str) {
    let row = config.drag.rows.borrow().get(value).cloned();
    if let Some(row) = row {
        row.focus();
        row.scroll_into_view();
    }
    if let Some(handle) = config.keys.rest_timer.borrow_mut().take() {
        clear_timeout(handle);
    }
    if let Some(oncursor) = config.oncursor.clone() {
        let v = value.to_string();
        let fire = move || oncursor.invoke(v.clone());
        if config.cursor_delay_ms == 0 {
            fire();
        } else {
            let cfg = Rc::downgrade(config);
            let handle = set_timeout(config.cursor_delay_ms, move || {
                if let Some(cfg) = cfg.upgrade() {
                    cfg.keys.rest_timer.borrow_mut().take();
                }
                fire();
            });
            *config.keys.rest_timer.borrow_mut() = Some(handle);
        }
    }
}

fn expand(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) {
    tree_state.controller.expand(value);
    if let Some(cb) = &config.onexpand {
        cb.invoke(value.to_string());
    }
}

/// A key pressed while focus is inside the tree.
fn on_tree_key(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    container: &NodeHandle,
    key: &rinch_core::KeyEventData,
) -> bool {
    // Only a row's own keys: a field inside a row (a rename box) keeps its.
    let Some(active) = container.active_element() else {
        return false;
    };
    let on_a_row = active.get_attribute("class").is_some_and(|c| {
        c.split_whitespace()
            .any(|one| one == "rinch-tree__node-content")
    });
    if !on_a_row {
        return false;
    }
    let Some(row_value) = active
        .parent_node()
        .and_then(|li| li.get_attribute("data-value"))
    else {
        return false;
    };
    set_focused(config, true);
    // The keys act from the row that has the keyboard: it is the cursor row
    // (a click on a row's own control, or a scripted focus, can put the
    // keyboard on another).
    let cursor = row_value;
    if untracked(|| tree_state.cursor.get()).as_deref() != Some(cursor.as_str()) {
        tree_state.cursor.set(Some(cursor.clone()));
    }
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(cursor_row) = shape.index(&cursor) else {
        return false;
    };

    if let Some(handle_key) = &config.handle_key {
        let expanded = untracked(|| tree_state.expanded.get());
        let block = shape.carried(
            &expanded,
            &untracked(|| tree_state.selected.get()),
            cursor_row,
            tree_state.controller.multiple,
        );
        // The cursor row alone: the rows drawn right above and below it
        // (#1601). A block of selected rows: the rows around the block.
        let (above, below) = if block == [cursor_row] {
            let visible = shape.visible(&expanded);
            let at = visible.iter().position(|&r| r == cursor_row);
            (
                at.and_then(|i| i.checked_sub(1)).map(|i| visible[i]),
                at.and_then(|i| visible.get(i + 1).copied()),
            )
        } else {
            shape.around(&expanded, &block)
        };
        let neighbor = |row: Option<usize>| row.map(|r| shape.rows[r].value.clone());
        let (rows, carried) = carried_from(config, tree_state, &cursor);
        let offered = TreeKey {
            key: key.key.clone(),
            ctrl: key.ctrl,
            shift: key.shift,
            alt: key.alt,
            meta: key.meta,
            parent: shape.rows[cursor_row]
                .parent
                .map(|p| shape.rows[p].value.clone()),
            above: neighbor(above),
            below: neighbor(below),
            cursor: cursor.clone(),
            shape: shape.clone(),
            rows,
            carried,
            judge_drop: config.judge_drop.clone(),
        };
        if handle_key(&offered) {
            return true;
        }
    }
    let multiple = tree_state.controller.multiple;
    // Selecting several rows from the keyboard, in a tree that selects
    // (#1601: one that does not select on click selects nothing from it).
    let selects_several = multiple && config.select_on_click;
    // A chord is never the tree's, Ctrl+A (Cmd+A) included: an app that
    // wants select-all binds it in `handle_key` (#3062).
    if key.ctrl || key.alt || key.meta {
        return false;
    }

    let expanded = untracked(|| tree_state.expanded.get());
    // Shift with a move: a run from the anchor to where the cursor goes
    // (only the cursor moves when the tree does not select).
    if key.shift && multiple {
        if let Some(Nav::To(row)) = nav(&shape, &expanded, cursor_row, &key.key) {
            let to = shape.rows[row].value.clone();
            if config.select_on_click {
                select_run(config, tree_state, &cursor, &to);
            }
            tree_state.cursor.set(Some(to.clone()));
            show_cursor(config, &to);
            return true;
        }
    }
    // Shift+Space: the run from the anchor (the row last selected) to the
    // cursor row.
    if key.key == " " && key.shift && selects_several && config.keys.typed.borrow().is_empty() {
        select_run(config, tree_state, &cursor, &cursor);
        return true;
    }
    // Enter in a `multiple` tree: what a click does, except that the row
    // joins the selection instead of replacing it (the runtime's activation
    // would be a plain click, which replaces it).
    if key.key == "Enter" && multiple && !shape.rows[cursor_row].disabled {
        let has_children = !shape.rows[cursor_row].children.is_empty();
        activate_row(config, tree_state, &cursor, has_children, Pick::Add);
        return true;
    }
    // `*` is a shifted key on most layouts (Shift+8 on a US keyboard).
    if !key.shift || key.key == "*" {
        if let Some(step) = nav(&shape, &expanded, cursor_row, &key.key) {
            match step {
                Nav::To(row) => move_cursor(config, tree_state, &shape, &shape.rows[row].value),
                Nav::Expand(row) => expand(config, tree_state, &shape.rows[row].value),
                Nav::Collapse(row) => {
                    let value = shape.rows[row].value.clone();
                    tree_state.controller.collapse(&value);
                    unselect_hidden(config, tree_state, &value);
                    if let Some(cb) = &config.oncollapse {
                        cb.invoke(value);
                    }
                }
                Nav::ExpandAll(rows) => {
                    for row in rows {
                        expand(config, tree_state, &shape.rows[row].value);
                    }
                }
            }
            return true;
        }
    }

    // Space selects the cursor row (toggles it in a `multiple` tree), unless
    // it is part of a type-ahead word.
    if key.key == " " && !key.shift && config.keys.typed.borrow().is_empty() {
        return select_from_keyboard(config, tree_state, &shape, &cursor);
    }

    // Type-ahead: a printable character.
    let mut letters = key.key.chars();
    let (Some(letter), None) = (letters.next(), letters.next()) else {
        return false;
    };
    if letter.is_control() || (letter == ' ' && config.keys.typed.borrow().is_empty()) {
        return false;
    }
    config.keys.typed.borrow_mut().push(letter);
    if let Some(handle) = config.keys.typed_timer.borrow_mut().take() {
        clear_timeout(handle);
    }
    let cfg = Rc::downgrade(config);
    let handle = set_timeout(TYPE_AHEAD_MS, move || {
        if let Some(cfg) = cfg.upgrade() {
            cfg.keys.typed.borrow_mut().clear();
            cfg.keys.typed_timer.borrow_mut().take();
        }
    });
    *config.keys.typed_timer.borrow_mut() = Some(handle);
    let typed = config.keys.typed.borrow().clone();
    // The app's text for a row, or its label.
    let mut labels = HashMap::new();
    if config.label_of.is_none() {
        fn collect(nodes: &[TreeNodeData], labels: &mut HashMap<String, String>) {
            for node in nodes {
                labels.insert(node.value.clone(), node.label.clone());
                collect(&node.children, labels);
            }
        }
        collect(&config.drag.data.borrow(), &mut labels);
    }
    let text_of = |value: &str| match &config.label_of {
        Some(label_of) => label_of(value),
        None => labels.get(value).cloned().unwrap_or_default(),
    };
    if let Some(row) = type_ahead(&shape, &expanded, cursor_row, &typed, &text_of) {
        move_cursor(config, tree_state, &shape, &shape.rows[row].value);
    }
    true
}

/// The tree's keyboard: its key handler while focus is inside it, the
/// cursor drawn on its row, and focus requests.
pub(super) fn wire_keyboard(
    __scope: &mut RenderScope,
    container: &NodeHandle,
    tree_state: UseTreeReturn,
    config: &Rc<TreeConfig>,
) {
    container.add_class("rinch-tree--keyboard");
    *config.keys.container.borrow_mut() = Some(container.clone());

    let cfg = config.clone();
    let owner = container.clone();
    let leave_cfg = Rc::downgrade(config);
    let handle = rinch_core::push_key_handler(
        container,
        move |key| on_tree_key(&cfg, tree_state, &owner, key),
        move || {
            // Focus that fell from a cursor row the tree removed (it was drawn
            // again) is still the tree's: the row as drawn now takes it back.
            // Any other focus that left the tree has left: to an editor, a
            // field, or nothing at all.
            let Some(cfg) = leave_cfg.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(&cfg);
            let _ = set_timeout(0, move || {
                let Some(cfg) = weak.upgrade() else { return };
                // The keyboard is inside the tree after all: the tree still
                // has it.
                if focus_inside(&cfg) {
                    return;
                }
                // A removed cursor row's keyboard is being given back
                // (`restore_keyboard`) unless it has gone somewhere else.
                if cfg.keys.lost_row.get() {
                    if !focus_elsewhere(&cfg) {
                        return;
                    }
                    cfg.keys.lost_row.set(false);
                }
                set_focused(&cfg, false);
            });
        },
    );
    __scope.on_cleanup(move || drop(handle));

    let cfg = config.clone();
    __scope.create_effect(move || {
        let cursor = tree_state.cursor.get();
        // A collapse can hide the cursor row, which moves the Tab stop.
        let _ = tree_state.expanded.get();
        untracked(|| {
            draw_cursor(&cfg, cursor.as_deref());
            refresh_stop(&cfg, tree_state);
            follow_focus(&cfg, cursor.as_deref());
        });
    });

    let cfg = config.clone();
    let first = Cell::new(true);
    __scope.create_effect(move || {
        let _ = tree_state.controller.focus_requests.get();
        if first.replace(false) {
            return;
        }
        untracked(|| {
            let cursor = tree_state
                .cursor
                .get()
                .filter(|c| cfg.drag.rows.borrow().contains_key(c));
            let target = cursor.or_else(|| {
                let shape = cfg.drag.shape(cfg.accepts_children.as_ref());
                shape.roots.first().map(|&r| shape.rows[r].value.clone())
            });
            if let Some(value) = target {
                tree_state.cursor.set(Some(value.clone()));
                let row = cfg.drag.rows.borrow().get(&value).cloned();
                if let Some(row) = row {
                    set_focused(&cfg, true);
                    row.focus();
                    row.scroll_into_view();
                }
            }
        });
    });
}
