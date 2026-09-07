//! In-app menu bar for Linux.
//!
//! On Linux, native menu bars don't work (muda needs a GTK window, winit uses
//! raw X11/Wayland). This module renders a DOM-based menu bar using the same
//! `Menu`/`MenuItem` API. Hover-to-switch uses `data-onenter` handlers
//! dispatched by the event loop when the hovered node changes.

// `MenuEntryInner`, not the public `MenuEntryRef`: this module is a descendant
// of `menu`, so it can read the private fields — and it needs
// `MenuItem::callback_owner`, which the public read-only view deliberately does
// not carry, to gate invocation the way `dispatch_menu_event` does.
use super::{Menu, MenuEntryInner};
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

    // Paint order here is z-index, not DOM order: content sits at the bottom,
    // the dismiss overlay above it at `z-index: 199`, and the bar with its
    // dropdowns on top at `201`. That only works because all three are boxes of
    // the *same* stacking context — see [`build_overlay`].

    // Content fills remaining space.
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

    // Click-outside overlay. The wrapper is the viewport's top-left, so the
    // overlay needs no offset of its own.
    let overlay = build_overlay(scope, active_menu, "");
    wrapper.append_child(&overlay);

    // Menu bar row, above the overlay by its `z-index` (DOM order only breaks
    // ties at one z level) and positioned by `absolute`.
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
/// Unlike `render_with_menu_bar()`, this does NOT wrap content. It returns an
/// absolutely positioned container carrying its own `z-index`, so it paints
/// above its siblings whatever its position among them; append it last anyway,
/// to keep the markup reading in paint order. The parent must have
/// `position: relative` and the content area needs `padding-top` equal to
/// `MENU_BAR_HEIGHT` to leave space for the bar.
///
/// `top_offset` is the vertical position for the bar (e.g., 36 for titlebar height).
pub(crate) fn render_menu_bar_standalone(
    scope: &mut RenderScope,
    menus: &[(&str, &Menu)],
    top_offset: u32,
) -> NodeHandle {
    let active_menu: Signal<i32> = Signal::new(-1);

    // Container uses absolute positioning so it doesn't participate in flex
    // layout. Within it the overlay (`z-index: 199`) sits below the bar
    // (`201`); both are boxes of this container's own stacking context, which
    // is what makes those two numbers comparable — see [`build_overlay`].
    let container = scope.create_element("div");
    container.set_attribute(
        "style",
        &format!(
            "position: absolute; top: {}px; left: 0; width: 100%; z-index: 200;",
            top_offset
        ),
    );

    // Click-outside overlay. This container is the overlay's containing block
    // and sits `top_offset` down the window, so the overlay climbs back up by
    // the same amount: `top: -top_offset` puts its top edge at the window's,
    // and the stylesheet's `100vh` then reaches exactly the window's bottom.
    let overlay = build_overlay(scope, active_menu, &format!("top: -{top_offset}px;"));
    container.append_child(&overlay);

    // Menu bar, above the overlay by its `z-index`.
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
    // The inline layer is pinned to the window's top-left, so no offset.
    build_overlay(scope, active_menu, "")
}

/// The click-outside overlay each menu-bar layout puts *under* its menus.
///
/// `base_style` is prepended to whatever the visibility effect writes, for the
/// layout whose overlay is not already anchored at the window's top-left.
///
/// # Why this is `position: absolute` and not `fixed`
///
/// `z-index` orders boxes within one stacking context and says nothing across
/// two, and a `position: fixed` box is hoisted out of every ancestor context to
/// the viewport's. This overlay is authored to sit *below* the menu row
/// (`z-index: 199` against the row's `201`), so the two have to be siblings in
/// the same context or the numbers are not compared at all.
///
/// Fixed used to be close enough, because the window container was not a
/// stacking context and both boxes surfaced at the viewport. `BorderlessWindow`
/// gives that container `overflow: hidden`, which in Rinch *does* form a
/// stacking context ([`rinch_dom::node::Node::creates_stacking_context`]) — so
/// the menus stayed trapped inside it at the container's own `z == 0` while the
/// overlay escaped to the viewport at `199` and covered them. Every entry click
/// hit the overlay and merely dismissed the menu (#527).
///
/// Keeping the overlay in the menus' own stacking context makes the ordering
/// hold under either rule.
///
/// **This is a workaround, not the fix.** That `overflow` forms a stacking
/// context at all is issue #324 — CSS says it does not, and rinch does it only
/// because its overflow clip is applied per stacking context. `DropdownMenu`
/// and `Select` had the identical bug and took the identical `absolute`
/// workaround (PR #317); this is the second site. The cost here is
/// `render_menu_bar_standalone`'s `top: -top_offset`, which exists purely to
/// undo the containing-block change this workaround forces. When #324 lands,
/// all three overlays should go back to `position: fixed` and that
/// compensation should be deleted — the tests below cover all three layouts
/// and should stay green across that revert.
fn build_overlay(
    scope: &mut RenderScope,
    active_menu: Signal<i32>,
    base_style: &str,
) -> NodeHandle {
    let overlay = scope.create_element("div");
    overlay.set_attribute("class", "rinch-app-menu-bar__overlay");
    {
        let overlay_handle = overlay.clone();
        let shown = base_style.to_string();
        let hidden = format!("{base_style}display: none;");
        Effect::new(move || {
            let style = if active_menu.get() >= 0 {
                &shown
            } else {
                &hidden
            };
            overlay_handle.set_attribute("style", style);
        });
    }
    // Click overlay → close all menus
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

    for entry in &menu.entries {
        match entry {
            MenuEntryInner::Item(item) => {
                let enabled = item.enabled;
                let entry_node = scope.create_element("div");
                let mut cls = "rinch-app-menu-entry".to_string();
                if !enabled {
                    cls.push_str(" rinch-app-menu-entry--disabled");
                }
                entry_node.set_attribute("class", &cls);

                let label_span = scope.create_element("span");
                label_span.set_attribute("class", "rinch-app-menu-entry__label");
                let text = scope.create_text(&item.label);
                label_span.append_child(&text);
                entry_node.append_child(&label_span);

                if let Some(shortcut) = item.shortcut.as_deref() {
                    let shortcut_span = scope.create_element("span");
                    shortcut_span.set_attribute("class", "rinch-app-menu-entry__shortcut");
                    let shortcut_text = scope.create_text(shortcut);
                    shortcut_span.append_child(&shortcut_text);
                    entry_node.append_child(&shortcut_span);
                }

                if enabled {
                    if let Some(cb) = item.callback.as_ref() {
                        let cb = Rc::clone(cb);
                        // The item is invoked straight from the `Menu` here — it
                        // never goes through the callback registry — so this
                        // path has to apply the #183 lifetime rule itself, or a
                        // click would run a callback whose component has
                        // unmounted and read its freed signals.
                        let owner = item.callback_owner.clone();
                        let handler_id = scope.register_handler(move || {
                            active_menu.set(-1);
                            super::invoke_menu_callback(&cb, owner.as_ref());
                        });
                        entry_node.set_attribute("data-rid", &handler_id.0.to_string());
                    }
                }

                container.append_child(&entry_node);
                y_offset += ENTRY_HEIGHT;
            }
            MenuEntryInner::Separator => {
                let sep = scope.create_element("div");
                sep.set_attribute("class", "rinch-app-menu-separator");
                container.append_child(&sep);
                y_offset += SEPARATOR_HEIGHT;
            }
            MenuEntryInner::Submenu { label, menu } => {
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

                // …and so does a click. Hover normally gets there first, so
                // this is a backstop — but without it a click on the trigger
                // finds no `data-rid` on the trigger or on any ancestor of it,
                // and so runs nothing at all. (Before #527 was fixed it was
                // worse than nothing: the dismiss overlay won the hit test
                // outright and closed the whole menu. That was the overlay
                // being above the trigger in paint order, not the click
                // "falling through" to it — the dispatcher's walk is up the
                // ancestor chain, and it never leaves the menu.)
                //
                // Carrying a `data-rid` also makes the trigger match every
                // other element in the bar for focus: the walk in
                // `click_handling.rs` that chooses `ClickFocus::PreserveEditor`
                // over `Blur` stops at any `data-rid`, so clicking a submenu
                // trigger no longer blurs a focused rich-text editor the way it
                // used to, while labels, entries and the overlay always
                // preserved it.
                let click_id = scope.register_handler(move || {
                    active_flyout.set(flyout_idx);
                });
                trigger.set_attribute("data-rid", &click_id.0.to_string());

                container.append_child(&trigger);
                y_offset += ENTRY_HEIGHT;
            }
        }
    }
}

#[cfg(all(test, feature = "components", feature = "theme"))]
mod tests {
    use super::{MENU_BAR_HEIGHT, render_inline_overlay, render_menu_items_inline};
    use crate::menu::{Menu, MenuItem};
    use rinch_core::dom::{DomDocument, RenderScope};
    use rinch_core::reactive::Signal;
    use rinch_dom::RinchDocument;
    use std::cell::RefCell;
    use std::rc::Rc;

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

    // ── #527: a click inside an open menu must reach the menu ────────────
    //
    // These fixtures are the two structures `BorderlessWindow` renders on Linux,
    // built from the *real* renderers and the *real* stylesheet — the defect was
    // entirely in how those two interact with the window container's own
    // `overflow: hidden`, so a hand-written approximation of either would have
    // been free to be wrong in exactly the way that hid it.

    const VIEWPORT_W: f32 = 1200.0;
    const VIEWPORT_H: f32 = 800.0;
    /// `BorderlessWindow`'s title bar height, as `app_builder` passes it.
    const TITLEBAR_HEIGHT: u32 = 36;

    /// Theme CSS then component CSS, as `run()` loads them. The bar's own rules
    /// read theme variables, so the order matters.
    fn load_stylesheets(doc: &Rc<RefCell<RinchDocument>>, body: rinch_core::dom::NodeId) {
        for css in [
            rinch_theme::css::generate_theme_css(&rinch_theme::Theme::default()),
            rinch_components::styles::generate_all_component_styles(),
        ] {
            let mut d = doc.borrow_mut();
            let style = d.create_element("style");
            let text = d.create_text(&css);
            d.append_child(style, text);
            d.append_child(body, style);
        }
    }

    /// Every node carrying `class` exactly, in tree order.
    fn nodes_with_class(doc: &RinchDocument, class: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![doc.tree.body_id];
        while let Some(id) = stack.pop() {
            let Some(node) = doc.tree.get(id) else {
                continue;
            };
            if node
                .attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|c| c == class))
            {
                out.push(id);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }

    fn rid_of(doc: &RinchDocument, node_id: usize) -> String {
        doc.tree
            .get(node_id)
            .and_then(|n| n.attributes.get("data-rid"))
            .cloned()
            .unwrap_or_else(|| panic!("node {node_id} carries no data-rid"))
    }

    /// The `data-rid` handler a click at `(x, y)` would run: hit-test, then walk
    /// up to the nearest ancestor carrying one — what
    /// `RinchApp::handle_click_with_button` does.
    fn rid_under(doc: &RinchDocument, x: f32, y: f32) -> Option<String> {
        let mut id = crate::app::hit_testing::hit_test(&doc.tree, x, y)?;
        loop {
            let node = doc.tree.get(id)?;
            if let Some(rid) = node.attributes.get("data-rid") {
                return Some(rid.clone());
            }
            id = node.parent?;
        }
    }

    /// Run a node's own `data-rid` handler, as the click dispatcher would.
    fn run_rid_handler(doc: &Rc<RefCell<RinchDocument>>, node_id: usize) {
        let rid: usize = rid_of(&doc.borrow(), node_id).parse().unwrap();
        rinch_core::events::dispatch_event(rinch_core::events::EventHandlerId(rid));
    }

    /// Run a node's `data-onenter` handler, as the hover dispatcher would.
    fn run_onenter_handler(doc: &Rc<RefCell<RinchDocument>>, node_id: usize) {
        let id: usize = doc
            .borrow()
            .tree
            .get(node_id)
            .and_then(|n| n.attributes.get("data-onenter"))
            .unwrap_or_else(|| panic!("node {node_id} carries no data-onenter"))
            .parse()
            .unwrap();
        rinch_core::events::dispatch_event(rinch_core::events::EventHandlerId(id));
    }

    fn center(doc: &RinchDocument, id: usize) -> (f32, f32) {
        let r = rinch_dom::paint::painted_border_box(&doc.tree, id, 1.0);
        (r.center().x as f32, r.center().y as f32)
    }

    fn is_displayed(doc: &RinchDocument, id: usize) -> bool {
        doc.tree.get(id).unwrap().computed_style.display
            != rinch_dom::computed_style::DisplayValue::None
    }

    /// `BorderlessWindow`'s InlineTitlebar tree, with menu index 0 open.
    ///
    /// Returns the document plus the ids of the dismiss overlay, the open menu's
    /// single entry, and the *other* top-level label — the hover-to-switch
    /// target, which the same defect made unreachable.
    fn open_inline_menu() -> (Rc<RefCell<RinchDocument>>, usize, usize, usize) {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        let body = doc.borrow().body();
        load_stylesheets(&doc, body);

        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let mut scope = RenderScope::new(doc_as_dom, body);
        let active_menu: Signal<i32> = Signal::new(-1);

        let container = scope.create_element("div");
        container.set_attribute("class", "rinch-borderlesswindow");
        container.set_attribute("style", "position: relative;");

        let titlebar = scope.create_element("div");
        titlebar.set_attribute("class", "rinch-borderlesswindow__titlebar");
        container.append_child(&titlebar);

        let content = scope.create_element("div");
        content.set_attribute("class", "rinch-borderlesswindow__content");
        container.append_child(&content);

        let layer = scope.create_element("div");
        layer.set_attribute("class", "rinch-app-menu-bar__inline-layer");
        let overlay = render_inline_overlay(&mut scope, active_menu);
        layer.append_child(&overlay);

        let row = scope.create_element("div");
        row.set_attribute("class", "rinch-app-menu-bar__inline-row");
        let theme_menu = Menu::new().item(
            MenuItem::new("Toggle Dark Mode")
                .shortcut("Ctrl+D")
                .on_click(|| {}),
        );
        let help_menu = Menu::new().item(MenuItem::new("About").on_click(|| {}));
        let menus: Vec<(&str, &Menu)> = vec![("Theme", &theme_menu), ("Help", &help_menu)];
        row.append_child(&render_menu_items_inline(&mut scope, &menus, active_menu));
        layer.append_child(&row);
        container.append_child(&layer);
        doc.borrow_mut().append_child(body, container.node_id());

        // Open the first menu through its own label handler, as a click does.
        let label = nodes_with_class(&doc.borrow(), "rinch-app-menu-item__label")[0];
        run_rid_handler(&doc, label);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);

        let (entry_id, other_label_id) = {
            let d = doc.borrow();
            let entries = nodes_with_class(&d, "rinch-app-menu-entry");
            let labels = nodes_with_class(&d, "rinch-app-menu-item__label");
            assert_eq!(entries.len(), 2, "one entry per menu");
            assert_eq!(labels.len(), 2, "one label per top-level menu");
            assert!(active_menu.get() == 0, "the first menu is open");
            (entries[0], labels[1])
        };
        (doc, overlay.node_id().0, entry_id, other_label_id)
    }

    /// The open dropdown sits *inside* the window container, whose
    /// `overflow: hidden` makes it a stacking context; the dismiss overlay used
    /// to be `position: fixed`, which hoists it out to the viewport's stacking
    /// context instead. Its `z-index: 199` and the menu row's `201` were then
    /// being compared across two different stacking contexts — which `z-index`
    /// does not do — so the overlay covered the menu and every entry click
    /// merely dismissed it (#527).
    #[test]
    fn a_click_inside_an_open_dropdown_runs_the_entry_not_the_dismiss_overlay() {
        let (doc, overlay_id, entry_id, _) = open_inline_menu();
        let d = doc.borrow();

        let (x, y) = center(&d, entry_id);
        let entry_rid = rid_of(&d, entry_id);
        let overlay_rid = rid_of(&d, overlay_id);

        // The fixture must not be degenerate: the entry has to be somewhere the
        // overlay also covers, or the two handlers were never in contention and
        // the assertion below would hold however the ordering came out.
        let o = rinch_dom::paint::painted_border_box(&d.tree, overlay_id, 1.0);
        assert!(
            o.contains(peniko::kurbo::Point::new(x as f64, y as f64)),
            "the overlay must cover the entry's centre for this test to \
             discriminate; overlay = {o:?}, entry centre = ({x}, {y})"
        );

        assert_eq!(
            rid_under(&d, x, y).as_deref(),
            Some(entry_rid.as_str()),
            "a click at the centre of an open menu entry must run that entry's \
             handler, not the dismiss overlay's ({overlay_rid})"
        );
    }

    /// The other half of the same ordering, and the guard against "fixing" the
    /// dropdown by making the overlay unhittable: a click *outside* the open
    /// menu must still reach the overlay and dismiss it.
    #[test]
    fn a_click_outside_an_open_menu_still_reaches_the_dismiss_overlay() {
        let (doc, overlay_id, _, _) = open_inline_menu();
        let d = doc.borrow();
        let overlay_rid = rid_of(&d, overlay_id);

        // The window's edges, not just its middle. The middle is covered by
        // every candidate offset and every candidate size, so it pins nothing
        // about the overlay's geometry — an overlay shifted or shrunk by any
        // amount short of half the window still answers there. This is the
        // layout `BorderlessWindow` apps actually use and the one #527 broke,
        // so it gets the strictest coverage of the three.
        let row = nodes_with_class(&d, "rinch-app-menu-bar__inline-row")[0];
        let row_box = rinch_dom::paint::painted_border_box(&d.tree, row, 1.0);
        let below_row = row_box.y1 as f32 + 2.0;
        assert!(
            row_box.x1 < (VIEWPORT_W - 2.0) as f64,
            "the sample at the top-right corner assumes the menu row does not \
             reach it; row = {row_box:?}"
        );

        for (x, y) in [
            (2.0, below_row),
            (VIEWPORT_W - 2.0, below_row),
            (2.0, VIEWPORT_H - 2.0),
            (VIEWPORT_W - 2.0, VIEWPORT_H - 2.0),
            // Beside the row rather than below it, which pins the top edge.
            (VIEWPORT_W - 2.0, 2.0),
            (VIEWPORT_W / 2.0, VIEWPORT_H / 2.0),
        ] {
            assert_eq!(
                rid_under(&d, x, y).as_deref(),
                Some(overlay_rid.as_str()),
                "a click at ({x}, {y}) must dismiss the open menu"
            );
        }
    }

    /// Hover-to-switch rides the same hit test (`data-onenter` is resolved from
    /// the hovered node), so the overlay swallowed that too: with one menu open,
    /// the *other* top-level label was unreachable.
    #[test]
    fn the_other_top_level_label_is_reachable_while_a_menu_is_open() {
        let (doc, _, _, other_label_id) = open_inline_menu();
        let d = doc.borrow();

        let (x, y) = center(&d, other_label_id);
        let hit = crate::app::hit_testing::hit_test(&d.tree, x, y);
        let mut id = hit.expect("something is under the second menu label");
        loop {
            if id == other_label_id {
                return;
            }
            match d.tree.get(id).and_then(|n| n.parent) {
                Some(p) => id = p,
                None => panic!(
                    "the second top-level menu label is not reachable while \
                     another menu is open; hit {hit:?} instead"
                ),
            }
        }
    }

    /// `BorderlessWindow`'s BelowTitlebar tree — the other Linux layout, whose
    /// bar renders under the title bar rather than inside it — menu 0 open.
    fn open_below_titlebar_menu() -> (Rc<RefCell<RinchDocument>>, usize, usize) {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        let body = doc.borrow().body();
        load_stylesheets(&doc, body);

        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let mut scope = RenderScope::new(doc_as_dom, body);

        let container = scope.create_element("div");
        container.set_attribute("class", "rinch-borderlesswindow");
        container.set_attribute("style", "position: relative;");

        let titlebar = scope.create_element("div");
        titlebar.set_attribute("class", "rinch-borderlesswindow__titlebar");
        container.append_child(&titlebar);

        let theme_menu = Menu::new().item(
            MenuItem::new("Toggle Dark Mode")
                .shortcut("Ctrl+D")
                .on_click(|| {}),
        );
        let menus: Vec<(&str, &Menu)> = vec![("Theme", &theme_menu)];
        let bar = super::render_menu_bar_standalone(&mut scope, &menus, TITLEBAR_HEIGHT);
        container.append_child(&bar);
        doc.borrow_mut().append_child(body, container.node_id());

        let label = nodes_with_class(&doc.borrow(), "rinch-app-menu-item__label")[0];
        run_rid_handler(&doc, label);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);

        let (overlay_id, entry_id) = {
            let d = doc.borrow();
            (
                nodes_with_class(&d, "rinch-app-menu-bar__overlay")[0],
                nodes_with_class(&d, "rinch-app-menu-entry")[0],
            )
        };
        (doc, overlay_id, entry_id)
    }

    /// The below-titlebar layout has the same defect and the same cure, but its
    /// overlay's containing block starts `TITLEBAR_HEIGHT` down the window — so
    /// the overlay has to climb back up over the title bar, and grow to match,
    /// or a click at either end of the window stops dismissing the menu.
    /// Sampled at both ends, where the offset is the only thing that can put the
    /// overlay there; the middle of the window is covered either way and would
    /// prove nothing.
    #[test]
    fn the_below_titlebar_overlay_covers_the_whole_window() {
        let (doc, overlay_id, entry_id) = open_below_titlebar_menu();
        let d = doc.borrow();
        let overlay_rid = rid_of(&d, overlay_id);

        assert_eq!(
            rid_under(&d, 600.0, 2.0).as_deref(),
            Some(overlay_rid.as_str()),
            "a click on the title bar must dismiss the open menu"
        );
        assert_eq!(
            rid_under(&d, 600.0, VIEWPORT_H - 2.0).as_deref(),
            Some(overlay_rid.as_str()),
            "a click at the bottom of the window must dismiss the open menu"
        );

        let (x, y) = center(&d, entry_id);
        assert_eq!(
            rid_under(&d, x, y).as_deref(),
            Some(rid_of(&d, entry_id).as_str()),
            "…and a click on the entry must still run the entry"
        );
    }

    /// The third layout: a *non*-borderless window, where `render_with_menu_bar`
    /// wraps the app's content itself and there is no `BorderlessWindow`
    /// container. Menu 0 open. Returns the overlay's and the entry's ids.
    fn open_wrapped_menu() -> (Rc<RefCell<RinchDocument>>, usize, usize) {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        let body = doc.borrow().body();
        load_stylesheets(&doc, body);

        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let mut scope = RenderScope::new(doc_as_dom, body);

        let content = scope.create_element("div");
        let filler = scope.create_text("app content");
        content.append_child(&filler);

        let theme_menu = Menu::new().item(
            MenuItem::new("Toggle Dark Mode")
                .shortcut("Ctrl+D")
                .on_click(|| {}),
        );
        let menus: Vec<(&str, &Menu)> = vec![("Theme", &theme_menu)];
        let wrapper = super::render_with_menu_bar(&mut scope, &menus, content, 0);
        doc.borrow_mut().append_child(body, wrapper.node_id());

        let label = nodes_with_class(&doc.borrow(), "rinch-app-menu-item__label")[0];
        run_rid_handler(&doc, label);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);

        let (overlay_id, entry_id) = {
            let d = doc.borrow();
            (
                nodes_with_class(&d, "rinch-app-menu-bar__overlay")[0],
                nodes_with_class(&d, "rinch-app-menu-entry")[0],
            )
        };
        (doc, overlay_id, entry_id)
    }

    /// The third layout's containing block is different again — the overlay's
    /// parent is the full-viewport wrapper rather than a bar container — so its
    /// coverage is pinned separately rather than inferred from the other two.
    ///
    /// This layout was **not** broken by #527: its wrapper carries no
    /// `overflow`, so nothing between the bar and the body formed a stacking
    /// context and the overlay's `199` was always compared against the bar's
    /// `201` in one sequence. The assertions are here to keep the move from
    /// `fixed` to `absolute` from regressing a layout that already worked.
    #[test]
    fn the_wrapped_layout_overlay_covers_the_whole_window() {
        let (doc, overlay_id, entry_id) = open_wrapped_menu();
        let d = doc.borrow();
        let overlay_rid = rid_of(&d, overlay_id);

        // The four corners of the area *below* the bar. The top two pixels of
        // the window are the bar itself in this layout (`top_offset` is 0), and
        // the bar is above the overlay by its `z-index` — a click there
        // correctly reaches no handler at all rather than dismissing.
        let below_bar = MENU_BAR_HEIGHT as f32 + 2.0;
        for (x, y) in [
            (2.0, below_bar),
            (VIEWPORT_W - 2.0, below_bar),
            (2.0, VIEWPORT_H - 2.0),
            (VIEWPORT_W - 2.0, VIEWPORT_H - 2.0),
        ] {
            assert_eq!(
                rid_under(&d, x, y).as_deref(),
                Some(overlay_rid.as_str()),
                "a click at ({x}, {y}) must dismiss the open menu"
            );
        }

        let (x, y) = center(&d, entry_id);
        assert_eq!(
            rid_under(&d, x, y).as_deref(),
            Some(rid_of(&d, entry_id).as_str()),
            "…and a click on the entry must still run the entry"
        );
    }

    /// A submenu trigger carried no `data-rid` at all, so a click on it reached
    /// no handler on any ancestor and opened nothing — while pre-fix the
    /// overlay won the hit test outright and closed the whole menu (#527).
    ///
    /// **Two** submenus, and the assertions are about the *second*. With one,
    /// `flyout_idx == 0` and `active_flyout.set(flyout_idx)` is indistinguishable
    /// from `set(0)` — the fixed point this repo keeps rediscovering. The same
    /// fixture pins the pre-existing `data-onenter` hover handler's index for
    /// free, which had the identical blind spot, so both are checked here.
    #[test]
    fn clicking_a_submenu_trigger_opens_its_flyout() {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        let body = doc.borrow().body();
        load_stylesheets(&doc, body);

        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let mut scope = RenderScope::new(doc_as_dom, body);
        let active_menu: Signal<i32> = Signal::new(-1);

        let container = scope.create_element("div");
        container.set_attribute("class", "rinch-borderlesswindow");
        container.set_attribute("style", "position: relative;");
        let layer = scope.create_element("div");
        layer.set_attribute("class", "rinch-app-menu-bar__inline-layer");
        layer.append_child(&render_inline_overlay(&mut scope, active_menu));
        let row = scope.create_element("div");
        row.set_attribute("class", "rinch-app-menu-bar__inline-row");

        let goto = Menu::new().item(MenuItem::new("Overview").on_click(|| {}));
        let recent = Menu::new().item(MenuItem::new("Reopen").on_click(|| {}));
        let view = Menu::new().submenu("Go To", goto).submenu("Recent", recent);
        let menus: Vec<(&str, &Menu)> = vec![("View", &view)];
        row.append_child(&render_menu_items_inline(&mut scope, &menus, active_menu));
        layer.append_child(&row);
        container.append_child(&layer);
        doc.borrow_mut().append_child(body, container.node_id());

        active_menu.set(0);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);

        let (triggers, flyouts) = {
            let d = doc.borrow();
            (
                nodes_with_class(&d, "rinch-app-menu-submenu__trigger"),
                nodes_with_class(&d, "rinch-app-menu-submenu__flyout"),
            )
        };
        assert_eq!(triggers.len(), 2, "two submenu triggers");
        assert_eq!(flyouts.len(), 2, "two flyout panels");

        /// Which flyouts are showing, as a `[bool; 2]`.
        fn shown(doc: &Rc<RefCell<RinchDocument>>, flyouts: &[usize]) -> Vec<bool> {
            let d = doc.borrow();
            flyouts.iter().map(|&f| is_displayed(&d, f)).collect()
        }

        assert_eq!(
            shown(&doc, &flyouts),
            [false, false],
            "both flyouts start hidden"
        );

        // The click routes to the second trigger's own handler…
        let (x, y) = center(&doc.borrow(), triggers[1]);
        assert_eq!(
            rid_under(&doc.borrow(), x, y).as_deref(),
            Some(rid_of(&doc.borrow(), triggers[1]).as_str()),
            "a click on a submenu trigger must run the trigger's own handler"
        );

        // …and running it opens the second flyout, not the first.
        run_rid_handler(&doc, triggers[1]);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);
        assert_eq!(
            shown(&doc, &flyouts),
            [false, true],
            "clicking the second submenu trigger must open the second flyout"
        );
        assert_eq!(
            active_menu.get(),
            0,
            "…and must not close the menu it lives in"
        );

        // Hover, the older path, indexes its flyout the same way. Driven from
        // the first trigger so it has to move the selection back, then from the
        // second so neither direction can be a no-op.
        run_onenter_handler(&doc, triggers[0]);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);
        assert_eq!(
            shown(&doc, &flyouts),
            [true, false],
            "hovering the first submenu trigger must open the first flyout"
        );
        run_onenter_handler(&doc, triggers[1]);
        doc.borrow_mut().resolve_layout(VIEWPORT_W, VIEWPORT_H);
        assert_eq!(
            shown(&doc, &flyouts),
            [false, true],
            "hovering the second submenu trigger must open the second flyout"
        );
    }
}
