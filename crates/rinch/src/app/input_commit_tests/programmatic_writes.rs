//! Programmatic writes to the focused field (issue #238).

use super::*;

// ── Programmatic writes to the focused field (issue #238) ───────────────────
//
// A `value_fn` effect (or any other app code) writing `set_attribute("value")`
// on the FOCUSED input must be adopted by the runtime's editable state — the
// state the next keystroke edits and `sync_input_cursor_to_dom` paints back —
// with the caret mapped to its logical position, deferred while an IME
// composition is in flight, and never committing `onchange` by itself.

/// The snap-back from the issue: "hi" + a rewrite to "HI!" + "x" must read
/// "HI!x", not "hix" (the keystroke editing the stale engine text and painting
/// it over the rewrite).
#[test]
fn a_programmatic_write_to_the_focused_field_is_adopted() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    write_value(&app, a_id, "HI!");
    type_str(&mut app, "x");

    assert_eq!(
        engine(&app).0,
        "HI!x",
        "the keystroke edits the rewritten text"
    );
    assert_eq!(
        attr(&app, a_id, "value").as_deref(),
        Some("HI!x"),
        "the field displays the rewritten text plus the keystroke"
    );
    assert_eq!(
        log.borrow().last().map(String::as_str),
        Some("a-input:HI!x"),
        "oninput carries the adopted text"
    );

    click_center(&mut app, b_id);
    assert_eq!(changes(&log), vec!["a-change:HI!x".to_string()]);
}

/// A write made by a normalizing `oninput` handler — the documented controlled
/// input pattern — is adopted inside the same keystroke.
#[test]
fn a_rewrite_from_the_oninput_handler_is_adopted_in_the_same_keystroke() {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (slot_in, handle_in, log_in) = (slot.clone(), handle.clone(), log.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        // Digits only: every non-digit is rejected by writing the filtered
        // text straight back — the pattern that "looked broken" on desktop.
        let h = handle_in.clone();
        let log = log_in.clone();
        let id = register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(v.clone());
            let filtered: String = v.chars().filter(char::is_ascii_digit).collect();
            if filtered != v
                && let Some(node) = h.borrow().as_ref()
            {
                node.set_attribute("value", &filtered);
            }
        }));
        input.set_attribute("data-oninput", &id.0.to_string());
        *handle_in.borrow_mut() = Some(input.clone());
        slot_in.set(Some(input.node_id().0));
        input
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = slot.get().unwrap();

    click_center(&mut app, id);
    type_str(&mut app, "1a2b3");

    assert_eq!(engine(&app).0, "123", "rejected characters never stick");
    assert_eq!(attr(&app, id, "value").as_deref(), Some("123"));
    assert_eq!(engine(&app).2, 3, "the caret sits after the kept text");
    assert_eq!(
        *log.borrow(),
        vec!["1", "1a", "12", "12b", "123"],
        "oninput saw each keystroke against the filtered text, not a stale buffer"
    );
}

/// The caret keeps its logical position through a rewrite: "abc" with the
/// caret before "c", rewritten to "ABC", then "z" → "ABzC".
#[test]
fn the_caret_survives_a_programmatic_rewrite() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "abc");
    key(&mut app, KeyCode::ArrowLeft, None);
    assert_eq!(engine(&app).2, 2);
    write_value(&app, a_id, "ABC");
    type_str(&mut app, "z");

    assert_eq!(engine(&app).0, "ABzC");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("ABzC"));
}

/// A write that lands outside any keystroke — a click handler, a menu, a
/// timer — is picked up by the frame's resolve, with the caret mapped and the
/// caret attributes the painter reads updated too.
#[test]
fn a_write_from_outside_an_edit_is_adopted_at_the_next_resolve() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "abcd");
    key(&mut app, KeyCode::Home, None);
    key(&mut app, KeyCode::ArrowRight, None);
    write_value(&app, a_id, "abcd!");
    app.resolve_and_repaint(800.0, 600.0);

    assert_eq!(
        engine(&app),
        ("abcd!".to_string(), 1, 1),
        "prefix kept → caret stays"
    );
    assert_eq!(attr(&app, a_id, "data-cursor-pos").as_deref(), Some("1"));
    assert_eq!(
        attr(&app, a_id, "data-selection-start").as_deref(),
        Some("1")
    );

    type_str(&mut app, "z");
    assert_eq!(engine(&app).0, "azbcd!");
}

/// Filling an empty focused field puts the caret after the fill, where a
/// browser's `.value` setter would leave it — the user types on from there.
#[test]
fn filling_an_empty_field_leaves_the_caret_after_the_fill() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    write_value(&app, a_id, "X");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(engine(&app), ("X".to_string(), 1, 1));
}

/// A selection survives a rewrite too: anchor and head map independently.
#[test]
fn a_selection_survives_a_programmatic_rewrite() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "abcd");
    // Select "cd" backwards: head 2, anchor 4.
    key(&mut app, KeyCode::ArrowLeft, None);
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::ArrowLeft,
            logical_key: None,
            text: None,
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    assert_eq!(engine(&app), ("abcd".to_string(), 3, 2));
    // A rewrite touching only the prefix keeps the selection where it is.
    write_value(&app, a_id, "ABcd");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(engine(&app), ("ABcd".to_string(), 3, 2));

    // Typing replaces the selection, as it would have without the rewrite.
    type_str(&mut app, "z");
    assert_eq!(engine(&app).0, "ABzd");
}

/// The #226 control: a programmatic write with no user edit in the gesture
/// is adopted (the engine now holds it) but is not a user change, so blur
/// commits nothing.
#[test]
fn a_programmatic_write_alone_still_never_commits() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    write_value(&app, a_id, "X");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(engine(&app).0, "X", "the write was adopted");

    click_center(&mut app, b_id);
    assert!(
        changes(&log).is_empty(),
        "a programmatic write alone is not a user change: {:?}",
        log.borrow()
    );
}

/// ...but a user edit AFTER an adopted write is a change, and commits the
/// edited text.
#[test]
fn a_user_edit_after_an_adopted_write_commits() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    write_value(&app, a_id, "X");
    app.resolve_and_repaint(800.0, 600.0);
    type_str(&mut app, "y");
    click_center(&mut app, b_id);

    assert_eq!(changes(&log), vec!["a-change:Xy".to_string()]);
}

/// A rewrite while an IME composition is in flight is deferred — the
/// composition's caret must not move under it — and applied when the
/// composition commits, before the committed text is inserted.
#[test]
fn a_rewrite_during_a_composition_is_deferred() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    write_value(&app, a_id, "HI!");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(
        engine(&app),
        ("hi".to_string(), 2, 2),
        "the engine is untouched while composing"
    );

    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Commit("你".to_string())),
        (800, 600),
        1.0,
    );

    assert_eq!(
        engine(&app).0,
        "HI!你",
        "the deferred write applied before the commit"
    );
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("HI!你"));
    assert_eq!(
        log.borrow().last().map(String::as_str),
        Some("a-input:HI!你")
    );
}

/// A deferred rewrite is also applied when the composition is cancelled
/// (`Disabled`) rather than committed.
#[test]
fn a_deferred_rewrite_applies_when_the_composition_is_cancelled() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    write_value(&app, a_id, "HI!");
    app.resolve_and_repaint(800.0, 600.0);
    app.handle_event(PlatformEvent::Ime(ImeEvent::Disabled), (800, 600), 1.0);

    assert_eq!(engine(&app), ("HI!".to_string(), 3, 3));
}

/// Offsets are bytes: with the caret after "é" (byte 3) and "é" rewritten to
/// the 3-byte "€", the caret lands after "€" (byte 4) — a char boundary — and
/// the next keystroke goes there.
#[test]
fn a_non_ascii_rewrite_maps_the_caret_by_bytes() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "héllo");
    key(&mut app, KeyCode::Home, None);
    key(&mut app, KeyCode::ArrowRight, None);
    key(&mut app, KeyCode::ArrowRight, None);
    assert_eq!(engine(&app).2, 3, "after 'é'");

    write_value(&app, a_id, "h€llo");
    type_str(&mut app, "z");

    assert_eq!(engine(&app).0, "h€zllo");
    assert_eq!(engine(&app).2, 1 + 3 + 1);
}

/// A write that lands mid-composition must not reach the DOM either: the
/// painter splices the preedit into the `value` attribute at the
/// `data-cursor-pos` offset, so a value those offsets don't index draws the
/// composition in the wrong place — and, on a multi-byte boundary, panics the
/// slice. The node stays coherent until the composition ends (#238 review).
#[test]
fn a_write_during_a_composition_leaves_the_dom_coherent() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "ab");
    key(&mut app, KeyCode::Home, None);
    key(&mut app, KeyCode::ArrowRight, None);
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    // A multi-byte rewrite: byte 1 is inside "€", so an incoherent DOM would
    // panic the painter's `&value[..cursor]`.
    write_value(&app, a_id, "€1");
    app.resolve_and_repaint(800.0, 600.0);

    let value = attr(&app, a_id, "value").unwrap_or_default();
    let cursor: usize = attr(&app, a_id, "data-cursor-pos")
        .unwrap_or_default()
        .parse()
        .unwrap();
    assert_eq!(value, "ab", "the DOM still shows the engine text");
    assert!(
        value.is_char_boundary(cursor) && cursor <= value.len(),
        "the caret offset indexes the value it is painted against: {cursor} in {value:?}"
    );

    // The write is still adopted when the composition commits.
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Commit("你".to_string())),
        (800, 600),
        1.0,
    );
    assert_eq!(engine(&app).0, "€1你");
}

/// An empty preedit IS the end of a composition on the winit backends (an IME
/// cancel delivers only that — no Commit, no Disabled), so it must drain a
/// deferred write instead of leaving it parked to win over a later one.
#[test]
fn an_empty_preedit_drains_a_deferred_write() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    write_value(&app, a_id, "HI!");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(engine(&app).0, "hi", "held while composing");

    // The IME cancels with an empty preedit and nothing else.
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    assert_eq!(engine(&app).0, "HI!", "the deferred write applied");

    // ...and it does not resurrect over a later write.
    write_value(&app, a_id, "later");
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(engine(&app).0, "later");
}

/// A re-click inside the focused input absorbs the DOM value into the commit
/// buffer; without adopting first (which moves the baseline for an untouched
/// field) a purely programmatic write would look like a user edit and commit.
#[test]
fn a_reclick_after_a_programmatic_write_still_never_commits() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    write_value(&app, a_id, "X");
    // No resolve: the click is the first thing to see the write.
    click_center(&mut app, a_id);
    click_center(&mut app, b_id);

    assert!(
        changes(&log).is_empty(),
        "a programmatic write plus a caret click is not a user change: {:?}",
        log.borrow()
    );
}

/// Enter is a commit boundary but not an edit command, so it must adopt before
/// gating: a write that restored the baseline text means there is nothing to
/// commit, and the payload must agree with the gate.
#[test]
fn enter_after_a_write_back_to_the_baseline_commits_nothing() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "a");
    // The handler reverts the field to its gesture-start text.
    write_value(&app, a_id, "");
    key(&mut app, KeyCode::Enter, None);

    assert!(
        changes(&log).is_empty(),
        "the field displays the baseline text, so nothing changed: {:?}",
        log.borrow()
    );
}
