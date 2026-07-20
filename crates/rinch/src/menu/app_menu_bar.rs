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
    // Publish how much window chrome sits above the content. Flow content
    // clears it via the padding-top below, but a `position: fixed` overlay
    // resolves against the real viewport (as it does in a browser, and as it
    // does on rinch-web), so it has no way to know. Custom properties inherit,
    // so any fixed descendant can opt in with
    // `top: var(--rinch-window-top-inset, 0px)`.
    wrapper.set_attribute(
        "style",
        &format!(
            "--rinch-window-top-inset: {}px;",
            top_offset + MENU_BAR_HEIGHT
        ),
    );
    // Also publish it on `body`: overlays are commonly rendered as a root-level
    // sibling of this wrapper rather than inside it, and custom properties
    // inherit down the DOM, not across it.
    scope.body_handle().set_style(
        "--rinch-window-top-inset",
        &format!("{}px", top_offset + MENU_BAR_HEIGHT),
    );

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

/// Approximate height of a menu entry for flyout positioning.
const ENTRY_HEIGHT: f32 = 30.0;
/// Approximate height of a separator.
const SEPARATOR_HEIGHT: f32 = 9.0;
/// Approximate height of the menu label row (the dropdown starts below this).
const LABEL_HEIGHT: f32 = 30.0;

/// Build a single top-level menu item (label + dropdown + flyouts).
///
/// Flyouts are rendered as siblings of the dropdown (children of the menu-item)
/// so they are NOT clipped by the dropdown's overflow-y. Flyout visibility is
/// controlled by an `active_flyout` signal set via `data-onenter` on triggers.
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

    // -1 = no flyout, 0..N = index of the visible flyout
    let active_flyout: Signal<i32> = Signal::new(-1);
    // Close flyouts when this menu closes
    {
        Effect::new(move || {
            if active_menu.get() != index {
                active_flyout.set(-1);
            }
        });
    }

    // Dropdown panel — visibility controlled by the active_menu signal.
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

    // Build entries into the dropdown and collect flyout data.
    let mut flyouts: Vec<FlyoutData> = Vec::new();
    build_menu_entries_with_flyouts(
        scope,
        &dropdown,
        menu,
        active_menu,
        active_flyout,
        &mut flyouts,
    );

    item.append_child(&dropdown);

    // Render flyouts as siblings of the dropdown (children of menu-item).
    // They escape the dropdown's overflow clip because they're outside it.
    // Position: right of dropdown, vertically aligned with the trigger row.
    // The dropdown starts at top: 100% of menu-item (≈ LABEL_HEIGHT).
    for (flyout_idx, flyout_data) in flyouts.into_iter().enumerate() {
        let flyout_idx = flyout_idx as i32;
        let flyout = scope.create_element("div");
        flyout.set_attribute("class", "rinch-app-menu-submenu__flyout");

        let top_px = LABEL_HEIGHT + flyout_data.trigger_y;
        let pos_style = format!("left: 220px; top: {top_px}px;");

        // Reactive visibility: show when this flyout is active AND menu is open
        {
            let flyout_handle = flyout.clone();
            let pos = pos_style.clone();
            Effect::new(move || {
                if active_flyout.get() == flyout_idx && active_menu.get() == index {
                    flyout_handle.set_attribute("style", &format!("{pos} display: block;"));
                } else {
                    flyout_handle.set_attribute("style", &format!("{pos} display: none;"));
                }
            });
        }

        // Build the flyout's menu entries
        build_menu_entries_with_flyouts(
            scope,
            &flyout,
            &flyout_data.menu_snapshot,
            active_menu,
            active_flyout,
            &mut Vec::new(),
        );

        item.append_child(&flyout);
    }

    item
}

/// Data collected for each submenu that needs a flyout.
struct FlyoutData {
    /// Vertical offset of the trigger within the dropdown (px).
    trigger_y: f32,
    /// Snapshot of the submenu's Menu for rendering.
    menu_snapshot: Menu,
}

/// Build menu entries into a container, collecting flyout data for submenus.
///
/// Submenu triggers go into the container. Flyout panels are NOT created here —
/// the caller renders them as siblings of the container so they escape overflow.
fn build_menu_entries_with_flyouts(
    scope: &mut RenderScope,
    container: &NodeHandle,
    menu: &Menu,
    active_menu: Signal<i32>,
    active_flyout: Signal<i32>,
    flyouts: &mut Vec<FlyoutData>,
) {
    let mut y_offset: f32 = 4.0; // dropdown padding-top

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

                let label_span = scope.create_element("span");
                label_span.set_attribute("class", "rinch-app-menu-entry__label");
                let text = scope.create_text(label);
                label_span.append_child(&text);
                entry_node.append_child(&label_span);

                if let Some(shortcut) = shortcut {
                    let shortcut_span = scope.create_element("span");
                    shortcut_span.set_attribute("class", "rinch-app-menu-entry__shortcut");
                    let shortcut_text = scope.create_text(shortcut);
                    shortcut_span.append_child(&shortcut_text);
                    entry_node.append_child(&shortcut_span);
                }

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
                y_offset += ENTRY_HEIGHT;
            }
            MenuEntryRef::Separator => {
                let sep = scope.create_element("div");
                sep.set_attribute("class", "rinch-app-menu-separator");
                container.append_child(&sep);
                y_offset += SEPARATOR_HEIGHT;
            }
            MenuEntryRef::Submenu { label, menu } => {
                // Record flyout data — the flyout itself is rendered by the caller
                let flyout_idx = flyouts.len() as i32;
                flyouts.push(FlyoutData {
                    trigger_y: y_offset,
                    menu_snapshot: menu.clone(),
                });

                // Build just the trigger row (no nested dropdown)
                let trigger = scope.create_element("div");
                trigger.set_attribute("class", "rinch-app-menu-submenu__trigger");

                let label_span = scope.create_element("span");
                label_span.set_attribute("class", "rinch-app-menu-submenu__label");
                let text = scope.create_text(label);
                label_span.append_child(&text);
                trigger.append_child(&label_span);

                let arrow = scope.create_element("span");
                arrow.set_attribute("class", "rinch-app-menu-submenu__arrow");
                let arrow_text = scope.create_text("\u{203A}");
                arrow.append_child(&arrow_text);
                trigger.append_child(&arrow);

                // Hover shows the flyout
                let enter_id = scope.register_handler(move || {
                    active_flyout.set(flyout_idx);
                });
                trigger.set_attribute("data-onenter", &enter_id.0.to_string());

                container.append_child(&trigger);
                y_offset += ENTRY_HEIGHT;
            }
        }
    }
}

#[cfg(all(test, feature = "components", feature = "theme"))]
mod tests {
    use super::MENU_BAR_HEIGHT;
    use rinch_core::dom::DomDocument;
    use rinch_dom::RinchDocument;

    /// The bar reserves `MENU_BAR_HEIGHT` of space with `padding-top`, but its
    /// own height is intrinsic — so if the stylesheet renders it any taller,
    /// content silently sits underneath it. It used to: the label inherited
    /// `line-height` from `body` (1.55) and the bar came out ~31px against 28px
    /// of reserved space. The label now pins `line-height`, making the two
    /// agree by construction. This asserts they still do.
    #[test]
    fn menu_bar_renders_exactly_menu_bar_height() {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        // Theme CSS first: the bar's `border-bottom` colour comes from
        // `var(--rinch-color-border, var(--rinch-color-gray-3))`, and with
        // neither defined the whole declaration is invalid at computed-value
        // time — the border drops to 0 and the bar measures 27px. `run()`
        // always loads the theme, so the test mirrors that.
        let theme_style = doc.create_element("style");
        let theme_css = doc.create_text(&rinch_theme::css::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        doc.append_child(theme_style, theme_css);
        doc.append_child(body, theme_style);

        let style = doc.create_element("style");
        let css = doc.create_text(&rinch_components::styles::generate_all_component_styles());
        doc.append_child(style, css);
        doc.append_child(body, style);

        let bar = doc.create_element("div");
        doc.set_attribute(bar, "class", "rinch-app-menu-bar");
        doc.append_child(body, bar);

        let item = doc.create_element("div");
        doc.set_attribute(item, "class", "rinch-app-menu-item");
        doc.append_child(bar, item);

        let label = doc.create_element("div");
        doc.set_attribute(label, "class", "rinch-app-menu-item__label");
        doc.append_child(item, label);
        let text = doc.create_text("File");
        doc.append_child(label, text);

        doc.resolve_layout(800.0, 600.0);

        let height = doc.tree.get(bar.0).unwrap().layout.height;
        assert_eq!(
            height, MENU_BAR_HEIGHT as f32,
            "the rendered menu bar must match the space reserved for it \
             (MENU_BAR_HEIGHT); content sits under the bar otherwise"
        );
    }
}
