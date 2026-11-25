use super::*;

/// a, b, c, d; a clicked (the anchor), then Down: the cursor is on b.
fn clicked_a_cursor_on_b(opts: Opts) -> P {
    let mut p = build(opts, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(p.state.cursor.get().as_deref(), Some("b"));
    assert_eq!(sel(&p), ["a"], "a plain arrow selects nothing");
    p
}

/// The file-manager model: the app selects the cursor row and moves
/// the anchor there, so Shift+Down runs from it, not from the row last
/// clicked. Kills `set_anchor` as a no-op, and `anchor()` answering
/// `None`.
#[test]
fn shift_down_runs_from_the_anchor_the_app_set() {
    let mut p = clicked_a_cursor_on_b(MULTI);
    p.state.controller.set_selection(["b".to_string()]);
    p.state.controller.set_anchor(Some("b"));
    assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(sel(&p), ["b", "c"]);
    assert_eq!(last_report(&p), ["b", "c"]);
}

/// The same for a Shift-click, in a tree with no keyboard: the anchor
/// is not the keyboard's. Kills `set_anchor` as a no-op.
#[test]
fn a_shift_click_runs_from_the_anchor_the_app_set_without_a_keyboard() {
    let mut p = build(
        Opts {
            keyboard: false,
            ..MULTI
        },
        flat(&["a", "b", "c", "d"]),
        &[],
    );
    click(&mut p, "a", Modifiers::default());
    p.state.controller.set_selection(["c".to_string()]);
    p.state.controller.set_anchor(Some("c"));
    click(&mut p, "d", shift());
    assert_eq!(sel(&p), ["c", "d"]);
}

/// `None` clears the anchor: Shift+Down runs from the cursor row, the
/// fallback, which becomes the anchor. Kills a `set_anchor` that only
/// writes `Some`.
#[test]
fn a_cleared_anchor_runs_from_the_cursor() {
    let mut p = clicked_a_cursor_on_b(MULTI);
    p.state.controller.set_anchor(None);
    assert_eq!(p.state.controller.anchor(), None);
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(sel(&p), ["b", "c"]);
    assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
}

/// A value not in the data is an anchor whose row was deleted: the run
/// starts at the cursor row. Kills a `set_anchor` that refuses (keeps
/// the old anchor for) a value it cannot find.
#[test]
fn an_anchor_not_in_the_data_runs_from_the_cursor() {
    let mut p = clicked_a_cursor_on_b(MULTI);
    p.state.controller.set_anchor(Some("nowhere"));
    assert_eq!(p.state.controller.anchor().as_deref(), Some("nowhere"));
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(sel(&p), ["b", "c"]);
    assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
}

/// Setting the anchor touches nothing else: no selection change, no
/// report, no `onselect`. Kills a `set_anchor` that also selects the
/// row (or reports).
#[test]
fn set_anchor_changes_and_reports_nothing() {
    let mut p = clicked_a_cursor_on_b(MULTI);
    let (changes, selects) = (p.changes.borrow().len(), p.selects.borrow().len());
    p.state.controller.set_anchor(Some("c"));
    p.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(p.changes.borrow().len(), changes, "onselectionchange");
    assert_eq!(p.selects.borrow().len(), selects, "onselect");
    assert_eq!(p.state.cursor.get().as_deref(), Some("b"), "the cursor");
}
