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

use crate::dropdown_menu::{
    MENU_CLOSE_ID_ATTR, register_menu_close_signal, unregister_menu_close_signal,
};

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
        Self { opened }
    }
}

impl Component for ContextMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let opened = self.opened;
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
        // A press of any button outside dismisses, not only a left one (#1093).
        overlay.set_attribute(rinch_core::events::BACKDROP_ATTRIBUTE, "");

        // Dropdown container — sibling of overlay so clicks inside it
        // don't bubble to the overlay's close handler
        let dropdown = rinch_macros::rsx! { div { class: "rinch-context-menu__dropdown" } };

        // Stamped for this menu's whole lifetime (issue #714): a
        // `DropdownMenuItem` closes its menu by walking up from the clicked
        // button to the nearest ancestor carrying this attribute, resolved at
        // CLICK time rather than at the item's own render time. That is what
        // lets an item inside a reactive block nested in the dropdown —
        // rendered again, long after this `render` returns, whenever that
        // block's own condition changes — still close this menu: the
        // attribute is still here on `dropdown` when the click happens,
        // whatever rendered when the item itself was built. See
        // `dropdown_menu::register_menu_close_signal`'s doc comment.
        let menu_close_id = register_menu_close_signal(opened);
        dropdown.set_attribute(MENU_CLOSE_ID_ATTR, &menu_close_id.to_string());

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
        // The cleanup removes the node and drops the registry entry above —
        // it touches no signal. `opened` is freed by the same disposal, so a
        // "close it on the way out" write here would be dropped and would
        // warn — see `close_menu_signal`'s freed-signal no-op.
        // `unregister_menu_close_signal` is what keeps a menu built and torn
        // down many times (a reactive block rebuilding its row) from growing
        // the registry by one dead entry per rebuild.
        {
            let portal_for_owner = portal.clone();
            let registered = rinch_core::reactive::on_cleanup(move || {
                unregister_menu_close_signal(menu_close_id);
                portal_for_owner.discard();
            });
            if !registered {
                let portal_for_root = portal.clone();
                __scope.on_cleanup(move || {
                    unregister_menu_close_signal(menu_close_id);
                    portal_for_root.discard();
                });
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
    use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
    use rinch_core::events::{EventHandlerId, dispatch_event};
    use rinch_core::reactive::Scope;
    use std::cell::{Cell, RefCell};
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

    /// The `data-rid` of the (one) descendant of `body` carrying
    /// `rinch-dropdown-menu__item` — the item a reactive block rendered.
    fn item_rid(doc: &Rc<RefCell<dyn DomDocument>>) -> Option<usize> {
        fn walk(d: &dyn DomDocument, node: NodeId) -> Option<usize> {
            if d.get_attribute(node, "class").is_some_and(|c| {
                c.split_whitespace()
                    .any(|c| c == "rinch-dropdown-menu__item")
            }) {
                return d
                    .get_attribute(node, "data-rid")
                    .and_then(|s| s.parse().ok());
            }
            d.get_children(node).into_iter().find_map(|c| walk(d, c))
        }
        let d = doc.borrow();
        walk(&*d, d.body())
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

    /// Disposing a menu's owner releases its close-signal registration too, so
    /// rebuilding a menu does not grow the registry (#714's review).
    #[test]
    fn disposing_the_owning_scope_releases_the_close_registration() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let before = crate::dropdown_menu::registered_menu_count();
        for _ in 0..3 {
            let owner = Scope::new();
            owner.run(|| {
                let _ = render_menu(&mut scope);
            });
            assert_eq!(crate::dropdown_menu::registered_menu_count(), before + 1);
            owner.dispose();
        }
        assert_eq!(
            crate::dropdown_menu::registered_menu_count(),
            before,
            "every disposed menu released its entry"
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

    /// Issue #714 (bug 1): `ContextMenuDropdown { DropdownMenuItem {..} if
    /// clipboard.get().is_some() { DropdownMenuItem {..} } }` — the second
    /// item renders again every time `clipboard` changes, long after
    /// `ContextMenu::render` has already returned. Simulated here with
    /// `show_dom`, which is exactly what that `if` desugars to: the item
    /// does not exist at all while `show_item` is `false`, and is rendered
    /// for the first time only after `ContextMenu::render` below has fully
    /// run and returned.
    ///
    /// Clicking that item must still close the menu. The old mechanism
    /// resolved the close signal through a thread-local the container
    /// cleared the moment its own `render` ran — which is before this item
    /// is ever built — so the click ran the item's own `onclick` and left
    /// `opened` untouched.
    #[test]
    fn an_item_rendered_by_a_reactive_block_still_closes_the_menu() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let show_item = rinch_core::Signal::new(false);
        let clicked = Rc::new(Cell::new(false));
        let clicked_for_item = clicked.clone();

        let target = scope.create_element("div");
        let items = scope.create_element("div");
        // Nothing renders here yet — `show_item` is false — mirroring an
        // `if` branch that has not fired.
        rinch_core::show_dom(
            &mut scope,
            &items,
            move || show_item.get(),
            move |s: &mut RenderScope| {
                crate::dropdown_menu::DropdownMenuItem {
                    onclick: Some(rinch_core::Callback::new({
                        let clicked = clicked_for_item.clone();
                        move || clicked.set(true)
                    })),
                    ..Default::default()
                }
                .render(s, &[])
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );

        let menu = ContextMenu::default();
        let opened = menu.opened;
        let _root = menu.render(&mut scope, &[target, items]);

        // `ContextMenu::render` has now fully returned. Only now does the
        // reactive block fire, rendering the item for the first time.
        opened.set(true);
        show_item.set(true);

        let rid = item_rid(&doc).expect("the late-rendered item carries a handler");
        dispatch_event(EventHandlerId(rid));

        assert!(clicked.get(), "the item's own onclick still runs");
        assert_eq!(
            opened.try_get(),
            Some(false),
            "and the menu it was built inside of closes, even though it \
             rendered after ContextMenu::render had already returned"
        );
    }

    /// An item whose own action hides it (a "Paste" that consumes the
    /// clipboard its `if` reads) still closes the menu. A lookup run *after*
    /// the callback starts with a `NodeHandle` access, which flushes the
    /// queued `show_dom` effect and detaches the button before the walk can
    /// climb past it — so the item resolves its menu before the callback runs
    /// (#714's review).
    #[test]
    fn an_item_that_hides_itself_on_click_still_closes_the_menu() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let show_item = rinch_core::Signal::new(false);
        let clicked = Rc::new(Cell::new(false));
        let clicked_for_item = clicked.clone();
        let show_item_for_item = show_item;

        let target = scope.create_element("div");
        let items = scope.create_element("div");
        rinch_core::show_dom(
            &mut scope,
            &items,
            move || show_item.get(),
            move |s: &mut RenderScope| {
                crate::dropdown_menu::DropdownMenuItem {
                    onclick: Some(rinch_core::Callback::new({
                        let clicked = clicked_for_item.clone();
                        move || {
                            clicked.set(true);
                            // the item's own action hides itself, same as a
                            // "Paste" item consuming the clipboard it was
                            // conditioned on.
                            show_item_for_item.set(false);
                        }
                    })),
                    ..Default::default()
                }
                .render(s, &[])
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );

        let menu = ContextMenu::default();
        let opened = menu.opened;
        let _root = menu.render(&mut scope, &[target, items]);

        opened.set(true);
        show_item.set(true);

        let rid = item_rid(&doc).expect("the late-rendered item carries a handler");
        dispatch_event(EventHandlerId(rid));

        assert!(clicked.get(), "the item's own onclick still runs");
        assert_eq!(
            opened.try_get(),
            Some(false),
            "the menu must still close even though the item's own click hid itself"
        );
    }

    /// Coverage gap probe (not in the PR): two independent ContextMenus open
    /// at once must not cross-wire their close signals via id collision.
    #[test]
    fn two_simultaneous_menus_do_not_cross_close() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let target_a = scope.create_element("div");
        let items_a = scope.create_element("div");
        let item_a = crate::dropdown_menu::DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(|| {})),
            ..Default::default()
        };
        let item_a_node = item_a.render(&mut scope, &[]);
        items_a.append_child(&item_a_node);
        let menu_a = ContextMenu::default();
        let opened_a = menu_a.opened;
        let _root_a = menu_a.render(&mut scope, &[target_a, items_a]);
        opened_a.set(true);

        let target_b = scope.create_element("div");
        let items_b = scope.create_element("div");
        let item_b = crate::dropdown_menu::DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(|| {})),
            ..Default::default()
        };
        let item_b_node = item_b.render(&mut scope, &[]);
        items_b.append_child(&item_b_node);
        let menu_b = ContextMenu::default();
        let opened_b = menu_b.opened;
        let _root_b = menu_b.render(&mut scope, &[target_b, items_b]);
        opened_b.set(true);

        let rid_a: usize = item_a_node
            .get_attribute("data-rid")
            .unwrap()
            .parse()
            .unwrap();
        dispatch_event(EventHandlerId(rid_a));

        assert_eq!(opened_a.try_get(), Some(false), "menu A closes");
        assert_eq!(opened_b.try_get(), Some(true), "menu B must stay open");
    }

    /// Nested menu attack: a `DropdownMenu` (submenu) rendered inside a
    /// `ContextMenu`'s dropdown. Clicking an item inside the inner
    /// `DropdownMenu` must close the INNER menu (nearest ancestor), not the
    /// outer `ContextMenu`.
    #[test]
    fn nested_menu_closes_the_nearest_one() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        // Build the inner DropdownMenu with one item.
        let inner_item = crate::dropdown_menu::DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(|| {})),
            ..Default::default()
        };
        let inner_item_node = inner_item.render(&mut scope, &[]);
        let inner_menu = crate::dropdown_menu::DropdownMenu::default();
        let inner_opened_signal = inner_menu._close_signal;
        let inner_root = inner_menu.render(&mut scope, std::slice::from_ref(&inner_item_node));

        // Outer ContextMenu, with the inner DropdownMenu's root as one of its
        // dropdown's children.
        let target = scope.create_element("div");
        let items = scope.create_element("div");
        items.append_child(&inner_root);
        let outer_menu = ContextMenu::default();
        let outer_opened = outer_menu.opened;
        let _outer_root = outer_menu.render(&mut scope, &[target, items]);

        outer_opened.set(true);
        // inner_opened_signal: DropdownMenu::_close_signal starts true; a
        // click sets it false (its own "closed" marker), unrelated to the
        // outer `opened` signal.
        assert_eq!(inner_opened_signal.try_get(), Some(true));

        let rid: usize = inner_item_node
            .get_attribute("data-rid")
            .unwrap()
            .parse()
            .unwrap();
        dispatch_event(EventHandlerId(rid));

        assert_eq!(
            inner_opened_signal.try_get(),
            Some(false),
            "the inner (nearest) menu's close signal fires"
        );
        assert_eq!(
            outer_opened.try_get(),
            Some(true),
            "the outer ContextMenu must NOT be closed by an inner submenu's item"
        );
    }
}
