//! A right or middle press focuses what it lands on (issue #452).
//!
//! A browser's mousedown focuses the nearest focusable ancestor of the hit node
//! whatever the button. Measured in Chrome 153 (Linux) with trusted input
//! (CDP `Input.dispatchMouseEvent`) on a `tabindex="0"` listbox holding a
//! `tabindex="-1"` item, a plain child and a `<button>`:
//!
//! | press (after a left press on the listbox) | `document.activeElement` |
//! |---|---|
//! | right on the item | the item |
//! | middle on the item | the item |
//! | right on the plain child | the listbox |
//! | right on the button | the button |
//! | right outside every focusable | `<body>` |
//! | right on a node whose `mousedown` is `preventDefault`ed | unchanged |
//!
//! Desktop's right/middle path used to run no claim at all: its release check
//! resolved the press to the item, saw it was not the claim holder, released
//! the listbox, and nothing took the keyboard — arrow keys dead until the next
//! left press or Tab. The press now runs the same claim a left press does
//! (`RinchApp::claim_press_focus`), so the release check agrees with it by
//! construction. `:active` stays a primary-button state, as in a browser.

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use std::cell::{Cell, RefCell};

const VP: (u32, u32) = (800, 600);

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, w, h) = painted_element_box(&d.tree, id);
    (ax + w / 2.0, ay + h / 2.0)
}

fn down(app: &mut RinchApp, id: usize, button: MouseButton) {
    let (x, y) = center(app, id);
    ev(app, PlatformEvent::MouseDown { x, y, button });
}

fn press(app: &mut RinchApp, id: usize, button: MouseButton) {
    let (x, y) = center(app, id);
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

struct Fixture {
    app: RinchApp,
    /// `tabindex="0"`, registered as a focus target.
    listbox: usize,
    /// `tabindex="-1"` inside the listbox, registered as a focus target.
    item: usize,
    /// No `tabindex`, inside the listbox.
    plain: usize,
    /// A `data-nofocus` `tabindex="0"` node outside the listbox.
    nofocus: usize,
    /// Nothing focusable, outside the listbox.
    outside: usize,
    log: Rc<RefCell<Vec<String>>>,
    /// `document.activeElement` when the item's `data-oncontextmenu` ran.
    focus_at_contextmenu: Rc<Cell<Option<Option<usize>>>>,
}

/// The issue's shape. Every box sits off the origin and off an even spacing,
/// so a press resolved against the wrong box lands on something else.
fn mount(item_contextmenu: bool) -> Fixture {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let focus_at_contextmenu: Rc<Cell<Option<Option<usize>>>> = Rc::new(Cell::new(None));
    let ids: Rc<Cell<Option<[usize; 5]>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let at_ctx_in = focus_at_contextmenu.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");

        let listbox = scope.create_element("div");
        listbox.set_attribute(
            "style",
            "position: absolute; left: 37px; top: 23px; width: 310px; height: 170px",
        );
        listbox.set_attribute("tabindex", "0");

        let item = scope.create_element("div");
        item.set_attribute("style", "margin-left: 11px; width: 250px; height: 43px");
        item.set_attribute("tabindex", "-1");

        let plain = scope.create_element("div");
        plain.set_attribute("style", "margin-left: 11px; width: 250px; height: 29px");

        listbox.append_child(&item);
        listbox.append_child(&plain);

        let nofocus = scope.create_element("div");
        nofocus.set_attribute(
            "style",
            "position: absolute; left: 413px; top: 61px; width: 130px; height: 37px",
        );
        nofocus.set_attribute("tabindex", "0");
        nofocus.set_attribute("data-nofocus", "");

        let outside = scope.create_element("div");
        outside.set_attribute(
            "style",
            "position: absolute; left: 413px; top: 257px; width: 130px; height: 37px",
        );

        for (node, tag) in [(&listbox, "listbox"), (&item, "item")] {
            register_focus_target(
                node,
                FocusEntry::new()
                    .on_focus_gained({
                        let log = log_in.clone();
                        move || log.borrow_mut().push(format!("{tag}:gained"))
                    })
                    .on_focus_lost({
                        let log = log_in.clone();
                        move || log.borrow_mut().push(format!("{tag}:lost"))
                    }),
            );
        }

        if item_contextmenu {
            let ctx = scope.register_handler({
                let log = log_in.clone();
                let at = at_ctx_in.clone();
                let item = item.clone();
                move || {
                    log.borrow_mut().push("item:contextmenu".into());
                    at.set(Some(item.active_element().map(|n| n.node_id().0)));
                }
            });
            item.set_attribute("data-oncontextmenu", &ctx.0.to_string());
        }

        root.append_child(&listbox);
        root.append_child(&nofocus);
        root.append_child(&outside);
        ids_in.set(Some([
            listbox.node_id().0,
            item.node_id().0,
            plain.node_id().0,
            nofocus.node_id().0,
            outside.node_id().0,
        ]));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let [listbox, item, plain, nofocus, outside] = ids.get().expect("ids captured at mount");
    Fixture {
        app,
        listbox,
        item,
        plain,
        nofocus,
        outside,
        log,
        focus_at_contextmenu,
    }
}

fn dom_focus(app: &RinchApp) -> Option<usize> {
    app.doc.as_ref().unwrap().borrow().tree.focused_node
}

/// The issue's repro: a right press on the nested item hands it the keyboard
/// instead of leaving nobody holding it.
#[test]
fn a_right_press_on_a_nested_focusable_takes_the_claim() {
    let mut f = mount(false);
    press(&mut f.app, f.listbox, MouseButton::Left);
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.listbox));
    f.log.borrow_mut().clear();

    press(&mut f.app, f.item, MouseButton::Right);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.item));
    assert_eq!(dom_focus(&f.app), Some(f.item), ":focus follows the claim");
    assert_eq!(
        *f.log.borrow(),
        vec!["listbox:lost".to_string(), "item:gained".to_string()],
        "the lifecycle callbacks announce the move, once each"
    );
}

#[test]
fn a_middle_press_on_a_nested_focusable_takes_the_claim() {
    let mut f = mount(false);
    press(&mut f.app, f.listbox, MouseButton::Left);

    press(&mut f.app, f.item, MouseButton::Middle);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.item));
    assert_eq!(dom_focus(&f.app), Some(f.item));
}

/// With nothing focused beforehand, a right press is enough to take the
/// keyboard — it is a focus gesture, not only a release of one.
#[test]
fn a_right_press_with_nothing_focused_takes_the_claim() {
    let mut f = mount(false);
    assert_eq!(f.app.focus_target, FocusTarget::None);

    press(&mut f.app, f.plain, MouseButton::Right);

    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.listbox),
        "a plain child resolves to the focusable around it"
    );
    assert_eq!(*f.log.borrow(), vec!["listbox:gained".to_string()]);
}

/// A right press re-pressing the holder's own plain child moves nothing and
/// announces nothing.
#[test]
fn a_right_press_on_the_holders_plain_child_announces_nothing() {
    let mut f = mount(false);
    press(&mut f.app, f.listbox, MouseButton::Left);
    f.log.borrow_mut().clear();

    press(&mut f.app, f.plain, MouseButton::Right);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.listbox));
    assert!(f.log.borrow().is_empty(), "{:?}", f.log.borrow());
}

/// Outside every focusable a right press releases the claim, as Chrome moves
/// focus to `<body>`.
#[test]
fn a_right_press_outside_every_focusable_releases_the_claim() {
    let mut f = mount(false);
    press(&mut f.app, f.listbox, MouseButton::Left);

    press(&mut f.app, f.outside, MouseButton::Right);

    assert_eq!(f.app.focus_target, FocusTarget::None);
}

/// `data-nofocus` — rinch's `preventDefault()`-on-mousedown — keeps the claim
/// where it was for a right press too.
#[test]
fn a_right_press_on_a_nofocus_region_keeps_the_claim() {
    let mut f = mount(false);
    press(&mut f.app, f.listbox, MouseButton::Left);

    press(&mut f.app, f.nofocus, MouseButton::Right);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.listbox));
    assert_eq!(dom_focus(&f.app), Some(f.listbox));
}

/// A `data-oncontextmenu` still wins the press, and it runs after the focus
/// move, as `contextmenu` follows `mousedown`'s default action in a browser —
/// so a handler drawing its own menu for the item sees the item focused.
#[test]
fn a_contextmenu_handler_runs_after_the_focus_moves() {
    let mut f = mount(true);
    press(&mut f.app, f.listbox, MouseButton::Left);
    f.log.borrow_mut().clear();

    press(&mut f.app, f.item, MouseButton::Right);

    assert_eq!(
        *f.log.borrow(),
        vec![
            "listbox:lost".to_string(),
            "item:gained".to_string(),
            "item:contextmenu".to_string()
        ]
    );
    assert_eq!(
        f.focus_at_contextmenu.get(),
        Some(Some(f.item)),
        "the handler ran with the item already focused"
    );
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.item));
}

/// `:active` is the primary button's: a right press focuses but does not
/// activate.
#[test]
fn a_right_press_does_not_set_active() {
    let mut f = mount(false);

    down(&mut f.app, f.item, MouseButton::Right);
    let active = f.app.doc.as_ref().unwrap().borrow().tree.active_node;
    assert_eq!(active, None, "a right press is not :active");
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.item));

    // Positive control: a left press does set it.
    let (x, y) = center(&f.app, f.item);
    ev(&mut f.app, PlatformEvent::MouseUp { x, y, button: MouseButton::Right });
    down(&mut f.app, f.item, MouseButton::Left);
    let active = f.app.doc.as_ref().unwrap().borrow().tree.active_node;
    assert!(active.is_some(), "a left press is :active");
}
