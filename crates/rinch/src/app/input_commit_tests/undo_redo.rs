//! Undo / redo on a focused field (issue #288).

use super::*;

// ── Undo / redo on a focused field (issue #288) ─────────────────────────────
//
// `EditableState` always kept an undo stack; nothing reached it. Ctrl+Z undoes,
// Ctrl+Y and Ctrl+Shift+Z redo (Cmd on macOS). A browser fires `input` for an
// undo, so `oninput` carries the restored text. One key press is one undo step
// — including the rewrite a controlled field's own `oninput` made in answer to
// it — so an undo never shows the half-applied state between a keystroke and
// its normalization, and a normalizing field can be undone past its rewrites.

/// The platform's primary shortcut modifier: Cmd on macOS, Ctrl elsewhere.
fn primary_mods(shift: bool) -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        shift,
        ..Default::default()
    }
}

fn chord(app: &mut RinchApp, key: KeyCode, text: &str, shift: bool) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            // A real Ctrl chord carries the key's text too; the `ctrl` gate
            // on the text-insertion arm is what must keep it out of the field.
            text: Some(text.to_string()),
            modifiers: primary_mods(shift),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

fn undo(app: &mut RinchApp) {
    chord(app, KeyCode::KeyZ, "z", false);
}

fn inputs(log: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    log.borrow()
        .iter()
        .filter(|e| e.starts_with("a-input"))
        .cloned()
        .collect()
}

/// Mount one `<input>` whose `oninput` upper-cases what it is given by writing
/// it back — the controlled-normalizer pattern of #238. Returns the app, the
/// input's id and the log of every `oninput` payload.
fn mount_uppercasing_field() -> (RinchApp, u64, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (slot_in, handle_in, log_in) = (slot.clone(), handle.clone(), log.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        let h = handle_in.clone();
        let log = log_in.clone();
        let id = register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(v.clone());
            let upper = v.to_uppercase();
            if upper != v
                && let Some(node) = h.borrow().as_ref()
            {
                node.set_attribute("value", &upper);
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
    (app, id, log)
}

/// Ctrl+Z undoes a keystroke, Ctrl+Y and Ctrl+Shift+Z redo one; each fires
/// `oninput` with the restored text, the field displays it, and the eventual
/// commit carries what the field shows.
#[test]
fn ctrl_z_undoes_and_ctrl_y_and_ctrl_shift_z_redo() {
    let (mut app, a_id, b_id, _div_id, log) = mount_fixture();
    // Windows' and Linux's chords, on every host: macOS has no Cmd+Y redo
    // (`cmd_y_is_not_redo_on_macos`).
    app.text_keymap = TextKeymap::Standard;

    click_center(&mut app, a_id);
    type_str(&mut app, "abc");
    undo(&mut app);
    assert_eq!(engine(&app).0, "ab");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("ab"));
    undo(&mut app);
    assert_eq!(engine(&app).0, "a");
    chord(&mut app, KeyCode::KeyY, "y", false);
    assert_eq!(engine(&app).0, "ab", "Ctrl+Y redoes");
    chord(&mut app, KeyCode::KeyZ, "Z", true);
    assert_eq!(engine(&app).0, "abc", "Ctrl+Shift+Z redoes");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("abc"));
    undo(&mut app);

    assert_eq!(
        inputs(&log),
        [
            "a-input:a",
            "a-input:ab",
            "a-input:abc",
            "a-input:ab",
            "a-input:a",
            "a-input:ab",
            "a-input:abc",
            "a-input:ab",
        ],
        "every undo and redo fires oninput with the restored text"
    );

    click_center(&mut app, b_id);
    assert_eq!(
        changes(&log),
        ["a-change:ab"],
        "the commit carries the undone text"
    );
}

/// On macOS, Cmd+Y opens History in Chrome, and redo is Cmd+Shift+Z only
/// (#3062): the primary chord with Y neither redoes nor types a "y".
#[test]
fn cmd_y_is_not_redo_on_macos() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();
    app.text_keymap = TextKeymap::MacOs;
    click_center(&mut app, a_id);
    type_str(&mut app, "abc");
    undo(&mut app);
    assert_eq!(engine(&app).0, "ab");
    chord(&mut app, KeyCode::KeyY, "y", false);
    assert_eq!(engine(&app).0, "ab", "Cmd+Y does nothing");
    chord(&mut app, KeyCode::KeyZ, "Z", true);
    assert_eq!(engine(&app).0, "abc", "Cmd+Shift+Z redoes");
}

/// Undo past the start of the stack, and redo with nothing to redo, change
/// nothing and fire nothing.
#[test]
fn undo_and_redo_with_nothing_to_do_fire_nothing() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "a");
    undo(&mut app);
    undo(&mut app);
    chord(&mut app, KeyCode::KeyY, "y", false);
    chord(&mut app, KeyCode::KeyY, "y", false);

    assert_eq!(engine(&app).0, "a");
    assert_eq!(inputs(&log), ["a-input:a", "a-input:", "a-input:a"]);
}

/// A programmatic write that is not the answer to a keystroke — a timer, a
/// click handler, a `value_fn` reset — clears the field's undo and redo
/// history, as Chrome does for any script write to `.value` (measured, Chrome
/// 153). Undo must not bring back text the app replaced. Typing afterwards
/// starts a fresh history.
#[test]
fn a_programmatic_write_clears_the_undo_history() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    undo(&mut app);
    assert_eq!(engine(&app).0, "h", "positive control: there was history");
    write_value(&app, a_id, "HI!");
    app.resolve_and_repaint(800.0, 600.0);
    let before = log.borrow().len();
    undo(&mut app);
    chord(&mut app, KeyCode::KeyY, "y", false);
    assert_eq!(
        engine(&app).0,
        "HI!",
        "neither undo nor the old redo reach past the write"
    );
    assert_eq!(
        log.borrow().len(),
        before,
        "nothing fired: {:?}",
        log.borrow()
    );

    type_str(&mut app, "x");
    undo(&mut app);
    assert_eq!(
        engine(&app).0,
        "HI!",
        "a keystroke after the write undoes as usual"
    );
}

/// The send-then-clear pattern: Enter submits, the app empties the field. The
/// clear is not an edit the user can undo — Ctrl+Z leaves the field empty and
/// hands the app's signal nothing back.
#[test]
fn clearing_the_field_after_submit_leaves_nothing_to_undo() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "secret");
    key(&mut app, KeyCode::Enter, None);
    // What the `onsubmit` handler (through `value_fn`) writes.
    write_value(&app, a_id, "");
    app.resolve_and_repaint(800.0, 600.0);
    let before = log.borrow().len();
    undo(&mut app);
    undo(&mut app);

    assert_eq!(engine(&app).0, "");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some(""));
    assert_eq!(log.borrow().len(), before, "no oninput: {:?}", log.borrow());
}

/// The write reaches the history whichever adopt takes it in: here the one at
/// the top of the next command, with no frame between the write and Ctrl+Z.
#[test]
fn a_write_adopted_by_the_undo_itself_still_clears_the_history() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "hi");
    write_value(&app, a_id, "HI!");
    undo(&mut app);
    assert_eq!(engine(&app).0, "HI!");
}

/// A field whose `oninput` normalizes what it is given can be undone past its
/// rewrites. With the keystroke and its rewrite as separate steps, the second
/// Ctrl+Z restores the raw lowercase letter, `oninput` upper-cases it again —
/// a new edit, which clears the redo stack — and the user can never get back
/// below it.
#[test]
fn a_normalizing_field_undoes_a_keystroke_and_its_rewrite_together() {
    let (mut app, id, log) = mount_uppercasing_field();

    click_center(&mut app, id);
    type_str(&mut app, "ab");
    assert_eq!(engine(&app).0, "AB");
    undo(&mut app);
    assert_eq!(engine(&app).0, "A");
    undo(&mut app);
    assert_eq!(engine(&app).0, "", "undone past both normalized keystrokes");
    assert_eq!(attr(&app, id, "value").as_deref(), Some(""));
    chord(&mut app, KeyCode::KeyZ, "Z", true);
    assert_eq!(engine(&app).0, "A", "redo replays the normalized keystroke");
    assert_eq!(*log.borrow(), ["a", "Ab", "A", "", "A"]);
}

/// Replacing a selection is one undo step: the delete of the selected text and
/// the insert of the typed one come back together.
#[test]
fn replacing_a_selection_is_one_undo_step() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "abc");
    chord(&mut app, KeyCode::KeyA, "a", false);
    type_str(&mut app, "x");
    assert_eq!(engine(&app).0, "x");
    undo(&mut app);
    assert_eq!(engine(&app).0, "abc", "one undo restores the replaced text");
}

/// A read-only field refuses undo: it changes nothing and fires nothing, like
/// every other text-changing command.
#[test]
fn a_read_only_field_refuses_undo_and_redo() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "ab");
    undo(&mut app);
    assert_eq!(
        engine(&app).0,
        "a",
        "positive control: undo works while writable"
    );
    {
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut()
            .set_attribute(rinch_core::dom::NodeId(a_id), "readonly", "");
    }
    let before = log.borrow().len();
    undo(&mut app);
    chord(&mut app, KeyCode::KeyY, "y", false);

    assert_eq!(engine(&app).0, "a");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("a"));
    assert_eq!(
        log.borrow().len(),
        before,
        "nothing fired: {:?}",
        log.borrow()
    );
}

/// A field that went disabled while focused refuses undo and releases the
/// keyboard, as for every other key (issue #315).
#[test]
fn a_field_disabled_while_focused_refuses_undo() {
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "ab");
    {
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut()
            .set_attribute(rinch_core::dom::NodeId(a_id), "disabled", "");
    }
    let before = log.borrow().len();
    undo(&mut app);

    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("ab"));
    assert_eq!(
        log.borrow().len(),
        before,
        "nothing fired: {:?}",
        log.borrow()
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "the claim was released"
    );
}

/// A field whose `oninput` rejects a keystroke — writes the field back to what
/// it showed — records no undo step for it: the next Ctrl+Z undoes the
/// keystroke before, rather than being spent on a step that changes nothing.
#[test]
fn a_rejected_keystroke_leaves_no_undo_step() {
    let (mut app, id, log) =
        mount_filtering_field(|v| v.chars().filter(char::is_ascii_digit).collect());

    click_center(&mut app, id);
    type_str(&mut app, "1");
    type_str(&mut app, "a");
    assert_eq!(engine(&app).0, "1", "the letter was rejected");
    undo(&mut app);
    assert_eq!(engine(&app).0, "", "one Ctrl+Z undoes the '1'");
    assert_eq!(*log.borrow(), ["1", "1a", ""]);
}

/// Windows reports AltGr as Ctrl+Alt, and AltGr+Z types a character on some
/// layouts (Polish `ż`), so Ctrl+Alt+Z is not undo — nor Ctrl+Alt+Y or
/// Ctrl+Alt+Shift+Z redo. AltGr's text is typed instead (#934); a real
/// Ctrl+Alt+Y reaches the app with no text (the shell's
/// `alt_graph_filtered_text`) and does nothing.
#[test]
fn ctrl_alt_z_and_y_are_not_undo_or_redo() {
    // The Windows fact: AltGr arrives as Ctrl+Alt.
    rinch_platform::set_alt_graph_is_ctrl_alt(true);
    let (mut app, a_id, _b_id, _div_id, log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "abc");
    undo(&mut app);
    assert_eq!(engine(&app).0, "ab", "positive control");
    let altgr = |shift| Modifiers {
        ctrl: true,
        alt: true,
        shift,
        ..Default::default()
    };
    for (key, text0, shift, text1) in [
        (KeyCode::KeyZ, Some("ż"), false, "abż"),
        (KeyCode::KeyZ, Some("Ż"), true, "abżŻ"),
        (KeyCode::KeyY, None, false, "abżŻ"),
    ] {
        let (text, want) = (text0, text1);
        app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: altgr(shift),
                repeat: KeyRepeat::Unknown,
            },
            (800, 600),
            1.0,
        );
        assert_eq!(engine(&app).0, want, "{key:?} shift={shift}");
    }
    assert_eq!(inputs(&log).len(), 6, "{:?}", log.borrow());
}

/// Undo and redo are refused while an IME composition is shown: the preedit
/// sits at the caret, and moving the text under it would land the commit
/// somewhere the user never composed.
#[test]
fn undo_is_refused_while_a_composition_is_shown() {
    let (mut app, a_id, _b_id, _div_id, _log) = mount_fixture();

    click_center(&mut app, a_id);
    type_str(&mut app, "ab");
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ni".to_string(),
            cursor: None,
        }),
        (800, 600),
        1.0,
    );
    undo(&mut app);
    assert_eq!(engine(&app).0, "ab", "the text under the composition stays");
    app.handle_event(
        PlatformEvent::Ime(ImeEvent::Commit("你".to_string())),
        (800, 600),
        1.0,
    );
    assert_eq!(engine(&app).0, "ab你");
    undo(&mut app);
    assert_eq!(
        engine(&app).0,
        "ab",
        "after the composition, undo works again"
    );
}

/// Mount one `<input>` whose `oninput` writes back `filter(value)` whenever it
/// differs. Returns the app, the input's id and every `oninput` payload.
fn mount_filtering_field(filter: fn(&str) -> String) -> (RinchApp, u64, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (slot_in, handle_in, log_in) = (slot.clone(), handle.clone(), log.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        let h = handle_in.clone();
        let log = log_in.clone();
        let id = register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(v.clone());
            let kept = filter(&v);
            if kept != v
                && let Some(node) = h.borrow().as_ref()
            {
                node.set_attribute("value", &kept);
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
    (app, id, log)
}

/// An `oninput` that moves focus to another field: the keystroke's step stays
/// with the field it was typed in, and the new field's history is its own.
/// Today the move is parked (`NodeHandle::focus` posts a request the event loop
/// applies after the keystroke), so the same-field check beside the fold in
/// `handle_input_edit_command` is not reached by it — this pins the behaviour
/// that check protects, should a focus move ever land synchronously.
#[test]
fn an_oninput_that_moves_focus_keeps_each_fields_history_its_own() {
    let slot: Rc<Cell<Option<(u64, u64)>>> = Rc::new(Cell::new(None));
    let b_handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (slot_in, b_in) = (slot.clone(), b_handle.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let a = scope.create_element("input");
        a.set_attribute("style", "width: 200px; height: 30px");
        let b_for_a = b_in.clone();
        let a_input = register_input_handler(InputCallback::new(move |_| {
            if let Some(b) = b_for_a.borrow().as_ref() {
                b.focus();
            }
        }));
        a.set_attribute("data-oninput", &a_input.0.to_string());
        let b = scope.create_element("input");
        b.set_attribute("style", "width: 200px; height: 30px");
        let b_input = register_input_handler(InputCallback::new(|_| {}));
        b.set_attribute("data-oninput", &b_input.0.to_string());
        root.append_child(&a);
        root.append_child(&b);
        *b_in.borrow_mut() = Some(b.clone());
        slot_in.set(Some((a.node_id().0, b.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a_id, b_id) = slot.get().unwrap();

    click_center(&mut app, a_id);
    type_str(&mut app, "x");
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(a_id),
        "the move is parked"
    );
    app.handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(b_id),
        "A's oninput moved focus"
    );
    type_str(&mut app, "12");
    undo(&mut app);
    assert_eq!(engine(&app).0, "1", "one Ctrl+Z is one of B's own steps");
    undo(&mut app);
    undo(&mut app);
    assert_eq!(engine(&app).0, "", "B's history does not reach into A's");
    assert_eq!(attr(&app, a_id, "value").as_deref(), Some("x"));
}
