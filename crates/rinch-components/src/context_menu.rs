//! ContextMenu component.
//!
//! A right-click context menu that positions itself at the cursor location.
//! Uses `DropdownMenuItem` for menu items.
//!
//! # Example
//!
//! ```ignore
//! ContextMenu {
//!     ContextMenuTarget {
//!         div { "Right-click me" }
//!     }
//!     ContextMenuDropdown {
//!         DropdownMenuItem { onclick: || println!("Edit"), "Edit" }
//!         DropdownMenuDivider {}
//!         DropdownMenuItem { onclick: || println!("Delete"), "Delete" }
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, Signal};

/// A context menu component that shows on right-click.
///
/// Children should be a `ContextMenuTarget` followed by a `ContextMenuDropdown`.
/// The target gets an `oncontextmenu` handler wired up automatically.
/// The dropdown is positioned at the click coordinates.
#[derive(Debug, Default)]
pub struct ContextMenu;

impl Component for ContextMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let opened = Signal::new(false);
        let pos_x = Signal::new(0.0f32);
        let pos_y = Signal::new(0.0f32);
        let vp_w = Signal::new(0.0f32);
        let vp_h = Signal::new(0.0f32);

        let root = rinch_macros::rsx! { div { class: "rinch-context-menu" } };

        // First child = target (gets oncontextmenu handler)
        if let Some(target) = children.first() {
            // Register a contextmenu handler on the target
            let handler_id = __scope.register_handler({
                move || {
                    let ctx = rinch_core::events::get_click_context();
                    pos_x.set(ctx.mouse_x);
                    pos_y.set(ctx.mouse_y);
                    vp_w.set(ctx.viewport_width);
                    vp_h.set(ctx.viewport_height);
                    opened.set(true);
                }
            });
            target.set_attribute("data-oncontextmenu", &handler_id.0.to_string());
            root.append_child(target);
        }

        // Portal — appended to body so position:fixed works correctly.
        // Taffy treats fixed as absolute, so the portal must be a direct child
        // of body for top:0/left:0 to mean viewport origin.
        let body = __scope.body_handle();
        let portal = rinch_macros::rsx! { div { class: "rinch-context-menu__portal" } };

        // Overlay — covers the full viewport; click-to-close
        let overlay = rinch_macros::rsx! { div { class: "rinch-context-menu__overlay" } };

        let close_handler_id = __scope.register_handler({
            move || {
                opened.set(false);
            }
        });
        overlay.set_attribute("data-rid", &close_handler_id.0.to_string());

        // Dropdown container — sibling of overlay so clicks inside it
        // don't bubble to the overlay's close handler.
        let dropdown = rinch_macros::rsx! { div { class: "rinch-context-menu__dropdown" } };

        // Append remaining children (ContextMenuDropdown contents) into the dropdown
        for child in children.iter().skip(1) {
            dropdown.append_child(child);
        }

        // Wrap every data-rid handler inside the dropdown so that clicking
        // a menu item also closes the context menu. Rinch's data-rid dispatch
        // only fires the nearest handler (no bubbling), so we intercept each
        // item's handler and chain our close logic onto it.
        wrap_handlers_recursive(__scope, &dropdown, opened);

        portal.append_child(&overlay);
        portal.append_child(&dropdown);

        // Show/hide portal and position dropdown reactively
        {
            let portal_clone = portal.clone();
            let overlay_clone = overlay.clone();
            let dropdown_clone = dropdown.clone();
            __scope.create_effect(move || {
                if opened.get() {
                    let x = pos_x.get();
                    let y = pos_y.get();
                    let vw = vp_w.get();
                    let vh = vp_h.get();

                    // Estimated dropdown size for viewport-aware positioning.
                    // The dropdown min-width is 180px in CSS; height ~200px is a
                    // conservative estimate for a typical context menu.
                    let menu_w = 180.0f32;
                    let menu_h = 200.0f32;
                    let margin = 4.0f32;

                    // Flip horizontally if menu would overflow right edge
                    let final_x = if x + menu_w + margin > vw && x > menu_w {
                        x - menu_w
                    } else {
                        x
                    };

                    // Flip vertically if menu would overflow bottom edge
                    let final_y = if y + menu_h + margin > vh && y > menu_h {
                        y - menu_h
                    } else {
                        y
                    };

                    portal_clone.set_attribute(
                        "style",
                        "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 200;",
                    );
                    overlay_clone.set_attribute(
                        "style",
                        "position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
                    );
                    dropdown_clone.set_attribute(
                        "style",
                        &format!(
                            "position: absolute; left: {}px; top: {}px;",
                            final_x, final_y,
                        ),
                    );
                } else {
                    portal_clone.set_attribute("style", "display: none;");
                    overlay_clone.set_attribute("style", "display: none;");
                    dropdown_clone.set_attribute("style", "display: none;");
                }
            });
        }
        body.append_child(&portal);

        root
    }
}

/// Wrapper for the right-clickable target element.
#[derive(Debug, Default)]
pub struct ContextMenuTarget;

impl Component for ContextMenuTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-context-menu__target" } };

        for child in children {
            target.append_child(child);
        }

        target
    }
}

/// Container for the context menu items.
///
/// Children are typically `DropdownMenuItem`, `DropdownMenuDivider`, etc.
#[derive(Debug, Default)]
pub struct ContextMenuDropdown;

impl Component for ContextMenuDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-context-menu__dropdown-inner" } };

        for child in children {
            dropdown.append_child(child);
        }

        dropdown
    }
}

/// Recursively walk the DOM subtree and wrap every `data-rid` handler so that
/// it also sets `opened` to false — closing the context menu on item click.
fn wrap_handlers_recursive(scope: &mut RenderScope, node: &NodeHandle, opened: Signal<bool>) {
    if let Some(rid_str) = node.get_attribute("data-rid") {
        if let Ok(original_id) = rid_str.parse::<usize>() {
            let original = EventHandlerId(original_id);
            let new_id = scope.register_handler(move || {
                dispatch_event(original);
                opened.set(false);
            });
            node.set_attribute("data-rid", &new_id.0.to_string());
        }
    }
    for child in node.children() {
        wrap_handlers_recursive(scope, &child, opened);
    }
}
