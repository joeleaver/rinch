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
use rinch_core::{Component, Signal};

use crate::dropdown_menu::set_menu_close_signal;

/// A context menu component that shows on right-click.
///
/// Children should be a `ContextMenuTarget` followed by a `ContextMenuDropdown`.
/// The target gets an `oncontextmenu` handler wired up automatically.
/// The dropdown is positioned at the click coordinates.
pub struct ContextMenu {
    /// Internal opened state — set automatically, not a user prop.
    #[doc(hidden)]
    pub opened: Signal<bool>,
}

impl std::fmt::Debug for ContextMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextMenu").finish()
    }
}

impl Default for ContextMenu {
    fn default() -> Self {
        let opened = Signal::new(false);
        // Set thread-local so DropdownMenuItem children (rendered after this
        // struct is constructed but before Component::render) can capture it.
        set_menu_close_signal(opened);
        Self { opened }
    }
}

impl Component for ContextMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let opened = self.opened;
        // Children have already been rendered and captured the close signal.
        // Clear it so nested menus don't leak.
        crate::dropdown_menu::clear_menu_close_signal();
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
        // don't bubble to the overlay's close handler
        let dropdown = rinch_macros::rsx! { div { class: "rinch-context-menu__dropdown" } };

        // Append remaining children (ContextMenuDropdown contents) into the dropdown
        for child in children.iter().skip(1) {
            dropdown.append_child(child);
        }

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

        // Take the portal down with whatever built this menu.
        //
        // The portal hangs off `body`, outside the subtree this component was
        // rendered into — that is the whole point of a portal, and it is why
        // nothing reclaimed it. When the owning scope is disposed (a reactive
        // `for`/`if` rebuilding the row the menu belongs to, or the root
        // unmounting) the subtree under `root` goes and this node does not: its
        // handlers are deregistered with the scope and its signals are freed,
        // but the markup stays on `body`. An open menu was then left on screen
        // that neither acted nor closed, which a person meets by right-clicking
        // while the tree behind them is refreshing.
        //
        // The ambient owner first, because that is the *rebuilding* one: a
        // reactive block pushes an owner per run and disposes it on the next,
        // and `RenderScope::on_cleanup` would instead tie the portal to the
        // whole root, which only a full unmount disposes. `on_cleanup` answers
        // `false` when there is no live ambient owner (a component built
        // straight at the root, outside any reactive block), and the root scope
        // is the right home for that case.
        //
        // The cleanup removes the node and touches no signal. `opened` is freed
        // by the same disposal, so a "close it on the way out" write here would
        // be dropped and would warn — see the `is_alive` guard on
        // `DropdownMenuItem`'s close write. Removing the node is the whole job.
        {
            let portal_for_owner = portal.clone();
            let registered = rinch_core::reactive::on_cleanup(move || portal_for_owner.discard());
            if !registered {
                let portal_for_root = portal.clone();
                __scope.on_cleanup(move || portal_for_root.discard());
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::dom::{DomDocument, NodeHandle};
    use rinch_core::reactive::Scope;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Direct children of `body` carrying `class`.
    fn portals_on_body(doc: &Rc<RefCell<dyn DomDocument>>, class: &str) -> usize {
        let d = doc.borrow();
        let body = d.body();
        d.get_children(body)
            .into_iter()
            .filter(|id| {
                d.get_attribute(*id, "class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == class))
            })
            .count()
    }

    fn render_menu(scope: &mut RenderScope) -> NodeHandle {
        let target = scope.create_element("div");
        let items = scope.create_element("div");
        ContextMenu::default().render(scope, &[target, items])
    }

    /// The portal lives on `body`, outside the subtree the menu was built into,
    /// so disposing that subtree used to leave it behind: markup on screen whose
    /// handlers had been deregistered with the scope, a menu that neither acted
    /// nor closed. It goes with its owner now.
    #[test]
    fn disposing_the_owning_scope_takes_the_portal_off_body() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        // A reactive block pushes an owner per run and disposes it on the next;
        // `Scope::run` is that shape, and it is the owner a rebuild reclaims.
        let owner = Scope::new();
        owner.run(|| {
            let _ = render_menu(&mut scope);
        });
        assert_eq!(
            portals_on_body(&doc, "rinch-context-menu__portal"),
            1,
            "the menu portals itself to body"
        );

        owner.dispose();
        assert_eq!(
            portals_on_body(&doc, "rinch-context-menu__portal"),
            0,
            "and the rebuild that disposed its owner must not orphan it there"
        );
    }

    /// Rebuilding five times must leave one portal, not five — the shape a tree
    /// that refreshes under the pointer produces.
    #[test]
    fn rebuilding_does_not_accumulate_portals() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let mut live: Option<Scope> = None;
        for run in 0..5 {
            let owner = Scope::new();
            owner.run(|| {
                let _ = render_menu(&mut scope);
            });
            // Disposing the previous run is what a reactive block does before
            // building the next.
            if let Some(previous) = live.replace(owner) {
                previous.dispose();
            }
            assert_eq!(
                portals_on_body(&doc, "rinch-context-menu__portal"),
                1,
                "rebuild {run} must leave exactly one portal on body"
            );
        }

        live.take().unwrap().dispose();
        assert_eq!(portals_on_body(&doc, "rinch-context-menu__portal"), 0);
    }

    /// With no ambient owner — a menu built straight at the root, outside any
    /// reactive block — the root scope is the right home for the cleanup, and
    /// the portal still goes when the root does.
    #[test]
    fn with_no_ambient_owner_the_root_scope_reclaims_the_portal() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let _ = render_menu(&mut scope);
        assert_eq!(portals_on_body(&doc, "rinch-context-menu__portal"), 1);

        scope.dispose();
        assert_eq!(
            portals_on_body(&doc, "rinch-context-menu__portal"),
            0,
            "an unmounting root must reclaim its portal too"
        );
    }
}
