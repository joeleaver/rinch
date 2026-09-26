//! Issue #1111: a native `<select>` is opened and operated by the **primary**
//! button only, as in a browser — a right press there is the context menu's
//! gesture and a middle press is autoscroll/paste, neither of which opens the
//! list or picks from it.
//!
//! The open popup's *dismissal* is the other half and is deliberately left on
//! every button: a right or middle press **outside** an open list closes it,
//! which is what a browser does too (the list is a transient popup that any
//! press elsewhere takes down).
//!
//! Driven through `handle_event` with a real `MouseDown`/`MouseUp` pair, so the
//! press takes the same route a mouse does, `data-oncontextmenu` and the text
//! context menu gesture included.

use super::*;
use std::cell::{Cell, RefCell};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

struct Fixture {
    app: RinchApp,
    select: usize,
    /// Every value the select's `oninput` reported.
    picks: Rc<RefCell<Vec<String>>>,
}

/// A 200x30 `<select>` with three options, starting on `one`, at a known place
/// in the window, with a sibling block below it to press on "outside".
fn mount() -> Fixture {
    let sel: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let sel_in = sel.clone();
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let picks_in = picks.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 20px; line-height: 20px; font-size: 16px");
        let select = scope.create_element("select");
        select.set_attribute("style", "width: 200px; height: 30px");
        for label in ["one", "two", "three"] {
            let opt = scope.create_element("option");
            opt.set_attribute("value", label);
            let t = scope.create_text(label);
            opt.append_child(&t);
            select.append_child(&opt);
        }
        let hid = scope.register_input_handler(move |v| picks_in.borrow_mut().push(v));
        select.set_attribute("data-oninput", &hid.0.to_string());
        sel_in.set(Some(select.node_id().0));
        root.append_child(&select);
        let below = scope.create_element("div");
        below.set_attribute("style", "height: 300px");
        root.append_child(&below);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let select = sel.get().expect("the select's node id");
    Fixture { app, select, picks }
}

fn press(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) {
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, (800, 600), 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, (800, 600), 1.0);
}

fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, node_id);
    assert!(w > 0.0 && h > 0.0, "node {node_id} has no box to aim at");
    (x + w / 2.0, y + h / 2.0)
}

/// Centre of option `i` in the open popup.
fn option_centre(app: &RinchApp, i: usize) -> (f32, f32) {
    let id = app.open_select.as_ref().expect("popup open").option_ids[i];
    centre(app, id)
}

/// A press well away from both the control and the popup.
const OUTSIDE: (f32, f32) = (600.0, 450.0);

// ── A closed select ──────────────────────────────────────────────────────────

/// The positive control: the same press, on the same point, with the primary
/// button opens the list. Without it the two refusals below would pass against
/// a fixture whose press missed the control.
#[test]
fn a_left_press_opens_a_closed_select() {
    let mut f = mount();
    let (x, y) = centre(&f.app, f.select);
    press(&mut f.app, x, y, MouseButton::Left);
    assert!(f.app.is_select_open(), "a primary press opens the list");
}

#[test]
fn a_right_press_does_not_open_a_closed_select() {
    let mut f = mount();
    let (x, y) = centre(&f.app, f.select);
    press(&mut f.app, x, y, MouseButton::Right);
    assert!(
        !f.app.is_select_open(),
        "a right press is the context menu's, not the list's"
    );
}

#[test]
fn a_middle_press_does_not_open_a_closed_select() {
    let mut f = mount();
    let (x, y) = centre(&f.app, f.select);
    press(&mut f.app, x, y, MouseButton::Middle);
    assert!(
        !f.app.is_select_open(),
        "a middle press does not open the list"
    );
}

// ── An open select ───────────────────────────────────────────────────────────

/// Open the list with a primary press, as a user would.
fn open(f: &mut Fixture) {
    let (x, y) = centre(&f.app, f.select);
    press(&mut f.app, x, y, MouseButton::Left);
    assert!(f.app.is_select_open(), "precondition: the list is open");
}

/// Positive control for the two below: a primary press on option `two` picks
/// it. Aimed at an option that is **not** the current one, so a pick is a
/// visible change and not a re-pick that looks like nothing happened.
#[test]
fn a_left_press_on_an_option_picks_it() {
    let mut f = mount();
    open(&mut f);
    let (x, y) = option_centre(&f.app, 1);
    press(&mut f.app, x, y, MouseButton::Left);
    assert_eq!(*f.picks.borrow(), vec!["two".to_string()]);
    assert!(!f.app.is_select_open(), "a pick closes the list");
}

#[test]
fn a_right_press_on_an_option_picks_nothing_and_leaves_the_list_open() {
    let mut f = mount();
    open(&mut f);
    let (x, y) = option_centre(&f.app, 1);
    press(&mut f.app, x, y, MouseButton::Right);
    assert!(f.picks.borrow().is_empty(), "a right press picks nothing");
    assert!(
        f.app.is_select_open(),
        "a press inside the list does not dismiss it"
    );
    // And the list still works for the primary button afterwards.
    let (x, y) = option_centre(&f.app, 2);
    press(&mut f.app, x, y, MouseButton::Left);
    assert_eq!(*f.picks.borrow(), vec!["three".to_string()]);
}

#[test]
fn a_middle_press_on_an_option_picks_nothing_and_leaves_the_list_open() {
    let mut f = mount();
    open(&mut f);
    let (x, y) = option_centre(&f.app, 1);
    press(&mut f.app, x, y, MouseButton::Middle);
    assert!(f.picks.borrow().is_empty(), "a middle press picks nothing");
    assert!(f.app.is_select_open());
}

/// The half that stays on every button: a press outside takes the list down.
#[test]
fn a_right_or_middle_press_outside_an_open_select_still_dismisses_it() {
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        let mut f = mount();
        open(&mut f);
        press(&mut f.app, OUTSIDE.0, OUTSIDE.1, button);
        assert!(
            !f.app.is_select_open(),
            "{button:?} press outside dismisses the list"
        );
        assert!(f.picks.borrow().is_empty(), "dismissing picks nothing");
    }
}
