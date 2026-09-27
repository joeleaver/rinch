//! A right or middle press does not click (issue #1093).
//!
//! A browser fires `click` for the primary button only: a right press is a
//! `contextmenu`, a middle press an `auxclick`, and neither runs `onclick`.
//! Desktop used to dispatch the `data-rid` under a right or middle press as
//! well — so a right-click on a `Button` ran its `onclick`, and so did an
//! Android long press, which the touch translation delivers as a right press.
//! rinch-web did too, unless a `data-oncontextmenu` was in the ancestry; its
//! twin is `rinch-web/tests/right_press_click_1093.rs`. A backdrop
//! (`data-backdrop`) is the exception: `backdrop_any_button_1093_tests`.
//!
//! `data-onmousedown` / `data-onmouseup` are per-button events and still fire
//! for every button, with that button in the click context; the fixtures use
//! them as the positive control that each press reached the element.

use super::*;
use rinch_core::events::InputCallback;
use std::cell::{Cell, RefCell};

const VP: (u32, u32) = (800, 600);

/// The clickable box, (40,30)-(240,70): off the origin, so a press resolved
/// against the wrong box lands on nothing.
const TARGET: (f32, f32) = (137.0, 51.0);

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn chord(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) -> Vec<AppAction> {
    let mut actions = ev(app, PlatformEvent::MouseDown { x, y, button });
    actions.extend(ev(app, PlatformEvent::MouseUp { x, y, button }));
    actions
}

#[derive(Default)]
struct Log {
    clicks: Cell<u32>,
    downs: RefCell<Vec<events::MouseButton>>,
    ups: RefCell<Vec<events::MouseButton>>,
}

/// One `data-rid` box that also carries `data-onmousedown` / `data-onmouseup`.
/// With `drag_window`, the box sits inside a `data-drag-window` titlebar.
fn mount(drag_window: bool) -> (RinchApp, Rc<Log>) {
    let log = Rc::new(Log::default());
    let log_in = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let parent = scope.create_element("div");
        parent.set_attribute("style", "width: 800px; height: 100px");
        if drag_window {
            parent.set_attribute("data-drag-window", "");
        }
        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 30px; width: 200px; height: 40px",
        );
        let click = scope.register_handler({
            let log = log_in.clone();
            move || log.clicks.set(log.clicks.get() + 1)
        });
        let down = scope.register_handler({
            let log = log_in.clone();
            move || {
                log.downs
                    .borrow_mut()
                    .push(events::get_click_context().button)
            }
        });
        let up = scope.register_handler({
            let log = log_in.clone();
            move || {
                log.ups
                    .borrow_mut()
                    .push(events::get_click_context().button)
            }
        });
        target.set_attribute("data-rid", &click.0.to_string());
        target.set_attribute("data-onmousedown", &down.0.to_string());
        target.set_attribute("data-onmouseup", &up.0.to_string());
        parent.append_child(&target);
        root.append_child(&parent);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, log)
}

#[test]
fn a_right_or_middle_press_does_not_click_a_data_rid() {
    let (mut app, log) = mount(false);

    chord(&mut app, TARGET, MouseButton::Right);
    assert_eq!(
        *log.downs.borrow(),
        vec![events::MouseButton::Right],
        "positive control: the right press reached the element"
    );
    assert_eq!(*log.ups.borrow(), vec![events::MouseButton::Right]);
    assert_eq!(log.clicks.get(), 0, "a right press is not a click");

    chord(&mut app, TARGET, MouseButton::Middle);
    assert_eq!(
        *log.downs.borrow(),
        vec![events::MouseButton::Right, events::MouseButton::Middle],
        "positive control: the middle press reached the element"
    );
    assert_eq!(log.clicks.get(), 0, "nor is a middle press");

    // Positive control: the primary button still clicks, once.
    chord(&mut app, TARGET, MouseButton::Left);
    assert_eq!(log.clicks.get(), 1, "a left press clicks");
    assert_eq!(
        events::get_click_context().button,
        events::MouseButton::Left,
        "and its click context names the left button"
    );
}

/// The `data-rid` still stops the claim walk for a right press: a right press
/// on a titlebar button neither clicks it nor starts a window drag from the
/// titlebar around it.
#[test]
fn a_right_press_on_a_titlebar_button_does_not_drag_the_window() {
    let (mut app, log) = mount(true);

    let actions = chord(&mut app, TARGET, MouseButton::Right);
    assert_eq!(*log.downs.borrow(), vec![events::MouseButton::Right]);
    assert_eq!(log.clicks.get(), 0);
    assert!(
        !actions.iter().any(|a| matches!(a, AppAction::DragWindow)),
        "the right press dragged the window"
    );

    // Positive control: a right press on the titlebar itself, off the button,
    // is the window's (unchanged by #1093).
    let actions = chord(&mut app, (611.0, 43.0), MouseButton::Right);
    assert!(actions.iter().any(|a| matches!(a, AppAction::DragWindow)));
}

/// The #813 text context menu opens for a right press on a field inside a
/// clickable container, and the container is not clicked.
#[test]
fn a_right_press_on_a_field_in_a_clickable_container_opens_the_menu_only() {
    let clicks = Rc::new(Cell::new(0u32));
    let node: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let (clicks_in, node_in) = (clicks.clone(), node.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrap = scope.create_element("div");
        wrap.set_attribute("style", "padding: 13px");
        let rid = scope.register_handler({
            let c = clicks_in.clone();
            move || c.set(c.get() + 1)
        });
        wrap.set_attribute("data-rid", &rid.0.to_string());
        let h = events::register_input_handler(InputCallback::new(|_| {}));
        let f = scope.create_element("input");
        f.set_attribute(
            "style",
            "width: 300px; height: 30px; padding: 0; margin: 0; font-size: 16px; \
             line-height: 20px; font-family: sans-serif",
        );
        f.set_attribute("value", "hello world");
        f.set_attribute("data-oninput", &h.0.to_string());
        wrap.append_child(&f);
        root.append_child(&wrap);
        node_in.set(Some(f.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let field = node.get().unwrap();

    // (13,13)-(313,43): inside the field.
    chord(&mut app, (151.0, 29.0), MouseButton::Right);
    assert!(
        app.is_text_context_menu_open(),
        "the right press opened the menu"
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(field),
        "and focused the field"
    );
    assert_eq!(clicks.get(), 0, "the container was not clicked");

    // Positive control: a left press on the padding clicks the container.
    // (The first left press is swallowed closing the menu.)
    chord(&mut app, (611.0, 457.0), MouseButton::Left);
    assert!(!app.is_text_context_menu_open());
    chord(&mut app, (5.0, 5.0), MouseButton::Left);
    assert_eq!(clicks.get(), 1);
}
