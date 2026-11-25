//! `RinchApp::key_down_unused` (#3062, PR #3072's review): a key is reported
//! used only when rinch did something with it.
//!
//! Two halves. The default actions (`handle_unclaimed_key_fallback`): Tab,
//! the caret keys and Enter used to be reported used whatever they did, so an
//! embedding host never got the arrows or Enter back with nothing focused.
//! And the focus arbiter's arms that end a key early: a `<select>` popup, a
//! focused editor that was unmounted, a focused `RenderSurface` (which uses a
//! key only by claiming it with `set_key_handler`).
//!
//! Each "used" case is paired with an "unused" one, so a flag stuck at either
//! answer fails one of the pair.

use super::key_down_unused_3062_tests::{VP, mount, press};
use super::*;
use rinch_macros::rsx;
use std::cell::Cell;

/// Press `key` with the given repeat answer and no text; answers
/// `key_down_unused` after the press. No release follows.
fn press_repeat(app: &mut RinchApp, key: KeyCode, repeat: KeyRepeat) -> bool {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat,
        },
        VP,
        1.0,
    );
    app.key_down_unused()
}

fn mount_app(f: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(f);
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

/// Focus `node` through the arbiter, as a script's `focus()` does.
fn focus(app: &mut RinchApp, node: &NodeHandle) {
    node.focus();
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
}

/// A document with nothing focusable and nothing to scroll.
fn mount_inert() -> RinchApp {
    mount_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let text = scope.create_text("nothing to focus");
        root.append_child(&text);
        root
    })
}

// ── The default actions ──────────────────────────────────────────────

/// With nothing focused, nothing focusable and nothing to scroll, the arrows,
/// Home, Enter, Backspace and Tab do nothing, so a host may use them.
#[test]
fn keys_with_nothing_to_act_on_are_unused() {
    let mut app = mount_inert();
    for key in [
        KeyCode::ArrowLeft,
        KeyCode::ArrowDown,
        KeyCode::Home,
        KeyCode::Backspace,
        KeyCode::Enter,
        KeyCode::Tab,
    ] {
        assert!(press(&mut app, key, None), "{key:?} did nothing: unused");
    }
    assert_eq!(app.focus_target, FocusTarget::None);
}

/// Tab with a focusable target moves the focus: used. Paired with the inert
/// document's Tab above.
#[test]
fn tab_that_moves_the_focus_is_used() {
    let mut f = mount();
    assert!(!f.press(KeyCode::Tab, None));
    assert!(matches!(f.app.focus_target, FocusTarget::Input(_)));
}

/// A focused text field takes its caret keys, at the edge of its value too:
/// the field owns them. The same ArrowLeft with nothing focused is unused.
#[test]
fn a_caret_key_in_a_focused_field_is_used() {
    let mut f = mount();
    assert!(f.press(KeyCode::ArrowLeft, None), "no field: unused");
    f.focus_input();
    f.press(KeyCode::KeyA, Some("a"));
    assert!(!f.press(KeyCode::ArrowLeft, None), "caret moved");
    assert!(
        !f.press(KeyCode::ArrowLeft, None),
        "at the start: still the field's"
    );
    assert!(!f.press(KeyCode::Delete, None), "deleted the a");
}

/// Enter in a field commits only through a handler: a field with `onsubmit`
/// uses it, one with only `oninput` does nothing with it.
#[test]
fn enter_in_a_field_is_used_only_when_it_submits() {
    let mut f = mount();
    f.focus_input();
    f.press(KeyCode::KeyA, Some("a"));
    assert!(f.press(KeyCode::Enter, None), "no change or submit handler");

    let submitted = Rc::new(Cell::new(0));
    let submitted_in = submitted.clone();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = mount_app(move |__scope: &mut RenderScope| {
        let submitted_in = submitted_in.clone();
        let root = rsx! {
            div {
                input {
                    oninput: move |_v: String| {},
                    onsubmit: move || submitted_in.set(submitted_in.get() + 1),
                }
            }
        };
        *slot_in.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    let input = slot.borrow_mut().take().expect("input");
    focus(&mut app, &input);
    assert!(!press(&mut app, KeyCode::Enter, None), "submitted");
    assert_eq!(submitted.get(), 1);
}

/// A focused scroller with room to scroll takes ArrowDown (#2977's keyboard
/// scroll); the inert document's ArrowDown above is unused.
#[test]
fn an_arrow_that_scrolls_is_used() {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = mount_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let s = scope.create_element("div");
        s.set_attribute("tabindex", "0");
        s.set_attribute("style", "width: 200px; height: 100px; overflow-y: auto");
        let inner = scope.create_element("div");
        inner.set_attribute("style", "height: 1000px");
        s.append_child(&inner);
        root.append_child(&s);
        *slot_in.borrow_mut() = Some(s);
        root
    });
    let scroller = slot.borrow_mut().take().expect("scroller");
    focus(&mut app, &scroller);
    assert!(matches!(app.focus_target, FocusTarget::Node(_)));
    assert!(!press(&mut app, KeyCode::ArrowDown, None), "scrolled");
}

/// Enter activates a focused button once per physical press (#228): the
/// fresh press is used, an auto-repeat does nothing and is unused.
#[test]
fn a_repeated_enter_on_a_focused_node_is_unused() {
    let clicks = Rc::new(Cell::new(0));
    let clicks_in = clicks.clone();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = mount_app(move |__scope: &mut RenderScope| {
        let clicks_in = clicks_in.clone();
        let root = rsx! {
            div {
                button { onclick: move || clicks_in.set(clicks_in.get() + 1), "go" }
            }
        };
        *slot_in.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    let button = slot.borrow_mut().take().expect("button");
    focus(&mut app, &button);
    assert!(matches!(app.focus_target, FocusTarget::Node(_)));
    assert!(!press_repeat(&mut app, KeyCode::Enter, KeyRepeat::Fresh));
    assert_eq!(clicks.get(), 1);
    assert!(press_repeat(&mut app, KeyCode::Enter, KeyRepeat::Repeat));
    assert_eq!(clicks.get(), 1, "the repeat did not click");
}

// ── The arbiter's arms ───────────────────────────────────────────────

/// An open `<select>` popup owns every key: used, Escape included (it closes
/// the popup, as Back does on Android). A `<select>` claim with no popup open
/// has no use for the key.
#[test]
fn the_select_arm_uses_keys_only_with_its_popup_open() {
    let ids = Rc::new(Cell::new(0u64));
    let ids_in = ids.clone();
    let mut app = mount_app(move |scope: &mut RenderScope| {
        let select = scope.create_element("select");
        for v in ["a", "b"] {
            let o = scope.create_element("option");
            o.set_attribute("value", v);
            let t = scope.create_text(v);
            o.append_child(&t);
            select.append_child(&o);
        }
        ids_in.set(select.node_id().0);
        select
    });
    let select = ids.get();
    app.open_select_popup(select, VP.0 as f32, VP.1 as f32);
    assert_eq!(app.focus_target, FocusTarget::Select(select));
    assert!(
        !press(&mut app, KeyCode::ArrowDown, None),
        "highlight moved"
    );
    assert!(!press(&mut app, KeyCode::Escape, None), "popup closed");
    assert!(app.open_select.is_none());

    app.focus_target = FocusTarget::Select(select);
    assert!(press(&mut app, KeyCode::Escape, None), "no popup: unused");
}

/// A focused editor unmounted from under its claim: the key is dropped with
/// the stale claim, unused, and the next key reaches the defaults (Tab moves
/// to the field: used).
#[test]
#[cfg(feature = "windowing")]
fn the_unmounted_editor_arm_reports_the_key_unused() {
    let mut f = mount();
    f.app.focus_target = FocusTarget::Editor(u64::MAX - 1);
    assert!(f.press(KeyCode::Escape, None), "stale editor: unused");
    assert_eq!(
        f.app.focus_target,
        FocusTarget::None,
        "the claim is dropped"
    );
    assert!(!f.press(KeyCode::Tab, None), "Tab now moves the focus");
}

/// A focused `RenderSurface` uses a key only by claiming it with
/// `set_key_handler`: its event handler hears every key, but Escape the key
/// handler does not claim is unused (so Android's Back still goes back), and
/// a claimed one is used.
#[test]
fn a_surface_uses_a_key_only_by_claiming_it() {
    use crate::render_surface::{RenderSurface, SurfaceEvent, create_render_surface};
    use rinch_core::Component;

    let surface = create_render_surface();
    let mounted = surface.clone();
    let mut app = mount_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        root.append_child(&child);
        root
    });
    app.focus_target = FocusTarget::Surface(surface.id());
    let heard = Rc::new(Cell::new(0));
    let heard_in = heard.clone();
    surface.set_event_handler(move |event| {
        if matches!(event, SurfaceEvent::KeyDown(_)) {
            heard_in.set(heard_in.get() + 1);
        }
    });

    assert!(press(&mut app, KeyCode::Escape, None), "heard, not claimed");
    assert_eq!(heard.get(), 1);

    surface.set_key_handler(|k| k.code == "Escape");
    assert!(!press(&mut app, KeyCode::Escape, None), "claimed");
    assert_eq!(heard.get(), 2);
}
