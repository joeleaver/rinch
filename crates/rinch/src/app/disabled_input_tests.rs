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
    select: usize,
    select_in_fieldset: usize,
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
    let change = |tag: &'static str, log: &Rc<RefCell<Vec<String>>>| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let plain_change = change("plain-change", &log);
    let handlers = [
        record("plain"),
        record("disabled"),
        record("readonly"),
        record("data-disabled"),
        record("disabled-false"),
        record("in-fieldset"),
        record("in-legend"),
        record("select-input"),
        record("select-change"),
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
        // A commit handler, so a suppressed `data-onchange` is observable as an
        // absence rather than assumed.
        plain.set_attribute("data-onchange", &plain_change.0.to_string());
        let disabled = field(scope, handlers[1].0);
        disabled.set_attribute("disabled", "");
        let readonly = field(scope, handlers[2].0);
        readonly.set_attribute("readonly", "");
        let data_disabled = field(scope, handlers[3].0);
        data_disabled.set_attribute("data-disabled", "");
        let disabled_false = field(scope, handlers[4].0);
        disabled_false.set_attribute("disabled", "false");

        // A `<select>` with the handlers any app with a change listener writes.
        // Starts **enabled** so a test can measure where its popup's options
        // land, then disable it and click the same place.
        let select = scope.create_element("select");
        select.set_attribute("style", "display: block; width: 200px; height: 30px");
        select.set_attribute("data-oninput", &handlers[7].0.to_string());
        select.set_attribute("data-onchange", &handlers[8].0.to_string());
        for (value, label) in [("a", "Alpha"), ("b", "Bravo")] {
            let opt = scope.create_element("option");
            opt.set_attribute("value", value);
            let t = scope.create_text(label);
            opt.append_child(&t);
            select.append_child(&opt);
        }

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
        wrapper.set_attribute("style", "width: 300px; height: 120px");
        let in_fieldset = field(scope, handlers[5].0);
        wrapper.append_child(&in_fieldset);
        // A `<select>` nested below the disabled fieldset — the guard asks
        // `node_is_disabled_in_tree`, not just the control's own attribute.
        let select_in_fieldset = scope.create_element("select");
        select_in_fieldset.set_attribute("style", "display: block; width: 200px; height: 30px");
        let fs_opt = scope.create_element("option");
        fs_opt.set_attribute("value", "x");
        let fs_text = scope.create_text("X");
        fs_opt.append_child(&fs_text);
        select_in_fieldset.append_child(&fs_opt);
        wrapper.append_child(&select_in_fieldset);
        fieldset.append_child(&legend);
        fieldset.append_child(&wrapper);

        for n in [
            &plain,
            &disabled,
            &readonly,
            &data_disabled,
            &disabled_false,
            &select,
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
            select: select.node_id().0,
            select_in_fieldset: select_in_fieldset.node_id().0,
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

/// The `value` attribute a field actually displays — what survives the claim
/// being released, unlike `focused_input_state`.
fn dom_value(app: &RinchApp, id: usize) -> String {
    app.doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
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

/// A disabled control takes no DOM `:focus` either. It owns no keyboard, so a
/// focus ring on it would be the style lying about who does — and the ring is
/// what the `.rinch-text-input:focus` rule paints, so this is visible.
#[test]
fn clicking_a_disabled_field_paints_no_focus_ring() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.plain);
    assert_eq!(
        app.doc.as_ref().unwrap().borrow().tree.focused_node,
        Some(ids.plain),
        "an enabled field does take DOM focus, so the assertion below means \
         something"
    );

    click_center(&mut app, ids.disabled);

    assert_eq!(
        app.doc.as_ref().unwrap().borrow().tree.focused_node,
        None,
        "a press on a disabled control leaves :focus nowhere"
    );
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
        dom_value(&app, ids.plain),
        "ab",
        "keys after the field went disabled must change nothing"
    );
    assert_eq!(
        *log.borrow(),
        vec!["plain:a".to_string(), "plain:ab".to_string()],
        "and must fire no oninput"
    );
}

/// …and the claim is **released**, the way a browser moves focus to the body.
/// Keeping an inert claim would leave a `:focus` ring on a control that owns
/// no keyboard, keep the OS IME enabled for it (`ime_state` reports
/// `enabled: true` for any `FocusTarget::Input`), and keep
/// `has_focused_input()` answering `true` — which is what an embed host routes
/// its keyboard on.
#[test]
fn a_field_that_goes_disabled_while_focused_releases_the_keyboard() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.plain);
    type_str(&mut app, "ab");
    set_attr(&mut app, ids.plain, "disabled", Some(""));
    type_str(&mut app, "c");

    assert_eq!(app.focus_target, FocusTarget::None);
    assert_eq!(app.focused_input_node_id, None);
    assert!(app.focused_input_state.is_none());
    assert!(!app.has_focused_input());
    assert!(
        !app.ime_state().enabled,
        "and the OS input method is switched back off"
    );
    assert_eq!(
        app.doc.as_ref().unwrap().borrow().tree.focused_node,
        None,
        "and no :focus ring is left behind"
    );
}

/// The release must **not** run the field's `data-onchange` commit. Everywhere
/// else that commit is load-bearing — a window blur retains the claim
/// precisely so alt-tabbing cannot fire it (#226) — but a control going
/// disabled is not the user committing an edit, and browsers dispatch no
/// `change` for it either.
#[test]
fn the_release_suppresses_the_change_commit() {
    let (mut app, ids, log) = mount_fixture();

    click_center(&mut app, ids.plain);
    type_str(&mut app, "ab");
    log.borrow_mut().clear();

    set_attr(&mut app, ids.plain, "disabled", Some(""));
    type_str(&mut app, "c");
    assert_eq!(app.focus_target, FocusTarget::None, "it did release");

    assert!(
        log.borrow().is_empty(),
        "no data-onchange from a control going disabled: {:?}",
        log.borrow()
    );

    // The same field, blurred the ordinary way, *does* commit — so the
    // assertion above is a suppression and not a handler that never fires.
    let (mut app, ids, log) = mount_fixture();
    click_center(&mut app, ids.plain);
    type_str(&mut app, "ab");
    log.borrow_mut().clear();
    click_center(&mut app, ids.readonly);
    assert_eq!(
        *log.borrow(),
        vec!["plain-change:ab".to_string()],
        "an ordinary blur commits"
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

    assert_eq!(dom_value(&app, ids.plain), "abc");
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

// ── 7. the sixth route: a <select> popup is a whole interaction ─────────────

/// `handle_click`'s Phase 0.5 hit-tests for a `<select>`, opens its popup and
/// **returns** — ahead of every focus and claim gate the rest of this PR
/// installs. So a disabled `<select>` opened, let an option be picked, and
/// fired the app's change handler: a disabled control mutating application
/// state, which is the exact thing the PR title claims to prevent.
///
/// The repro is spatial, so it is built the way a user would hit it: measure
/// where the popup's second option lands while the control is enabled, close
/// it, disable the control, then click the control and that same point.
#[test]
fn a_disabled_select_neither_opens_nor_commits() {
    let (mut app, ids, log) = mount_fixture();

    // 1. Enabled: the popup opens and the option is where we think it is.
    click_center(&mut app, ids.select);
    assert!(app.is_select_open(), "the enabled control opens its popup");
    let option_pt = {
        let open = app.open_select.as_ref().expect("popup is open");
        let bravo = open.option_ids[1];
        let d = app.doc.as_ref().unwrap().borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, bravo);
        (x + w / 2.0, y + h / 2.0)
    };
    key(&mut app, KeyCode::Escape, None);
    assert!(!app.is_select_open());
    log.borrow_mut().clear();

    // 2. Disabled: the same two clicks must do nothing at all.
    set_attr(&mut app, ids.select, "disabled", Some(""));
    click_center(&mut app, ids.select);
    assert!(!app.is_select_open(), "a disabled <select> opens no popup");
    click(&mut app, option_pt.0, option_pt.1);

    assert!(
        log.borrow().is_empty(),
        "a disabled <select> must not reach the app's handlers: {:?}",
        log.borrow()
    );
    assert_eq!(
        dom_value(&app, ids.select),
        "",
        "and must not write its own value"
    );
}

/// The guard sits on the popup's single constructor, so it holds for every
/// route in — not just the mouse one. Called directly, the way Enter/Space and
/// Alt+Down call it.
#[test]
fn the_popup_constructor_itself_refuses_a_disabled_control() {
    let (mut app, ids, _log) = mount_fixture();
    set_attr(&mut app, ids.select, "disabled", Some(""));

    app.open_select_popup(ids.select, 800.0, 600.0);

    assert!(!app.is_select_open());
    assert_eq!(app.focus_target, FocusTarget::None);
}

/// A `<select>` inside a disabled `<fieldset>` is disabled too, by the same
/// rule the rest of this PR applies — the guard asks
/// `node_is_disabled_in_tree`, not just the control's own attribute.
#[test]
fn a_select_in_a_disabled_fieldset_opens_nothing() {
    let (mut app, ids, _log) = mount_fixture();

    click_center(&mut app, ids.select_in_fieldset);
    assert!(!app.is_select_open(), "a click opens nothing");

    app.open_select_popup(ids.select_in_fieldset, 800.0, 600.0);
    assert!(!app.is_select_open(), "and neither does the constructor");
}

// ── 8. IME composes nothing into a disabled field ──────────────────────────

/// `dispatch_input_ime`'s `Preedit` arm writes `data-preedit` straight to the
/// DOM without consulting `live_focused_input_handler`, so it sat outside
/// every gate this PR installs: a preedit painted into a disabled field and —
/// since `Commit` *is* gated — could never resolve.
#[test]
fn a_disabled_field_composes_nothing() {
    let (mut app, ids, log) = mount_fixture();

    click_center(&mut app, ids.plain);
    set_attr(&mut app, ids.plain, "disabled", Some(""));

    app.handle_event(
        PlatformEvent::Ime(rinch_platform::ImeEvent::Preedit {
            text: "\u{3053}".into(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );

    let preedit = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(ids.plain)
        .and_then(|n| n.attributes.get("data-preedit").cloned());
    assert!(
        preedit.is_none(),
        "no preedit paints into a disabled field: {preedit:?}"
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "and the IME event releases the claim through the same path a \
         keystroke would, so the two orders agree"
    );
    assert!(log.borrow().len() <= 1, "no commit: {:?}", log.borrow());
}

// ── 6. the route neither the collector nor the claim can guard ──────────────

/// Programmatic focus — `request_focus`, or a click handler calling
/// `node.focus()` — is the one way into a disabled field that neither the Tab
/// collector nor the mousedown claim stands in front of: both filter *before*
/// `try_focus_input` is reached, so its own refusal is the only thing there.
///
/// Found by mutation during review: deleting that refusal left the entire
/// workspace suite green. It was correct and load-bearing, and completely
/// unverified.
#[test]
fn programmatic_focus_refuses_a_disabled_field() {
    let (mut app, ids, _log) = mount_fixture();

    app.try_focus_input(ids.disabled);

    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "a disabled field takes no claim from `request_focus` either"
    );
    assert!(
        app.focused_input_state.is_none(),
        "and no EditableState is installed over it"
    );
}

/// The same for a field disabled by an enclosing `<fieldset>` rather than by
/// its own attribute — the refusal asks `node_is_disabled_in_tree`, so it must
/// hold for the inherited case too.
#[test]
fn programmatic_focus_refuses_a_field_inside_a_disabled_fieldset() {
    let (mut app, ids, _log) = mount_fixture();

    app.try_focus_input(ids.in_fieldset);

    assert_eq!(app.focus_target, FocusTarget::None);
    assert!(app.focused_input_state.is_none());
}

// ── 7. the guarantee a merge nearly deleted ────────────────────────────────

/// A **generic focusable** inside a `<fieldset disabled>` takes no claim from a
/// pointer press either.
///
/// This pins `resolve_click_focus`'s use of `node_is_disabled_in_tree` rather
/// than `node_is_disabled`, and it exists because that call was one merge away
/// from being deleted with nothing failing.
///
/// #316 item 3 collapsed the mousedown claim's inline walk and `handle_click`'s
/// release check into the single `resolve_click_focus`. This branch's inline
/// walk asked the **fieldset-aware** question; the collapsed function on `main`
/// asked the self-only one. Resolving that conflict by taking `main`'s version
/// verbatim — the obvious, ordinary-looking resolution — compiles, passes every
/// other test, and silently removes this guarantee, because the only tests that
/// would notice live on the branch whose behaviour was discarded.
///
/// A `<fieldset disabled>` is the only shape that separates the two predicates:
/// every other disabled control fails both. So this is the one test that can
/// tell them apart, and without it the difference is invisible.
#[test]
fn a_press_on_a_focusable_inside_a_disabled_fieldset_claims_nothing() {
    let ids: Rc<std::cell::Cell<(usize, usize)>> = Rc::new(std::cell::Cell::new((0, 0)));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        // Enabled control outside the fieldset, as the control case: the same
        // markup must still claim when nothing disables it.
        let free = scope.create_element("div");
        free.set_attribute("style", "display: block; width: 120px; height: 24px");
        free.set_attribute("tabindex", "0");

        let fieldset = scope.create_element("fieldset");
        fieldset.set_attribute("style", "display: block");
        fieldset.set_attribute("disabled", "");
        // Not an `<input>` — a plain `tabindex` node, so the walk reaches the
        // focusable arm rather than short-circuiting on `data-oninput`.
        let inside = scope.create_element("div");
        inside.set_attribute("style", "display: block; width: 120px; height: 24px");
        inside.set_attribute("tabindex", "0");
        fieldset.append_child(&inside);

        root.append_child(&free);
        root.append_child(&fieldset);
        ids_in.set((free.node_id().0, inside.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (free, inside) = ids.get();

    // Control: the same shape outside the fieldset does claim.
    click_center(&mut app, free);
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(free),
        "an enabled tabindex node still takes the claim"
    );

    // The guarantee: inside a disabled fieldset it does not — and the previous
    // claim is released rather than left stranded.
    click_center(&mut app, inside);
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "a focusable inside a <fieldset disabled> takes no claim from a press"
    );

    // …and the DOM `:focus` does not land on it either, so no ring is painted
    // on a control that owns no keyboard.
    let focused_node = app.doc.as_ref().unwrap().borrow().tree.focused_node;
    assert_ne!(
        focused_node,
        Some(inside),
        "and no :focus ring is painted on it"
    );
}
