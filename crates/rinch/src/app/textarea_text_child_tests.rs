//! A `<textarea>` whose text arrived as a **child** is edited from that text
//! (#1159).
//!
//! The children are its default value — what a browser's `.value` holds until
//! something writes it — so the desktop field starts editing from them, and
//! the first keystroke writes `value` with them in it. Before, focus read only
//! the (absent) `value` attribute: the field started empty and the first key
//! replaced the text with itself, while the child went on painting underneath.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

fn mount(child: &'static str) -> (RinchApp, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let input_id = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(v)
    }));
    let id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element("textarea");
        field.set_attribute("style", "display: block; width: 200px; height: 60px");
        field.set_attribute("data-oninput", &input_id.0.to_string());
        let text = scope.create_text(child);
        field.append_child(&text);
        root.append_child(&field);
        id_in.set(Some(field.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, id.get().expect("node id captured at mount"), log)
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

fn click_into(app: &mut RinchApp, id: usize) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
    for ev in [
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
        app.handle_event(ev, (800, 600), 1.0);
    }
}

fn value_attr(app: &RinchApp, id: usize) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
}

#[test]
fn typing_into_a_text_child_textarea_edits_its_text() {
    let (mut app, id, log) = mount("hello");
    click_into(&mut app, id);
    // A frame between the focus and the key: the per-frame adoption of a
    // programmatic `value` must read the children too, or it adopts "".
    app.resolve_and_repaint(800.0, 600.0);
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("helloX"));
    assert_eq!(log.borrow().last().map(String::as_str), Some("helloX"));
}

/// The first click into a text-child textarea places the caret from the child
/// text: a click right of "hello" lands at its end (review of #1179 — the
/// click→caret map reads `control_value` too).
#[test]
fn the_first_click_places_the_caret_in_the_child_text() {
    let (mut app, id, _log) = mount("hello");
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, _h) = painted_element_box(&d.tree, id);
        (ax + w - 10.0, ay + 6.0)
    };
    for ev in [
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
        app.handle_event(ev, (800, 600), 1.0);
    }
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("helloX"));
}

// ── The dirty value flag (#1186) ──────────────────────────────────────────
//
// A textarea's text children stay its value until the user edits it or
// something writes `value` — HTML's dirty value flag. Focus and blur are
// neither. Chrome 153, measured: after `focus(); blur()` a child change shows;
// while focused and unedited a child change shows and the caret keeps its
// offset (UTF-16 units, clamped to the new length); after an edit — even one
// that retypes the same character — or a script `.value` write, it does not.

/// [`mount`], plus a `data-onchange` handler; returns its log as well.
#[allow(clippy::type_complexity)]
fn mount_with_change(child: &'static str) -> (RinchApp, usize, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"))
        }))
    };
    let input_id = record("input");
    let change_id = record("change");
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element("textarea");
        field.set_attribute("style", "display: block; width: 200px; height: 60px");
        field.set_attribute("data-oninput", &input_id.0.to_string());
        field.set_attribute("data-onchange", &change_id.0.to_string());
        let text = scope.create_text(child);
        field.append_child(&text);
        root.append_child(&field);
        ids_in.set(Some((field.node_id().0, text.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (field, text) = ids.get().expect("node ids captured at mount");
    (app, field, text, log)
}

fn live(app: &RinchApp, id: usize) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.live_value(rinch_core::dom::NodeId(id))
}

/// The reactive `textarea { {|| draft.get()} }` shape: the child text changes
/// and a frame runs.
fn set_child(app: &mut RinchApp, text_id: usize, s: &str) {
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_text_content(rinch_core::dom::NodeId(text_id), s);
    app.resolve_and_repaint(800.0, 600.0);
}

fn blur(app: &mut RinchApp) {
    for ev in [
        PlatformEvent::MouseDown {
            x: 700.0,
            y: 500.0,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x: 700.0,
            y: 500.0,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(ev, (800, 600), 1.0);
    }
    app.resolve_and_repaint(800.0, 600.0);
}

fn shift_left(app: &mut RinchApp) {
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
}

/// Caret after the `n`th character from the start of the (single) line.
fn caret_at(app: &mut RinchApp, n: usize) {
    press(app, KeyCode::Home, None);
    for _ in 0..n {
        press(app, KeyCode::ArrowRight, None);
    }
}

#[test]
fn focus_and_blur_without_an_edit_keep_following_the_child_text() {
    let (mut app, id, text, log) = mount_with_change("hello");
    click_into(&mut app, id);
    app.resolve_and_repaint(800.0, 600.0);
    blur(&mut app);
    assert_eq!(value_attr(&app, id), None, "focus is not an edit");
    set_child(&mut app, text, "changed");
    assert_eq!(live(&app, id).as_deref(), Some("changed"));
    // Focusing again edits what it shows now.
    click_into(&mut app, id);
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("changedX"));
    assert_eq!(log.borrow().as_slice(), ["input:changedX"]);
}

#[test]
fn a_child_change_while_focused_and_unedited_shows_and_keeps_the_caret_offset() {
    let (mut app, id, text, log) = mount_with_change("hello world");
    click_into(&mut app, id);
    caret_at(&mut app, 3);
    set_child(&mut app, text, "changed text longer");
    assert_eq!(live(&app, id).as_deref(), Some("changed text longer"));
    assert_eq!(
        value_attr(&app, id),
        None,
        "a default-value change is not an edit"
    );
    // Chrome keeps the caret at offset 3, not after the rewritten text.
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(
        value_attr(&app, id).as_deref(),
        Some("chaXnged text longer")
    );
    // Neither the child change nor the focus is the user's; only the key is.
    assert_eq!(log.borrow().as_slice(), ["input:chaXnged text longer"]);
}

#[test]
fn a_shorter_child_text_clamps_the_caret() {
    let (mut app, id, text, _log) = mount_with_change("hello world");
    click_into(&mut app, id);
    caret_at(&mut app, 5);
    set_child(&mut app, text, "ab");
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("abX"));
}

/// The kept offset counts characters (UTF-16 units, as Chrome's
/// `selectionStart` does), not bytes, on both sides: three characters into
/// "ééééé" (byte 6) is three characters into "ñññññ" (byte 6), and ASCII
/// on either side would hide a byte count on that side.
#[test]
fn the_kept_caret_offset_counts_characters_not_bytes() {
    let (mut app, id, text, _log) = mount_with_change("ééééé");
    click_into(&mut app, id);
    caret_at(&mut app, 3);
    set_child(&mut app, text, "ñabcd");
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("ñabXcd"));
}

/// A default-value change while focused is not the user's change: blurring
/// afterwards commits nothing (#226).
#[test]
fn a_child_change_while_focused_commits_no_change_at_blur() {
    let (mut app, id, text, log) = mount_with_change("hello");
    click_into(&mut app, id);
    app.resolve_and_repaint(800.0, 600.0);
    set_child(&mut app, text, "changed");
    blur(&mut app);
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
    // And the field still follows its children.
    set_child(&mut app, text, "again");
    assert_eq!(live(&app, id).as_deref(), Some("again"));
}

#[test]
fn an_edit_freezes_the_value_against_later_child_changes() {
    let (mut app, id, text, _log) = mount_with_change("hello");
    click_into(&mut app, id);
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    set_child(&mut app, text, "changed while focused");
    assert_eq!(live(&app, id).as_deref(), Some("helloX"));
    blur(&mut app);
    set_child(&mut app, text, "changed after blur");
    assert_eq!(live(&app, id).as_deref(), Some("helloX"));
}

/// Retyping the selected character over itself leaves the text as it was, and
/// is still the user's edit (Chrome: the dirty flag is set).
#[test]
fn retyping_the_same_character_is_still_an_edit() {
    let (mut app, id, text, _log) = mount_with_change("hello");
    click_into(&mut app, id);
    press(&mut app, KeyCode::End, None);
    shift_left(&mut app);
    press(&mut app, KeyCode::KeyO, Some("o"));
    blur(&mut app);
    set_child(&mut app, text, "changed");
    assert_eq!(live(&app, id).as_deref(), Some("hello"));
}

/// A programmatic `value` write sets the flag too.
#[test]
fn a_value_write_freezes_the_value_against_later_child_changes() {
    let (mut app, id, text, _log) = mount_with_change("hello");
    click_into(&mut app, id);
    app.resolve_and_repaint(800.0, 600.0);
    app.doc.as_ref().unwrap().borrow_mut().set_attribute(
        rinch_core::dom::NodeId(id),
        "value",
        "prog",
    );
    app.resolve_and_repaint(800.0, 600.0);
    set_child(&mut app, text, "changed");
    assert_eq!(live(&app, id).as_deref(), Some("prog"));
    press(&mut app, KeyCode::End, None);
    press(&mut app, KeyCode::KeyX, Some("X"));
    assert_eq!(value_attr(&app, id).as_deref(), Some("progX"));
}

// ── Chrome 153 sequences from the review of #1208 ─────────────────────────
mod review1208 {
    use super::*;

    fn chord(app: &mut RinchApp, key: KeyCode) {
        app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: None,
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                repeat: KeyRepeat::Unknown,
            },
            (800, 600),
            1.0,
        );
    }
    fn ime(app: &mut RinchApp, ev: rinch_platform::ImeEvent) {
        app.handle_event(PlatformEvent::Ime(ev), (800, 600), 1.0);
    }
    fn preedit(app: &mut RinchApp, t: &str) {
        ime(
            app,
            rinch_platform::ImeEvent::Preedit {
                text: t.into(),
                cursor: None,
            },
        );
    }

    /// Chrome C4: Ctrl+Z on an unedited field is no edit.
    #[test]
    fn r_undo_on_unedited_follows() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        chord(&mut app, KeyCode::KeyZ);
        assert_eq!(value_attr(&app, id), None);
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("CHANGED"));
    }

    /// Chrome C5/C6/C8: no-op deletes and an empty cut are no edit.
    #[test]
    fn r_noop_deletes_follow() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        press(&mut app, KeyCode::Home, None);
        press(&mut app, KeyCode::Backspace, None);
        press(&mut app, KeyCode::End, None);
        press(&mut app, KeyCode::Delete, None);
        chord(&mut app, KeyCode::KeyX);
        assert_eq!(value_attr(&app, id), None);
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("CHANGED"));
    }

    /// Chrome C7: select-all is no edit.
    #[test]
    fn r_select_all_follows_and_keeps_selection() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        chord(&mut app, KeyCode::KeyA);
        assert_eq!(value_attr(&app, id), None);
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("CHANGED"));
        // Chrome keeps 0..5; typing replaces "CHANG".
        press(&mut app, KeyCode::KeyX, Some("X"));
        assert_eq!(value_attr(&app, id).as_deref(), Some("XED"));
    }

    /// Chrome C3: type then undo back to the original — frozen.
    #[test]
    fn r_type_then_undo_stays_frozen() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        press(&mut app, KeyCode::End, None);
        press(&mut app, KeyCode::KeyX, Some("X"));
        chord(&mut app, KeyCode::KeyZ);
        assert_eq!(value_attr(&app, id).as_deref(), Some("hello"));
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("hello"));
    }

    /// Chrome C10: a cancelled composition sets the flag (Chrome's `.value`
    /// held the preedit).
    #[test]
    fn r_ime_cancel_then_child_change() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        ime(&mut app, rinch_platform::ImeEvent::Enabled);
        preedit(&mut app, "あ");
        preedit(&mut app, "");
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("hello"));
    }

    /// Chrome C11: a child change during a composition does not reach
    /// `.value`, during it or after it ends.
    #[test]
    fn r_child_change_during_composition() {
        let (mut app, id, text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        ime(&mut app, rinch_platform::ImeEvent::Enabled);
        preedit(&mut app, "あ");
        set_child(&mut app, text, "CHANGED");
        assert_eq!(live(&app, id).as_deref(), Some("hello"));
        preedit(&mut app, "");
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(live(&app, id).as_deref(), Some("hello"));
    }

    /// An `<input>` with no `value` attribute still gets one at focus.
    #[test]
    fn r_input_still_writes_value_at_focus() {
        let id_cell: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let id_in = id_cell.clone();
        let h = register_input_handler(InputCallback::new(|_v: String| {}));
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let field = scope.create_element("input");
            field.set_attribute("style", "display: block; width: 200px; height: 30px");
            field.set_attribute("data-oninput", &h.0.to_string());
            root.append_child(&field);
            id_in.set(Some(field.node_id().0));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let id = id_cell.get().unwrap();
        click_into(&mut app, id);
        assert_eq!(value_attr(&app, id).as_deref(), Some(""));
    }

    /// A `value` attribute removed after an edit empties the field and leaves
    /// its dirty value flag set (#1222): rinch-web's removal writes `.value =
    /// ""`, which sets the browser's flag, and a page cannot clear it. Before
    /// #1222 desktop read the removal as "pristine again" and fell back to the
    /// child text — the opposite of what the web can express. Replaces the
    /// #1179/#1186 fixture that pinned that fallback.
    #[test]
    fn r_value_removed_after_an_edit_empties_the_field() {
        let (mut app, id, _text, _log) = mount_with_change("hello");
        click_into(&mut app, id);
        press(&mut app, KeyCode::End, None);
        press(&mut app, KeyCode::KeyY, Some("Y"));
        assert_eq!(value_attr(&app, id).as_deref(), Some("helloY"));
        app.doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .remove_attribute(rinch_core::dom::NodeId(id), "value");
        app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(live(&app, id).as_deref(), Some(""));
        press(&mut app, KeyCode::End, None);
        press(&mut app, KeyCode::KeyX, Some("X"));
        assert_eq!(value_attr(&app, id).as_deref(), Some("X"));
    }
    /// A child change during a composition must not reach paint while the
    /// caret attributes still index the engine text (kills M6: drop the
    /// text compare in `keeps_default`).
    #[test]
    fn r_child_change_during_composition_keeps_the_node_coherent() {
        let (mut app, id, text, _log) = mount_with_change("héllo");
        click_into(&mut app, id);
        caret_at(&mut app, 1);
        ime(&mut app, rinch_platform::ImeEvent::Enabled);
        preedit(&mut app, "あ");
        set_child(&mut app, text, "ééé");
        assert_eq!(live(&app, id).as_deref(), Some("héllo"));
        app.resolve_and_repaint(800.0, 600.0);
    }
}

// ── #1222: where the dirty value flag lives must agree with rinch-web ──────
//
// The twins are `crates/rinch-web/tests/textarea_text_child.rs`'s
// `*_1222` fixtures (Chrome 153). The rule both backends follow: a `value`
// write — equal to the shown text or not — sets the flag, and removing the
// attribute is a write of `""` that leaves it set. Nothing a page can do
// clears it again (only a form reset, which desktop does not model).
mod dirty_flag_1222 {
    use super::*;

    fn write(app: &mut RinchApp, id: usize, v: Option<&str>) {
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            match v {
                Some(v) => d.set_attribute(rinch_core::dom::NodeId(id), "value", v),
                None => d.remove_attribute(rinch_core::dom::NodeId(id), "value"),
            }
        }
        app.resolve_and_repaint(800.0, 600.0);
    }

    /// Issue #1222 case 1: write, remove, then change the child.
    #[test]
    fn removing_a_written_value_leaves_the_field_empty_and_dirty() {
        let (mut app, id, text, _log) = mount_with_change("child");
        write(&mut app, id, Some("written"));
        assert_eq!(live(&app, id).as_deref(), Some("written"));
        write(&mut app, id, None);
        assert_eq!(
            live(&app, id).as_deref(),
            Some(""),
            "the removal empties it"
        );
        assert_eq!(
            value_attr(&app, id),
            None,
            "the attribute is gone, as `getAttribute` says on the web"
        );
        set_child(&mut app, text, "changed");
        assert_eq!(
            live(&app, id).as_deref(),
            Some(""),
            "the flag survives the removal: a child change is not shown"
        );
    }

    /// Removing a `value` the pristine field never had is the same write of
    /// `""` on the web (the shown "child" differs from it), so the field
    /// empties and stops following its children.
    #[test]
    fn removing_an_absent_value_from_a_pristine_textarea_empties_it() {
        let (mut app, id, text, _log) = mount_with_change("child");
        assert_eq!(live(&app, id).as_deref(), Some("child"), "positive control");
        write(&mut app, id, None);
        assert_eq!(live(&app, id).as_deref(), Some(""));
        set_child(&mut app, text, "changed");
        assert_eq!(live(&app, id).as_deref(), Some(""));
    }

    /// Issue #1222 case 2: a write equal to the shown text still sets the flag
    /// (desktop did this already; the web now does too).
    #[test]
    fn an_equal_value_write_sets_the_flag() {
        let (mut app, id, text, _log) = mount_with_change("same");
        write(&mut app, id, Some("same"));
        set_child(&mut app, text, "changed");
        assert_eq!(live(&app, id).as_deref(), Some("same"));
    }

    /// The shell edits the emptied field from `""`, not from the child text:
    /// the removal reached the focus path, not only `live_value`.
    #[test]
    fn a_removal_on_a_pristine_textarea_is_typed_into_from_empty() {
        let (mut app, id, _text, _log) = mount_with_change("child");
        write(&mut app, id, None);
        click_into(&mut app, id);
        app.resolve_and_repaint(800.0, 600.0);
        press(&mut app, KeyCode::End, None);
        press(&mut app, KeyCode::KeyX, Some("X"));
        assert_eq!(value_attr(&app, id).as_deref(), Some("X"));
    }

    /// The removal reaches paint: the incremental frame after it holds what
    /// a from-scratch frame holds over the field, and that differs from the
    /// frame before (the child text is gone). Kills dropping the paint
    /// invalidation from `remove_attribute`'s absent-attribute arm.
    #[cfg(software_shell)]
    #[test]
    fn a_removal_on_a_pristine_textarea_repaints_it() {
        const SIZE: (u32, u32) = (800, 600);
        let (mut app, id, _text, _log) = mount_with_change("child text");
        let frame = |app: &mut RinchApp, full: bool| {
            if full {
                app.scene_dirty = true;
                app.has_previous_frame = false;
            }
            let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
            let _ = app.end_perf_frame();
            px
        };
        let before = frame(&mut app, true);
        {
            let mut d = app.doc.as_ref().unwrap().borrow_mut();
            d.remove_attribute(rinch_core::dom::NodeId(id), "value");
        }
        app.resolve_and_repaint(800.0, 600.0);
        let incremental = frame(&mut app, false);
        let fresh = frame(&mut app, true);
        let (x, y, w, h) = {
            let d = app.doc.as_ref().unwrap().borrow();
            painted_element_box(&d.tree, id)
        };
        let region = |px: &[u8]| -> Vec<u8> {
            let mut out = Vec::new();
            for row in (y as usize)..((y + h) as usize) {
                let start = (row * SIZE.0 as usize + x as usize) * 4;
                out.extend_from_slice(&px[start..start + (w as usize) * 4]);
            }
            out
        };
        assert_ne!(
            region(&before),
            region(&fresh),
            "positive control: the child text was painted and is gone"
        );
        assert!(
            region(&incremental) == region(&fresh),
            "the incremental frame repainted the emptied field"
        );
    }
}
