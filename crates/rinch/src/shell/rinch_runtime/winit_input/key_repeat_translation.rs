use super::*;
use winit::keyboard::{Key, KeyCode as WK, SmolStr};

fn press(repeat: bool) -> PlatformEvent {
    // The spacebar's logical key is its character, not a named key — the
    // W3C spells it `" "`, and so does winit.
    RinchRuntime::key_down_from_winit(
        WK::Space,
        &Key::Character(SmolStr::new(" ")),
        Some(" "),
        repeat,
        Modifiers::default(),
    )
}

fn repeat_of(event: &PlatformEvent) -> KeyRepeat {
    match event {
        PlatformEvent::KeyDown { repeat, .. } => *repeat,
        other => panic!("expected a KeyDown, got {other:?}"),
    }
}

/// winit's `repeat` is documented `true` **iff** the OS produced this press
/// by holding the key, so the mapping is the identity and not a negation.
/// Getting it backwards would make desktop refuse every real press and
/// activate on every auto-repeat — #463 inverted, which is worse than #463.
#[test]
fn winits_flag_is_reported_not_inverted() {
    assert_eq!(repeat_of(&press(false)), KeyRepeat::Fresh);
    assert_eq!(repeat_of(&press(true)), KeyRepeat::Repeat);
}

/// And never `Unknown`: this backend always knows, so it must never fall
/// back to the latch — which is the whole of #463 on desktop. Kills a
/// shell that quietly answers `KeyRepeat::Unknown` (the `Default`, and so
/// the shape a careless refactor reaches for).
#[test]
fn the_desktop_shell_never_answers_unknown() {
    for r in [false, true] {
        assert_ne!(repeat_of(&press(r)), KeyRepeat::Unknown);
    }
}

/// The extraction moved the whole press translation, not just the flag, so
/// the rest of it is pinned here too — the physical key, the layout-mapped
/// `logical_key` and the inserted `text` all still ride along.
#[test]
fn the_rest_of_the_press_survives_the_split() {
    let PlatformEvent::KeyDown {
        key,
        logical_key,
        text,
        ..
    } = press(false)
    else {
        panic!("expected a KeyDown");
    };
    assert_eq!(key, KeyCode::Space);
    assert_eq!(logical_key.as_deref(), Some(" "));
    assert_eq!(text.as_deref(), Some(" "));
}
