//! The focus lifecycle a custom keyboard-owning component can observe
//! (issue #147).
//!
//! A `tabindex` node registered through
//! [`register_focus_target`](crate::focus_registry::register_focus_target)
//! hears three things the closed `FocusTarget` enum could never tell it: that
//! it gained the keyboard, that it lost it, and every key while it holds it.
//! These tests pin the *lifecycle* — who owns the claim, when each callback
//! fires, and (test `an_unmounted_registered_target_releases_focus_without_calling_back`)
//! when one deliberately does **not**.

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// A mounted app plus everything the assertions need: node ids, the ordered
/// callback log, the per-widget click counters, and the mutable set of keys the
/// widgets' `on_key` should consume.
struct Fixture {
    app: RinchApp,
    input_id: usize,
    a_id: usize,
    b_id: usize,
    log: Rc<RefCell<Vec<String>>>,
    consume: Rc<RefCell<Vec<String>>>,
}

/// Build one registered focus target: a `tabindex="0"` div with a click handler
/// and all three callbacks, each recording `"{tag}:…"` into the shared log.
fn widget(
    scope: &mut RenderScope,
    tag: &'static str,
    log: &Rc<RefCell<Vec<String>>>,
    consume: &Rc<RefCell<Vec<String>>>,
) -> NodeHandle {
    let div = scope.create_element("div");
    div.set_attribute("style", "width: 200px; height: 40px");
    div.set_attribute("tabindex", "0");
    let rid = scope.register_handler({
        let log = log.clone();
        move || log.borrow_mut().push(format!("{tag}:click"))
    });
    div.set_attribute("data-rid", &rid.0.to_string());
    register_focus_target(
        &div,
        FocusEntry::new()
            .on_focus_gained({
                let log = log.clone();
                move || log.borrow_mut().push(format!("{tag}:gained"))
            })
            .on_focus_lost({
                let log = log.clone();
                move || log.borrow_mut().push(format!("{tag}:lost"))
            })
            .on_key({
                let log = log.clone();
                let consume = consume.clone();
                move |k| {
                    // Presses and releases are logged under different names
                    // (issue #337): `on_key` sees both now, so a test that
                    // counted `key:` entries would otherwise double silently.
                    let phase = if k.is_up() { "keyup" } else { "key" };
                    log.borrow_mut().push(format!("{tag}:{phase}:{}", k.key));
                    consume.borrow().iter().any(|c| c == &k.key)
                }
            }),
    );
    div
}

/// `<input>`, then two registered `tabindex="0"` widgets A and B, in DOM order.
fn mount_fixture() -> Fixture {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let consume: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let oninput_id = register_input_handler(InputCallback::new(|_| {}));
    let ids: Rc<Cell<Option<(usize, usize, usize)>>> = Rc::new(Cell::new(None));

    let (log_in, consume_in, ids_in) = (log.clone(), consume.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        input.set_attribute("data-oninput", &oninput_id.0.to_string());
        let a = widget(scope, "a", &log_in, &consume_in);
        let b = widget(scope, "b", &log_in, &consume_in);
        root.append_child(&input);
        root.append_child(&a);
        root.append_child(&b);
        ids_in.set(Some((input.node_id().0, a.node_id().0, b.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (input_id, a_id, b_id) = ids.get().expect("node ids captured at mount");
    Fixture {
        app,
        input_id,
        a_id,
        b_id,
        log,
        consume,
    }
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
}

fn tab(app: &mut RinchApp) {
    key(app, KeyCode::Tab, None);
}

fn abs_center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, ax_w, ay_h) = painted_element_box(&d.tree, id);
    (ax + ax_w / 2.0, ay + ay_h / 2.0)
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

fn click_node(app: &mut RinchApp, id: usize) {
    let (x, y) = abs_center(app, id);
    click(app, x, y);
}

fn window_focus(app: &mut RinchApp, focused: bool) {
    app.handle_event(PlatformEvent::WindowFocus(focused), (800, 600), 1.0);
}

fn log_of(f: &Fixture) -> Vec<String> {
    f.log.borrow().clone()
}

fn count(log: &[String], entry: &str) -> usize {
    log.iter().filter(|e| e.as_str() == entry).count()
}

// ── 1. blur ─────────────────────────────────────────────────────────────────

/// The reported defect: a custom focusable can see that it *gained* focus but
/// never that it *lost* it. Tab in, Tab out — `on_focus_lost` fires exactly
/// once, and the successor's `on_focus_gained` fires too.
#[test]
fn blur_notifies_a_registered_target() {
    let mut f = mount_fixture();

    tab(&mut f.app); // → the <input>
    assert_eq!(f.app.focused_input_node_id, Some(f.input_id));
    tab(&mut f.app); // → widget A
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));
    assert_eq!(
        count(&log_of(&f), "a:gained"),
        1,
        "Tab onto A announces the gain: {:?}",
        log_of(&f)
    );
    assert_eq!(count(&log_of(&f), "a:lost"), 0);

    tab(&mut f.app); // → widget B
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.b_id));
    let log = log_of(&f);
    assert_eq!(
        count(&log, "a:lost"),
        1,
        "Tab off A must announce the loss exactly once: {log:?}"
    );
    assert_eq!(count(&log, "b:gained"), 1, "B took the keyboard: {log:?}");
}

/// Blur is announced whoever takes the keyboard next — including a built-in
/// text engine, which is the case a custom widget cannot observe any other way.
#[test]
fn an_input_claiming_focus_blurs_a_registered_target() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.a_id);
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));

    click_node(&mut f.app, f.input_id);
    assert_eq!(f.app.focus_target, FocusTarget::Input(f.input_id));
    assert_eq!(
        count(&log_of(&f), "a:lost"),
        1,
        "the <input> taking focus blurs the registered node: {:?}",
        log_of(&f)
    );
}

// ── 2. mousedown claims the keyboard ────────────────────────────────────────

/// Decision 2 (web parity): a pointer press on a `tabindex` node claims
/// `FocusTarget::Node`, so a click-focused custom control has live keys
/// immediately instead of only after being reached by Tab.
#[test]
fn a_click_on_a_registered_node_claims_the_keyboard() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.a_id);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.a_id),
        "mousedown claims the arbiter, not just DOM :focus"
    );
    assert_eq!(count(&log_of(&f), "a:gained"), 1);

    key(&mut f.app, KeyCode::Enter, None);
    let log = log_of(&f);
    assert!(
        log.contains(&"a:key:Enter".to_string()),
        "the click-focused widget receives Enter: {log:?}"
    );
}

/// The claim walks up to the nearest focusable ancestor, like a browser: a
/// press on a child of the widget focuses the *widget*.
#[test]
fn a_click_on_a_child_claims_the_focusable_ancestor() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let consume: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let (log_in, consume_in, ids_in) = (log.clone(), consume.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let a = widget(scope, "a", &log_in, &consume_in);
        let child = scope.create_element("div");
        child.set_attribute("style", "width: 100px; height: 20px");
        a.append_child(&child);
        root.append_child(&a);
        ids_in.set(Some((a.node_id().0, child.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a_id, child_id) = ids.get().unwrap();

    click_node(&mut app, child_id);
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(a_id),
        "the claim resolves to the focusable ancestor, not the hit child"
    );
    assert_eq!(count(&log.borrow(), "a:gained"), 1);
}

// ── 3. mutual exclusion ─────────────────────────────────────────────────────

/// Exactly one owner at a time. The reporter's workaround (a registry of their
/// own, revoked from every sibling on mousedown) exists only because the
/// runtime would not do this.
#[test]
fn two_registered_widgets_blur_each_other() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.a_id);
    click_node(&mut f.app, f.b_id);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.b_id));
    let focus_log: Vec<String> = log_of(&f)
        .into_iter()
        .filter(|e| e.ends_with(":gained") || e.ends_with(":lost"))
        .collect();
    assert_eq!(
        focus_log,
        vec![
            "a:gained".to_string(),
            "a:lost".to_string(),
            "b:gained".to_string()
        ],
        "one owner at a time, in order"
    );

    // And only B's keys are routed now.
    key(&mut f.app, KeyCode::ArrowDown, None);
    let log = log_of(&f);
    assert!(log.contains(&"b:key:ArrowDown".to_string()), "{log:?}");
    assert!(
        !log.contains(&"a:key:ArrowDown".to_string()),
        "the blurred widget must not still receive keys: {log:?}"
    );
}

// ── 4. unmount ──────────────────────────────────────────────────────────────

/// Decision 5: unmounting a focused registered target deregisters it
/// **silently**. Firing `on_focus_lost` here would run user code against a
/// scope whose signals were just freed and panic (issue #141 PR4).
///
/// It also pins the #304 recycled-slot hazard for registered targets: the
/// arbiter releases the claim because the registration is *gone*, a push
/// notification, not because an attribute probe happened to fail.
#[test]
fn an_unmounted_registered_target_releases_focus_without_calling_back() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let consume: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let visible = rinch_core::Signal::new(true);

    let (log_in, consume_in, id_in) = (log.clone(), consume.clone(), id.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let parent = NodeHandle::new(root.node_id(), scope.doc_weak());
        rinch_core::show_dom(
            scope,
            &parent,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let w = widget(s, "a", &log_in, &consume_in);
                id_in.set(Some(w.node_id().0));
                w
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let a_id = id.get().expect("branch rendered at mount");

    click_node(&mut app, a_id);
    assert_eq!(app.focus_target, FocusTarget::Node(a_id));
    assert_eq!(count(&log.borrow(), "a:gained"), 1);

    // Unmount the branch: `show_dom` disposes its scope, which deregisters.
    visible.set(false);
    app.resolve_and_repaint(800.0, 600.0);

    key(&mut app, KeyCode::Enter, None);
    let l = log.borrow().clone();
    assert_eq!(
        count(&l, "a:lost"),
        0,
        "unmount must NOT call back into the disposed scope: {l:?}"
    );
    assert!(
        !l.iter().any(|e| e.starts_with("a:key:")),
        "an unmounted target must not still receive keys: {l:?}"
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "the arbiter releases the vanished claim"
    );
}

// ── 5. window blur ──────────────────────────────────────────────────────────

/// Decision 1: `WindowFocus(false)` **notifies and retains**. Releasing to
/// `None` would fire `data-onchange` on every alt-tab — a #226 regression — so
/// the claim stays and the target is re-notified when the window comes back.
#[test]
fn window_blur_notifies_the_focused_target_and_keeps_the_claim() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.a_id);
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));

    window_focus(&mut f.app, false);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.a_id),
        "window blur must KEEP the in-document claim"
    );
    assert_eq!(
        count(&log_of(&f), "a:lost"),
        1,
        "…but announce it: {:?}",
        log_of(&f)
    );

    window_focus(&mut f.app, true);
    assert_eq!(
        count(&log_of(&f), "a:gained"),
        2,
        "refocus re-announces the gain: {:?}",
        log_of(&f)
    );
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));

    // A repeat of the same state is not a second notification.
    window_focus(&mut f.app, true);
    assert_eq!(count(&log_of(&f), "a:gained"), 2);
}

/// The observable half of "the claim is retained": while the window is blurred
/// the runtime reports IME disabled, so the OS candidate box follows the window
/// that actually has the keyboard — even though the `<input>` still owns the
/// in-document claim.
#[test]
fn window_blur_disables_ime_without_releasing_the_input() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.input_id);
    assert_eq!(f.app.focus_target, FocusTarget::Input(f.input_id));
    assert!(f.app.ime_state().enabled, "a focused input drives IME");

    window_focus(&mut f.app, false);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Input(f.input_id),
        "the input keeps focus across window blur (no #226 onchange storm)"
    );
    assert!(!f.app.ime_state().enabled, "a blurred window drives no IME");

    window_focus(&mut f.app, true);
    assert!(f.app.ime_state().enabled, "…and it comes back");
}

// ── 6. counterfactual guard + key consumption ───────────────────────────────

/// The counterfactual: "scope the callbacks by focus" is trivially satisfiable
/// by never firing them at all. A focused registered target must actually see
/// its own keys — named keys, text keys, and the Enter/Space the runtime also
/// wants.
#[test]
fn a_registered_target_still_receives_its_own_keys() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.a_id);

    key(&mut f.app, KeyCode::ArrowDown, None);
    key(&mut f.app, KeyCode::KeyX, Some("x"));
    key(&mut f.app, KeyCode::Enter, None);
    key(&mut f.app, KeyCode::Space, Some(" "));

    let log = log_of(&f);
    for expected in ["a:key:ArrowDown", "a:key:x", "a:key:Enter", "a:key:Space"] {
        assert!(
            log.contains(&expected.to_string()),
            "missing {expected}: {log:?}"
        );
    }
}

/// `on_key` returning `true` consumes the key: the runtime's own Enter/Space
/// activation must not also run.
#[test]
fn a_consumed_key_does_not_reach_the_runtime() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.a_id);
    f.log.borrow_mut().clear();

    // Not consumed: Enter activates the node's click handler.
    key(&mut f.app, KeyCode::Enter, None);
    assert_eq!(
        count(&log_of(&f), "a:click"),
        1,
        "unconsumed Enter still activates: {:?}",
        log_of(&f)
    );

    f.consume.borrow_mut().push("Enter".to_string());
    key(&mut f.app, KeyCode::Enter, None);
    let log = log_of(&f);
    assert_eq!(
        count(&log, "a:key:Enter"),
        2,
        "the widget saw both presses: {log:?}"
    );
    assert_eq!(
        count(&log, "a:click"),
        1,
        "a consumed Enter must not also activate the node: {log:?}"
    );
}

/// A consumed Tab keeps focus where it is — the crispest proof that
/// consumption short-circuits the global fallthrough (Tab navigation lives
/// there).
#[test]
fn a_consumed_tab_does_not_move_focus() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.a_id);
    f.consume.borrow_mut().push("Tab".to_string());

    tab(&mut f.app);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.a_id),
        "the widget swallowed Tab, so focus did not move"
    );
    assert!(log_of(&f).contains(&"a:key:Tab".to_string()));
}

// ── Programmatic focus ──────────────────────────────────────────────────────

/// `request_focus` / `NodeHandle::focus()` announce the gain too — the
/// programmatic path is the same arbiter transition as Tab.
#[test]
fn programmatic_focus_announces_the_gain() {
    let mut f = mount_fixture();

    f.app.try_focus_input(f.a_id);
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));
    assert_eq!(count(&log_of(&f), "a:gained"), 1, "{:?}", log_of(&f));

    f.app.try_focus_input(f.b_id);
    let log = log_of(&f);
    assert_eq!(count(&log, "a:lost"), 1, "{log:?}");
    assert_eq!(count(&log, "b:gained"), 1, "{log:?}");
}

/// Re-focusing the target that already holds the claim is **not** a new gain.
///
/// `set_focus_target_deferred` reports "no change" for a re-focus, but the
/// announce used to run unconditionally afterwards — so `request_focus` on the
/// focused node, `Tab` in a document with a single focusable, or a
/// `NodeHandle::focus()` inside a re-running effect handed the widget a second
/// `on_focus_gained` with no `on_focus_lost` between them. A component that
/// pairs the two (start/stop a blink timer, push/pop a keymap) leaks one per
/// repeat.
#[test]
fn refocusing_the_same_target_does_not_re_announce_the_gain() {
    let mut f = mount_fixture();

    // Programmatic (`request_focus` / `NodeHandle::focus()`).
    f.app.try_focus_input(f.a_id);
    assert_eq!(count(&log_of(&f), "a:gained"), 1);
    f.app.try_focus_input(f.a_id);

    // Keyboard (`focus_element`, the Tab landing).
    f.app.focus_element(f.a_id);

    // Pointer (the mousedown claim).
    click_node(&mut f.app, f.a_id);

    let log = log_of(&f);
    assert_eq!(count(&log, "a:gained"), 1, "exactly one gain: {log:?}");
    assert_eq!(count(&log, "a:lost"), 0, "and no phantom loss: {log:?}");
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.a_id));
}
