use super::*;
use winit::event::KeyEvent;
use winit::keyboard::{
    Key, KeyCode as WK, KeyLocation, NativeKey, NativeKeyCode, PhysicalKey, SmolStr,
};

/// A `VK_PACKET` event for `ch` as winit-win32 finalizes it, with
/// `physical` from the lParam scan code and `text` as winit's `WM_CHAR`
/// handling leaves it (`None` under Ctrl).
pub(super) fn packet(
    physical: PhysicalKey,
    ch: &str,
    state: ElementState,
    ctrl_held: bool,
) -> KeyEvent {
    let pressed = state == ElementState::Pressed;
    let packet = Key::Unidentified(NativeKey::Windows(VK_PACKET));
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

fn text_of(event: Option<PlatformEvent>) -> Option<String> {
    match event? {
        PlatformEvent::KeyDown {
            key,
            logical_key,
            modifiers,
            repeat,
            text,
        } => {
            assert_eq!(key, KeyCode::Other, "no physical key typed it");
            assert_eq!(logical_key, None, "and no key value spells it");
            assert_eq!(
                modifiers,
                Modifiers::default(),
                "held modifiers are not its"
            );
            assert_eq!(repeat, KeyRepeat::Fresh);
            text
        }
        other => panic!("expected a KeyDown, got {other:?}"),
    }
}

/// The press is its text alone, whichever physical key the scan code
/// named: a real key (`" "` is scan code 0x20, the D key), the spacebar
/// (`"9"`, 0x39), or none.
#[test]
fn a_packet_press_is_its_text_whatever_physical_key_it_names() {
    for (physical, ch) in [
        (PhysicalKey::Code(WK::KeyD), " "),
        (PhysicalKey::Code(WK::Space), "9"),
        (PhysicalKey::Unidentified(NativeKeyCode::Windows(0)), "é"),
    ] {
        let event = packet(physical, ch, ElementState::Pressed, false);
        assert!(RinchRuntime::is_text_only_key(&event), "{ch:?}");
        assert_eq!(
            text_of(RinchRuntime::text_only_key_down(&event)).as_deref(),
            Some(ch),
            "{ch:?}"
        );
    }
}

/// Under a held Ctrl winit-win32 leaves `text` empty for a packet; the
/// text then comes from `text_with_all_modifiers`, and the press still
/// carries no Ctrl.
#[test]
fn a_packet_typed_under_ctrl_keeps_its_text() {
    let event = packet(
        PhysicalKey::Code(WK::KeyB),
        "0",
        ElementState::Pressed,
        true,
    );
    assert_eq!(event.text, None, "winit's Ctrl text for a packet is empty");
    assert_eq!(
        text_of(RinchRuntime::text_only_key_down(&event)).as_deref(),
        Some("0")
    );
}

/// The release, and a press whose text is a control character (a line
/// break or tab is a key, sent as one), are nothing.
#[test]
fn a_packet_release_and_control_text_are_nothing() {
    let up = packet(
        PhysicalKey::Code(WK::KeyD),
        " ",
        ElementState::Released,
        false,
    );
    assert!(RinchRuntime::is_text_only_key(&up));
    assert!(RinchRuntime::text_only_key_down(&up).is_none());
    let cr = packet(
        PhysicalKey::Code(WK::Enter),
        "\r",
        ElementState::Pressed,
        false,
    );
    assert!(RinchRuntime::text_only_key_down(&cr).is_none());
}

/// A key winit could not place but which types text (an exotic key on any
/// backend) is text too; one with no text is nothing, as it always was.
#[test]
fn an_unidentified_key_is_its_text_or_nothing() {
    let mut event = KeyEvent {
        physical_key: PhysicalKey::Unidentified(NativeKeyCode::Xkb(0x1ff)),
        logical_key: Key::Character(SmolStr::new("ß")),
        text: Some(SmolStr::new("ß")),
        location: KeyLocation::Standard,
        state: ElementState::Pressed,
        repeat: false,
        text_with_all_modifiers: Some(SmolStr::new("ß")),
        key_without_modifiers: Key::Character(SmolStr::new("ß")),
    };
    assert!(RinchRuntime::is_text_only_key(&event));
    assert_eq!(
        text_of(RinchRuntime::text_only_key_down(&event)).as_deref(),
        Some("ß")
    );
    event.text = None;
    event.text_with_all_modifiers = None;
    assert!(RinchRuntime::text_only_key_down(&event).is_none());
}

/// A real key stays a key: its physical key and logical key are its own,
/// so it takes the key path (`key_down_from_winit`) as before.
#[test]
fn a_real_key_is_not_text_only() {
    let event = KeyEvent {
        physical_key: PhysicalKey::Code(WK::KeyD),
        logical_key: Key::Character(SmolStr::new("d")),
        text: Some(SmolStr::new("d")),
        location: KeyLocation::Standard,
        state: ElementState::Pressed,
        repeat: false,
        text_with_all_modifiers: Some(SmolStr::new("d")),
        key_without_modifiers: Key::Character(SmolStr::new("d")),
    };
    assert!(!RinchRuntime::is_text_only_key(&event));
    let other_unidentified = KeyEvent {
        logical_key: Key::Unidentified(NativeKey::Windows(0xE6)),
        ..event
    };
    assert!(
        !RinchRuntime::is_text_only_key(&other_unidentified),
        "only VK_PACKET's logical key marks a packet"
    );
}
