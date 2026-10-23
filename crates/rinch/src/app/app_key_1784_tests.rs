//! App-wide key listeners (issue #1784): `rinch_core::on_app_key` hears every
//! key the window receives that nothing ahead of it consumed — with nothing
//! focused, with a text field focused, and with a render surface focused for
//! the keys the surface does not claim.
//!
//! Driven through the real `rsx!` output and the real key path
//! (`handle_event`), the way the desktop loop drives them.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use crate::render_surface::create_render_surface;
use rinch_core::Component;
use rinch_core::events::{
    KeyEventData, on_app_key, push_dismiss_handler, set_keyboard_interceptor,
};
use rinch_core::reactive::Signal;
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    app: RinchApp,
    input: NodeHandle,
    tile: NodeHandle,
    surface: crate::render_surface::RenderSurfaceHandle,
    show: Signal<bool>,
    /// What the listener registered by the `if show` branch heard.
    branch_log: Log,
}

/// Registers an app key listener inside the component rendering it.
fn listening_branch(__scope: &mut RenderScope, log: Log) -> NodeHandle {
    let l = log.clone();
    on_app_key(move |k| {
        l.borrow_mut().push(format!("branch:{}", k.key));
        false
    });
    rsx! { span { "listening" } }
}

/// An `<input>`, a `tabindex` tile with an `onkeydown` that takes F2, a render
/// surface, and an `if show` branch whose component registers a listener.
fn mount() -> Fixture {
    let branch_log: Log = Rc::new(RefCell::new(Vec::new()));
    let surface = create_render_surface();
    let show = Signal::new(true);
    let handles: Rc<RefCell<Option<[NodeHandle; 2]>>> = Rc::new(RefCell::new(None));
    let (bl, hs, sf) = (branch_log.clone(), handles.clone(), surface.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let bl = bl.clone();
        let root = rsx! {
            div {
                input {
                    style: "width: 200px; height: 30px; font-size: 16px; padding: 0 8px; border: 0",
                    oninput: move |_v: String| {},
                }
                div {
                    tabindex: "0",
                    style: "width: 50px; height: 20px",
                    onkeydown: move |k| k.key == "F2",
                }
                if show.get() {
                    {listening_branch(__scope, bl.clone())}
                }
            }
        };
        let kids = root.children();
        *hs.borrow_mut() = Some([kids[0].clone(), kids[1].clone()]);
        let canvas = crate::render_surface::RenderSurface {
            surface: Some(sf.clone()),
        }
        .render(__scope, &[]);
        root.append_child(&canvas);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let [input, tile] = handles.borrow_mut().take().expect("handles");
    Fixture {
        app,
        input,
        tile,
        surface,
        show,
        branch_log,
    }
}

impl Fixture {
    fn down(&mut self, key: KeyCode, text: Option<&str>) -> Vec<AppAction> {
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        )
    }

    fn up(&mut self, key: KeyCode) {
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers: Modifiers::default(),
            },
            VP,
            1.0,
        );
    }

    fn press(&mut self, key: KeyCode, text: Option<&str>) {
        self.down(key, text);
        self.up(key);
    }

    fn click(&mut self, node: &NodeHandle) {
        let (x, y) = {
            let d = self.app.doc.as_ref().unwrap().borrow();
            let (x, y, w, h) = painted_element_box(&d.tree, node.node_id().0);
            (x + w.min(6.0) / 2.0, y + h / 2.0)
        };
        for ev in [
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
        ] {
            self.app.handle_event(ev, VP, 1.0);
        }
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }

    fn value(&self, node: &NodeHandle) -> String {
        let d = self.app.doc.as_ref().unwrap().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, node.node_id().0)
            .unwrap_or_default()
            .into_owned()
    }
}

/// A listener that logs every key it hears as `"<tag>:<Down|Up>:<key>"` and
/// consumes the keys `takes` names.
fn listen(tag: &'static str, log: &Log, takes: &'static [&'static str]) {
    let l = log.clone();
    on_app_key(move |k: &KeyEventData| {
        l.borrow_mut().push(format!("{tag}:{:?}:{}", k.kind, k.key));
        takes.contains(&k.key.as_str())
    });
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// The reported hole: with nothing focused no handler heard a key. A
/// listener hears the press and the release.
#[test]
fn with_nothing_focused_a_listener_hears_presses_and_releases() {
    let mut f = mount();
    assert!(matches!(f.app.focus_target, FocusTarget::None));
    let log: Log = Default::default();
    listen("app", &log, &[]);
    f.press(KeyCode::Delete, None);
    f.press(KeyCode::KeyZ, Some("z"));
    assert_eq!(
        take(&log),
        ["app:Down:Delete", "app:Up:Delete", "app:Down:z", "app:Up:z"]
    );
}

/// Two parties: both hear a key, in registration order (the branch's listener
/// was registered at mount, before this one), and the first to consume a press
/// stops the later one — and the release still reaches both.
#[test]
fn two_listeners_hear_in_order_and_a_consumed_press_stops_the_later_one() {
    let mut f = mount();
    let log = f.branch_log.clone();
    listen("app", &log, &["Escape"]);
    f.press(KeyCode::KeyQ, Some("q"));
    assert_eq!(
        take(&log),
        ["branch:q", "app:Down:q", "branch:q", "app:Up:q"]
    );

    // The branch listener is first and consumes nothing; register a third
    // that is offered only after "app" consumed.
    let late: Log = Default::default();
    listen("late", &late, &[]);
    f.press(KeyCode::Escape, None);
    assert_eq!(
        take(&late),
        ["late:Up:Escape"],
        "the press stopped at \"app\""
    );
}

/// A key typed into a focused field reaches the listener before the field's
/// own editing; a consumed key is not inserted, an unconsumed one is.
#[test]
fn a_focused_field_offers_its_keys_and_a_consumed_one_is_not_typed() {
    let mut f = mount();
    let input = f.input.clone();
    f.click(&input);
    assert!(matches!(f.app.focus_target, FocusTarget::Input(_)));
    let log: Log = Default::default();
    listen("app", &log, &["x"]);
    // The guide's "leave fields alone" check.
    let tag = Rc::new(RefCell::new(None));
    let (t, r) = (tag.clone(), input.clone());
    on_app_key(move |_| {
        *t.borrow_mut() = r.active_element().and_then(|e| e.tag_name());
        false
    });
    f.press(KeyCode::KeyA, Some("a"));
    assert_eq!(tag.borrow().as_deref(), Some("input"));
    f.press(KeyCode::KeyX, Some("x"));
    f.press(KeyCode::KeyB, Some("b"));
    assert_eq!(f.value(&input), "ab", "x was consumed");
    assert_eq!(
        take(&log),
        [
            "app:Down:a",
            "app:Up:a",
            "app:Down:x",
            "app:Up:x",
            "app:Down:b",
            "app:Up:b"
        ]
    );
}

/// A consumed Tab moves no focus: the listener runs before the default.
#[test]
fn a_consumed_tab_moves_no_focus() {
    let mut f = mount();
    let input = f.input.clone();
    f.click(&input);
    let before = f.app.focus_target;
    let log: Log = Default::default();
    listen("app", &log, &["Tab"]);
    f.press(KeyCode::Tab, None);
    assert_eq!(f.app.focus_target, before);
    assert_eq!(take(&log), ["app:Down:Tab", "app:Up:Tab"]);
}

/// Everything ahead of the listeners wins: the element's own `onkeydown`, the
/// interceptor and the dismiss stack each consume a press it then never hears.
#[test]
fn earlier_stages_that_consume_a_press_keep_it_from_the_listeners() {
    let mut f = mount();
    let tile = f.tile.clone();
    tile.focus();
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert!(matches!(f.app.focus_target, FocusTarget::Node(_)));
    let log: Log = Default::default();
    listen("app", &log, &[]);

    // The tile's onkeydown takes F2.
    f.press(KeyCode::F2, None);
    assert_eq!(take(&log), ["app:Up:F2"]);

    // A dismiss handler takes Escape.
    let doc_key = f.app.doc_key();
    let _dismiss = push_dismiss_handler(doc_key, || true);
    f.press(KeyCode::Escape, None);
    assert_eq!(take(&log), ["app:Up:Escape"]);

    // The interceptor takes F3.
    set_keyboard_interceptor(|k| k.key == "F3");
    f.press(KeyCode::F3, None);
    assert_eq!(take(&log), ["app:Up:F3"]);
    // ... and passes F4 on.
    f.press(KeyCode::F4, None);
    assert_eq!(take(&log), ["app:Down:F4", "app:Up:F4"]);
}

/// A focused render surface keeps the keys it claims — press and release —
/// and the listeners hear the rest.
#[test]
fn a_focused_surface_keeps_its_claimed_keys_and_passes_the_rest() {
    let mut f = mount();
    f.app.focus_target = FocusTarget::Surface(f.surface.id());
    f.surface.set_key_handler(|k| k.code == "KeyW");
    let log: Log = Default::default();
    listen("app", &log, &[]);
    f.press(KeyCode::KeyW, Some("w"));
    f.press(KeyCode::KeyK, Some("k"));
    assert_eq!(take(&log), ["app:Down:k", "app:Up:k"]);
}

/// A listener consumed while a surface is focused stops the global fallback
/// (here: inspect mode would otherwise toggle).
#[test]
fn a_listener_consuming_an_unclaimed_surface_key_stops_the_fallback() {
    let mut f = mount();
    f.app.focus_target = FocusTarget::Surface(f.surface.id());
    on_app_key(|k| k.key == "F12");
    let actions = f.down(KeyCode::F12, None);
    assert!(!actions.contains(&AppAction::ToggleDevTools), "{actions:?}");
}

/// Unmounting the component that registered a listener releases it.
#[test]
fn unmounting_the_registering_component_releases_its_listener() {
    let mut f = mount();
    f.press(KeyCode::KeyQ, Some("q"));
    assert_eq!(take(&f.branch_log), ["branch:q", "branch:q"]);
    let before = rinch_core::events::app_key_listener_count();
    f.show.set(false);
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(rinch_core::events::app_key_listener_count(), before - 1);
    f.press(KeyCode::KeyQ, Some("q"));
    assert!(take(&f.branch_log).is_empty());
}

/// A focused registered widget (`register_focus_target` with `on_key`) gets
/// first refusal on its own keys, ahead of the app listeners — it owns the
/// keyboard, as a focused element's own keydown handler runs before a
/// browser's `window` listener. What it leaves, the listeners hear.
#[test]
fn a_focused_registered_widget_takes_its_keys_before_the_listeners() {
    let mut f = mount();
    let tile = f.tile.clone();
    let widget: Log = Default::default();
    let w = widget.clone();
    // Registered outside any render: app lifetime, released with the thread.
    crate::focus_registry::register_focus_target(
        &tile,
        crate::focus_registry::FocusEntry::new().on_key(move |k| {
            w.borrow_mut().push(k.key.clone());
            k.key == "ArrowDown"
        }),
    );
    tile.focus();
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert!(matches!(f.app.focus_target, FocusTarget::Node(_)));
    let log: Log = Default::default();
    listen("app", &log, &["ArrowDown", "ArrowUp"]);
    f.press(KeyCode::ArrowDown, None);
    f.press(KeyCode::ArrowUp, None);
    assert_eq!(
        take(&log),
        ["app:Up:ArrowDown", "app:Down:ArrowUp", "app:Up:ArrowUp"]
    );
    // The widget is offered its releases too, after the listeners.
    assert_eq!(
        widget.borrow().iter().filter(|k| *k == "ArrowDown").count(),
        2,
        "{:?}",
        widget.borrow()
    );
    assert!(
        widget.borrow().contains(&"ArrowUp".to_string()),
        "{:?}",
        widget.borrow()
    );
}
