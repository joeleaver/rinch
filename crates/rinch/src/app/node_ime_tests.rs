//! IME for a registered focus target (issue #176).
//!
//! Desktop IME is two halves, and both used to be closed to anything but the
//! rich-text editor and a built-in `<input>`: `ime_state()` decides whether the
//! window's input method is switched **on** at all, and the `PlatformEvent::Ime`
//! arm decides who the composition is **delivered** to. A custom text component
//! that registers [`FocusEntry::on_ime`](crate::focus_registry::FocusEntry::on_ime)
//! now participates in both, through the same arbiter — so these tests pin the
//! pair together. Fixing one alone is silent: routing without enablement is dead
//! code (the OS never composes), enablement without routing swallows the user's
//! typing.
//!
//! The other half of the contract is what does **not** happen: a focusable node
//! that is not a text target must not turn the OS input method on, and an
//! unmounted target must receive nothing at all (issue #141 PR4 — calling back
//! into a disposed scope reads freed signals and panics).

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use std::cell::Cell;

/// A caret rect the test can move, so "the candidate box follows the caret" is
/// observable without a text engine.
type Caret = Rc<Cell<Option<(f32, f32, f32, f32)>>>;

/// A mounted app plus everything the assertions need: the two widgets' node
/// ids, the ordered IME log, and widget A's movable caret.
struct Fixture {
    app: RinchApp,
    /// A text target: registers `on_ime` + `caret_rect`.
    text_id: usize,
    /// A focusable non-text target: registers the focus lifecycle but no `on_ime`.
    plain_id: usize,
    log: Rc<RefCell<Vec<String>>>,
    caret: Caret,
}

/// How a test writes down what it saw, so an event delivered to the wrong
/// widget is as visible as one not delivered at all.
fn describe(tag: &str, e: &ImeEvent) -> String {
    match e {
        ImeEvent::Enabled => format!("{tag}:enabled"),
        ImeEvent::Preedit { text, cursor } => format!("{tag}:preedit:{text}:{cursor:?}"),
        ImeEvent::Commit(text) => format!("{tag}:commit:{text}"),
        ImeEvent::DeleteSurrounding { before, after } => {
            format!("{tag}:delete:{before}:{after}")
        }
        ImeEvent::Disabled => format!("{tag}:disabled"),
    }
}

/// A `tabindex="0"` div registered as a **text** target: it consumes
/// composition and reports a caret.
fn text_widget(
    scope: &mut RenderScope,
    tag: &'static str,
    log: &Rc<RefCell<Vec<String>>>,
    caret: &Caret,
) -> NodeHandle {
    let div = scope.create_element("div");
    div.set_attribute("style", "width: 200px; height: 40px");
    div.set_attribute("tabindex", "0");
    register_focus_target(
        &div,
        FocusEntry::new()
            .on_ime({
                let log = log.clone();
                move |e| log.borrow_mut().push(describe(tag, e))
            })
            .caret_rect({
                let caret = caret.clone();
                move || caret.get()
            }),
    );
    div
}

/// A `tabindex="0"` div registered for the focus lifecycle but **not** for
/// composition — a card, a toolbar button, a custom checkbox.
fn plain_widget(
    scope: &mut RenderScope,
    tag: &'static str,
    log: &Rc<RefCell<Vec<String>>>,
) -> NodeHandle {
    let div = scope.create_element("div");
    div.set_attribute("style", "width: 200px; height: 40px");
    div.set_attribute("tabindex", "0");
    register_focus_target(
        &div,
        FocusEntry::new().on_focus_gained({
            let log = log.clone();
            move || log.borrow_mut().push(format!("{tag}:gained"))
        }),
    );
    div
}

fn mount_fixture() -> Fixture {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let caret: Caret = Rc::new(Cell::new(Some((10.0, 20.0, 2.0, 16.0))));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));

    let (log_in, caret_in, ids_in) = (log.clone(), caret.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let t = text_widget(scope, "t", &log_in, &caret_in);
        let p = plain_widget(scope, "p", &log_in);
        root.append_child(&t);
        root.append_child(&p);
        ids_in.set(Some((t.node_id().0, p.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (text_id, plain_id) = ids.get().expect("node ids captured at mount");
    Fixture {
        app,
        text_id,
        plain_id,
        log,
        caret,
    }
}

fn abs_center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, ax_w, ay_h) = painted_element_box(&d.tree, id);
    (ax + ax_w / 2.0, ay + ay_h / 2.0)
}

fn click_node(app: &mut RinchApp, id: usize) {
    let (x, y) = abs_center(app, id);
    for down in [true, false] {
        let event = if down {
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            }
        } else {
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            }
        };
        app.handle_event(event, (800, 600), 1.0);
    }
}

fn ime(app: &mut RinchApp, event: ImeEvent) {
    app.handle_event(PlatformEvent::Ime(event), (800, 600), 1.0);
}

fn preedit(app: &mut RinchApp, text: &str) {
    ime(
        app,
        ImeEvent::Preedit {
            text: text.to_string(),
            cursor: None,
        },
    );
}

fn window_focus(app: &mut RinchApp, focused: bool) {
    app.handle_event(PlatformEvent::WindowFocus(focused), (800, 600), 1.0);
}

fn log_of(f: &Fixture) -> Vec<String> {
    f.log.borrow().clone()
}

// ── 1. enablement ───────────────────────────────────────────────────────────

/// The half the issue calls out as most likely to be missed: without this the
/// window's IME is never switched on, so no composition is ever generated and
/// the routing below is dead code.
#[test]
fn a_registered_text_target_enables_ime_with_its_own_caret_rect() {
    let mut f = mount_fixture();
    assert!(!f.app.ime_state().enabled, "nothing focused yet, so no IME");

    click_node(&mut f.app, f.text_id);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.text_id));
    let state = f.app.ime_state();
    assert!(state.enabled, "a focused custom text target drives IME");
    assert_eq!(
        state.cursor_area,
        Some((10.0, 20.0, 2.0, 16.0)),
        "the candidate box sits at the target's own caret rect"
    );
}

/// The counterfactual to the test above: "enable IME for `FocusTarget::Node`"
/// is the wrong fix. A focusable node that consumes no composition must leave
/// the OS input method off — switching it on would hand the IM keys the widget
/// wanted (and pop a candidate window over a button).
#[test]
fn a_focusable_node_that_is_not_a_text_target_drives_no_ime() {
    let mut f = mount_fixture();

    click_node(&mut f.app, f.plain_id);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.plain_id));
    assert!(
        log_of(&f).contains(&"p:gained".to_string()),
        "…and it really is focused: {:?}",
        log_of(&f)
    );
    assert!(
        !f.app.ime_state().enabled,
        "a registration without `on_ime` is not a text target"
    );
}

/// `caret_rect` is re-read on every reconcile, not cached at focus time, so the
/// OS candidate box tracks the caret as it moves. A stale rect puts the box in
/// the wrong place, which is the whole point of the seam.
#[test]
fn the_candidate_box_follows_the_caret() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.text_id);
    assert_eq!(f.app.ime_state().cursor_area, Some((10.0, 20.0, 2.0, 16.0)));

    f.caret.set(Some((120.0, 44.0, 2.0, 18.0)));
    assert_eq!(
        f.app.ime_state().cursor_area,
        Some((120.0, 44.0, 2.0, 18.0)),
        "the rect is polled, not captured"
    );

    // No caret right now (an empty selection-less state) is not "no IME".
    f.caret.set(None);
    let state = f.app.ime_state();
    assert!(state.enabled, "still a text target");
    assert_eq!(
        state.cursor_area, None,
        "placement falls back to the platform"
    );
}

// ── 2. delivery ─────────────────────────────────────────────────────────────

/// The routing half: all five portable `ImeEvent` variants reach the focused
/// registered target, unchanged — the same contract the editor and `<input>`
/// engines consume.
#[test]
fn every_composition_event_reaches_the_focused_target() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.text_id);

    ime(&mut f.app, ImeEvent::Enabled);
    ime(
        &mut f.app,
        ImeEvent::Preedit {
            text: "にほん".into(),
            cursor: Some((0, 9)),
        },
    );
    ime(&mut f.app, ImeEvent::Commit("日本".into()));
    ime(
        &mut f.app,
        ImeEvent::DeleteSurrounding {
            before: 2,
            after: 1,
        },
    );
    ime(&mut f.app, ImeEvent::Disabled);

    assert_eq!(
        log_of(&f),
        vec![
            "t:enabled".to_string(),
            "t:preedit:にほん:Some((0, 9))".to_string(),
            "t:commit:日本".to_string(),
            "t:delete:2:1".to_string(),
            "t:disabled".to_string(),
        ],
        "every variant, in order, verbatim"
    );
}

/// Composition goes to whoever owns the keyboard and nobody else — the arbiter
/// is the router, so an unfocused text target hears nothing.
#[test]
fn composition_does_not_reach_an_unfocused_target() {
    let mut f = mount_fixture();

    preedit(&mut f.app, "にほん");
    assert!(
        log_of(&f).is_empty(),
        "nothing is focused: {:?}",
        log_of(&f)
    );

    click_node(&mut f.app, f.plain_id);
    preedit(&mut f.app, "にほん");
    assert!(
        !log_of(&f).iter().any(|e| e.starts_with("t:")),
        "the text target does not hold the claim: {:?}",
        log_of(&f)
    );
}

// ── 3. the silence rule (issue #141 PR4) ────────────────────────────────────

/// An unmounted target receives **neither** half: it drives no IME and gets no
/// composition. Its scope disposal deregistered it, so both lookups miss — the
/// same push-notification liveness the focus callbacks rely on, not an
/// attribute probe. Calling back here would read freed signals and panic.
#[test]
fn an_unmounted_target_gets_no_ime_state_and_no_composition() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let caret: Caret = Rc::new(Cell::new(Some((10.0, 20.0, 2.0, 16.0))));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let visible = rinch_core::Signal::new(true);

    let (log_in, caret_in, id_in) = (log.clone(), caret.clone(), id.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let parent = NodeHandle::new(root.node_id(), scope.doc_weak());
        rinch_core::show_dom(
            scope,
            &parent,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let w = text_widget(s, "t", &log_in, &caret_in);
                id_in.set(Some(w.node_id().0));
                w
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let t_id = id.get().expect("branch rendered at mount");

    click_node(&mut app, t_id);
    assert_eq!(app.focus_target, FocusTarget::Node(t_id));
    assert!(app.ime_state().enabled);

    // Unmount the branch: `show_dom` disposes its scope, which deregisters.
    visible.set(false);
    app.resolve_and_repaint(800.0, 600.0);

    assert!(
        !app.ime_state().enabled,
        "an unmounted target must not keep the OS input method on"
    );

    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Commit("日本".into())),
        (800, 600),
        1.0,
    );
    assert!(
        log.borrow().is_empty(),
        "an unmounted target must receive nothing: {:?}",
        log.borrow()
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "the arbiter releases the vanished claim"
    );
}

// ── 4. window blur ──────────────────────────────────────────────────────────

/// Notify-and-retain (issue #147 decision 1, issue #226) has to hold for a text
/// target too: the claim survives an alt-tab, but the OS candidate box must
/// follow the window that actually has the keyboard — so IME reports disabled
/// while blurred, and comes back on refocus.
#[test]
fn window_blur_disables_ime_without_releasing_the_registered_target() {
    let mut f = mount_fixture();
    click_node(&mut f.app, f.text_id);
    assert!(f.app.ime_state().enabled);

    window_focus(&mut f.app, false);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.text_id),
        "window blur keeps the in-document claim"
    );
    assert!(!f.app.ime_state().enabled, "a blurred window drives no IME");

    window_focus(&mut f.app, true);
    let state = f.app.ime_state();
    assert!(state.enabled, "…and it comes back");
    assert_eq!(state.cursor_area, Some((10.0, 20.0, 2.0, 16.0)));
}
