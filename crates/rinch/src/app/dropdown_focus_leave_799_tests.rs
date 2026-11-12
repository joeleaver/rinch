//! Issue #799: an open `DropdownMenu` closes when the focus leaves it — Tab
//! past its last item walked into the page behind the menu and left it open.
//! The WAI-ARIA menu-button pattern closes the menu on Tab and moves on, as
//! does Chrome's own menu on deactivation; the menu here closes on any focus
//! move out of its root (trigger + panel) and on a window blur, the
//! `ColorInput` dropdown's rule (#1572).
//!
//! Driven through `handle_event` under the real stylesheets. The positive
//! control is the Tab *between* items, which keeps the menu open — a menu that
//! closed on every Tab, or on every focus move, fails it.

use super::*;
use rinch_components::{DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget};
use rinch_core::{Callback, Component, Signal};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

fn key(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn focused(app: &RinchApp) -> Option<u64> {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_core::dom::DomDocument::active_element(&*d).map(|n| n.0)
}

struct Fixture {
    app: RinchApp,
    open: Signal<bool>,
    closes: Rc<Cell<u32>>,
    items: [u64; 2],
}

fn mount() -> Fixture {
    let open = Signal::new(true);
    let closes = Rc::new(Cell::new(0u32));
    let items: Rc<Cell<[u64; 2]>> = Rc::new(Cell::new([0, 0]));
    let (c, items_in) = (closes.clone(), items.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let trigger = scope.create_element("button");
        trigger.set_attribute("style", "width: 60px; height: 24px;");
        let target = DropdownMenuTarget.render(scope, &[trigger]);
        let mut kids = Vec::new();
        let mut ids = [0; 2];
        for (i, label) in ["Alpha", "Beta"].into_iter().enumerate() {
            let t = scope.create_text(label);
            let it = DropdownMenuItem {
                onclick: Some(Callback::new(|| {})),
                ..Default::default()
            }
            .render(scope, &[t]);
            ids[i] = it.node_id().0;
            kids.push(it);
        }
        items_in.set(ids);
        let dropdown = DropdownMenuDropdown.render(scope, &kids);
        let c = c.clone();
        let menu = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            close_on_click_outside: false,
            on_close: Some(Callback::new(move || {
                c.set(c.get() + 1);
                open.set(false);
            })),
            ..Default::default()
        }
        .render(scope, &[target, dropdown]);
        let after = scope.create_element("button");
        after.set_attribute("style", "width: 60px; height: 24px; margin-top: 200px;");
        root.append_child(&menu);
        root.append_child(&after);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    Fixture {
        app,
        open,
        closes,
        items: items.get(),
    }
}

#[test]
fn tab_past_the_last_item_closes_the_menu_and_tab_between_items_does_not() {
    let mut f = mount();
    key(&mut f.app, KeyCode::ArrowDown, Modifiers::default());
    assert_eq!(focused(&f.app), Some(f.items[0]), "precondition: on Alpha");

    key(&mut f.app, KeyCode::Tab, Modifiers::default());
    assert_eq!(focused(&f.app), Some(f.items[1]), "Tab reached Beta");
    assert_eq!(f.closes.get(), 0, "focus inside the menu: still open");
    assert!(f.open.get());

    key(&mut f.app, KeyCode::Tab, Modifiers::default());
    assert_eq!(f.closes.get(), 1, "focus left the menu: it closed");
    assert!(!f.open.get());
}

#[test]
fn a_window_blur_closes_the_open_menu() {
    let mut f = mount();
    key(&mut f.app, KeyCode::ArrowDown, Modifiers::default());
    assert_eq!(f.closes.get(), 0);
    f.app
        .handle_event(PlatformEvent::WindowFocus(false), VP, 1.0);
    assert_eq!(f.closes.get(), 1, "Chrome's menus close on deactivation");
}
