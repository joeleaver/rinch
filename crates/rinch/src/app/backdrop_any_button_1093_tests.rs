//! A press of any button outside an open overlay dismisses it (#1093).
//!
//! #1093 stopped a right or middle press from clicking a `data-rid`. Every
//! dismiss backdrop is a `data-rid` too, and outside-press dismissal is a
//! `mousedown` of any button — so without `data-backdrop` a right press outside
//! an open overlay was swallowed by the backdrop and did nothing (the review of
//! PR #1110 measured it on `ContextMenu`: the menu could neither close nor
//! re-open under a right press elsewhere). Driven through the real event path
//! under the real theme and component stylesheets.

use super::*;
#[cfg(any(feature = "desktop", feature = "android"))]
use crate::shell::touch_gesture::{TouchAction, TouchGesture};
use rinch_components::context_menu::ContextMenu;
use rinch_components::{Drawer, DropdownMenu, Modal, Popover};
use rinch_core::{Callback, Component, Signal};
use std::cell::Cell;
#[cfg(any(feature = "desktop", feature = "android"))]
use std::time::{Duration, Instant};

const VP: (u32, u32) = (800, 600);

/// Far from every panel: bottom right, where no overlay's own box reaches.
const OUTSIDE: (f32, f32) = (781.0, 577.0);

fn press(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
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
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

// ── ContextMenu (its own `opened` signal) ──────────────────────────────────

fn mount_context_menu() -> (RinchApp, Signal<bool>) {
    let opened: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let opened_in = opened.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let target = scope.create_element("div");
        target.set_attribute("style", "width: 300px; height: 200px");
        let item = scope.create_element("div");
        item.set_attribute("style", "width: 100px; height: 20px");
        let cm = ContextMenu::default();
        opened_in.set(Some(cm.opened));
        let node = cm.render(scope, &[target, item]);
        root.append_child(&node);
        root
    });
    (app, opened.get().unwrap())
}

#[test]
fn a_press_of_any_button_outside_an_open_context_menu_closes_it() {
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        let (mut app, opened) = mount_context_menu();
        press(&mut app, (53.0, 47.0), MouseButton::Right);
        assert!(
            opened.get(),
            "positive control: a right press on the target opens"
        );
        press(&mut app, OUTSIDE, button);
        assert!(!opened.get(), "a {button:?} press outside left it open");
    }
}

/// An Android long press is a right press (`touch_gesture`), so a long press
/// outside an open context menu — Pimble's tree — dismisses it too. Fed through
/// the real recogniser rather than as a hand-built right press.
#[cfg(any(feature = "desktop", feature = "android"))]
#[test]
fn a_long_press_outside_an_open_context_menu_closes_it() {
    let (mut app, opened) = mount_context_menu();
    press(&mut app, (53.0, 47.0), MouseButton::Right);
    assert!(opened.get(), "positive control: open");

    let mut gesture = TouchGesture::new();
    let t0 = Instant::now();
    let mut events = Vec::new();
    let (x, y) = OUTSIDE;
    gesture.process(TouchAction::Down, x, y, t0, &mut events);
    gesture.tick_long_press(t0 + Duration::from_millis(600), &mut events);
    gesture.process(
        TouchAction::Up,
        x,
        y,
        t0 + Duration::from_millis(700),
        &mut events,
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlatformEvent::MouseDown {
                button: MouseButton::Right,
                ..
            }
        )),
        "positive control: the recogniser made a right press: {events:?}"
    );
    for event in events {
        app.handle_event(event, VP, 1.0);
    }
    assert!(!opened.get(), "the long press outside left it open");
}

// ── Overlays that report through `onclose` ─────────────────────────────────

fn recorder() -> (Rc<Cell<usize>>, Callback) {
    let hits: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let h = hits.clone();
    (hits, Callback::new(move || h.set(h.get() + 1)))
}

/// Each kind open, with its `onclose` counted. `opened_fn` stays true, so the
/// backdrop stays up and every press outside is counted.
#[test]
fn a_press_of_any_button_outside_an_open_overlay_asks_it_to_close() {
    type Build = fn(Callback) -> Box<dyn Fn(&mut RenderScope) -> NodeHandle>;
    let kinds: Vec<(&str, Build)> = vec![
        ("Modal", |cb| {
            Box::new(move |s| {
                Modal {
                    opened_fn: Some(Rc::new(|| true)),
                    onclose: Some(cb.clone()),
                    ..Default::default()
                }
                .render(s, &[])
            })
        }),
        ("Drawer", |cb| {
            Box::new(move |s| {
                Drawer {
                    opened_fn: Some(Rc::new(|| true)),
                    onclose: Some(cb.clone()),
                    ..Default::default()
                }
                .render(s, &[])
            })
        }),
        ("Popover", |cb| {
            Box::new(move |s| {
                Popover {
                    opened_fn: Some(Rc::new(|| true)),
                    onclose: Some(cb.clone()),
                    ..Default::default()
                }
                .render(s, &[])
            })
        }),
        ("DropdownMenu", |cb| {
            Box::new(move |s| {
                DropdownMenu {
                    opened_fn: Some(Rc::new(|| true)),
                    on_close: Some(cb.clone()),
                    ..Default::default()
                }
                .render(s, &[])
            })
        }),
    ];
    for (kind, build) in kinds {
        let (closes, cb) = recorder();
        let render = build(cb);
        let mut app = mount(move |s| render(s));
        press(&mut app, OUTSIDE, MouseButton::Left);
        assert_eq!(
            closes.get(),
            1,
            "positive control: {kind}: a left press outside"
        );
        press(&mut app, OUTSIDE, MouseButton::Right);
        assert_eq!(closes.get(), 2, "{kind}: a right press outside");
        press(&mut app, OUTSIDE, MouseButton::Middle);
        assert_eq!(closes.get(), 3, "{kind}: a middle press outside");
    }
}

/// A live `data-oncontextmenu` above a backdrop takes the right press first
/// (the contextmenu claim is offered before the click path); a stale one (its
/// handler freed, #141) is no handler and leaves the press to the backdrop.
/// The web twin is `rinch-web/tests/right_press_click_1093.rs`.
#[test]
fn a_contextmenu_handler_above_a_backdrop_takes_the_right_press() {
    for live in [true, false] {
        let (closes, cb) = recorder();
        let menus: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let m = menus.clone();
        let mut app = mount(move |s| {
            let wrap = s.create_element("div");
            let ctx = s.register_handler(move || m.set(m.get() + 1));
            if !live {
                events::unregister_handler(ctx);
            }
            wrap.set_attribute("data-oncontextmenu", &ctx.0.to_string());
            let pop = Popover {
                opened_fn: Some(Rc::new(|| true)),
                onclose: Some(cb.clone()),
                ..Default::default()
            }
            .render(s, &[]);
            wrap.append_child(&pop);
            wrap
        });
        press(&mut app, OUTSIDE, MouseButton::Right);
        if live {
            assert_eq!(
                (closes.get(), menus.get()),
                (0, 1),
                "a live handler takes it"
            );
            press(&mut app, OUTSIDE, MouseButton::Left);
            assert_eq!(closes.get(), 1, "positive control: a left press dismisses");
        } else {
            assert_eq!((closes.get(), menus.get()), (1, 0), "a stale one does not");
        }
    }
}

/// `data-backdrop` follows rinch's `data-` boolean convention: `"false"` (any
/// case) is off, and `"0"` is on — `data_attr_is_on`, like `data-nofocus`.
#[test]
fn data_backdrop_false_is_not_a_backdrop() {
    for (value, expected) in [("", 1usize), ("false", 0), ("FALSE", 0), ("0", 1)] {
        let hits: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut app = mount(move |s| {
            let b = s.create_element("div");
            b.set_attribute("style", "width: 800px; height: 600px");
            b.set_attribute(events::BACKDROP_ATTRIBUTE, value);
            let id = s.register_handler(move || h.set(h.get() + 1));
            b.set_attribute("data-rid", &id.0.to_string());
            b
        });
        press(&mut app, OUTSIDE, MouseButton::Right);
        assert_eq!(hits.get(), expected, "data-backdrop={value:?}");
    }
}
