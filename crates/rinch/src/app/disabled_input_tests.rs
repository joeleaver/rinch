//! Disabled means disabled, and read-only means read-only (issue #315).
//!
//! `node_is_disabled` used to read only `data-disabled`, while the whole
//! component library writes the plain HTML `disabled` — so a
//! `TextInput { disabled: true }` was a Tab stop, took a click, and was fully
//! typable. There are three legs to that, and the third is the one that is
//! easy to miss: the field that goes disabled **while focused** stays typable
//! however carefully the focus paths are guarded, because nothing re-checks at
//! edit time.
//!
//! Read-only is the weaker neighbour and is tested here beside it precisely
//! because it is *not* the same rule: a read-only field focuses, moves its
//! caret, selects and copies — it refuses only the commands that change text.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// Ids captured at mount, in DOM order.
#[derive(Clone, Copy)]
struct Ids {
    plain: usize,
    disabled: usize,
    readonly: usize,
    data_disabled: usize,
    disabled_false: usize,
    in_fieldset: usize,
    in_legend: usize,
}

/// One document holding every case: an ordinary `<input>`, a `disabled` one, a
/// `readonly` one, a `data-disabled` one (the old spelling, which must keep
/// working), a `disabled="false"` one (the documented opt-out), and a
/// `<fieldset disabled>` wrapping one control in its `<legend>` and one below.
fn mount_fixture() -> (RinchApp, Ids, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let handlers = [
        record("plain"),
        record("disabled"),
        record("readonly"),
        record("data-disabled"),
        record("disabled-false"),
        record("in-fieldset"),
        record("in-legend"),
    ];

    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = |scope: &mut RenderScope, handler: usize| {
            let n = scope.create_element("input");
            n.set_attribute("style", "width: 200px; height: 30px");
            n.set_attribute("data-oninput", &handler.to_string());
            n
        };
        let plain = field(scope, handlers[0].0);
        let disabled = field(scope, handlers[1].0);
        disabled.set_attribute("disabled", "");
        let readonly = field(scope, handlers[2].0);
        readonly.set_attribute("readonly", "");
        let data_disabled = field(scope, handlers[3].0);
        data_disabled.set_attribute("data-disabled", "");
        let disabled_false = field(scope, handlers[4].0);
        disabled_false.set_attribute("disabled", "false");

        let fieldset = scope.create_element("fieldset");
        fieldset.set_attribute("style", "width: 400px; height: 200px");
        fieldset.set_attribute("disabled", "");
        let legend = scope.create_element("legend");
        legend.set_attribute("style", "width: 300px; height: 40px");
        let in_legend = field(scope, handlers[6].0);
        legend.append_child(&in_legend);
        // A wrapper, so the inherited disable is tested through a level of
        // nesting rather than only parent-to-child.
        let wrapper = scope.create_element("div");
        wrapper.set_attribute("style", "width: 300px; height: 60px");
        let in_fieldset = field(scope, handlers[5].0);
        wrapper.append_child(&in_fieldset);
        fieldset.append_child(&legend);
        fieldset.append_child(&wrapper);

        for n in [
            &plain,
            &disabled,
            &readonly,
            &data_disabled,
            &disabled_false,
        ] {
            root.append_child(n);
        }
        root.append_child(&fieldset);

        ids_in.set(Some(Ids {
            plain: plain.node_id().0,
            disabled: disabled.node_id().0,
            readonly: readonly.node_id().0,
            data_disabled: data_disabled.node_id().0,
            disabled_false: disabled_false.node_id().0,
            in_fieldset: in_fieldset.node_id().0,
            in_legend: in_legend.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ids = ids.get().expect("node ids captured at mount");
    (app, ids, log)
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
}

fn type_str(app: &mut RinchApp, text: &str) {
    for ch in text.chars() {
        key(app, KeyCode::KeyA, Some(&ch.to_string()));
    }
}

fn click_center(app: &mut RinchApp, id: usize) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
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

/// Set (or clear) an attribute on a live node, the way a reactive prop would.
fn set_attr(app: &mut RinchApp, id: usize, name: &str, value: Option<&str>) {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    let node = d.tree.get_mut(id).expect("node is live");
    match value {
        Some(v) => {
            node.attributes.insert(name.to_string(), v.to_string());
        }
        None => {
            node.attributes.remove(name);
        }
    }
}

fn focused_text(app: &RinchApp) -> String {
    app.focused_input_state
        .as_ref()
        .map(|s| s.document.to_text())
        .unwrap_or_default()
}

// ── 1. the Tab order ────────────────────────────────────────────────────────

/// The predicate in one assertion: only the enabled fields are Tab stops, and
/// the two disabled spellings are equally excluded.
#[test]
fn disabled_fields_are_not_tab_stops() {
    let (app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    assert!(
        order.contains(&ids.plain),
        "the ordinary field is a Tab stop"
    );
    assert!(
        order.contains(&ids.readonly),
        "read-only is not disabled — it stays reachable"
    );
    assert!(
        order.contains(&ids.disabled_false),
        "`disabled=\"false\"` is the documented opt-out"
    );
    assert!(
        !order.contains(&ids.disabled),
        "the HTML spelling the component library writes must exclude: {order:?}"
    );
    assert!(
        !order.contains(&ids.data_disabled),
        "the old spelling still excludes: {order:?}"
    );
}

/// Tab walks past the disabled field rather than landing on it.
#[test]
fn tab_skips_a_disabled_field() {
    let (mut app, ids, _log) = mount_fixture();

    key(&mut app, KeyCode::Tab, None);
    assert_eq!(app.focused_input_node_id, Some(ids.plain));
    key(&mut app, KeyCode::Tab, None);
    assert_eq!(
        app.focused_input_node_id,
        Some(ids.readonly),
        "the disabled field between them is not a stop"
    );
}

// ── 2. the pointer claim ────────────────────────────────────────────────────

/// A press on a disabled field takes no input claim — and, just as important,
/// does not fall through to whatever encloses it.
#[test]
fn clicking_a_disabled_field_claims_nothing() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.disabled);

    assert_eq!(app.focused_input_node_id, None);
    assert_eq!(app.focus_target, FocusTarget::None);
    assert!(app.focused_input_state.is_none());
}

/// The control it *should* claim, for contrast — so the test above is not
/// passing because the click machinery is broken in the fixture.
#[test]
fn clicking_an_enabled_field_still_claims_it() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.plain);

    assert_eq!(app.focused_input_node_id, Some(ids.plain));
}

// ── 3. the edit path — the leg that is easy to skip ─────────────────────────

/// The whole point of the third leg. Focus an enabled field, type into it,
/// then have it go disabled under the caret (a reactive `disabled` prop). The
/// focus-side guards have already run and cannot help; only an edit-time check
/// stops the next keystroke.
#[test]
fn a_field_that_goes_disabled_while_focused_stops_accepting_keys() {
    let (mut app, ids, log) = mount_fixture();

    click_center(&mut app, ids.plain);
    type_str(&mut app, "ab");
    assert_eq!(focused_text(&app), "ab", "typing works while enabled");

    set_attr(&mut app, ids.plain, "disabled", Some(""));
    type_str(&mut app, "cd");

    assert_eq!(
        focused_text(&app),
        "ab",
        "keys after the field went disabled must change nothing"
    );
    assert_eq!(
        *log.borrow(),
        vec!["plain:a".to_string(), "plain:ab".to_string()],
        "and must fire no oninput"
    );
}

/// Backspace and Delete go through the same choke point, so they are covered
/// by the same guard — pinned separately because "typing" is easy to read as
/// "insertion only".
#[test]
fn a_disabled_field_refuses_deletion_too() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.plain);
    type_str(&mut app, "abc");
    set_attr(&mut app, ids.plain, "disabled", Some(""));

    key(&mut app, KeyCode::Backspace, None);
    key(&mut app, KeyCode::Delete, None);

    assert_eq!(focused_text(&app), "abc");
}

/// Enter is a commit boundary rather than an edit command, and it takes the
/// same route, so a disabled field does not fire `data-onchange` either.
#[test]
fn a_disabled_field_does_not_commit_on_enter() {
    let (mut app, ids, log) = mount_fixture();

    click_center(&mut app, ids.plain);
    type_str(&mut app, "x");
    log.borrow_mut().clear();
    set_attr(&mut app, ids.plain, "disabled", Some(""));

    key(&mut app, KeyCode::Enter, None);

    assert!(
        log.borrow().is_empty(),
        "no commit from a disabled field: {:?}",
        log.borrow()
    );
}

// ── 4. read-only: reachable, selectable, unchangeable ───────────────────────

/// A read-only field focuses and holds the claim — that is what separates it
/// from disabled.
#[test]
fn a_readonly_field_still_takes_focus() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.readonly);

    assert_eq!(app.focused_input_node_id, Some(ids.readonly));
}

/// …but refuses every command that would change its text.
#[test]
fn a_readonly_field_refuses_text_changes() {
    let (mut app, ids, log) = mount_fixture();

    click_center(&mut app, ids.readonly);
    type_str(&mut app, "nope");
    key(&mut app, KeyCode::Backspace, None);

    assert_eq!(focused_text(&app), "");
    assert!(
        log.borrow().is_empty(),
        "no oninput from a read-only field: {:?}",
        log.borrow()
    );
}

/// Selection and caret motion are *not* mutations, so they keep working — the
/// difference `EditCommand::mutates_text` exists to draw.
#[test]
fn a_readonly_field_still_selects() {
    let (mut app, ids, _log) = mount_fixture();

    // Give it text the way an app would (a `value` write), then focus it.
    set_attr(&mut app, ids.readonly, "value", Some("hello"));
    click_center(&mut app, ids.readonly);
    assert_eq!(focused_text(&app), "hello");

    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyA,
            logical_key: None,
            text: None,
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        (800, 600),
        1.0,
    );

    let state = app.focused_input_state.as_ref().expect("still focused");
    assert!(
        state.selection.anchor != state.selection.head,
        "Ctrl+A selected the text of a read-only field"
    );
    assert_eq!(focused_text(&app), "hello", "and changed nothing");
}

// ── 5. <fieldset disabled> ──────────────────────────────────────────────────

/// A disabled `<fieldset>` is the one element whose `disabled` reaches its
/// descendants — that is what the element is for.
#[test]
fn a_disabled_fieldset_disables_its_subtree() {
    let (mut app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    assert!(
        !order.contains(&ids.in_fieldset),
        "a control nested below a disabled fieldset is not a Tab stop: {order:?}"
    );

    click_center(&mut app, ids.in_fieldset);
    assert_eq!(
        app.focused_input_node_id, None,
        "and takes no claim from a press"
    );
}

/// HTML's carve-out: controls inside the fieldset's first `<legend>` stay
/// enabled, so a form can put the "enable this section" control there.
#[test]
fn the_first_legend_escapes_a_disabled_fieldset() {
    let (mut app, ids, _log) = mount_fixture();
    let order = app.collect_focusable_nodes();

    assert!(
        order.contains(&ids.in_legend),
        "the legend's control stays reachable: {order:?}"
    );

    click_center(&mut app, ids.in_legend);
    assert_eq!(app.focused_input_node_id, Some(ids.in_legend));
    type_str(&mut app, "ok");
    assert_eq!(focused_text(&app), "ok", "and stays editable");
}

// ── 6. the boolean-attribute rule ───────────────────────────────────────────

/// Presence is what disables; the explicit `"false"` is the only opt-out, and
/// it holds for both spellings.
#[test]
fn the_boolean_rule_is_presence_with_a_false_opt_out() {
    let (mut app, ids, _log) = mount_fixture();

    // `disabled="false"` opts out — it focuses and types.
    click_center(&mut app, ids.disabled_false);
    assert_eq!(app.focused_input_node_id, Some(ids.disabled_false));

    // Any other value disables, whatever it says.
    set_attr(&mut app, ids.disabled_false, "disabled", Some("no"));
    type_str(&mut app, "z");
    assert_eq!(focused_text(&app), "");
}
