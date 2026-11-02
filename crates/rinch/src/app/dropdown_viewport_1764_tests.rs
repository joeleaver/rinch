//! Issue #1764: a `DropdownMenu` or `Popover` dropdown opened near the
//! window's bottom or right edge ran past it. Measured on main (800x480, a
//! 40x20 trigger at (700, 430), ten 200x30 rows): `DropdownMenu` painted at
//! (700, 454)–(922, 776), `Popover` at (607, 458)–(833, 784).
//!
//! Now each open measures the dropdown and keeps it inside the viewport less
//! `floating::EDGE_MARGIN` (8px): flipped to the side that fits, shifted on
//! the cross axis, capped and scrolling when it fits on neither side. On
//! desktop the measurement is the layout's, so the placement is written after
//! that layout and laid out again in the same frame
//! (`RinchApp::settle_bounds_writes`): no painted frame shows it elsewhere.
//!
//! Every geometry here is from declared sizes (rows `width`/`height`, no
//! text), so it does not depend on the host's fonts.

use super::*;

use rinch_components::{
    Component, DropdownMenu, DropdownMenuDropdown, DropdownMenuTarget, Popover, PopoverDropdown,
    PopoverTarget,
};
use rinch_core::Signal;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::reactive::ElementBounds;
use std::rc::Rc;

const VW: f32 = 800.0;
const VH: f32 = 480.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Menu,
    Popover,
}

/// A trigger box at `(x, y)` (40x20) holding a `kind` whose dropdown holds
/// `rows` 200x30 rows, opened by `open`.
fn mount(kind: Kind, x: f32, y: f32, rows: usize, open: Signal<bool>, position: &str) -> RinchApp {
    mount_styled(kind, x, y, rows, open, position, None)
}

/// [`mount`], with `dd_style` as the dropdown's own inline style (what a
/// caller's `style:` on `DropdownMenuDropdown` / `PopoverDropdown` writes).
fn mount_styled(
    kind: Kind,
    x: f32,
    y: f32,
    rows: usize,
    open: Signal<bool>,
    position: &str,
    dd_style: Option<&'static str>,
) -> RinchApp {
    let position = position.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 40px; height: 20px;");
        let mut items = Vec::new();
        for _ in 0..rows {
            let row = scope.create_element("div");
            row.set_attribute("style", "width: 200px; height: 30px;");
            items.push(row);
        }
        let opened_fn: Rc<dyn Fn() -> bool> = Rc::new(move || open.get());
        let comp = match kind {
            Kind::Menu => {
                let target = DropdownMenuTarget.render(scope, &[trigger]);
                let dropdown = DropdownMenuDropdown.render(scope, &items);
                if let Some(st) = dd_style {
                    dropdown.set_attribute("style", st);
                }
                DropdownMenu {
                    opened_fn: Some(opened_fn),
                    position: position.clone(),
                    ..Default::default()
                }
                .render(scope, &[target, dropdown])
            }
            Kind::Popover => {
                let target = PopoverTarget.render(scope, &[trigger]);
                let dropdown = PopoverDropdown.render(scope, &items);
                if let Some(st) = dd_style {
                    dropdown.set_attribute("style", st);
                }
                Popover {
                    opened_fn: Some(opened_fn),
                    position: position.clone(),
                    ..Default::default()
                }
                .render(scope, &[target, dropdown])
            }
        };
        let wrapper = scope.create_element("div");
        wrapper.set_attribute(
            "style",
            &format!("position: absolute; left: {x}px; top: {y}px; line-height: 0;"),
        );
        wrapper.append_child(&comp);
        wrapper
    });
    app.mount_component(VW, VH);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VW, VH);
    app
}

fn find(app: &RinchApp, class: &str) -> u64 {
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

fn dropdown_class(kind: Kind) -> &'static str {
    match kind {
        Kind::Menu => "rinch-dropdown-menu__dropdown",
        Kind::Popover => "rinch-popover__dropdown",
    }
}

/// The dropdown's painted box after one frame, and its opacity then.
fn painted(app: &RinchApp, kind: Kind) -> (ElementBounds, f32) {
    let id = find(app, dropdown_class(kind));
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    let b = d.bounding_rect(NodeId(id)).expect("in the document");
    (b, d.tree.nodes[id].computed_style.opacity)
}

fn assert_close(got: ElementBounds, want: (f32, f32, f32, f32), what: &str) {
    let ok = (got.x - want.0).abs() < 0.6
        && (got.y - want.1).abs() < 0.6
        && (got.width - want.2).abs() < 0.6
        && (got.height - want.3).abs() < 0.6;
    assert!(ok, "{what}: painted {got:?}, want {want:?}");
}

fn inside_viewport(b: ElementBounds, what: &str) {
    assert!(
        b.x >= 8.0 - 0.5 && b.y >= 8.0 - 0.5,
        "{what}: {b:?} starts outside the margin"
    );
    assert!(
        b.x + b.width <= VW - 8.0 + 0.5 && b.y + b.height <= VH - 8.0 + 0.5,
        "{what}: {b:?} ends outside the margin"
    );
}

/// The issue's DropdownMenu: opened from the start, it flips up (there is
/// room above, none below) and shifts left to end 8px from the right edge.
#[test]
fn a_menu_at_the_bottom_right_corner_flips_up_and_shifts_left() {
    let open = Signal::new(true);
    let app = mount(Kind::Menu, 700.0, 430.0, 10, open, "");
    let (b, opacity) = painted(&app, Kind::Menu);
    // 222x322: ten 200px rows + the panel's padding and border. Its bottom
    // sits the 4px offset above the trigger's top.
    assert_close(
        b,
        (800.0 - 8.0 - 222.0, 430.0 - 4.0 - 322.0, 222.0, 322.0),
        "menu",
    );
    assert_eq!(opacity, 1.0, "revealed in the same frame it was placed");
    // It fits above, so it is flipped, not capped: no scroll container.
    let id = find(&app, "rinch-dropdown-menu__dropdown");
    let doc = app.doc.as_ref().unwrap();
    let style = doc.borrow().tree.nodes[id].attributes.get("style").cloned();
    assert!(
        !style.as_deref().unwrap_or("").contains("max-height"),
        "flipped, not capped: {style:?}"
    );
}

/// The issue's Popover (default `bottom`, centred): flipped up by its 8px
/// offset, shifted left.
#[test]
fn a_popover_at_the_bottom_right_corner_flips_up_and_shifts_left() {
    let open = Signal::new(true);
    let app = mount(Kind::Popover, 700.0, 430.0, 10, open, "");
    let (b, opacity) = painted(&app, Kind::Popover);
    assert_close(
        b,
        (800.0 - 8.0 - 226.0, 430.0 - 8.0 - 326.0, 226.0, 326.0),
        "popover",
    );
    // Its own 150ms fade has started (the measuring `opacity: 0` is gone).
    let _ = opacity;
    let id = find(&app, "rinch-popover__dropdown");
    let doc = app.doc.as_ref().unwrap();
    let style = doc.borrow().tree.nodes[id].attributes.get("style").cloned();
    assert!(
        !style.as_deref().unwrap_or("").contains("opacity"),
        "revealed in the frame it was placed: {style:?}"
    );
}

/// Opened by a write after mount (the real open path): placed in the frame
/// that opens it, never painted where it overflows.
#[test]
fn a_menu_opened_later_is_placed_in_its_first_frame() {
    let open = Signal::new(false);
    let mut app = mount(Kind::Menu, 700.0, 430.0, 10, open, "bottom-start");
    open.set(true);
    app.resolve_and_repaint(VW, VH);
    let (b, opacity) = painted(&app, Kind::Menu);
    inside_viewport(b, "menu opened later");
    assert_eq!(opacity, 1.0);
    // Closed and opened again: placed again, from its own size.
    open.set(false);
    app.resolve_and_repaint(VW, VH);
    open.set(true);
    app.resolve_and_repaint(VW, VH);
    let (b2, _) = painted(&app, Kind::Menu);
    assert_close(b2, (b.x, b.y, b.width, b.height), "reopened");
}

/// A dropdown that fits below its trigger is where it always was.
#[test]
fn a_dropdown_that_fits_is_not_moved() {
    let open = Signal::new(true);
    let app = mount(Kind::Menu, 120.0, 40.0, 3, open, "");
    let (b, _) = painted(&app, Kind::Menu);
    assert_close(b, (120.0, 40.0 + 20.0 + 4.0, 222.0, 112.0), "menu below");
    let app = mount(Kind::Popover, 300.0, 40.0, 3, open, "");
    let (b, _) = painted(&app, Kind::Popover);
    assert_close(
        b,
        (300.0 + 20.0 - 113.0, 40.0 + 20.0 + 8.0, 226.0, 116.0),
        "popover below",
    );
}

/// A preferred `top` with no room above opens below.
#[test]
fn a_top_popover_without_room_above_flips_down() {
    let open = Signal::new(true);
    let app = mount(Kind::Popover, 300.0, 30.0, 3, open, "top");
    let (b, _) = painted(&app, Kind::Popover);
    assert_close(
        b,
        (300.0 + 20.0 - 113.0, 30.0 + 20.0 + 8.0, 226.0, 116.0),
        "flipped down",
    );
}

/// Taller than the room on either side: on the side with more room, capped
/// to it, and scrolling.
#[test]
fn a_menu_taller_than_both_sides_is_capped_and_scrolls() {
    let open = Signal::new(true);
    // 200 rows above, 256 below (480 - 8 - (200 + 20 + 4)).
    let app = mount(Kind::Menu, 100.0, 200.0, 20, open, "");
    let (b, _) = painted(&app, Kind::Menu);
    let room_below = VH - 8.0 - (200.0 + 20.0 + 4.0);
    assert_close(b, (100.0, 224.0, 222.0, room_below), "capped below");
    let id = find(&app, "rinch-dropdown-menu__dropdown");
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let style = &d.tree.nodes[id].computed_style;
    assert!(
        format!("{:?}", style.overflow_y).contains("Auto"),
        "the capped panel scrolls: {:?}",
        style.overflow_y
    );
}

/// A menu nested in an outer menu's panel: the outer one flips up, the inner
/// one opens below its own trigger. The outer root's `--top` class must not
/// pull the inner panel up through the descendant position rules.
#[test]
fn a_nested_menu_opens_below_its_own_trigger_when_the_outer_flipped() {
    let open = Signal::new(true);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let inner_trigger = scope.create_element("div");
        inner_trigger.set_attribute("style", "width: 40px; height: 20px;");
        let inner_row = scope.create_element("div");
        inner_row.set_attribute("style", "width: 100px; height: 30px;");
        let inner_target = DropdownMenuTarget.render(scope, &[inner_trigger]);
        let inner_dd = DropdownMenuDropdown.render(scope, &[inner_row]);
        let inner = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dd]);
        inner.set_attribute("id", "inner");
        let filler = scope.create_element("div");
        filler.set_attribute("style", "width: 200px; height: 200px;");
        let outer_trigger = scope.create_element("div");
        outer_trigger.set_attribute("style", "width: 40px; height: 20px;");
        let outer_target = DropdownMenuTarget.render(scope, &[outer_trigger]);
        let outer_dd = DropdownMenuDropdown.render(scope, &[inner, filler]);
        let outer = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dd]);
        let wrapper = scope.create_element("div");
        wrapper.set_attribute(
            "style",
            "position: absolute; left: 100px; top: 400px; line-height: 0;",
        );
        wrapper.append_child(&outer);
        wrapper
    });
    app.mount_component(VW, VH);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VW, VH);
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    let panels: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes.get("class").is_some_and(|c| {
                c.split_whitespace()
                    .any(|one| one == "rinch-dropdown-menu__dropdown")
            })
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(panels.len(), 2);
    let inner_root = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").is_some_and(|v| v == "inner"))
        .map(|(id, _)| id)
        .unwrap();
    let inner_root_box = d.bounding_rect(NodeId(inner_root)).unwrap();
    let (outer_panel, inner_panel) = {
        let a = d.bounding_rect(NodeId(panels[0])).unwrap();
        let b = d.bounding_rect(NodeId(panels[1])).unwrap();
        if a.height > b.height { (a, b) } else { (b, a) }
    };
    inside_viewport(outer_panel, "outer");
    assert!(
        outer_panel.y + outer_panel.height <= 400.0,
        "the outer menu flipped up: {outer_panel:?}"
    );
    assert!(
        (inner_panel.x - inner_root_box.x).abs() < 0.6
            && (inner_panel.y - (inner_root_box.y + inner_root_box.height + 4.0)).abs() < 0.6,
        "inner below its own trigger: panel {inner_panel:?}, trigger {inner_root_box:?}"
    );
}

fn inline_style(app: &RinchApp, kind: Kind) -> String {
    let id = find(app, dropdown_class(kind));
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id]
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default()
}

/// Review of #1812, F1: a caller's own `max-height` on the dropdown survives
/// the open — it fits, so nothing is capped and the caller's 100px holds.
#[test]
fn a_callers_max_height_survives_an_open_where_it_fits() {
    let open = Signal::new(false);
    let mut app = mount_styled(
        Kind::Menu,
        120.0,
        40.0,
        10,
        open,
        "",
        Some("max-height: 100px; overflow-y: auto"),
    );
    open.set(true);
    app.resolve_and_repaint(VW, VH);
    let (b, _) = painted(&app, Kind::Menu);
    assert_close(b, (120.0, 64.0, 222.0, 100.0), "capped by the caller");
    let style = inline_style(&app, Kind::Menu);
    assert!(
        style.contains("max-height: 100px") && style.contains("overflow-y: auto"),
        "the caller's declarations are still there: {style:?}"
    );
}

/// A cap below the caller's own is the smaller of the two: room 248 under a
/// caller's 400px, and the caller's 400px comes back on the next open.
#[test]
fn a_cap_is_the_smaller_of_the_callers_and_the_room() {
    let open = Signal::new(true);
    let mut app = mount_styled(
        Kind::Menu,
        100.0,
        200.0,
        20,
        open,
        "",
        Some("max-height: 400px; overflow-y: auto"),
    );
    let (b, _) = painted(&app, Kind::Menu);
    // 400 fits neither above (184) nor below (248): capped to the room below.
    assert_close(b, (100.0, 224.0, 222.0, 248.0), "the room wins");
    open.set(false);
    app.resolve_and_repaint(VW, VH);
    let style = inline_style(&app, Kind::Menu);
    assert!(
        style.contains("max-height: 248px"),
        "still placed while closed: {style:?}"
    );
    open.set(true);
    app.resolve_and_repaint(VW + 1.0, VH);
    let style = inline_style(&app, Kind::Menu);
    assert!(
        style.contains("overflow-y: auto") && !style.contains("max-height: 400px"),
        "capped again on reopen: {style:?}"
    );
}

/// A caller's `width` on a popover's dropdown survives the second open (the
/// first open's pinned width used to be wiped, and the caller's with it).
#[test]
fn a_callers_width_survives_reopening() {
    let open = Signal::new(false);
    let mut app = mount_styled(
        Kind::Popover,
        300.0,
        40.0,
        3,
        open,
        "",
        Some("width: 300px"),
    );
    for frame in 0..3 {
        open.set(true);
        app.resolve_and_repaint(VW + frame as f32, VH);
        let (b, _) = painted(&app, Kind::Popover);
        assert!((b.width - 300.0).abs() < 0.6, "open {frame}: {b:?}");
        open.set(false);
        app.resolve_and_repaint(VW + frame as f32 + 0.5, VH);
    }
    let style = inline_style(&app, Kind::Popover);
    assert!(style.contains("width: 300px"), "{style:?}");
}
