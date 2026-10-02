//! `NodeHandle::select()` / `set_selection_range()` for a text control (issue
//! #552): desktop's half of the "applied to the focused input directly, or
//! stashed for the next time it gains focus" contract documented on
//! `DomDocument::set_selection_range` and `RinchApp::apply_or_stash_text_selection`.

use super::*;
use rinch_core::dom::SelectionDirection;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

/// Node ids captured at mount, in DOM order.
#[derive(Clone, Copy)]
struct Ids {
    a: usize,
    b: usize,
}

/// Two plain `<input>`s, `a` carrying `value="ab🙂"` (3 chars / 6 bytes / 4
/// UTF-16 units — a surrogate pair so a byte-offset mutant would be caught)
/// and `b` carrying `value="hello"`.
fn mount_fixture() -> (RinchApp, Ids) {
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        // `try_focus_input` only installs an `EditableState` for a node
        // `node_takes_text_focus` *and* carrying `data-oninput` routes as a
        // text field (issue #424) — a no-op handler is enough, since neither
        // test here drives a real `oninput` round trip.
        let noop = || register_input_handler(InputCallback::new(|_: String| {})).0;
        let a = scope.create_element("input");
        a.set_attribute("style", "width: 200px; height: 30px");
        a.set_attribute("value", "ab\u{1F642}");
        a.set_attribute("data-oninput", &noop().to_string());
        let b = scope.create_element("input");
        b.set_attribute("style", "width: 200px; height: 30px");
        b.set_attribute("value", "hello");
        b.set_attribute("data-oninput", &noop().to_string());
        root.append_child(&a);
        root.append_child(&b);
        ids_in.set(Some(Ids {
            a: a.node_id().0,
            b: b.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ids = ids.get().expect("node ids captured at mount");
    (app, ids)
}

/// A `NodeHandle` for a mounted node, through the same `Rc::downgrade(..) as
/// _` unsizing every other fixture in this crate uses to reach `NodeHandle`
/// from a concrete `RinchDocument`.
fn handle_for(app: &RinchApp, node_id: usize) -> NodeHandle {
    let doc = app.doc.clone().expect("mounted");
    let weak: std::rc::Weak<RefCell<dyn DomDocument>> = Rc::downgrade(&doc) as _;
    NodeHandle::new(rinch_core::dom::NodeId(node_id), weak)
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
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

/// `select()` on the **focused** field selects its entire text, converting
/// the UTF-16 length `select_text`'s default computes back to the right byte
/// range — "ab🙂" is 6 bytes but only 4 UTF-16 units, so a mutant that treated
/// the UTF-16 units as byte offsets (or vice versa) would select only part of
/// the emoji, not all 6 bytes.
#[test]
fn select_on_the_focused_field_selects_its_whole_text() {
    let (mut app, ids) = mount_fixture();
    app.try_focus_input(ids.a);
    assert_eq!(app.focused_input_node_id, Some(ids.a));

    handle_for(&app, ids.a).select();
    app.drain_pending_text_selection();

    let state = app.focused_input_state.as_ref().expect("focused");
    assert_eq!(state.document.to_text(), "ab\u{1F642}");
    assert_eq!(
        state.selection.range(),
        rinch_editable::Range::new(0, state.document.to_text().len()),
        "select() must cover the whole 6-byte text, not the 4 UTF-16 units \
         it was computed from"
    );
}

/// `set_selection_range()` on the focused field, then typing a character,
/// replaces exactly the selected range — the ordinary "selected text makes
/// way for a keystroke" behaviour, proving the UTF-16-to-byte conversion
/// landed the selection on the right characters and not merely a span of the
/// right *length*.
#[test]
fn set_selection_range_then_typing_replaces_the_selection() {
    let (mut app, ids) = mount_fixture();
    app.try_focus_input(ids.b);

    // "hello" — select "ell" (UTF-16 units 1..4, same as bytes here).
    handle_for(&app, ids.b).set_selection_range(1, 4, SelectionDirection::Forward);
    app.drain_pending_text_selection();

    key(&mut app, KeyCode::KeyX, Some("X"));

    assert_eq!(
        app.focused_input_value, "hXo",
        "typing over a [1,4) selection in \"hello\" must leave \"hXo\""
    );
}

/// `set_selection_range()` on a field that is **not yet focused** is honoured
/// the first time it *is* focused — the same "set now, applied at the next
/// focus" rule a browser's own `setSelectionRange()` follows (issue #552) —
/// rather than being silently dropped.
#[test]
fn set_selection_range_on_an_unfocused_field_is_honoured_at_the_next_focus() {
    let (mut app, ids) = mount_fixture();
    assert_eq!(app.focused_input_node_id, None, "nothing focused yet");

    handle_for(&app, ids.b).set_selection_range(1, 4, SelectionDirection::Backward);
    app.drain_pending_text_selection();

    // Stashed, not applied: no EditableState exists for `b` yet.
    assert!(app.focused_input_state.is_none());
    assert!(
        app.pending_text_selection.contains_key(&ids.b),
        "an unfocused node's request must be stashed for its next focus"
    );

    app.try_focus_input(ids.b);

    let state = app.focused_input_state.as_ref().expect("now focused");
    assert_eq!(
        state.selection.range(),
        rinch_editable::Range::new(1, 4),
        "the stashed [1,4) range must be applied once `b` is focused"
    );
    assert!(
        !state.selection.is_forward(),
        "Backward must put the anchor at the high end"
    );
    assert!(
        !app.pending_text_selection.contains_key(&ids.b),
        "the stash is consumed, not replayed on every future focus"
    );
}
