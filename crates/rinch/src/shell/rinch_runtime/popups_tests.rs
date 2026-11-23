//! Popups through the runtime without an event loop (#2552): a bar (a second
//! window) with a menu and a submenu made as the shell mounts windows, then
//! closed, pressed, keyed and dismissed through the runtime's own paths.

use super::*;
use crate::popup::{PopupAnchor, PopupProps, open_popup};
use rinch_core::element::{Dismissal, Placement, PopupKind};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<(&'static str, Dismissal)>>>;

thread_local! {
    static ROW: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

/// A 200x40 box whose 20 px first row is kept to anchor a popup to.
fn with_row(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "width: 200px; height: 40px");
    let row = scope.create_element("div");
    row.set_attribute("style", "height: 20px");
    root.append_child(&row);
    ROW.with(|r| *r.borrow_mut() = Some(row));
    root
}

fn row() -> NodeHandle {
    ROW.with(|r| r.borrow().clone()).expect("a row is mounted")
}

/// A popup on the last row mounted, made as the shell mounts it (no OS
/// window), owned by nothing.
fn popup(runtime: &mut RinchRuntime, name: &'static str, log: &Log) -> (WindowHandle, u64) {
    let log = log.clone();
    let props = PopupProps {
        anchor: PopupAnchor::Node(row()),
        placement: Placement::BOTTOM_START,
        on_dismissed: Some(Rc::new(move |why| log.borrow_mut().push((name, why)))),
        ..PopupProps::default()
    };
    let handle = rinch_core::reactive::unowned(|| open_popup(props, with_row))
        .expect("popups are supported here");
    let id = make(runtime, handle);
    (handle, id)
}

/// A bar with a menu and a submenu, the bar pressed first.
fn bar_menu_sub() -> (RinchRuntime, u64, u64, u64, Log) {
    crate::popup::set_supported(true);
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, bar) = open(&mut runtime, with_row);
    crate::window_handle::note_press(doc_of(&runtime, bar));
    let log: Log = Rc::default();
    let (_, menu) = popup(&mut runtime, "menu", &log);
    let (_, sub) = popup(&mut runtime, "sub", &log);
    (runtime, bar, menu, sub, log)
}

#[test]
fn closing_the_bar_closes_its_chain_topmost_first_told_parent_closed() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // The compositor closes the bar's layer surface (its output went): the
    // close it agrees to with no veto.
    runtime.close_soon(WindowRole::Other(bar));
    assert!(!runtime.settle_closes_now());
    assert!(runtime.others.is_empty());
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::ParentClosed)
        ]
    );
    for id in [bar, menu, sub] {
        assert!(!WindowHandle::from_id(id).is_open());
    }
    crate::popup::set_supported(false);
}

#[test]
fn keys_reaching_the_bar_go_to_the_topmost_menu_and_its_blur_is_ignored() {
    let (mut runtime, bar, menu, sub, _log) = bar_menu_sub();
    let ime = WindowEvent::Ime(winit::event::Ime::Enabled);
    for from in [bar, menu, sub] {
        assert_eq!(
            runtime.input_role(WindowRole::Other(from), &ime),
            WindowRole::Other(sub),
            "from {from}"
        );
    }
    // Modifiers too: the topmost menu reads them for its chords and clicks.
    let mods = WindowEvent::ModifiersChanged(winit::keyboard::ModifiersState::SHIFT.into());
    assert_eq!(
        runtime.input_role(WindowRole::Other(bar), &mods),
        WindowRole::Other(sub)
    );
    // The main window is in no chain: its keys stay.
    assert_eq!(runtime.input_role(WindowRole::Main, &ime), WindowRole::Main);
    // A pointer event is never moved.
    let left = WindowEvent::PointerLeft {
        device_id: None,
        primary: true,
        position: None,
        kind: winit::event::PointerKind::Mouse,
    };
    assert_eq!(
        runtime.input_role(WindowRole::Other(bar), &left),
        WindowRole::Other(bar)
    );
    assert!(runtime.in_menu_chain(WindowRole::Other(bar)));
    assert!(!runtime.in_menu_chain(WindowRole::Main));
    crate::popup::set_supported(false);
}

#[test]
fn a_press_in_the_bar_closes_the_chain_after_it_is_delivered() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // On the bar's row, the menu's anchor.
    runtime.w(WindowRole::Other(bar)).app.cursor_pos = Some((5.0, 5.0));
    runtime.light_dismiss_press(WindowRole::Other(bar));
    // Not yet: closed at the end of the handler.
    assert!(WindowHandle::from_id(menu).is_open());
    runtime.settle_closes_now();
    assert!(!WindowHandle::from_id(menu).is_open() && !WindowHandle::from_id(sub).is_open());
    let told = log.borrow().clone();
    assert!(told.contains(&("menu", Dismissal::PressOutside { on_anchor: true })));
    assert!(told.contains(&("sub", Dismissal::PressOutside { on_anchor: false })));
    crate::popup::set_supported(false);
}

#[test]
fn popup_done_closes_with_no_veto_told_system() {
    let (mut runtime, _bar, menu, sub, log) = bar_menu_sub();
    assert!(runtime.popup_done(WindowRole::Other(menu)));
    runtime.settle_closes_now();
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::System)
        ]
    );
    assert!(!WindowHandle::from_id(sub).is_open());
    assert!(
        !runtime.popup_done(WindowRole::Main),
        "the main window is no popup"
    );
    crate::popup::set_supported(false);
}

#[test]
fn a_new_menu_replaces_the_open_chain_and_waits_a_turn() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // Another menu, on the bar itself (not on the topmost menu): the next
    // tray item's.
    let props = PopupProps {
        kind: PopupKind::Menu,
        anchor: PopupAnchor::Rect {
            window: Some(WindowHandle::from_id(bar)),
            x: 50.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        },
        ..PopupProps::default()
    };
    let other = rinch_core::reactive::unowned(|| open_popup(props, with_row)).unwrap();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let event_loop = RecordingLoop::default();
    runtime.open_pending_popup(other.id(), &event_loop);
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::Replaced)
        ]
    );
    assert!(!WindowHandle::from_id(menu).is_open() && !WindowHandle::from_id(sub).is_open());
    // Not made in this iteration: asked for again at its end.
    assert!(other.is_open());
    assert!(!runtime.others.iter().any(|w| w.handle == other.id()));
    runtime.popups_end_of_turn(&event_loop);
    let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(
        queued.iter().any(
            |(to, e)| matches!(to, EventFor::Window(w) if *w == other.id())
                && matches!(e, RinchNativeEvent::OpenWindow(id) if *id == other.id())
        ),
        "{queued:?}"
    );
    crate::popup::set_supported(false);
}

#[test]
fn a_menu_closed_while_it_waits_for_the_keyboard_puts_the_bar_back() {
    crate::popup::set_supported(true);
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, bar) = open(&mut runtime, with_row);
    crate::window_handle::note_press(doc_of(&runtime, bar));
    let props = PopupProps {
        anchor: PopupAnchor::Node(row()),
        keyboard_via_parent: true,
        ..PopupProps::default()
    };
    let menu = rinch_core::reactive::unowned(|| open_popup(props, with_row)).unwrap();
    // Waiting for the bar's keyboard, the bar raised (as
    // `raise_parent_keyboard` leaves them).
    let pending = crate::window_handle::take_pending(menu.id()).unwrap();
    runtime.popups.awaiting_keyboard.push((bar, pending));
    runtime
        .popups
        .raised
        .push((bar, rinch_core::element::LayerKeyboard::None));
    // A second press on the tray icon closes it before the keyboard came.
    runtime.close_other(menu.id());
    assert!(
        runtime.popups.awaiting_keyboard.is_empty(),
        "it waits no more"
    );
    assert!(runtime.popups.raised.is_empty(), "and the bar is put back");
    assert!(!menu.is_open());
    crate::popup::set_supported(false);
}

#[test]
fn a_blur_held_while_the_chain_is_open_is_told_when_it_closes() {
    let (mut runtime, bar, menu, _sub, _log) = bar_menu_sub();
    let event_loop = RecordingLoop::default();
    runtime.w(WindowRole::Other(bar)).app.window_focused = true;
    runtime.window_event_for(
        WindowRole::Other(bar),
        &event_loop,
        WindowEvent::Focused(false),
    );
    assert!(
        runtime.w(WindowRole::Other(bar)).app.window_focused,
        "held while the menu is open"
    );
    runtime.close_other(menu);
    runtime.popups_end_of_turn(&event_loop);
    assert!(
        !runtime.w(WindowRole::Other(bar)).app.window_focused,
        "told once the chain closed"
    );
    crate::popup::set_supported(false);
}
