use super::*;
use winit::keyboard::{Key, NamedKey, SmolStr};

fn spell(key: Key) -> Option<String> {
    RinchRuntime::winit_logical_key_str(&key)
}

/// A character key is passed through verbatim — case included, because a
/// browser's `KeyboardEvent.key` is case-accurate and a lowercased source
/// made `Shift+A` release as `"a"` after pressing as `"A"` (issue #337's
/// review finding). Non-letters ride the same arm now: `Shift+1` on a US
/// layout is `"!"` down *and* up, where the old single-ASCII-letter filter
/// dropped it and the release fell back to the physical `"1"`.
#[test]
fn a_character_key_is_verbatim_case_included() {
    assert_eq!(
        spell(Key::Character(SmolStr::new("a"))).as_deref(),
        Some("a")
    );
    assert_eq!(
        spell(Key::Character(SmolStr::new("A"))).as_deref(),
        Some("A"),
        "Shift+A is \"A\" in a browser — no lowercasing at the source"
    );
    assert_eq!(
        spell(Key::Character(SmolStr::new("!"))).as_deref(),
        Some("!")
    );
    assert_eq!(
        spell(Key::Character(SmolStr::new("é"))).as_deref(),
        Some("é")
    );
    assert_eq!(
        spell(Key::Character(SmolStr::new(" "))).as_deref(),
        Some(" "),
        "the spacebar's key value is the space character, per spec"
    );
}

/// A named key spells its W3C key-values name — `NamedKey` *is*
/// `keyboard_types::NamedKey` and its `Display` is the spec string, so
/// this pins the transcription, not a hand-kept table.
#[test]
fn a_named_key_spells_its_w3c_name() {
    assert_eq!(spell(Key::Named(NamedKey::Enter)).as_deref(), Some("Enter"));
    assert_eq!(
        spell(Key::Named(NamedKey::ArrowLeft)).as_deref(),
        Some("ArrowLeft")
    );
    assert_eq!(spell(Key::Named(NamedKey::Shift)).as_deref(), Some("Shift"));
    assert_eq!(
        spell(Key::Named(NamedKey::Meta)).as_deref(),
        Some("Meta"),
        "the OS key: winit folds X11's Super into Meta the way browsers do"
    );
}

/// A dead key is `"Dead"` (every browser's value for one); an unidentified
/// key is `None`, so the consumer falls back to the physical `key` instead
/// of pairing two different unknown keys by a shared placeholder.
#[test]
fn dead_spells_dead_and_unidentified_spells_nothing() {
    assert_eq!(spell(Key::Dead(Some('^'))).as_deref(), Some("Dead"));
    assert_eq!(spell(Key::Dead(None)).as_deref(), Some("Dead"));
    assert_eq!(
        spell(Key::Unidentified(winit::keyboard::NativeKey::Unidentified)),
        None
    );
}
