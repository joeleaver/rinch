//! In-app menu bar for Linux.
//!
//! On Linux, native menu bars don't work (muda needs a GTK window, winit uses
//! raw X11/Wayland). This module renders a DOM-based menu bar using the same
//! `Menu`/`MenuItem` API. Hover-to-switch uses `data-onenter` handlers
//! dispatched by the event loop when the hovered node changes.

use super::{Menu, MenuEntryRef};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::{Effect, Signal};
use std::rc::Rc;

/// Menu bar height in pixels (labels + padding).
pub(crate) const MENU_BAR_HEIGHT: u32 = 28;

/// Wrap `content` with an in-app menu bar rendered from the given menu data.
///
/// `top_offset` is the vertical offset (in px) for the menu bar. For borderless
/// windows with a custom title bar, pass the title bar height (typically 36) so
/// the menu bar appears below it. For regular windows, pass 0.
pub(crate) fn render_with_menu_bar(
    scope: &mut RenderScope,
    menus: &[(&str, &Menu)],
    content: NodeHandle,
    top_offset: u32,
) -> NodeHandle {
    // -1 = no menu open, 0..N = index of the open menu
    let active_menu: Signal<i32> = Signal::new(-1);

    // Outer wrapper filling the viewport.
    let wrapper = scope.create_element("div");
    wrapper.set_attribute("class", "rinch-app-menu-bar-wrapper");

    // DOM order matters for hit testing (last child = topmost).
    // We need: content (bottom) → overlay (middle) → bar+dropdowns (top).

    // Content fills remaining space (DOM first = hit-tested last).
    // padding-top clears space for both the title bar and menu bar.
    let content_wrapper = scope.create_element("div");
    content_wrapper.set_attribute(
        "style",
        &format!(
            "height: 100%; padding-top: {}px; overflow: auto;",
            top_offset + MENU_BAR_HEIGHT
        ),
    );
    content_wrapper.append_child(&content);
    wrapper.append_child(&content_wrapper);

    // Click-outside overlay (DOM middle)
    {
        let overlay = scope.create_element("div");
        // Reactive: show/hide based on active_menu
        {
            let overlay_handle = overlay.clone();
            Effect::new(move || {
                if active_menu.get() >= 0 {
                    overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                    overlay_handle.set_attribute("style", "");
                } else {
                    overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                    overlay_handle.set_attribute("style", "display: none;");
                }
            });
        }
        // Click overlay → close all menus
        let handler_id = scope.register_handler(move || {
            active_menu.set(-1);
        });
        overlay.set_attribute("data-rid", &handler_id.0.to_string());
        wrapper.append_child(&overlay);
    }

    // Menu bar row (DOM last = hit-tested first, visually positioned via absolute)
    let bar = scope.create_element("div");
    bar.set_attribute(
        "style",
        &format!(
            "position: absolute; top: {}px; left: 0; width: 100%; z-index: 201;",
            top_offset
        ),
    );
    // Reactive class: add --engaged when any menu is open
    {
        let bar_handle = bar.clone();
        Effect::new(move || {
            let cls = if active_menu.get() >= 0 {
                "rinch-app-menu-bar rinch-app-menu-bar--engaged"
            } else {
                "rinch-app-menu-bar"
            };
            bar_handle.set_attribute("class", cls);
        });
    }

    // Build each top-level menu item
    for (idx, &(label, menu)) in menus.iter().enumerate() {
        let item_node = build_top_level_item(scope, label, menu, idx as i32, active_menu);
        bar.append_child(&item_node);
    }

    wrapper.append_child(&bar);

    wrapper
}

/// Render the menu bar + click-outside overlay as a standalone DOM piece.
///
/// Unlike `render_with_menu_bar()`, this does NOT wrap content. It returns a
/// container with absolute positioning meant to be the LAST child of its parent
/// (for correct hit-test ordering: last child = tested first). The parent must
/// have `position: relative` and the content area needs `padding-top` equal to
/// `MENU_BAR_HEIGHT` to leave space for the bar.
///
/// `top_offset` is the vertical position for the bar (e.g., 36 for titlebar height).
pub(crate) fn render_menu_bar_standalone(
    scope: &mut RenderScope,
    menus: &[(&str, &Menu)],
    top_offset: u32,
) -> NodeHandle {
    let active_menu: Signal<i32> = Signal::new(-1);

    // Container uses absolute positioning so it doesn't participate in flex layout.
    // Must be the LAST child of its parent for correct hit testing (DOM order).
    // DOM order within: overlay (first, hit-tested last) → bar (last, hit-tested first).
    let container = scope.create_element("div");
    container.set_attribute(
        "style",
        &format!(
            "position: absolute; top: {}px; left: 0; width: 100%; z-index: 200;",
            top_offset
        ),
    );

    // Click-outside overlay (fixed, covers entire viewport)
    {
        let overlay = scope.create_element("div");
        {
            let overlay_handle = overlay.clone();
            Effect::new(move || {
                if active_menu.get() >= 0 {
                    overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                    overlay_handle.set_attribute("style", "");
                } else {
                    overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                    overlay_handle.set_attribute("style", "display: none;");
                }
            });
        }
        let handler_id = scope.register_handler(move || {
            active_menu.set(-1);
        });
        overlay.set_attribute("data-rid", &handler_id.0.to_string());
        container.append_child(&overlay);
    }

    // Menu bar (DOM last within container = hit-tested first)
    let bar = scope.create_element("div");
    bar.set_attribute("style", "position: relative; z-index: 201;");
    {
        let bar_handle = bar.clone();
        Effect::new(move || {
            let cls = if active_menu.get() >= 0 {
                "rinch-app-menu-bar rinch-app-menu-bar--engaged"
            } else {
                "rinch-app-menu-bar"
            };
            bar_handle.set_attribute("class", cls);
        });
    }

    for (idx, &(label, menu)) in menus.iter().enumerate() {
        let item_node = build_top_level_item(scope, label, menu, idx as i32, active_menu);
        bar.append_child(&item_node);
    }

    container.append_child(&bar);
    container
}

/// Render just the menu items as a flat row (for inline titlebar layout).
///
/// Returns a flex container with the top-level menu items. Uses a shared
/// `active_menu` signal so the overlay and items stay in sync.
pub(crate) fn render_menu_items_inline(
    scope: &mut RenderScope,
    menus: &[(&str, &Menu)],
    active_menu: Signal<i32>,
) -> NodeHandle {
    let row = scope.create_element("div");
    // Reactive class: add --engaged when any menu is open
    {
        let row_handle = row.clone();
        Effect::new(move || {
            let cls = if active_menu.get() >= 0 {
                "rinch-app-menu-bar__inline-items rinch-app-menu-bar--engaged"
            } else {
                "rinch-app-menu-bar__inline-items"
            };
            row_handle.set_attribute("class", cls);
        });
    }

    for (idx, &(label, menu)) in menus.iter().enumerate() {
        let item_node = build_top_level_item(scope, label, menu, idx as i32, active_menu);
        row.append_child(&item_node);
    }

    row
}

/// Render just the click-outside overlay (for inline titlebar layout).
///
/// Uses a shared `active_menu` signal so clicking the overlay closes menus.
pub(crate) fn render_inline_overlay(
    scope: &mut RenderScope,
    active_menu: Signal<i32>,
) -> NodeHandle {
    let overlay = scope.create_element("div");
    {
        let overlay_handle = overlay.clone();
        Effect::new(move || {
            if active_menu.get() >= 0 {
                overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                overlay_handle.set_attribute("style", "");
            } else {
                overlay_handle.set_attribute("class", "rinch-app-menu-bar__overlay");
                overlay_handle.set_attribute("style", "display: none;");
            }
        });
    }
    let handler_id = scope.register_handler(move || {
        active_menu.set(-1);
    });
    overlay.set_attribute("data-rid", &handler_id.0.to_string());
    overlay
}

/// Build a single top-level menu item (label + dropdown).
fn build_top_level_item(
    scope: &mut RenderScope,
    label: &str,
    menu: &Menu,
    index: i32,
    active_menu: Signal<i32>,
) -> NodeHandle {
    let item = scope.create_element("div");

    // Reactive class: add --opened when this menu is active
    {
        let item_handle = item.clone();
        Effect::new(move || {
            let cls = if active_menu.get() == index {
                "rinch-app-menu-item rinch-app-menu-item--opened"
            } else {
                "rinch-app-menu-item"
            };
            item_handle.set_attribute("class", cls);
        });
    }

    // Hover-to-switch: when engaged (any menu open), entering this item switches to it
    let onenter_id = scope.register_handler(move || {
        if active_menu.get() >= 0 {
            active_menu.set(index);
        }
    });
    item.set_attribute("data-onenter", &onenter_id.0.to_string());

    // Label (clickable)
    let label_node = scope.create_element("div");
    label_node.set_attribute("class", "rinch-app-menu-item__label");
    let text = scope.create_text(label);
    label_node.append_child(&text);

    // Click handler: toggle this menu
    let handler_id = scope.register_handler(move || {
        let current = active_menu.get();
        if current == index {
            active_menu.set(-1);
        } else {
            active_menu.set(index);
        }
    });
    label_node.set_attribute("data-rid", &handler_id.0.to_string());

    item.append_child(&label_node);

    // Dropdown panel — visibility controlled by the active_menu signal.
    // The data-onenter handler on each item switches active_menu on hover.
    let dropdown = scope.create_element("div");
    {
        let dropdown_handle = dropdown.clone();
        Effect::new(move || {
            if active_menu.get() == index {
                dropdown_handle.set_attribute(
                    "class",
                    "rinch-app-menu-item__dropdown rinch-app-menu-item__dropdown--visible",
                );
            } else {
                dropdown_handle.set_attribute("class", "rinch-app-menu-item__dropdown");
            }
        });
    }

    // Inner scroll wrapper — avoids overflow bugs on abs-pos dropdowns
    let scroll = scope.create_element("div");
    scroll.set_attribute("class", "rinch-app-menu-dropdown__scroll");
    build_menu_entries(scope, &scroll, menu, active_menu);
    dropdown.append_child(&scroll);
    item.append_child(&dropdown);

    item
}

/// Recursively build menu entries inside a dropdown container.
fn build_menu_entries(
    scope: &mut RenderScope,
    container: &NodeHandle,
    menu: &Menu,
    active_menu: Signal<i32>,
) {
    for entry in menu.iter_entries() {
        match entry {
            MenuEntryRef::Item {
                label,
                shortcut,
                enabled,
                callback,
            } => {
                let entry_node = scope.create_element("div");
                let mut cls = "rinch-app-menu-entry".to_string();
                if !enabled {
                    cls.push_str(" rinch-app-menu-entry--disabled");
                }
                entry_node.set_attribute("class", &cls);

                // Label
                let label_span = scope.create_element("span");
                label_span.set_attribute("class", "rinch-app-menu-entry__label");
                let text = scope.create_text(label);
                label_span.append_child(&text);
                entry_node.append_child(&label_span);

                // Shortcut hint
                if let Some(shortcut) = shortcut {
                    let shortcut_span = scope.create_element("span");
                    shortcut_span.set_attribute("class", "rinch-app-menu-entry__shortcut");
                    let shortcut_text = scope.create_text(shortcut);
                    shortcut_span.append_child(&shortcut_text);
                    entry_node.append_child(&shortcut_span);
                }

                // Click handler: invoke callback and close menu
                if enabled {
                    if let Some(cb) = callback {
                        let cb = Rc::clone(cb);
                        let handler_id = scope.register_handler(move || {
                            active_menu.set(-1);
                            cb();
                        });
                        entry_node.set_attribute("data-rid", &handler_id.0.to_string());
                    }
                }

                container.append_child(&entry_node);
            }
            MenuEntryRef::Separator => {
                let sep = scope.create_element("div");
                sep.set_attribute("class", "rinch-app-menu-separator");
                container.append_child(&sep);
            }
            MenuEntryRef::Submenu { label, menu } => {
                let submenu_node = scope.create_element("div");
                submenu_node.set_attribute("class", "rinch-app-menu-submenu");

                // Trigger row (label + arrow)
                let trigger = scope.create_element("div");
                trigger.set_attribute("class", "rinch-app-menu-submenu__trigger");

                let label_span = scope.create_element("span");
                label_span.set_attribute("class", "rinch-app-menu-submenu__label");
                let text = scope.create_text(label);
                label_span.append_child(&text);
                trigger.append_child(&label_span);

                let arrow = scope.create_element("span");
                arrow.set_attribute("class", "rinch-app-menu-submenu__arrow");
                let arrow_text = scope.create_text("\u{203A}"); // › character
                arrow.append_child(&arrow_text);
                trigger.append_child(&arrow);

                submenu_node.append_child(&trigger);

                // Nested dropdown with inner scroll wrapper
                let nested_dropdown = scope.create_element("div");
                nested_dropdown.set_attribute("class", "rinch-app-menu-submenu__dropdown");
                let nested_scroll = scope.create_element("div");
                nested_scroll.set_attribute("class", "rinch-app-menu-dropdown__scroll");
                build_menu_entries(scope, &nested_scroll, menu, active_menu);
                nested_dropdown.append_child(&nested_scroll);
                submenu_node.append_child(&nested_dropdown);

                container.append_child(&submenu_node);
            }
        }
    }
}
