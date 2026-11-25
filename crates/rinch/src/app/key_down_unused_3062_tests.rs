//! `RinchApp::key_down_unused` (#3062): whether rinch had no use for the last
//! `KeyDown`. The Android shell hands such a key back to the system, so Back
//! (delivered as Escape) goes back when no popup or handler took it.
//!
//! Each "used" case below is paired with the same key unused, so a flag stuck
//! at either answer fails one of the pair.

use super::*;
use rinch_core::events::{on_app_key, push_dismiss_handler};
use rinch_macros::rsx;
use std::cell::Cell;

pub(super) const VP: (u32, u32) = (800, 600);

pub(super) struct Fixture {
    pub(super) app: RinchApp,
    input: NodeHandle,
}

pub(super) fn mount() -> Fixture {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = rsx! {
            div {
                input {
                    style: "width: 200px; height: 30px; font-size: 16px; padding: 0 8px; border: 0",
                    oninput: move |_v: String| {},
                }
            }
        };
        *slot_in.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let input = slot.borrow_mut().take().expect("input");
    Fixture { app, input }
}

/// Press and release; answers `key_down_unused` read after the press, and
/// checks that the release leaves it.
pub(super) fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) -> bool {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    let after_press = app.key_down_unused();
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        VP,
        1.0,
    );
    assert_eq!(
        app.key_down_unused(),
        after_press,
        "a release leaves the press's answer"
    );
    after_press
}

impl Fixture {
    pub(super) fn press(&mut self, key: KeyCode, text: Option<&str>) -> bool {
        press(&mut self.app, key, text)
    }

    pub(super) fn focus_input(&mut self) {
        self.input.focus();
        self.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        assert!(matches!(self.app.focus_target, FocusTarget::Input(_)));
    }

    fn value(&self) -> String {
        let d = self.app.doc.as_ref().unwrap().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, self.input.node_id().0)
            .unwrap_or_default()
            .into_owned()
    }
}

#[test]
fn false_before_any_key() {
    let f = mount();
    assert!(!f.app.key_down_unused());
}

/// Android's Back with nothing open: rinch has no use for Escape.
#[test]
fn escape_with_nothing_to_close_is_unused() {
    let mut f = mount();
    assert!(f.press(KeyCode::Escape, None));
}

/// Back closes the top popup first: a dismiss entry (a modal, a menu, a
/// popover) takes Escape, and the next Escape, with nothing open, is unused.
#[test]
fn escape_a_dismiss_entry_takes_is_used() {
    let mut f = mount();
    let doc_key = f.app.doc_key();
    let dismiss = push_dismiss_handler(doc_key, || true);
    assert!(!f.press(KeyCode::Escape, None));
    drop(dismiss);
    assert!(f.press(KeyCode::Escape, None));
}

/// An app-wide key listener that takes Escape keeps it; one that only listens
/// does not.
#[test]
fn escape_an_app_listener_takes_is_used() {
    let mut f = mount();
    let take = Rc::new(Cell::new(true));
    let take_in = take.clone();
    on_app_key(move |k| k.key == "Escape" && take_in.get());
    assert!(!f.press(KeyCode::Escape, None));
    take.set(false);
    assert!(f.press(KeyCode::Escape, None));
}

/// Text typed into a focused field is used; the same key with no field to
/// type into is not.
#[test]
fn a_typed_character_is_used_only_with_a_field_to_type_into() {
    let mut f = mount();
    assert!(f.press(KeyCode::KeyA, Some("a")), "no field: unused");
    f.focus_input();
    assert!(!f.press(KeyCode::KeyA, Some("a")), "typed");
    assert_eq!(f.value(), "a");
    assert!(
        f.press(KeyCode::Escape, None),
        "Escape in a field is unused"
    );
}

/// A key with a default action in rinch is used: Tab moves the focus, and
/// Backspace edits the field.
#[test]
fn keys_with_a_default_action_are_used() {
    let mut f = mount();
    assert!(!f.press(KeyCode::Tab, None));
    f.focus_input();
    f.press(KeyCode::KeyA, Some("a"));
    assert!(!f.press(KeyCode::Backspace, None));
    assert_eq!(f.value(), "");
}

/// Each press answers afresh: a used key after an unused one is used, and the
/// other way round.
#[test]
fn each_press_answers_afresh() {
    let mut f = mount();
    assert!(f.press(KeyCode::Escape, None));
    assert!(!f.press(KeyCode::Tab, None));
    assert!(f.press(KeyCode::Escape, None));
}

/// The focused rich-text editor: a key it has no binding or text for is
/// unused, a character it types is used.
#[test]
#[cfg(feature = "windowing")]
fn the_focused_editor_uses_text_and_not_escape() {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>hello</p>"));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.append_child(&handle_in.mount(scope));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    handle.focus();
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    let before = handle.doc().content_size();
    assert!(press(&mut app, KeyCode::Escape, None));
    assert_eq!(handle.doc().content_size(), before, "Escape edits nothing");
    assert!(!press(&mut app, KeyCode::KeyX, Some("x")));
    assert_eq!(handle.doc().content_size(), before + 1, "the x was typed");
}
