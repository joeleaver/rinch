//! Popups driven by the harness (#2552): a bar (a layer surface, as
//! deskrinch's) with a menu, a submenu and a sub-submenu, each anchored to a
//! row of the one below. Opened, sized by content, dismissed by every cause,
//! closed topmost first.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{Dismissal, LayerSurface, Placement, PopupKind, WindowProps};

use crate::WindowHandle;
use crate::popup::{PopupAnchor, PopupError, PopupProps, open_popup};
use crate::testing::TestApp;

type Log = Rc<RefCell<Vec<(&'static str, Dismissal)>>>;
type Slot = Rc<RefCell<Option<NodeHandle>>>;

/// A content closure: a `width`x`height` box holding one `row` 20 px high
/// at its top, kept in `row` to anchor the next popup to.
fn boxed(
    width: u32,
    height: u32,
    row: Slot,
) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    move |s: &mut RenderScope| {
        let root = s.create_element("div");
        root.set_attribute(
            "style",
            &format!("width: {width}px; height: {height}px; margin: 0"),
        );
        let r = s.create_element("div");
        r.set_attribute("style", "height: 20px");
        root.append_child(&r);
        *row.borrow_mut() = Some(r);
        root
    }
}

fn props(anchor: &Slot, placement: Placement, name: &'static str, log: &Log) -> PopupProps {
    let log = log.clone();
    PopupProps {
        anchor: PopupAnchor::Node(anchor.borrow().clone().expect("the anchor is mounted")),
        placement,
        on_dismissed: Some(Rc::new(move |why| log.borrow_mut().push((name, why)))),
        ..PopupProps::default()
    }
}

/// The bar, its menu, submenu and sub-submenu, open, with what each was
/// told.
struct Chain {
    app: TestApp,
    bar: WindowHandle,
    menu: WindowHandle,
    sub: WindowHandle,
    subsub: WindowHandle,
    log: Log,
}

fn bar_with_chain() -> Chain {
    let mut app = TestApp::mount(|s: &mut RenderScope| s.create_element("div"));
    let button: Slot = Rc::default();
    let bar = crate::open_window(
        WindowProps {
            width: 800,
            height: 40,
            wayland_layer: Some(LayerSurface::default()),
            ..WindowProps::default()
        },
        boxed(800, 40, button.clone()),
    );
    app.settle();
    // The press a menu's grab names.
    app.window(bar).click(5.0, 5.0);
    let log: Log = Rc::default();
    let (menu_row, sub_row, subsub_row): (Slot, Slot, Slot) = Default::default();
    let menu = open_popup(
        props(&button, Placement::TOP_START, "menu", &log),
        boxed(160, 200, menu_row.clone()),
    )
    .expect("a menu after a press in its bar");
    app.settle();
    let sub = open_popup(
        props(&menu_row, Placement::RIGHT_START, "sub", &log),
        boxed(140, 120, sub_row.clone()),
    )
    .expect("a submenu");
    app.settle();
    let subsub = open_popup(
        props(&sub_row, Placement::RIGHT_START, "subsub", &log),
        boxed(100, 60, subsub_row),
    )
    .expect("a sub-submenu");
    app.settle();
    assert_eq!(app.windows(), vec![bar, menu, sub, subsub]);
    Chain {
        app,
        bar,
        menu,
        sub,
        subsub,
        log,
    }
}

#[test]
fn a_menu_needs_a_press_in_its_parent_chain() {
    let mut app = TestApp::mount(|s: &mut RenderScope| s.create_element("div"));
    let button: Slot = Rc::default();
    let bar = crate::open_window(WindowProps::default(), boxed(100, 40, button.clone()));
    app.settle();
    let log: Log = Rc::default();
    let refused = open_popup(
        props(&button, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    assert_eq!(refused.err(), Some(PopupError::NoUserInput));
    // A tooltip needs none.
    let tip = open_popup(
        PopupProps {
            kind: PopupKind::Tooltip,
            ..props(&button, Placement::BOTTOM_START, "tip", &log)
        },
        boxed(10, 10, Rc::default()),
    );
    assert!(tip.is_ok());
    app.window(bar).click(5.0, 5.0);
    let ok = open_popup(
        props(&button, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    assert!(ok.is_ok(), "after a press in the bar");
}

#[test]
fn a_menu_cannot_open_on_a_tooltip_and_popups_can_be_turned_off() {
    let mut app = TestApp::mount(|s: &mut RenderScope| s.create_element("div"));
    let button: Slot = Rc::default();
    let bar = crate::open_window(WindowProps::default(), boxed(100, 40, button.clone()));
    app.settle();
    app.window(bar).click(5.0, 5.0);
    let log: Log = Rc::default();
    let tip_row: Slot = Rc::default();
    let tip = open_popup(
        PopupProps {
            kind: PopupKind::Tooltip,
            ..props(&button, Placement::BOTTOM_START, "tip", &log)
        },
        boxed(40, 20, tip_row.clone()),
    )
    .unwrap();
    app.settle();
    assert!(tip.is_popup());
    assert_eq!(
        app.window(tip).props().input_region,
        rinch_core::element::InputRegion::None,
        "a tooltip takes no pointer input"
    );
    let on_tip = open_popup(
        props(&tip_row, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    assert_eq!(on_tip.err(), Some(PopupError::MenuOnTooltip));
    crate::popup::set_turned_off(true);
    let off = open_popup(
        props(&button, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    crate::popup::set_turned_off(false);
    assert_eq!(off.err(), Some(PopupError::Unsupported));
}

#[test]
fn a_popup_is_made_at_its_contents_size() {
    let Chain {
        mut app, menu, sub, ..
    } = bar_with_chain();
    assert_eq!(app.window(menu).size(), (160, 200));
    assert_eq!(app.window(sub).size(), (140, 120));
}

#[test]
fn the_bar_closed_by_the_system_closes_its_chain_topmost_first() {
    // deskrinch: an output removed with a menu and a submenu open.
    let Chain {
        mut app, bar, log, ..
    } = bar_with_chain();
    app.window(bar).close_by_system();
    assert!(app.windows().is_empty());
    assert_eq!(
        *log.borrow(),
        vec![
            ("subsub", Dismissal::ParentClosed),
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::ParentClosed),
        ]
    );
}

#[test]
fn escape_closes_the_topmost_menu_only() {
    let Chain {
        mut app,
        bar,
        menu,
        sub,
        subsub,
        log,
    } = bar_with_chain();
    // Typed where the keyboard is (the bar, on wlroots): the topmost menu
    // takes it.
    app.window(bar).key_down("Escape");
    assert_eq!(*log.borrow(), vec![("subsub", Dismissal::Escape)]);
    assert!(!subsub.is_open() && sub.is_open() && menu.is_open());
    app.window(bar).key_down("Escape");
    assert_eq!(log.borrow()[1], ("sub", Dismissal::Escape));
    assert!(menu.is_open());
}

#[test]
fn a_press_closes_the_popups_above_it() {
    let Chain {
        mut app,
        bar,
        menu,
        sub,
        subsub,
        log,
    } = bar_with_chain();
    // In the menu, below its first row (the submenu's anchor): the submenu
    // and what is on it.
    app.window(menu).click(50.0, 100.0);
    assert!(!sub.is_open() && !subsub.is_open() && menu.is_open());
    let told: Vec<_> = log.borrow().clone();
    assert!(told.contains(&("sub", Dismissal::PressOutside { on_anchor: false })));
    assert!(told.contains(&("subsub", Dismissal::PressOutside { on_anchor: false })));
    // On the bar's button, the menu's anchor: the menu, told so.
    log.borrow_mut().clear();
    app.window(bar).click(5.0, 5.0);
    assert!(!menu.is_open());
    assert_eq!(
        *log.borrow(),
        vec![("menu", Dismissal::PressOutside { on_anchor: true })]
    );
}

#[test]
fn the_system_dismissal_and_the_apps_own_close() {
    let Chain {
        mut app,
        menu,
        sub,
        subsub,
        log,
        ..
    } = bar_with_chain();
    app.dismiss_popup_by_system(sub);
    assert!(!sub.is_open() && !subsub.is_open() && menu.is_open());
    assert_eq!(
        *log.borrow(),
        vec![
            ("subsub", Dismissal::ParentClosed),
            ("sub", Dismissal::System)
        ]
    );
    // The app's own close tells nothing.
    log.borrow_mut().clear();
    menu.close_popup_chain();
    app.settle();
    assert!(!menu.is_open());
    assert!(log.borrow().is_empty());
}

#[test]
fn keys_reaching_the_bar_go_to_the_topmost_menu() {
    let Chain {
        mut app,
        bar,
        subsub,
        ..
    } = bar_with_chain();
    let typed: Rc<RefCell<Vec<u64>>> = Rc::default();
    // Where each window's keys land: the dispatching document.
    let t = typed.clone();
    // The thread's fallback interceptor (no document is current here),
    // asked in whichever document a key is dispatched in.
    rinch_core::set_keyboard_interceptor(move |_| {
        if let Some(doc) = rinch_core::current_dispatching_doc() {
            t.borrow_mut().push(doc);
        }
        false
    });
    let subsub_doc = app.window(subsub).doc();
    let bar_doc = app.window(bar).doc();
    app.window(bar).key_down("a");
    rinch_core::clear_keyboard_interceptor();
    assert_eq!(
        typed.borrow().last(),
        Some(&subsub_doc),
        "not the bar's {bar_doc}"
    );
}

#[test]
fn a_popup_says_what_it_is_in_its_perf_lines() {
    crate::window_perf::capture::listen();
    let Chain { menu, bar, .. } = bar_with_chain();
    let lines = crate::window_perf::capture::take();
    let want = format!(
        "[PERF] window {} popup: kind=menu parent={}",
        menu.id(),
        bar.id()
    );
    assert!(lines.contains(&want), "{lines:#?}");
}

#[test]
fn a_press_long_ago_is_no_cause_for_a_menu() {
    let mut app = TestApp::mount(|s: &mut RenderScope| s.create_element("div"));
    let button: Slot = Rc::default();
    let bar = crate::open_window(WindowProps::default(), boxed(100, 40, button.clone()));
    app.settle();
    app.window(bar).click(5.0, 5.0);
    app.advance_time(crate::popup::PRESS_RECENT + std::time::Duration::from_millis(1));
    let log: Log = Rc::default();
    let late = open_popup(
        props(&button, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    assert_eq!(late.err(), Some(PopupError::NoUserInput));
    app.window(bar).click(5.0, 5.0);
    app.advance_time(crate::popup::PRESS_RECENT);
    let in_time = open_popup(
        props(&button, Placement::BOTTOM_START, "menu", &log),
        boxed(10, 10, Rc::default()),
    );
    assert!(
        in_time.is_ok(),
        "a press exactly the bound ago still names it"
    );
}
