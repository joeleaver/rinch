//! A `ContextMenu` stays inside the window (#1761).
//!
//! The menu hangs from the pointer, or stands on it when there is little room
//! below. Until #1761 nothing moved it back once it was laid out, so a 430px
//! menu opened at y = 70 in a 480px window ran 24px past the bottom edge and
//! its last item was cut off. As in Chrome and native menus it is now shifted
//! up (and left) by its overflow, kept `EDGE_MARGIN` (4px) from the edges,
//! and a menu taller than the whole window scrolls.
//!
//! Driven through the real event path under the real theme and component
//! stylesheets. Every item height is declared, so nothing here depends on the
//! host's fonts.

use super::*;
use rinch_components::context_menu::ContextMenu;
use rinch_core::{Component, Signal};
use std::cell::Cell;

const VP: (u32, u32) = (800, 480);
const EDGE: f32 = 4.0;

fn right_press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Right;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn left_press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

/// A window-filling target and a menu of `items` rows, each `item_w` x 30px.
/// The dropdown adds the theme's `xs` padding (10px) top and bottom and a 1px
/// border, so its border box is `items * 30 + 22` tall and at least 180 wide.
fn mount(items: usize, item_w: f32) -> (RinchApp, Signal<bool>) {
    let opened: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let opened_in = opened.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 480px");
        let target = scope.create_element("div");
        target.set_attribute("style", "width: 800px; height: 480px");
        let list = scope.create_element("div");
        for i in 0..items {
            let row = scope.create_element("div");
            row.set_attribute("class", &format!("row-{i}"));
            row.set_attribute("style", &format!("width: {item_w}px; height: 30px"));
            list.append_child(&row);
        }
        let cm = ContextMenu::default();
        opened_in.set(Some(cm.opened));
        let node = cm.render(scope, &[target, list]);
        root.append_child(&node);
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
    (app, opened.get().unwrap())
}

fn find_by_class(app: &RinchApp, class: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .expect(class)
}

/// The painted box of the node carrying `class`: `(left, top, right, bottom)`.
fn painted(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let id = find_by_class(app, class);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = super::hit_testing::painted_element_box(&d.tree, id);
    (x, y, x + w, y + h)
}

fn menu(app: &RinchApp) -> (f32, f32, f32, f32) {
    painted(app, "rinch-context-menu__dropdown")
}

/// The issue's case: 14 rows (442px) opened at y = 70 in a 480px window.
/// Hung from the pointer it ends at 512, 36px past the edge; shifted up it
/// ends 4px from the bottom and starts at 34.
#[test]
fn a_menu_running_off_the_bottom_is_shifted_up_by_its_overflow() {
    let (mut app, opened) = mount(14, 120.0);
    right_press(&mut app, (53.0, 70.0));
    assert!(opened.get(), "positive control: the right press opened it");
    let (l, t, _r, b) = menu(&app);
    assert_eq!(b - t, 442.0, "the declared height: {t}..{b}");
    assert_eq!(b, VP.1 as f32 - EDGE, "ends EDGE_MARGIN above the bottom");
    assert_eq!(t, 34.0, "shifted up by exactly its 36px overflow");
    assert_eq!(l, 53.0, "and not moved sideways");
}

/// A menu wider than the room right of the pointer, where the room is still
/// enough for its minimum width (so it is anchored by `left`), is shifted left
/// by its overflow.
#[test]
fn a_menu_running_off_the_right_is_shifted_left_by_its_overflow() {
    let (mut app, opened) = mount(3, 300.0);
    right_press(&mut app, (600.0, 70.0));
    assert!(opened.get(), "positive control: the right press opened it");
    let (l, t, r, _b) = menu(&app);
    // 300px of rows plus a 1px border each side.
    assert_eq!(r - l, 302.0, "{l}..{r}");
    assert_eq!(r, VP.0 as f32 - EDGE, "ends EDGE_MARGIN left of the edge");
    assert_eq!(l, 494.0, "shifted left by exactly its 106px overflow");
    assert_eq!(t, 70.0, "and not moved vertically");
}

/// A menu that fits is exactly where it always was, and the open settles in
/// one layout: a second resolve at the same size computes nothing.
#[test]
fn a_menu_that_fits_is_unchanged_and_settles_at_once() {
    let (mut app, opened) = mount(3, 120.0);
    right_press(&mut app, (53.0, 70.0));
    assert!(opened.get());
    assert_eq!(menu(&app), (53.0, 70.0, 233.0, 182.0));
    // The frame around the menu takes no pointer events, the menu does: a
    // press on it is the menu's, not the overlay's.
    left_press(&mut app, (100.0, 120.0));
    assert!(
        opened.get(),
        "a press on the menu reached the dismiss overlay"
    );
    // A press near the bottom still stands on the pointer.
    left_press(&mut app, (781.0, 20.0));
    assert!(
        !opened.get(),
        "positive control: the outside press closed it"
    );
    right_press(&mut app, (53.0, 400.0));
    assert_eq!(menu(&app), (53.0, 288.0, 233.0, 400.0));

    let computes = {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow().tree.taffy_computes
    };
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let doc = app.doc.as_ref().unwrap();
    assert_eq!(
        doc.borrow().tree.taffy_computes,
        computes,
        "no second layout after the open"
    );
}

/// Re-opening at another point starts from that point's placement, not from
/// the previous open's shift: at y = 30 the same 442px menu fits (it ends at
/// 472) and hangs from the pointer unmoved.
#[test]
fn reopening_at_a_new_point_starts_from_the_new_placement() {
    let (mut app, opened) = mount(14, 120.0);
    right_press(&mut app, (53.0, 70.0));
    assert_eq!(menu(&app).1, 34.0, "first open shifted up");
    left_press(&mut app, (781.0, 20.0));
    assert!(
        !opened.get(),
        "positive control: the outside press closed it"
    );
    right_press(&mut app, (90.0, 30.0));
    assert!(opened.get());
    assert_eq!(menu(&app), (90.0, 30.0, 270.0, 472.0));
}

/// A menu that stands on the pointer (little room below) and is taller than
/// the room above is shifted down to EDGE_MARGIN.
#[test]
fn a_menu_standing_on_the_pointer_is_shifted_down_into_the_window() {
    let (mut app, _opened) = mount(14, 120.0);
    right_press(&mut app, (53.0, 300.0));
    let (_l, t, _r, b) = menu(&app);
    assert_eq!(t, EDGE, "starts EDGE_MARGIN below the top");
    assert_eq!(b, 446.0, "shifted down by its 146px overflow");
}

/// A menu taller than the whole window (20 rows, 622px) starts at
/// EDGE_MARGIN and scrolls: a wheel over it brings its last row into view.
#[test]
fn a_menu_taller_than_the_window_scrolls() {
    let (mut app, opened) = mount(20, 120.0);
    right_press(&mut app, (53.0, 70.0));
    assert!(opened.get());
    let (_l, t, _r, b) = menu(&app);
    assert_eq!(t, EDGE, "starts EDGE_MARGIN below the top");
    assert_eq!(b - t, 622.0);
    let last = painted(&app, "row-19");
    assert!(
        last.3 > VP.1 as f32,
        "positive control: the last row is off-screen"
    );

    for _ in 0..5 {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x: 100.0,
                y: 200.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
            VP,
            1.0,
        );
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
    let last = painted(&app, "row-19");
    assert!(
        last.3 <= VP.1 as f32 && last.1 >= 0.0,
        "the wheel brought the last row into view: {last:?}"
    );
    let (_l, t, _r, b) = menu(&app);
    assert!(t < 0.0, "the menu itself scrolled: {t}..{b}");
}
