//! The HTML focusable set: what is a desktop Tab stop, and what a focused one
//! does (issues #252, #314, #424).
//!
//! Before this, desktop focusability came from an explicit `tabindex` or a
//! `data-oninput`, and nothing else. The whole component library — `Button`,
//! `ActionIcon`, `CloseButton`, `Tab`, `AccordionControl`, `Pagination`,
//! `DropdownMenuItem`, `NavLink`, every Modal/Drawer/Alert/Notification closer,
//! the `BorderlessWindow` controls — was unreachable by keyboard, while the
//! same components are ordinary Tab stops on `rinch-web`.
//!
//! Widening the set makes a second defect matter, so it is fixed here too:
//! `try_focus_input` branched on `data-oninput` with no tag guard, and a
//! `<select>` with a change handler writes exactly that attribute. Focusing one
//! installed an `EditableState` over its `value` — a typable text field with a
//! dropdown (#424).

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Ids {
    button: usize,
    disabled_button: usize,
    stepper: usize,
    link: usize,
    bare_anchor: usize,
    select: usize,
    bare_select: usize,
    hidden_checkbox: usize,
    checkbox_label: usize,
    text_input: usize,
    summary: usize,
    plain_div: usize,
}

/// One document holding the whole tag set, plus the two shapes that must stay
/// *out*: an explicit `tabindex="-1"` stepper and a bare `<a>` with no href.
fn mount_fixture() -> (RinchApp, Ids, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let click = |tag: &'static str, log: &Rc<RefCell<Vec<String>>>| {
        let log = log.clone();
        rinch_core::register_handler(Rc::new(move || log.borrow_mut().push(tag.to_string())))
    };
    let button_rid = click("button", &log);
    let label_rid = click("label", &log);
    let select_oninput = register_input_handler(InputCallback::new(|_| {}));
    let input_oninput = register_input_handler(InputCallback::new(|_| {}));

    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let select_log = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        // `display: block` throughout: `<a>` and `<label>` are inline by
        // default and would ignore the width/height, leaving a zero-size box
        // that the collector's own filter (correctly) skips.
        let sized = |scope: &mut RenderScope, tag: &str| {
            let n = scope.create_element(tag);
            n.set_attribute("style", "display: block; width: 80px; height: 24px");
            n
        };

        let button = sized(scope, "button");
        button.set_attribute("data-rid", &button_rid.0.to_string());

        let disabled_button = sized(scope, "button");
        disabled_button.set_attribute("disabled", "");

        // The `NumberInput`/`PasswordInput` opt-out: explicit `-1` must keep
        // winning over the tag's implied `0`.
        let stepper = sized(scope, "button");
        stepper.set_attribute("tabindex", "-1");

        let link = sized(scope, "a");
        link.set_attribute("href", "https://example.com");

        let bare_anchor = sized(scope, "a");

        // The #424 shape: a `<select>` an app has a change handler on.
        let select = sized(scope, "select");
        select.set_attribute("data-oninput", &select_oninput.0.to_string());
        let option = scope.create_element("option");
        option.set_attribute("value", "a");
        let option_text = scope.create_text("A");
        option.append_child(&option_text);
        select.append_child(&option);

        // A `<select>` with **no** handler at all — the shape that reaches the
        // claim walk's tabindex arm, since a `data-oninput` one short-circuits
        // before it. Registered purely so the tests can *observe* the
        // arbiter's transitions: a Node claim taken and handed to the popup
        // inside one event is otherwise invisible from outside.
        let bare_select = sized(scope, "select");
        let bare_option = scope.create_element("option");
        bare_option.set_attribute("value", "b");
        let bare_option_text = scope.create_text("B");
        bare_option.append_child(&bare_option_text);
        bare_select.append_child(&bare_option);
        register_focus_target(
            &bare_select,
            FocusEntry::new().on_focus_gained({
                let log = select_log.clone();
                move || log.borrow_mut().push("select:gained".to_string())
            }),
        );

        // The `Checkbox` shape: a visually-hidden `<input>` with the handler on
        // the enclosing `<label>`.
        let checkbox_label = scope.create_element("label");
        checkbox_label.set_attribute("style", "display: block; width: 120px; height: 24px");
        checkbox_label.set_attribute("data-rid", &label_rid.0.to_string());
        let hidden_checkbox = sized(scope, "input");
        hidden_checkbox.set_attribute("type", "checkbox");
        checkbox_label.append_child(&hidden_checkbox);

        let text_input = sized(scope, "input");
        text_input.set_attribute("data-oninput", &input_oninput.0.to_string());

        let summary = sized(scope, "summary");
        let plain_div = sized(scope, "div");

        for n in [
            &button,
            &disabled_button,
            &stepper,
            &link,
            &bare_anchor,
            &select,
            &bare_select,
            &checkbox_label,
            &text_input,
            &summary,
            &plain_div,
        ] {
            root.append_child(n);
        }

        ids_in.set(Some(Ids {
            button: button.node_id().0,
            disabled_button: disabled_button.node_id().0,
            stepper: stepper.node_id().0,
            link: link.node_id().0,
            bare_anchor: bare_anchor.node_id().0,
            select: select.node_id().0,
            bare_select: bare_select.node_id().0,
            hidden_checkbox: hidden_checkbox.node_id().0,
            checkbox_label: checkbox_label.node_id().0,
            text_input: text_input.node_id().0,
            summary: summary.node_id().0,
            plain_div: plain_div.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ids = ids.get().expect("node ids captured at mount");
    (app, ids, log)
}

fn key(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(PlatformEvent::KeyUp { key, modifiers }, (800, 600), 1.0);
}

fn tab(app: &mut RinchApp) {
    key(app, KeyCode::Tab, Modifiers::default());
}

// ── 1. membership ───────────────────────────────────────────────────────────

#[test]
fn the_implicitly_focusable_tags_are_tab_stops() {
    let (app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    for (name, id) in [
        ("button", ids.button),
        ("a[href]", ids.link),
        ("select", ids.select),
        ("input", ids.text_input),
        ("the Checkbox's hidden input", ids.hidden_checkbox),
    ] {
        assert!(order.contains(&id), "{name} must be a Tab stop: {order:?}");
    }
}

#[test]
fn the_shapes_that_must_stay_out_stay_out() {
    let (app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    assert!(
        !order.contains(&ids.stepper),
        "an explicit tabindex=\"-1\" still wins over the tag (the NumberInput \
         and PasswordInput steppers depend on it): {order:?}"
    );
    assert!(
        !order.contains(&ids.disabled_button),
        "a disabled button is not focusable (#315): {order:?}"
    );
    assert!(
        !order.contains(&ids.bare_anchor),
        "an <a> with no href is not a link and not focusable: {order:?}"
    );
    assert!(
        !order.contains(&ids.summary),
        "<summary> is left out on purpose — rinch has no <details> disclosure \
         behaviour, so it would be a Tab stop that does nothing: {order:?}"
    );
    assert!(!order.contains(&ids.plain_div), "{order:?}");
    assert!(
        !order.contains(&ids.checkbox_label),
        "a `data-rid` alone is not focusability — the DropdownMenu's \
         full-screen backdrop carries one, and clickable cards and rows do \
         too: {order:?}"
    );
}

/// The order is DOM order, and the excluded shapes are skipped in place rather
/// than reordering what surrounds them.
///
/// Note the `<label>` is **not** in it. It carries a `data-rid` and nothing
/// else, and `data-rid` is deliberately not the focusability signal: the
/// `DropdownMenu`'s full-screen dismissal backdrop carries one, and so do
/// clickable cards, table rows and list items. The focusable thing here is the
/// hidden `<input>` the label wraps, exactly as in a browser.
#[test]
fn the_tab_order_follows_the_dom() {
    let (app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    let expected = [
        ids.button,
        ids.link,
        ids.select,
        ids.bare_select,
        ids.hidden_checkbox,
        ids.text_input,
    ];
    let filtered: Vec<usize> = order
        .iter()
        .copied()
        .filter(|id| expected.contains(id))
        .collect();
    assert_eq!(filtered, expected, "full order was {order:?}");
}

// ── 2. what a focused one does ──────────────────────────────────────────────

/// Enter on a focused `<button>` dispatches its own handler.
#[test]
fn enter_activates_a_focused_button() {
    let (mut app, ids, log) = mount_fixture();

    tab(&mut app);
    assert_eq!(app.focus_target, FocusTarget::Node(ids.button));
    key(&mut app, KeyCode::Enter, Modifiers::default());

    assert_eq!(*log.borrow(), vec!["button".to_string()]);
}

/// Space on the `Checkbox`'s hidden `<input>` walks up to the label's handler
/// — the pattern the component was already styled for
/// (`.rinch-checkbox__input:focus + .rinch-checkbox__box`), which had never had
/// a Tab stop to fire it.
#[test]
fn space_on_a_hidden_checkbox_input_activates_its_label() {
    let (mut app, ids, log) = mount_fixture();

    app.try_focus_input(ids.hidden_checkbox);
    assert_eq!(app.focus_target, FocusTarget::Node(ids.hidden_checkbox));
    key(&mut app, KeyCode::Space, Modifiers::default());

    assert_eq!(*log.borrow(), vec!["label".to_string()]);
}

// ── 3. #424: a <select> is not a text field ─────────────────────────────────

/// The live bug the widening would have multiplied. A `<select>` carrying
/// `data-oninput` — which is what any app with a change handler writes — used
/// to take `FocusTarget::Input`, installing an `EditableState` over its
/// `value`. It must take generic node focus instead: focused, and **closed**,
/// like a browser.
#[test]
fn focusing_a_select_does_not_make_it_a_text_field() {
    let (mut app, ids, _log) = mount_fixture();

    app.try_focus_input(ids.select);

    assert_eq!(
        app.focus_target,
        FocusTarget::Node(ids.select),
        "a <select> is not the text engine's"
    );
    assert_eq!(app.focused_input_node_id, None);
    assert!(
        app.focused_input_state.is_none(),
        "no EditableState over a dropdown"
    );
}

/// The consequence, driven end to end: keys aimed at a focused `<select>` must
/// not rewrite its `value`.
#[test]
fn typing_at_a_focused_select_changes_nothing() {
    let (mut app, ids, _log) = mount_fixture();
    app.try_focus_input(ids.select);

    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyZ,
            logical_key: None,
            text: Some("z".into()),
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );

    let d = app.doc.as_ref().unwrap().borrow();
    let value = d
        .tree
        .get(ids.select)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert!(
        value.is_none_or(|v| v.is_empty()),
        "a keystroke must not write into a <select>'s value"
    );
}

/// Tab reaches it the same way, through the same predicate.
#[test]
fn tab_onto_a_select_takes_node_focus_not_input_focus() {
    let (mut app, ids, _log) = mount_fixture();

    tab(&mut app); // button
    tab(&mut app); // link
    tab(&mut app); // select

    assert_eq!(app.focus_target, FocusTarget::Node(ids.select));
    assert!(app.focused_input_state.is_none());
}

// ── 4. #314: a focused <select> opens its own popup ─────────────────────────

/// Enter on a focused `<select>` opens the popup instead of walking up for an
/// ancestor's `data-rid` — which would have fired the enclosing card's or
/// form's handler, an activation landing somewhere the user was not aiming.
#[test]
fn enter_on_a_focused_select_opens_its_popup() {
    let (mut app, ids, _log) = mount_fixture();
    app.try_focus_input(ids.select);

    key(&mut app, KeyCode::Enter, Modifiers::default());

    assert_eq!(app.focus_target, FocusTarget::Select(ids.select));
}

#[test]
fn alt_down_opens_it_too() {
    let (mut app, ids, _log) = mount_fixture();
    app.try_focus_input(ids.select);

    key(
        &mut app,
        KeyCode::ArrowDown,
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );

    assert_eq!(app.focus_target, FocusTarget::Select(ids.select));
}

/// A press on a `<select>` still goes straight to its popup, and does **not**
/// take a generic node claim on the way — that would announce a gain and an
/// immediate loss on the same control. The select is registered with the focus
/// registry in the fixture purely so that transient claim is observable.
#[test]
fn a_press_on_a_select_goes_straight_to_the_popup() {
    let (mut app, ids, log) = mount_fixture();

    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, ids.bare_select);
        (ax + w / 2.0, ay + h / 2.0)
    };
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );

    assert_eq!(app.focus_target, FocusTarget::Select(ids.bare_select));
    assert!(
        !log.borrow().iter().any(|e| e == "select:gained"),
        "the mousedown claim must leave a <select> to its popup rather than \
         announcing a Node gain it hands away in the same event: {:?}",
        log.borrow()
    );
}

// ── 5. the predicate itself ─────────────────────────────────────────────────

#[test]
fn an_explicit_tabindex_always_wins_over_the_tag() {
    let (app, ids, _log) = mount_fixture();
    let d = app.doc.as_ref().unwrap().borrow();
    let of = |id: usize| RinchApp::effective_tabindex(d.tree.get(id).unwrap());

    assert_eq!(of(ids.stepper), Some(-1), "explicit -1 beats the implied 0");
    assert_eq!(of(ids.button), Some(0));
    assert_eq!(of(ids.link), Some(0));
    assert_eq!(of(ids.bare_anchor), None);
    assert_eq!(of(ids.summary), None);
    assert_eq!(of(ids.plain_div), None);
}

// ── 6. the keyboard route must not strand Tab ──────────────────────────────

/// The regression this PR introduces if left alone. Making a `<select>`
/// keyboard-reachable creates a route the mouse never took, and that route
/// used to destroy Tab's anchor:
///
/// `focus_element` sets **both** `focus_target = Node(select)` and the DOM
/// `focused_node = select`. Opening the popup runs the `Node` teardown, which
/// clears `focused_node`. Closing it sets `FocusTarget::None`, and the
/// `Select` teardown restores nothing. `handle_tab`'s fallback — which walks
/// up from `focused_node` when `focus_target` names nothing in the order —
/// then found neither, so `current_idx` was `None` and Tab restarted at the
/// **top of the document**.
///
/// Closing now returns focus to the control, which is what a browser does
/// anyway.
#[test]
fn escape_from_a_select_leaves_tab_where_it_was() {
    let (mut app, ids, _log) = mount_fixture();

    tab(&mut app); // button
    tab(&mut app); // link
    tab(&mut app); // select
    assert_eq!(app.focus_target, FocusTarget::Node(ids.select));

    key(&mut app, KeyCode::Enter, Modifiers::default());
    assert_eq!(app.focus_target, FocusTarget::Select(ids.select));
    key(&mut app, KeyCode::Escape, Modifiers::default());

    assert_eq!(
        app.focus_target,
        FocusTarget::Node(ids.select),
        "Escape returns to the closed control, browser-style"
    );

    tab(&mut app);
    assert_ne!(
        app.focus_target,
        FocusTarget::Node(ids.button),
        "Tab must not restart at the top of the document"
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(ids.bare_select),
        "it continues from the select, to the next stop after it"
    );
}

/// Committing an option returns focus to the control too — the other close
/// path a browser leaves focused.
#[test]
fn committing_an_option_leaves_focus_on_the_control() {
    let (mut app, ids, _log) = mount_fixture();

    app.try_focus_input(ids.select);
    key(&mut app, KeyCode::Enter, Modifiers::default());
    assert_eq!(app.focus_target, FocusTarget::Select(ids.select));

    // Enter on the highlighted option commits and closes.
    key(&mut app, KeyCode::Enter, Modifiers::default());

    assert_eq!(app.focus_target, FocusTarget::Node(ids.select));
    assert!(!app.is_select_open());
}

/// The **mouse** route never had the defect — its mousedown claim leaves
/// `focused_node` on the select, so the ancestor-walk fallback always had
/// something to find. Pinned so the fix is not mistaken for the thing that
/// made the mouse path work.
#[test]
fn the_mouse_route_still_anchors_tab() {
    let (mut app, ids, _log) = mount_fixture();

    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, ids.bare_select);
        (ax + w / 2.0, ay + h / 2.0)
    };
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    assert_eq!(app.focus_target, FocusTarget::Select(ids.bare_select));
    key(&mut app, KeyCode::Escape, Modifiers::default());

    tab(&mut app);
    assert_ne!(
        app.focus_target,
        FocusTarget::Node(ids.button),
        "the mouse route anchors Tab too"
    );
}

/// A change handler that **disables** the `<select>` it just committed must not
/// be handed the keyboard back.
///
/// `commit_select` deliberately restores focus *after* dispatching the
/// handlers, so by the time `focus_select_control` runs the control may have
/// become disabled under it — which is why that function re-checks. Nothing
/// exercised the re-check: removing it left the whole suite green (found by
/// mutation during review), so the restore silently handed `FocusTarget::Node`
/// to a disabled control, undoing #315 through the one door #437 opens.
#[test]
fn a_commit_handler_that_disables_the_select_does_not_get_focus_back() {
    let ids: Rc<Cell<(usize, usize)>> = Rc::new(Cell::new((0, 0)));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let before = scope.create_element("button");
        before.set_attribute("style", "display: block; width: 100px; height: 24px");

        let select = scope.create_element("select");
        select.set_attribute("style", "display: block; width: 100px; height: 24px");
        select.set_attribute("value", "a");
        // The handler disables the very control it was fired from — the shape
        // `commit_select`'s restore-after-dispatch ordering exists for.
        let disabling = select.clone();
        let handler = register_input_handler(InputCallback::new(move |_v: String| {
            disabling.set_attribute("disabled", "");
        }));
        select.set_attribute("data-oninput", &handler.0.to_string());
        for v in ["a", "b"] {
            let o = scope.create_element("option");
            o.set_attribute("value", v);
            let t = scope.create_text(v);
            o.append_child(&t);
            select.append_child(&o);
        }

        for n in [&before, &select] {
            root.append_child(n);
        }
        ids_in.set((before.node_id().0, select.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (_before, select) = ids.get();

    tab(&mut app);
    tab(&mut app);
    assert_eq!(app.focus_target, FocusTarget::Node(select));

    key(&mut app, KeyCode::Enter, Modifiers::default()); // open
    assert_eq!(app.focus_target, FocusTarget::Select(select));
    key(&mut app, KeyCode::ArrowDown, Modifiers::default()); // highlight "b"
    key(&mut app, KeyCode::Enter, Modifiers::default()); // commit -> handler disables

    assert_ne!(
        app.focus_target,
        FocusTarget::Node(select),
        "a control the commit handler disabled must not take the keyboard back"
    );
    assert_eq!(app.focus_target, FocusTarget::None);
}
