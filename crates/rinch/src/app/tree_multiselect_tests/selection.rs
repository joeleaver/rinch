//! The multi-select rules (F2-F15): disabled, deleted and hidden anchors,
//! collapses, reports, the cursor, Space/Shift+Space/Enter, clicks, and the keys
//! `handle_key` is offered.

use super::*;

// ---- Disabled rows (F2) ----------------------------------------------------

#[test]
fn a_shift_click_run_leaves_out_a_disabled_row() {
    let mut p = build(MULTI, with_disabled_b(), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "c"]);
    assert_eq!(last_report(&p), ["a", "c"]);
}

// ---- Without select_on_click (F3, #1601's rule) -----------------------------

#[test]
fn without_select_on_click_shift_arrows_only_move_the_cursor() {
    let mut p = build(MULTI_NO_SELECT, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    assert!(sel(&p).is_empty());
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(
        p.state.cursor.get().as_deref(),
        Some("b"),
        "the cursor moves"
    );
    assert!(sel(&p).is_empty(), "Shift+Down selected {:?}", sel(&p));
}

// ---- A deleted anchor (F5) and deleted rows (F6) ----------------------------

/// The data drops b, the anchor; the cursor (on b) moves on to c.
fn drop_the_anchor(p: &mut P) {
    click(p, "b", Modifiers::default());
    assert_eq!(sel(p), ["b"]);
    p.data.set(flat(&["a", "c", "d"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    p.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

#[test]
fn a_shift_arrow_after_the_anchor_row_is_deleted_runs_from_the_cursor() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    drop_the_anchor(&mut p);
    assert_eq!(
        p.state.cursor.get().as_deref(),
        Some("c"),
        "the cursor moved on"
    );
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("d"));
    assert_eq!(sel(&p), ["c", "d"]);
    assert_eq!(last_report(&p), ["c", "d"]);
}

#[test]
fn a_shift_click_after_the_anchor_row_is_deleted_selects_the_clicked_row() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    drop_the_anchor(&mut p);
    click(&mut p, "d", shift());
    assert_eq!(sel(&p), ["d"]);
    // And the next run starts there.
    click(&mut p, "a", shift());
    assert_eq!(sel(&p), ["a", "c", "d"]);
}

#[test]
fn a_deleted_row_is_not_reported() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "b", Modifiers::default());
    p.data.set(flat(&["a", "c"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "c", ctrl());
    assert_eq!(last_report(&p), ["c"]);
}

// ---- A hidden anchor (F11) and collapses (F12) ------------------------------

#[test]
fn a_run_from_an_anchor_hidden_by_a_collapse_starts_at_its_shown_parent() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    // The app collapses a: the anchor a1 is hidden (and stays selected:
    // only the tree's own collapses unselect).
    p.state.controller.collapse("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "b", "c"]);
}

#[test]
fn a_left_collapse_unselects_the_rows_it_hides() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "a2", ctrl());
    click(&mut p, "a", ctrl());
    assert_eq!(sel(&p), ["a", "a1", "a2"]);
    press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "collapsed");
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(last_report(&p), ["a"]);
}

/// A tree that does not select leaves the selection to the app: no collapse
/// (row click, chevron or Left) unselects what it hides, and nothing is
/// reported.
#[test]
fn a_tree_that_does_not_select_keeps_the_apps_selection_on_collapse() {
    let mut p = build(MULTI_NO_SELECT, nested(), &["a"]);
    p.state
        .controller
        .set_selection(["a1".to_string(), "b".to_string()]);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "a", Modifiers::default());
    assert!(
        !p.state.controller.is_expanded("a"),
        "the click collapsed a"
    );
    assert_eq!(sel(&p), ["a1", "b"]);
    p.state.controller.expand("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click_chevron(&mut p, "a");
    assert!(
        !p.state.controller.is_expanded("a"),
        "the chevron collapsed a"
    );
    p.state.controller.expand("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "Left collapsed a");
    assert_eq!(sel(&p), ["a1", "b"]);
    assert!(p.changes.borrow().is_empty(), "{:?}", p.changes.borrow());
}

/// The chevron's collapse unselects the rows it hides, as Left does.
#[test]
fn a_chevron_collapse_unselects_the_rows_it_hides() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    assert_eq!(sel(&p), ["a1", "b"]);
    click_chevron(&mut p, "a");
    assert!(
        !p.state.controller.is_expanded("a"),
        "the chevron collapsed a"
    );
    assert_eq!(sel(&p), ["b"]);
    assert_eq!(last_report(&p), ["b"]);
}

/// A click that collapses a row and selects it is one report, with the
/// final selection.
#[test]
fn a_collapse_click_reports_once() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    let before = p.changes.borrow().len();
    click(&mut p, "a", Modifiers::default());
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(p.changes.borrow()[before..], [vec!["a".to_string()]]);
}

/// Enter on an open row collapses it: the rows it hides leave the
/// selection, the row joins it, and that is one report.
#[test]
fn an_enter_collapse_drops_the_hidden_rows_and_reports_once() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    click(&mut p, "a", ctrl()); // the cursor on a
    assert_eq!(sel(&p), ["a", "a1", "b"]);
    p.state.controller.deselect("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let before = p.changes.borrow().len();
    press(&mut p, KeyCode::Enter, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "Enter collapsed a");
    assert_eq!(sel(&p), ["a", "b"]);
    let after = p.changes.borrow()[before..].to_vec();
    assert_eq!(after, [vec!["a".to_string(), "b".to_string()]]);
}

/// Click the chevron of the row of `value`.
fn click_chevron(p: &mut P, value: &str) {
    let chevron = {
        let doc = p.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let content = row(&p.app, value);
        d.tree.nodes[content]
            .children
            .iter()
            .copied()
            .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__chevron"))
            .expect("the chevron")
    };
    let (x, y) = centre(&p.app, chevron);
    for event in [
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
        p.app.handle_event(event, (800, 600), 1.0);
    }
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

// ---- Reports (F10) ----------------------------------------------------------

#[test]
fn nothing_is_reported_when_the_selection_did_not_change() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    let n = p.changes.borrow().len();
    assert_eq!(n, 1, "the first click is reported");
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowUp, None, shift()); // already at the top
    assert_eq!(
        p.changes.borrow().len(),
        n,
        "{:?}",
        &p.changes.borrow()[n..]
    );
    press(&mut p, KeyCode::End, None, shift());
    assert_eq!(p.changes.borrow().len(), n + 1, "Shift+End changed it");
    press(&mut p, KeyCode::End, None, shift());
    assert_eq!(
        p.changes.borrow().len(),
        n + 1,
        "a second Shift+End did not"
    );
}

#[test]
fn the_report_is_top_to_bottom() {
    let rows = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let mut p = build(MULTI, flat(&rows), &[]);
    click(&mut p, "h", Modifiers::default());
    for v in rows.iter().rev().skip(1) {
        click(&mut p, v, ctrl());
    }
    assert_eq!(last_report(&p), rows);
}

// ---- The cursor (F8) --------------------------------------------------------

#[test]
fn oncursor_hears_a_shift_arrow_move() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    p.rested.borrow_mut().clear();
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(*p.rested.borrow(), ["b"], "a plain arrow is reported");
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    assert_eq!(*p.rested.borrow(), ["b", "c"]);
}

// ---- Space, Shift+Space and Enter (F4, F9) ----------------------------------

#[test]
fn space_tells_onselect_of_a_row_it_selects() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    assert_eq!(*p.selects.borrow(), ["a"], "a click tells onselect");
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"]);
    assert_eq!(*p.selects.borrow(), ["a", "b"]);
    // Space again unselects b, and tells onselect nothing.
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(*p.selects.borrow(), ["a", "b"]);
}

/// Shift+Space with Shift held as a real keyboard reports it.
fn shift_space(p: &mut P) {
    p.app
        .handle_event(PlatformEvent::ModifiersChanged(shift()), (800, 600), 1.0);
    press(p, KeyCode::Space, Some(" "), shift());
    p.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
}

#[test]
fn shift_space_selects_the_run_from_the_row_last_clicked() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    shift_space(&mut p);
    assert_eq!(
        sel(&p),
        ["a", "b", "c"],
        "a plain arrow does not move the anchor"
    );
}

#[test]
fn shift_space_selects_the_run_from_the_row_last_toggled() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"]);
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    p.selects.borrow_mut().clear();
    shift_space(&mut p);
    assert_eq!(sel(&p), ["b", "c"], "the run starts at b, where Space was");
    assert_eq!(last_report(&p), ["b", "c"]);
    assert!(
        p.selects.borrow().is_empty(),
        "no click: {:?}",
        p.selects.borrow()
    );
}

#[test]
fn enter_keeps_a_multi_selection_and_adds_the_cursor_row() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    assert_eq!(sel(&p), ["a", "c"]);
    press(&mut p, KeyCode::ArrowUp, None, Modifiers::default());
    p.selects.borrow_mut().clear();
    press(&mut p, KeyCode::Enter, None, Modifiers::default());
    assert_eq!(sel(&p), ["a", "b", "c"]);
    assert_eq!(
        *p.selects.borrow(),
        ["b"],
        "Enter tells onselect, as a click does"
    );
}

// ---- Anchors and clicks (F15) -----------------------------------------------

#[test]
fn a_ctrl_click_moves_the_anchor() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    click(&mut p, "d", shift());
    assert_eq!(sel(&p), ["c", "d"]);
}

#[test]
fn a_plain_click_replaces_the_selection() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "b", ctrl());
    click(&mut p, "c", Modifiers::default());
    assert_eq!(sel(&p), ["c"]);
    assert_eq!(last_report(&p), ["c"]);
}

#[test]
fn a_tree_without_keyboard_or_drag_selects_a_run() {
    let mut p = build(
        Opts {
            keyboard: false,
            ..MULTI
        },
        flat(&["a", "b", "c", "d"]),
        &[],
    );
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "b", "c"]);
}

/// #3062: Ctrl+A and Cmd+A are not the tree's. It selected the cursor row's
/// siblings; now the selection is unchanged, nothing is reported, and the
/// app's `handle_key` (offered every key first) is where it binds one.
#[test]
fn ctrl_a_and_cmd_a_are_left_to_the_app() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    let reports = p.changes.borrow().len();
    let cmd = Modifiers {
        meta: true,
        ..Default::default()
    };
    for modifiers in [ctrl(), cmd] {
        p.seen.borrow_mut().clear();
        press(&mut p, KeyCode::KeyA, Some("a"), modifiers);
        assert_eq!(sel(&p), ["a1"], "the selection is unchanged");
        assert_eq!(p.changes.borrow().len(), reports, "nothing is reported");
        assert_eq!(
            p.seen.borrow().first().map(|s| s.0.as_str()),
            Some("a"),
            "the app was offered it"
        );
    }
}

// ---- What handle_key is offered (F7) ----------------------------------------

/// #1601: `TreeKey::below` of a row alone is the row drawn right below it,
/// an open parent's first child.
#[test]
fn below_an_open_parent_is_its_first_child() {
    let data = vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
    ];
    let single = Opts {
        multiple: false,
        ..MULTI
    };
    for opts in [single, MULTI] {
        let mut p = build(opts, data.clone(), &["a"]);
        click(&mut p, "a", Modifiers::default()); // a click toggles a closed
        p.state.controller.expand("a");
        p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        press(&mut p, KeyCode::F2, None, Modifiers::default());
        let seen = p.seen.borrow().last().cloned().unwrap();
        assert_eq!(seen.1.as_deref(), Some("a1"), "multiple: {}", opts.multiple);
    }
}

/// A block of selected rows: below is the row below the block, and Down
/// lands after it (a gap in the block closes).
#[test]
fn a_non_contiguous_block_moves_down_as_one() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d", "e"]), &[]);
    // The cursor on b, the first of the block: the row below the block
    // (e) is not the row drawn below b (c).
    click(&mut p, "d", Modifiers::default());
    click(&mut p, "b", ctrl());
    assert_eq!(p.state.cursor.get().as_deref(), Some("b"));
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow().last().cloned().unwrap();
    assert_eq!(seen.1.as_deref(), Some("e"));
    assert_eq!(seen.2, ["b", "d"]);
    assert_eq!(seen.3, Some((None, TreeSlot::After("e".into()))));
}

/// Shift+Space is the tree's own key, not the runtime's activation click: on
/// a disabled cursor row (which takes no click) it still runs to it.
#[test]
fn shift_space_on_a_disabled_row_runs_up_to_it() {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[2] = TreeNodeData::new("c", "C").disabled(true);
    let mut p = build(MULTI, data, &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    shift_space(&mut p);
    assert_eq!(sel(&p), ["a", "b"]);
}
