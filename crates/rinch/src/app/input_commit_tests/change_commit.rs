//! The `data-onchange` commit boundary: leaving the field, Enter, selects and
//! compositions.

use super::*;

/// Typing fires `oninput` per keystroke and never `onchange` — the gesture is
/// still live.
#[test]
fn typing_fires_oninput_per_keystroke_and_no_change() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");

    assert_eq!(
        *log.borrow(),
        vec!["a-input:h".to_string(), "a-input:hi".to_string()],
        "per-keystroke oninput only; no change while the gesture is live"
    );
}

/// Clicking a second input ends the gesture: `onchange` fires exactly once
/// with the final text.
#[test]
fn leaving_for_another_input_commits_once_with_the_final_text() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    click_center(&mut app, b_id);

    let changes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .cloned()
        .collect();
    assert_eq!(
        changes,
        vec!["a-change:hi".to_string()],
        "one commit, carrying the final text"
    );
}

/// Tab away from an input commits it (the Tab target here is the next input).
#[test]
fn tab_away_commits() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "x");
    key(&mut app, KeyCode::Tab, None);

    assert_eq!(app.focus_target, FocusTarget::Input(b_id));
    assert!(
        log.borrow().contains(&"a-change:x".to_string()),
        "Tab ended the gesture: {:?}",
        log.borrow()
    );
}

/// Tab onto a generic `tabindex` node (`FocusTarget::Node`, issue #228) is a
/// gesture end too — the commit rides the same arbiter teardown.
#[test]
fn tab_onto_a_tabindex_node_commits() {
    let (mut app, _a_id, b_id, div_id, log) = mount_fixture();

    click_center(&mut app, b_id);
    type_str(&mut app, "q");
    key(&mut app, KeyCode::Tab, None);

    assert_eq!(app.focus_target, FocusTarget::Node(div_id));
    assert_eq!(
        *log.borrow(),
        vec!["b-change:q".to_string()],
        "the Node claim tore the input down through the arbiter, committing it"
    );
}

/// Focus-then-leave without typing commits nothing: HTML change never fires
/// for an unchanged exit.
#[test]
fn an_unchanged_exit_commits_nothing() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    click_center(&mut app, b_id);
    click_center(&mut app, a_id);

    assert!(
        log.borrow().is_empty(),
        "no edits, no commits: {:?}",
        log.borrow()
    );
}

/// A click into dead space (the page body) blurs the input and commits.
#[test]
fn a_click_into_empty_space_commits() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "z");
    click(&mut app, 700.0, 500.0);

    assert_eq!(app.focus_target, FocusTarget::None);
    assert!(
        log.borrow().contains(&"a-change:z".to_string()),
        "blur into empty space is a gesture end: {:?}",
        log.borrow()
    );
}

/// Enter is an explicit commit: `onchange` fires before `onsubmit` (HTML
/// ordering), and the eventual real blur does not re-fire — the baseline
/// resets at the Enter commit.
#[test]
fn enter_commits_change_before_submit_and_blur_does_not_refire() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    key(&mut app, KeyCode::Enter, None);

    let non_input: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| !e.starts_with("a-input"))
        .cloned()
        .collect();
    assert_eq!(
        non_input,
        vec!["a-change:hi".to_string(), "a-submit".to_string()],
        "change fires before submit"
    );

    click_center(&mut app, b_id);
    let changes = log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .count();
    assert_eq!(
        changes, 1,
        "the blur after an Enter commit must not re-fire"
    );
}

/// Enter commits even without an `onsubmit` handler — it is a commit gesture
/// in its own right, not a side effect of submit.
#[test]
fn enter_without_a_submit_handler_still_commits() {
    let (mut app, _a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, b_id);
    type_str(&mut app, "w");
    key(&mut app, KeyCode::Enter, None);

    assert_eq!(*log.borrow(), vec!["b-change:w".to_string()]);

    click(&mut app, 700.0, 500.0);
    assert_eq!(
        *log.borrow(),
        vec!["b-change:w".to_string()],
        "already committed by Enter; the blur adds nothing"
    );
}

/// A re-click inside the already-focused input moves the caret, not the
/// baseline: type, re-click, type more, blur → ONE change with the full final
/// value. (The baseline-seeding is gated on `set_focus_target` actually
/// changing focus — this is the test for that gate.)
#[test]
fn a_reclick_inside_the_input_does_not_reset_the_baseline() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "ab");

    // Re-click near the input's right edge: caret to the end, focus unchanged.
    let (cx, cy) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, ax_w, ay_h) = painted_element_box(&d.tree, a_id);
        (ax + ax_w - 3.0, ay + ay_h / 2.0)
    };
    click(&mut app, cx, cy);
    assert_eq!(app.focus_target, FocusTarget::Input(a_id));

    type_str(&mut app, "cd");
    click_center(&mut app, b_id);

    let changes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .cloned()
        .collect();
    assert_eq!(
        changes,
        vec!["a-change:abcd".to_string()],
        "one gesture, one commit, the full final value — a baseline reset at \
         the re-click would have split it into two (or dropped 'ab')"
    );
}

/// Typing a value back to exactly what it was at focus is an unchanged exit:
/// the text is compared against the gesture baseline, not merely "did
/// keystrokes happen".
#[test]
fn typing_back_to_the_baseline_commits_nothing() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "x");
    key(&mut app, KeyCode::Backspace, None);
    click_center(&mut app, b_id);

    let changes = log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .count();
    assert_eq!(changes, 0, "the value ended where it began: no commit");
}

/// A native `<select>` commits at selection: `data-oninput` on every commit
/// (existing behavior), `data-onchange` only when the value actually changed
/// (HTML `<select>` semantics).
#[test]
fn a_select_commit_fires_change_only_when_the_value_changed() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("input:{v}"))
    }));
    let change_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("change:{v}"))
    }));

    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let sel = scope.create_element("select");
        sel.set_attribute("style", "width: 200px; height: 30px");
        sel.set_attribute("data-oninput", &input_id.0.to_string());
        sel.set_attribute("data-onchange", &change_id.0.to_string());
        for (value, label) in [("a", "Apple"), ("b", "Banana")] {
            let o = scope.create_element("option");
            o.set_attribute("value", value);
            let t = scope.create_text(label);
            o.append_child(&t);
            sel.append_child(&o);
        }
        root.append_child(&sel);
        ids_in.set(Some(sel.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let sel_id = ids.get().expect("select id captured");

    // Open the popup, move to "Banana", commit.
    click_center(&mut app, sel_id);
    assert_eq!(app.focus_target, FocusTarget::Select(sel_id));
    key(&mut app, KeyCode::ArrowDown, None);
    key(&mut app, KeyCode::Enter, None);
    assert_eq!(
        *log.borrow(),
        vec!["input:b".to_string(), "change:b".to_string()],
        "a selection that changes the value fires input then change"
    );

    // Re-open and commit the same option: input fires, change does not.
    click_center(&mut app, sel_id);
    key(&mut app, KeyCode::Enter, None);
    assert_eq!(
        *log.borrow(),
        vec![
            "input:b".to_string(),
            "change:b".to_string(),
            "input:b".to_string(),
        ],
        "re-committing the same value is not a change"
    );
}

/// A value-less `<select>` already displays its resolved default option, so
/// re-picking that default is NOT a change — the reference is the displayed
/// option at popup-open, not the (absent) `value` attribute (#244 review).
#[test]
fn picking_the_displayed_default_option_is_not_a_change() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("input:{v}"))
    }));
    let change_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("change:{v}"))
    }));

    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let sel = scope.create_element("select");
        sel.set_attribute("style", "width: 200px; height: 30px");
        sel.set_attribute("data-oninput", &input_id.0.to_string());
        sel.set_attribute("data-onchange", &change_id.0.to_string());
        for (value, label) in [("a", "Apple"), ("b", "Banana")] {
            let o = scope.create_element("option");
            o.set_attribute("value", value);
            let t = scope.create_text(label);
            o.append_child(&t);
            sel.append_child(&o);
        }
        root.append_child(&sel);
        ids_in.set(Some(sel.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let sel_id = ids.get().expect("select id captured");

    // Open and immediately commit the highlighted default ("a", displayed).
    click_center(&mut app, sel_id);
    key(&mut app, KeyCode::Enter, None);
    assert_eq!(
        *log.borrow(),
        vec!["input:a".to_string()],
        "committing the already-displayed default fires input but not change"
    );
}

/// Enter in a `<textarea>` is not a commit: browsers never fire change there
/// on Enter, so the gesture (and its baseline) runs until blur (#244 review).
#[test]
fn enter_in_a_textarea_does_not_commit() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new(|_| {}));
    let change_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("change:{v}"))
    }));

    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let ta = scope.create_element("textarea");
        ta.set_attribute("style", "width: 200px; height: 60px");
        ta.set_attribute("data-oninput", &input_id.0.to_string());
        ta.set_attribute("data-onchange", &change_id.0.to_string());
        root.append_child(&ta);
        ids_in.set(Some(ta.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ta_id = ids.get().expect("textarea id captured");

    click_center(&mut app, ta_id);
    type_str(&mut app, "x");
    key(&mut app, KeyCode::Enter, None);
    assert!(
        log.borrow().is_empty(),
        "Enter in a textarea must not commit: {:?}",
        log.borrow()
    );

    click(&mut app, 700.0, 500.0);
    assert_eq!(
        *log.borrow(),
        vec!["change:x\n".to_string()],
        "the textarea gesture commits at blur, carrying the full text — \
         including the line break Enter inserted rather than committed"
    );
}

/// `change` bubbles in the browser, so a delegating ancestor's
/// `data-onchange` must receive the commit on desktop too (#244 review).
#[test]
fn an_ancestor_onchange_receives_the_commit() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new(|_| {}));
    let change_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("change:{v}"))
    }));

    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrapper = scope.create_element("div");
        wrapper.set_attribute("style", "width: 300px; height: 50px");
        wrapper.set_attribute("data-onchange", &change_id.0.to_string());
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        input.set_attribute("data-oninput", &input_id.0.to_string());
        wrapper.append_child(&input);
        root.append_child(&wrapper);
        ids_in.set(Some(input.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let input_id_node = ids.get().expect("input id captured");

    click_center(&mut app, input_id_node);
    type_str(&mut app, "q");
    click(&mut app, 700.0, 500.0);

    assert_eq!(
        *log.borrow(),
        vec!["change:q".to_string()],
        "the wrapper's data-onchange received the input's commit"
    );
}

/// The commit payload is the live `value` attribute — what the field displays
/// and what the web backend delivers — so a programmatic rewrite that landed
/// mid-gesture is committed as displayed, not as last typed (#244 review).
/// The user-edit gate itself stays on the keystroke buffer: without a user
/// edit, a programmatic write alone never commits (the browser's dirty flag).
#[test]
fn a_programmatic_rewrite_mid_gesture_commits_the_displayed_value() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    // An app effect rewrites the displayed value under the gesture.
    {
        let doc = app.doc.clone().expect("doc");
        let mut d = doc.borrow_mut();
        d.set_attribute(rinch_core::dom::NodeId(a_id), "value", "HI!");
    }
    click_center(&mut app, b_id);

    let changes: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("a-change"))
        .cloned()
        .collect();
    assert_eq!(
        changes,
        vec!["a-change:HI!".to_string()],
        "the commit carries the displayed (rewritten) value"
    );
}

/// Blur mid-IME-composition commits the pending preedit first — the browser's
/// compositionend-before-blur — so the composed text is not silently dropped
/// and the change commit carries it (#244 review).
#[test]
fn blur_mid_composition_commits_the_preedit() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    click_center(&mut app, b_id);

    assert_eq!(
        *log.borrow(),
        vec!["a-input:ni".to_string(), "a-change:ni".to_string()],
        "the composition committed (oninput) and the commit carried it (change)"
    );
}
