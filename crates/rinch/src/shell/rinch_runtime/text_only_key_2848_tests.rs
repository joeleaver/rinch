//! Text no key typed reaches the focused field, and nothing else (#2848).
//!
//! `SendInput` with `KEYEVENTF_UNICODE` — what dictation tools, on-screen
//! keyboards and remote-input apps type with on Windows — arrives as a
//! `VK_PACKET` press carrying its character as text. Its physical key comes
//! from the scan-code field. Measured on Windows 11, that field is 0 and the
//! key is `Unidentified(Windows(0))`, which the shell dropped, so the text
//! never arrived. A packet with a nonzero scan code would have been handled
//! as whatever real key has that scan code: a menu chord, a button's
//! activation, an editor binding. The fixtures cover both.
//!
//! Each fixture builds the winit [`KeyEvent`] the way winit-win32 does and
//! hands it to [`RinchRuntime::route_key_event`], the whole of the shell's
//! `KeyboardInput` arm, then gives the app what the shell would. Off the fixed
//! points: the field already holds `ab` (an insertion shows as `ab?`), each
//! physical key named is one that does something (`Ctrl+D` is an armed menu
//! chord, `Space` activates the focused button, `Ctrl+B` is the editor's bold),
//! and a real key is driven beside each packet as the positive control.

use super::*;
use crate::app::FocusTarget;
use rinch_core::events::{InputCallback, register_input_handler};
use rinch_platform::{KeyCode, KeyRepeat, Modifiers, MouseButton as Button, PlatformEvent};
use std::cell::Cell;
use std::rc::Rc;
use winit::event::KeyEvent;
use winit::keyboard::{
    Key, KeyCode as WK, KeyLocation, NativeKey, NativeKeyCode, PhysicalKey, SmolStr,
};

const VP: (u32, u32) = (800, 600);

const FIELD_STYLE: &str = "display: block; width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

/// A `VK_PACKET` (0xE7) event for `ch` as winit-win32 finalizes it: the
/// logical key `Unidentified(Windows(VK_PACKET))`, the physical key from the
/// scan code, and `text` empty under a held Ctrl (winit re-derives Ctrl text
/// from the layout, which has none for the packet key).
fn packet(physical: PhysicalKey, ch: &str, state: ElementState, ctrl_held: bool) -> KeyEvent {
    let pressed = state == ElementState::Pressed;
    let packet = Key::Unidentified(NativeKey::Windows(0xE7));
    KeyEvent {
        physical_key: physical,
        logical_key: packet.clone(),
        text: (pressed && !ctrl_held).then(|| SmolStr::new(ch)),
        location: KeyLocation::Standard,
        state,
        repeat: false,
        text_with_all_modifiers: pressed.then(|| SmolStr::new(ch)),
        key_without_modifiers: packet,
    }
}

/// A real key, as winit reports one: physical and logical keys its own.
fn real(physical: WK, ch: &str, state: ElementState, ctrl_held: bool) -> KeyEvent {
    let pressed = state == ElementState::Pressed;
    KeyEvent {
        physical_key: PhysicalKey::Code(physical),
        logical_key: Key::Character(SmolStr::new(ch)),
        text: (pressed && !ctrl_held).then(|| SmolStr::new(ch)),
        location: KeyLocation::Standard,
        state,
        repeat: false,
        text_with_all_modifiers: pressed.then(|| SmolStr::new(ch)),
        key_without_modifiers: Key::Character(SmolStr::new(ch)),
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

/// One tap — press, then release — through the shell: what
/// `RinchRuntime::window_event` does with each `KeyboardInput`.
fn tap(app: &mut RinchApp, mods: Modifiers, down: KeyEvent) {
    let up = KeyEvent {
        state: ElementState::Released,
        text: None,
        text_with_all_modifiers: None,
        ..down.clone()
    };
    for event in [down, up] {
        if let KeyRoute::Event(event) = RinchRuntime::route_key_event(app, &event, mods, false) {
            app.handle_event(event, VP, 1.0);
        }
    }
}

fn click(app: &mut RinchApp, id: u64) {
    let (x, y, w, h) = {
        let d = app.doc.as_ref().unwrap().borrow();
        crate::app::painted_element_box(&d.tree, id)
    };
    let (x, y) = (x + w - 2.0, y + h / 2.0);
    let button = Button::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

fn value(app: &RinchApp, id: u64) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(id)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
}

/// An `<input>` holding `ab`, and a button counting its clicks.
fn page() -> (RinchApp, u64, u64, Rc<Cell<u32>>) {
    let input_h = register_input_handler(InputCallback::new(|_v: String| {}));
    let clicks = Rc::new(Cell::new(0u32));
    let clicks_in = clicks.clone();
    let ids: Rc<Cell<Option<(u64, u64)>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", "ab");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        let button = scope.create_element("button");
        button.set_attribute("style", "display: block; width: 100px; height: 30px");
        let rid = scope.register_handler(move || clicks_in.set(clicks_in.get() + 1));
        button.set_attribute("data-rid", &rid.0.to_string());
        button.append_child(&scope.create_text("Go"));
        root.append_child(&input);
        root.append_child(&button);
        ids_in.set(Some((input.node_id().0, button.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (input, button) = ids.get().expect("ids captured at mount");
    (app, input, button, clicks)
}

/// A packet types its text into the focused `<input>`: one with no physical
/// key, as Windows sends it (dropped before #2848), and one whose scan code is the D key's
/// while Ctrl is held — a push-to-talk chord — with `Ctrl+D` armed as a menu
/// chord, which ran in place of the text before.
#[test]
fn a_packet_types_into_a_focused_input_and_runs_no_chord() {
    let (mut app, input, _button, _clicks) = page();
    let fired = Rc::new(Cell::new(0u32));
    let f = fired.clone();
    let menu = crate::menu::Menu::new().item(
        crate::menu::MenuItem::new("Duplicate")
            .shortcut("Ctrl+D")
            .on_click(move || f.set(f.get() + 1)),
    );
    let _chords = crate::menu::register_menu_shortcuts(&[("Edit", &menu)]);
    click(&mut app, input);
    assert_eq!(app.focus_target, FocusTarget::Input(input));

    let unplaced = PhysicalKey::Unidentified(NativeKeyCode::Windows(0)); // as measured
    tap(
        &mut app,
        Modifiers::default(),
        packet(unplaced, "é", ElementState::Pressed, false),
    );
    assert_eq!(value(&app, input), "abé", "a packet with no physical key");

    tap(
        &mut app,
        ctrl(),
        packet(
            PhysicalKey::Code(WK::KeyD),
            " ",
            ElementState::Pressed,
            true,
        ),
    );
    assert_eq!(
        value(&app, input),
        "abé ",
        "a packet whose scan code is D's"
    );
    assert_eq!(fired.get(), 0, "and no Ctrl+D");

    // Positive control: the real Ctrl+D is the chord.
    tap(
        &mut app,
        ctrl(),
        real(WK::KeyD, "d", ElementState::Pressed, true),
    );
    assert_eq!(fired.get(), 1, "the real Ctrl+D still runs its item");
    assert_eq!(value(&app, input), "abé ");
}

/// A packet whose scan code is the spacebar's (`"9"`, 0x39) does not activate
/// the focused button, press or release; a real Space does.
#[test]
fn a_packet_does_not_activate_a_focused_button() {
    let (mut app, _input, button, clicks) = page();
    click(&mut app, button);
    let before = clicks.get();
    assert_eq!(app.focus_target, FocusTarget::Node(button));

    tap(
        &mut app,
        Modifiers::default(),
        packet(
            PhysicalKey::Code(WK::Space),
            "9",
            ElementState::Pressed,
            false,
        ),
    );
    assert_eq!(clicks.get(), before, "the packet is no Space");

    tap(
        &mut app,
        Modifiers::default(),
        real(WK::Space, " ", ElementState::Pressed, false),
    );
    assert_eq!(
        clicks.get(),
        before + 1,
        "positive control: a real Space clicks"
    );
}

/// A packet types into the focused editor through its text path, not its
/// keymap: `"0"` (scan code 0x30, the B key's) under a held Ctrl inserts `0`
/// and leaves bold off, where it was `Mod-b` before. A real Ctrl+B is bold.
#[test]
fn a_packet_types_into_a_focused_editor_and_runs_no_binding() {
    let slot: Rc<std::cell::RefCell<Option<crate::editor::EditorHandle>>> =
        Rc::new(std::cell::RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>ab</p>");
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, rinch_editor_core::Pos(3))
        .expect("a caret");
    let button = Button::Left;
    let (x, y) = (x + 1.0, y + h / 2.0);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));

    let text = |handle: &crate::editor::EditorHandle| -> String {
        let doc = handle.doc();
        let para = doc.child(0);
        (0..para.child_count())
            .filter_map(|i| para.child(i).text().map(str::to_string))
            .collect()
    };
    tap(
        &mut app,
        ctrl(),
        packet(
            PhysicalKey::Code(WK::KeyB),
            "0",
            ElementState::Pressed,
            true,
        ),
    );
    assert_eq!(text(&handle), "ab0", "the packet's text, typed");
    assert!(!handle.is_mark_active("bold"), "and no Mod-b");

    tap(
        &mut app,
        Modifiers::default(),
        packet(
            PhysicalKey::Unidentified(NativeKeyCode::Windows(0)),
            "ü",
            ElementState::Pressed,
            false,
        ),
    );
    assert_eq!(text(&handle), "ab0ü", "a packet with no physical key");

    // Positive control: the real Ctrl+B is the binding.
    tap(
        &mut app,
        ctrl(),
        real(WK::KeyB, "b", ElementState::Pressed, true),
    );
    assert!(handle.is_mark_active("bold"), "a real Ctrl+B turns bold on");
}

/// The shell's answer for each half of a packet tap, directly: the press is
/// `KeyDown { Other, text }` with no modifiers, the release nothing at all —
/// not a `KeyUp` of the key its scan code names.
#[test]
fn the_shell_routes_a_packet_press_as_text_and_its_release_as_nothing() {
    let (app, _input, _button, _clicks) = page();
    let down = packet(
        PhysicalKey::Code(WK::Space),
        "9",
        ElementState::Pressed,
        true,
    );
    match RinchRuntime::route_key_event(&app, &down, ctrl(), false) {
        KeyRoute::Event(PlatformEvent::KeyDown {
            key: KeyCode::Other,
            logical_key: None,
            text: Some(text),
            modifiers,
            repeat: KeyRepeat::Fresh,
        }) => {
            assert_eq!(text, "9");
            assert_eq!(modifiers, Modifiers::default());
        }
        other => panic!("expected the packet's text, got {other:?}"),
    }
    let up = packet(
        PhysicalKey::Code(WK::Space),
        "9",
        ElementState::Released,
        true,
    );
    assert!(matches!(
        RinchRuntime::route_key_event(&app, &up, ctrl(), false),
        KeyRoute::Nothing
    ));
}

/// A character outside the BMP typed by `SendInput` (#2850): Windows sends
/// U+1F600 as two `VK_PACKET` taps, one per UTF-16 surrogate. The pinned
/// winit fork (joeleaver/winit-fork#14) holds the high surrogate and reports
/// the first press with no text, the second with the whole character. Both
/// reach the field as one emoji, with and without a held Ctrl (where `text`
/// is empty and the character is in `text_with_all_modifiers`), and the
/// textless first half types nothing.
#[test]
fn an_emoji_from_two_surrogate_packets_reaches_a_focused_input() {
    let (mut app, input, _button, _clicks) = page();
    click(&mut app, input);
    assert_eq!(app.focus_target, FocusTarget::Input(input));
    let unplaced = PhysicalKey::Unidentified(NativeKeyCode::Windows(0)); // as measured

    let half = |mut ev: KeyEvent| {
        ev.text = None;
        ev.text_with_all_modifiers = None;
        ev
    };
    tap(
        &mut app,
        Modifiers::default(),
        half(packet(unplaced, "", ElementState::Pressed, false)),
    );
    assert_eq!(
        value(&app, input),
        "ab",
        "the high-surrogate press types nothing"
    );
    tap(
        &mut app,
        Modifiers::default(),
        packet(unplaced, "\u{1F600}", ElementState::Pressed, false),
    );
    assert_eq!(value(&app, input), "ab\u{1F600}");

    // Under a push-to-talk Ctrl, the character comes from
    // `text_with_all_modifiers`.
    tap(
        &mut app,
        ctrl(),
        half(packet(unplaced, "", ElementState::Pressed, true)),
    );
    tap(
        &mut app,
        ctrl(),
        packet(unplaced, "\u{1F44D}", ElementState::Pressed, true),
    );
    assert_eq!(value(&app, input), "ab\u{1F600}\u{1F44D}");
}
