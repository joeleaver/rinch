//! #1372: a `position: fixed` box is contained by a transformed ancestor, in a
//! browser and in rinch, so a component whose full-window dismiss backdrop is
//! `fixed` must not sit inside a transformed component box. `Drawer`'s open
//! panel was `translateX(0)` and `Popover`'s centred dropdown
//! `translate(-50%)`: a `DropdownMenu` inside either got a backdrop the size of
//! the panel, and a tap outside the menu closed the drawer or the popover
//! instead (review of #2723). Driven through the real event path under the real
//! theme and component stylesheets.

use super::*;
use rinch_components::{
    Drawer, DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget, Popover,
    PopoverDropdown, PopoverTarget,
};
use rinch_core::{Callback, Component};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
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
    // Twice: an open dropdown is placed after the layout it is measured in.
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

fn recorder() -> (Rc<Cell<usize>>, Callback) {
    let hits: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let h = hits.clone();
    (hits, Callback::new(move || h.set(h.get() + 1)))
}

/// An open `DropdownMenu` with one item, its closes counted.
fn menu(s: &mut RenderScope, on_close: Callback) -> NodeHandle {
    let text = s.create_text("Menu");
    let target = DropdownMenuTarget.render(s, &[text]);
    let label = s.create_text("Item");
    let item = DropdownMenuItem::default().render(s, &[label]);
    let dropdown = DropdownMenuDropdown.render(s, &[item]);
    DropdownMenu {
        opened_fn: Some(Rc::new(|| true)),
        on_close: Some(on_close),
        ..Default::default()
    }
    .render(s, &[target, dropdown])
}

/// An open `Popover` (position `bottom`, centred) holding `inside` in its
/// dropdown, its closes counted.
fn popover(s: &mut RenderScope, onclose: Callback, inside: NodeHandle) -> NodeHandle {
    let button = s.create_element("button");
    button.set_attribute("style", "width: 61px; height: 23px");
    let target = PopoverTarget.render(s, &[button]);
    let dropdown = PopoverDropdown.render(s, &[inside]);
    Popover {
        opened_fn: Some(Rc::new(|| true)),
        onclose: Some(onclose),
        ..Default::default()
    }
    .render(s, &[target, dropdown])
}

/// Every element of `class` that is displayed is laid out over the whole
/// window, and there is at least one.
fn backdrops_cover_the_window(app: &RinchApp, class: &str) {
    let doc = app.doc.as_ref().unwrap().borrow();
    let found = rinch_dom::testing::query_selector(&doc.tree, &format!(".{class}"));
    let shown: Vec<_> = found
        .into_iter()
        .filter(|&id| doc.tree.get(id).is_some_and(|n| n.layout.width > 0.0))
        .collect();
    assert!(!shown.is_empty(), "positive control: a {class} is shown");
    for id in shown {
        let r = rinch_dom::paint::painted_border_box(&doc.tree, id, 1.0);
        assert_eq!(
            (r.x0, r.y0, r.x1, r.y1),
            (0.0, 0.0, 800.0, 600.0),
            "{class} covers the window"
        );
    }
}

/// Is `class` laid out (displayed) anywhere: the panel is open.
fn shown(app: &RinchApp, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap().borrow();
    rinch_dom::testing::query_selector(&doc.tree, &format!(".{class}"))
        .into_iter()
        .any(|id| doc.tree.get(id).is_some_and(|n| n.layout.width > 0.0))
}

#[test]
fn a_tap_on_a_drawers_overlay_closes_the_menu_open_inside_it() {
    let (menu_closes, on_menu) = recorder();
    let (drawer_closes, on_drawer) = recorder();
    let mut app = mount(move |s| {
        let m = menu(s, on_menu);
        Drawer {
            opened_fn: Some(Rc::new(|| true)),
            onclose: Some(on_drawer),
            position: "right".into(),
            ..Default::default()
        }
        .render(s, &[m])
    });
    assert!(shown(&app, "rinch-drawer"), "positive control: open");
    backdrops_cover_the_window(&app, "rinch-dropdown-menu__backdrop");
    // On the dimmed overlay, left of the right-hand panel.
    press(&mut app, (97.0, 301.0));
    assert_eq!(
        (menu_closes.get(), drawer_closes.get()),
        (1, 0),
        "the menu closes, the drawer does not"
    );
}

#[test]
fn a_tap_outside_a_menu_in_a_popover_closes_only_the_menu() {
    let (menu_closes, on_menu) = recorder();
    let (popover_closes, on_popover) = recorder();
    let mut app = mount(move |s| {
        let m = menu(s, on_menu);
        let wrap = s.create_element("div");
        wrap.set_attribute("style", "padding: 101px 0 0 301px");
        let p = popover(s, on_popover, m);
        wrap.append_child(&p);
        wrap
    });
    assert!(
        shown(&app, "rinch-popover__dropdown"),
        "positive control: open"
    );
    backdrops_cover_the_window(&app, "rinch-dropdown-menu__backdrop");
    press(&mut app, (781.0, 577.0));
    assert_eq!(
        (menu_closes.get(), popover_closes.get()),
        (1, 0),
        "the menu closes, the popover does not"
    );
}

#[test]
fn a_tap_outside_nested_popovers_closes_only_the_inner_one() {
    let (inner_closes, on_inner) = recorder();
    let (outer_closes, on_outer) = recorder();
    let mut app = mount(move |s| {
        let text = s.create_text("Inner content");
        let inner = popover(s, on_inner, text);
        let wrap = s.create_element("div");
        wrap.set_attribute("style", "padding: 101px 0 0 301px");
        let outer = popover(s, on_outer, inner);
        wrap.append_child(&outer);
        wrap
    });
    backdrops_cover_the_window(&app, "rinch-popover__backdrop");
    press(&mut app, (781.0, 577.0));
    assert_eq!(
        (inner_closes.get(), outer_closes.get()),
        (1, 0),
        "the inner popover closes, the outer does not"
    );
}

/// The centred dropdown is still centred under its target, by `floating.rs`
/// now instead of a `translate(-50%)`.
#[test]
fn a_centred_popover_dropdown_is_centred_without_a_transform() {
    let (_c, on) = recorder();
    let app = mount(move |s| {
        let inside = s.create_element("div");
        inside.set_attribute("style", "width: 171px; height: 37px");
        let wrap = s.create_element("div");
        wrap.set_attribute("style", "padding: 101px 0 0 301px");
        let p = popover(s, on, inside);
        wrap.append_child(&p);
        wrap
    });
    let doc = app.doc.as_ref().unwrap().borrow();
    let target = rinch_dom::testing::query_selector(&doc.tree, ".rinch-popover")[0];
    let dd = rinch_dom::testing::query_selector(&doc.tree, ".rinch-popover__dropdown")[0];
    let t = rinch_dom::paint::painted_border_box(&doc.tree, target, 1.0);
    let d = rinch_dom::paint::painted_border_box(&doc.tree, dd, 1.0);
    assert!(
        !doc.tree.get(dd).unwrap().has_transform(),
        "no transform on the dropdown"
    );
    let (tc, dc) = ((t.x0 + t.x1) / 2.0, (d.x0 + d.x1) / 2.0);
    assert!(
        (tc - dc).abs() <= 1.0,
        "centred: target {t:?}, dropdown {d:?}"
    );
    assert!(d.y0 > t.y1, "below the target: {t:?} {d:?}");
}

/// A centred left or right dropdown whose content grows while it is open
/// stays centred on its target: `floating.rs` places it again from its new
/// height, as the `translateY(-50%)` it replaced re-centred it (re-review of
/// #2723). It used to keep the `top` measured at open, so 100px of growth
/// ended 50px below centre.
#[test]
fn a_side_popover_stays_centred_when_its_content_grows() {
    for position in ["right", "left"] {
        let (_c, on) = recorder();
        let content: Rc<std::cell::RefCell<Option<NodeHandle>>> = Rc::default();
        let content_in = content.clone();
        let mut app = mount(move |s| {
            let inside = s.create_element("div");
            inside.set_attribute("style", "width: 171px; height: 37px");
            *content_in.borrow_mut() = Some(inside.clone());
            let button = s.create_element("button");
            button.set_attribute("style", "width: 61px; height: 23px");
            let target = PopoverTarget.render(s, &[button]);
            let dropdown = PopoverDropdown.render(s, &[inside]);
            let p = Popover {
                opened_fn: Some(Rc::new(|| true)),
                onclose: Some(on),
                position: position.into(),
                ..Default::default()
            }
            .render(s, &[target, dropdown]);
            let wrap = s.create_element("div");
            wrap.set_attribute("style", "padding: 233px 0 0 301px");
            wrap.append_child(&p);
            wrap
        });
        let centres = |app: &RinchApp| {
            let doc = app.doc.as_ref().unwrap().borrow();
            let target = rinch_dom::testing::query_selector(&doc.tree, ".rinch-popover")[0];
            let dd = rinch_dom::testing::query_selector(&doc.tree, ".rinch-popover__dropdown")[0];
            let t = rinch_dom::paint::painted_border_box(&doc.tree, target, 1.0);
            let d = rinch_dom::paint::painted_border_box(&doc.tree, dd, 1.0);
            ((t.y0 + t.y1) / 2.0, (d.y0 + d.y1) / 2.0, d.height())
        };
        let (tc, dc, h0) = centres(&app);
        assert!(
            (tc - dc).abs() <= 1.0,
            "{position}: centred at open: {tc} {dc}"
        );
        content
            .borrow()
            .as_ref()
            .unwrap()
            .set_style("height", "137px");
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        let (tc, dc, h1) = centres(&app);
        assert!(
            h1 >= h0 + 99.0,
            "{position}: positive control: it grew ({h0} -> {h1})"
        );
        assert!(
            (tc - dc).abs() <= 1.0,
            "{position}: still centred after growing: target {tc}, dropdown {dc}"
        );
    }
}
