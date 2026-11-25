//! Rendering rows, with and without a tree state.

use super::*;

// =============================================================================
// Render Functions
// =============================================================================

/// Render a single tree node with full reactive state.
pub(super) fn render_tree_node(
    __scope: &mut RenderScope,
    node: &TreeNodeData,
    level: usize,
    tree_state: UseTreeReturn,
    config: Rc<TreeConfig>,
) -> NodeHandle {
    let expanded_signal = tree_state.expanded;
    let selected_signal = tree_state.selected;
    let has_children = node.has_children();
    let node_value = node.value.clone();

    // Create <li> wrapper
    let li = rinch_macros::rsx! { li { class: "rinch-tree__node" } };
    // The row's content box is the treeitem (it is what takes the keyboard,
    // so a screen reader announces the item and its state); the list item
    // around it is only structure.
    li.set_attribute("role", "none");
    li.set_attribute("data-value", &node.value);
    let content = rinch_macros::rsx! { div {} };
    content.set_attribute("role", "treeitem");
    content.set_attribute("aria-level", &(level + 1).to_string());
    if node.disabled {
        content.set_attribute("aria-disabled", "true");
    }

    // Reactive aria-expanded
    if has_children {
        let li_clone = content.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_expanded = expanded_signal.get().contains(&nv);
            li_clone.set_attribute("aria-expanded", if is_expanded { "true" } else { "false" });
        });
    }

    // Reactive aria-selected
    {
        let li_clone = content.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_selected = selected_signal.get().contains(&nv);
            if is_selected {
                li_clone.set_attribute("aria-selected", "true");
            } else {
                li_clone.set_attribute("aria-selected", "false");
            }
        });
    }

    // Create content wrapper.
    //
    // The base class is written **here**, not by the effect below. It used to
    // be the effect's job — the effect rewrote the whole `class` attribute on
    // every run, so its first run was also what established the node's identity
    // — and that is exactly the coupling issue #717 is about. The static
    // sibling path (`render_tree_node_static`) has always written it at
    // creation; this path now matches it.
    content.set_attribute(
        "class",
        if node.disabled {
            "rinch-tree__node-content rinch-tree__node-content--disabled"
        } else {
            "rinch-tree__node-content"
        },
    );
    // A keyboard tree is one Tab stop: the row its cursor is on, or its first
    // row (roving tabindex, `set_stop`). Every row of any other tree is one.
    let stop =
        !config.keyboard || config.keys.stop.borrow().as_deref() == Some(node_value.as_str());
    content.set_attribute("tabindex", if stop { "0" } else { "-1" });
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );

    if config.draggable || config.keyboard || tree_state.controller.multiple {
        // A row drawn again replaces the old box, which keeps nothing of a
        // drag. (No borrow of the tree's state is held across a DOM call
        // anywhere here: one can run pending effects, which can draw rows,
        // which come back here.)
        let old = config
            .drag
            .rows
            .borrow_mut()
            .insert(node_value.clone(), content.clone());
        if let Some(old) = old {
            clear_drawn(&old);
            old.remove_class(DRAGGING_CLASS);
            old.remove_class(CURSOR_CLASS);
        }
        // A row gone for good (not drawn again) leaves the map.
        let drag = Rc::downgrade(&config.drag);
        let value = node_value.clone();
        let mine = content.node_id();
        __scope.on_cleanup(move || {
            if let Some(drag) = drag.upgrade() {
                let mut rows = drag.rows.borrow_mut();
                if rows.get(&value).is_some_and(|row| row.node_id() == mine) {
                    rows.remove(&value);
                }
            }
        });
    }
    if config.draggable {
        wire_drag(__scope, &content, &node_value, tree_state, &config);
    }
    if config.keyboard {
        if untracked(|| tree_state.cursor.get()).as_deref() == Some(node_value.as_str()) {
            content.add_class(CURSOR_CLASS);
            *config.keys.drawn.borrow_mut() = Some(node_value.clone());
            // The tree is waiting for this row to give it back the keyboard.
            if config.keys.lost_row.get() {
                let cfg = Rc::downgrade(&config);
                let _ = set_timeout(0, move || {
                    if let Some(cfg) = cfg.upgrade() {
                        restore_keyboard(&cfg, tree_state);
                    }
                });
            }
        }
        // The cursor row removed while it has the keyboard (drawn again: its
        // data changed, or it moved): the focus falling from it stays the
        // tree's, and the row as drawn now takes it back (`wire_keyboard`).
        let cfg = Rc::downgrade(&config);
        let value = node_value.clone();
        __scope.on_cleanup(move || {
            if let Some(cfg) = cfg.upgrade() {
                let is_cursor =
                    untracked(|| tree_state.cursor.get()).as_deref() == Some(value.as_str());
                if cfg.keys.focused.get() && is_cursor {
                    cfg.keys.lost_row.set(true);
                    // A browser says nothing when the focused element goes,
                    // so the tree gives the keyboard back itself.
                    let weak = Rc::downgrade(&cfg);
                    let _ = set_timeout(0, move || {
                        if let Some(cfg) = weak.upgrade() {
                            restore_keyboard(&cfg, tree_state);
                        }
                    });
                }
            }
        });
    }

    // Reactive class for content (selection only — `disabled` is not reactive)
    {
        let content_clone = content.clone();
        let nv = node_value.clone();
        // Adding and removing the one class, never rewriting the attribute
        // (issue #717): `disabled` and the base class are written above and are
        // not this effect's business, so selection owns exactly one class — and
        // a rewrite would drop whatever a parent patched onto the row, which is
        // how every #474 category C prop travels.
        __scope.create_effect(move || {
            if selected_signal.get().contains(&nv) {
                content_clone.add_class(SELECTED_CLASS);
            } else {
                content_clone.remove_class(SELECTED_CLASS);
            }
        });
    }

    // Chevron (for expandable nodes)
    if has_children {
        // The base class is written here, not by the effect below — the same
        // repair the content box above took (issue #717), and what the static
        // sibling `render_tree_node_static` has always done.
        let chevron = rinch_macros::rsx! { span { class: "rinch-tree__chevron" } };
        let icon = crate::icons::chevron_right_dom(__scope);
        chevron.append_child(&icon);

        // Reactive chevron class: adds and removes the one modifier, never
        // rewriting the attribute (issue #717).
        let chevron_clone = chevron.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            if expanded_signal.get().contains(&nv) {
                chevron_clone.add_class(CHEVRON_EXPANDED_CLASS);
            } else {
                chevron_clone.remove_class(CHEVRON_EXPANDED_CLASS);
            }
        });

        // Dedicated click handler on chevron — always toggles expand/collapse
        // so that expand_on_click can be false while the chevron still works.
        {
            let controller = tree_state.controller;
            let value = node_value.clone();
            let onexpand = config.onexpand.clone();
            let oncollapse = config.oncollapse.clone();
            let cfg = config.clone();
            let handler_id = __scope.register_handler(move || {
                let was_expanded = expanded_signal.get().contains(&value);
                controller.toggle(&value);
                if !was_expanded {
                    if let Some(ref cb) = onexpand {
                        cb.invoke(value.clone());
                    }
                } else {
                    unselect_hidden(&cfg, tree_state, &value);
                    if let Some(ref cb) = oncollapse {
                        cb.invoke(value.clone());
                    }
                }
            });
            chevron.set_attribute("data-rid", &handler_id.0.to_string());
        }

        content.append_child(&chevron);
    } else {
        let spacer = rinch_macros::rsx! { span { class: "rinch-tree__spacer" } };
        content.append_child(&spacer);
    }

    // Render node icon if present
    if let Some(ref icon) = node.icon {
        let icon_wrapper = rinch_macros::rsx! { span { class: "rinch-tree__icon" } };
        let icon_el = render_tabler_icon(__scope, *icon, TablerIconStyle::Outline);
        icon_wrapper.append_child(&icon_el);
        content.append_child(&icon_wrapper);
    }

    // Label or custom render
    if let Some(ref render_fn) = config.render_node {
        let payload = RenderTreeNodePayload {
            node,
            expanded_signal,
            selected_signal,
            node_value: &node.value,
            has_children,
            level,
        };
        let custom = render_fn(&payload, __scope);
        content.append_child(&custom);
    } else {
        let label =
            rinch_macros::rsx! { span { class: "rinch-tree__label", {node.label.clone()} } };
        content.append_child(&label);
    }

    // Click handler
    if !node.disabled {
        let value = node.value.clone();
        let cfg = config.clone();
        let handler_id = __scope.register_handler(move || {
            if cfg.keyboard {
                tree_state.cursor.set(Some(value.clone()));
                set_focused(&cfg, true);
            }
            let modifiers = get_click_context().modifiers;
            let pick = if modifiers.ctrl || modifiers.meta {
                Pick::Toggle
            } else if modifiers.shift {
                Pick::Run
            } else {
                Pick::Replace
            };
            activate_row(&cfg, tree_state, &value, has_children, pick);
        });
        content.set_attribute("data-rid", &handler_id.0.to_string());
    }

    li.append_child(&content);

    // Children — always render, toggle visibility with display:none
    if has_children {
        let children_ul = rinch_macros::rsx! { ul { class: "rinch-tree__subtree" } };
        children_ul.set_attribute("role", "group");
        // The group is the treeitem's, though it sits beside it in the DOM.
        let group_id = next_group_id();
        children_ul.set_attribute("id", &group_id);
        content.set_attribute("aria-owns", &group_id);

        // With guides the children sit in a box with their guide beside
        // them, and the box is what shows and hides.
        let shown = if config.guides {
            let wrap = rinch_macros::rsx! { div { class: "rinch-tree__subtree-wrap" } };
            let guide = rinch_macros::rsx! { div { class: "rinch-tree__guide" } };
            guide.set_attribute(
                "style",
                &format!(
                    "left: calc(var(--tree-level-offset) * {level} + var(--tree-guide-offset))"
                ),
            );
            wrap.append_child(&guide);
            wrap.append_child(&children_ul);
            wrap
        } else {
            children_ul.clone()
        };

        // Reactive display toggle — preserves all child DOM and Effects
        let children_ul_vis = shown.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            if expanded_signal.get().contains(&nv) {
                children_ul_vis.set_style("display", "");
            } else {
                children_ul_vis.set_style("display", "none");
            }
        });

        // The children, keyed: this row's own list (`TreeConfig::rows`), read
        // again whenever the data is, so a child added, removed, moved or
        // changed is reconciled here and the rows above are left alone.
        let first: Vec<Row> = node.children.iter().cloned().map(Row).collect();
        let child_level = level + 1;
        let cfg = config.clone();
        let list_cfg = Rc::downgrade(&config);
        let version = config.rows_version;
        let nv = node_value.clone();
        let first = RefCell::new(Some(first));

        for_each_dom_typed(
            __scope,
            &children_ul,
            move || {
                let _ = version.get();
                // The first run draws what this row was drawn with; the index
                // may hold the same, or (a row drawn by a list below the root)
                // a newer read.
                if let Some(rows) = first.borrow_mut().take() {
                    return rows;
                }
                list_cfg
                    .upgrade()
                    .and_then(|cfg| cfg.rows.borrow().get(&nv).cloned())
                    .unwrap_or_default()
            },
            |row: &Row| row.0.value.clone(),
            move |row: Row, __scope: &mut RenderScope| {
                render_tree_node(__scope, &row.0, child_level, tree_state, cfg.clone())
            },
        );

        li.append_child(&shown);
    }

    li
}

/// Render a tree node without reactive state (fallback when no tree state provided).
#[allow(clippy::only_used_in_recursion)]
pub(super) fn render_tree_node_static(
    __scope: &mut RenderScope,
    node: &TreeNodeData,
    level: usize,
    config: &Rc<TreeConfig>,
) -> NodeHandle {
    let has_children = node.has_children();

    let li = rinch_macros::rsx! { li { class: "rinch-tree__node" } };
    li.set_attribute("role", "none");
    li.set_attribute("data-value", &node.value);

    let content = rinch_macros::rsx! { div {} };
    content.set_attribute("role", "treeitem");
    content.set_attribute("aria-level", &(level + 1).to_string());
    if node.disabled {
        content.set_attribute("aria-disabled", "true");
    }
    let base_class = if node.disabled {
        "rinch-tree__node-content rinch-tree__node-content--disabled"
    } else {
        "rinch-tree__node-content"
    };
    content.set_attribute("class", base_class);
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );
    content.set_attribute("tabindex", "0");

    if has_children {
        let chevron = rinch_macros::rsx! { span { class: "rinch-tree__chevron" } };
        let icon = crate::icons::chevron_right_dom(__scope);
        chevron.append_child(&icon);
        content.append_child(&chevron);
    } else {
        let spacer = rinch_macros::rsx! { span { class: "rinch-tree__spacer" } };
        content.append_child(&spacer);
    }

    if let Some(ref icon) = node.icon {
        let icon_wrapper = rinch_macros::rsx! { span { class: "rinch-tree__icon" } };
        let icon_el = render_tabler_icon(__scope, *icon, TablerIconStyle::Outline);
        icon_wrapper.append_child(&icon_el);
        content.append_child(&icon_wrapper);
    }

    let label = rinch_macros::rsx! { span { class: "rinch-tree__label", {node.label.clone()} } };
    content.append_child(&label);

    li.append_child(&content);

    if has_children {
        let children_ul = rinch_macros::rsx! { ul { class: "rinch-tree__subtree" } };
        children_ul.set_attribute("role", "group");
        let group_id = next_group_id();
        children_ul.set_attribute("id", &group_id);
        content.set_attribute("aria-owns", &group_id);

        for child in &node.children {
            let child_handle = render_tree_node_static(__scope, child, level + 1, config);
            children_ul.append_child(&child_handle);
        }

        li.append_child(&children_ul);
    }

    li
}
