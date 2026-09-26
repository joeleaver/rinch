//! Every outside-press backdrop answers a press of any button (#1093).
//!
//! A right or middle press does not click a `data-rid` (#1093): a browser's
//! `click` is primary-only. An overlay's backdrop is a `data-rid` too, and an
//! outside-press dismissal is a `mousedown` of any button (native menus, and
//! Mantine's `useClickOutside`), so every backdrop carries `data-backdrop`,
//! which lets a press of any button through to its handler on both backends.
//!
//! This walks the rendered tree of each overlay kind and demands the marker on
//! every `__backdrop` / `__overlay` node that carries a `data-rid`. The desktop
//! event path is pinned by `rinch/src/app/backdrop_any_button_1093_tests.rs`,
//! the web one by `rinch-web/tests/right_press_click_1093.rs`.
//!
//! The kinds are listed by hand, so a **new** overlay component that forgets
//! the marker is not caught here: add it to the list when you add it.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::context_menu::ContextMenu;
use rinch_components::{Drawer, DropdownMenu, Modal, Popover, Select};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::BACKDROP_ATTRIBUTE;
use rinch_core::{Callback, Component};

fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
    out.push(node.clone());
    for child in node.children() {
        walk(&child, out);
    }
}

/// The backdrops `render` mounts anywhere in the document (the root, or a
/// body portal), as `(class, carries the marker)`.
fn backdrops(render: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Vec<(String, bool)> {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = render(&mut scope);
    scope.body_handle().append_child(&root);
    let mut all = Vec::new();
    walk(&scope.body_handle(), &mut all);
    all.iter()
        .filter(|n| n.get_attribute("data-rid").is_some())
        .filter_map(|n| {
            let class = n.get_attribute("class")?;
            class
                .split_whitespace()
                .any(|c| c.ends_with("__backdrop") || c.ends_with("__overlay"))
                .then(|| (class, n.get_attribute(BACKDROP_ATTRIBUTE).is_some()))
        })
        .collect()
}

#[test]
fn every_overlay_backdrop_answers_any_button() {
    let kinds: Vec<(&str, Vec<(String, bool)>)> = vec![
        (
            "ContextMenu",
            backdrops(|s| {
                let target = s.create_element("div");
                ContextMenu::default().render(s, &[target])
            }),
        ),
        (
            "DropdownMenu",
            backdrops(|s| {
                DropdownMenu {
                    on_close: Some(Callback::new(|| {})),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
        ("Select", backdrops(|s| Select::default().render(s, &[]))),
        (
            "Popover",
            backdrops(|s| {
                Popover {
                    onclose: Some(Callback::new(|| {})),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
        (
            "ColorInput",
            backdrops(|s| ColorInput::default().render(s, &[])),
        ),
        (
            "Modal",
            backdrops(|s| {
                Modal {
                    onclose: Some(Callback::new(|| {})),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
        (
            "Drawer",
            backdrops(|s| {
                Drawer {
                    onclose: Some(Callback::new(|| {})),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
    ];
    for (kind, found) in kinds {
        assert!(
            !found.is_empty(),
            "positive control: {kind} mounted a backdrop carrying a data-rid"
        );
        for (class, marked) in found {
            assert!(
                marked,
                "{kind}'s `{class}` has a data-rid and no `{BACKDROP_ATTRIBUTE}`: \
                 a right or middle press outside it would not dismiss"
            );
        }
    }
}
