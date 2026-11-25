//! The layout table behind the web AltGr rule (`key_layout`).

use super::*;

/// A press of `code` reporting `key`, no modifier held.
fn plain<'a>(code: &'a str, key: &'a str) -> Press<'a> {
    Press {
        code,
        key,
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
        alt_graph: false,
        composing: false,
        caps_lock: false,
    }
}

/// An AltGr press as Windows reports it: Ctrl+Alt with `AltGraph` held.
fn alt_gr<'a>(code: &'a str, key: &'a str) -> Press<'a> {
    Press {
        ctrl: true,
        alt: true,
        alt_graph: true,
        ..plain(code, key)
    }
}

fn shifted(press: Press<'_>) -> Press<'_> {
    Press {
        shift: true,
        ..press
    }
}

/// What desktop answers for the same physical press, given the text the
/// platform says it typed (`None`: nothing) and `key_without_modifiers`.
fn desktop(press: &Press, text: Option<&str>, unmodified: &str) -> bool {
    rinch::platform::is_alt_graph_text(
        true,
        rinch::platform::Modifiers {
            shift: press.shift,
            ctrl: press.ctrl,
            alt: press.alt,
            meta: press.meta,
        },
        text,
        Some(unmodified),
    )
}

#[test]
fn a_seeded_entry_is_used_before_any_press() {
    let mut layout = KeyLayout::default();
    assert_eq!(
        layout.base("Digit1", false).as_deref(),
        Some("1"),
        "US fallback"
    );
    // French: `Digit1` types `&`.
    layout.seed("Digit1", "&");
    assert_eq!(layout.base("Digit1", false).as_deref(), Some("&"));
    // So French AltGr+1 reporting the key's own `&` typed nothing: the chord.
    assert!(!layout.is_alt_graph_text(&alt_gr("Digit1", "&")));
}

#[test]
fn a_seed_never_replaces_a_learned_entry() {
    let mut layout = KeyLayout::default();
    layout.learn(&plain("KeyQ", "a"));
    layout.seed("KeyQ", "q");
    assert_eq!(layout.base("KeyQ", false).as_deref(), Some("a"));
}

#[test]
fn a_press_replaces_a_seed() {
    // The layout map was read before the user switched layouts.
    let mut layout = KeyLayout::default();
    layout.seed("KeyQ", "q");
    layout.learn(&plain("KeyQ", "a"));
    assert_eq!(layout.base("KeyQ", false).as_deref(), Some("a"));
}

#[test]
fn a_seed_that_is_not_one_character_is_ignored() {
    let mut layout = KeyLayout::default();
    layout.seed("Digit1", "Dead");
    layout.seed("", "x");
    assert_eq!(layout.base("Digit1", false).as_deref(), Some("1"));
    assert_eq!(known_or_none(&layout, ""), None);
}

fn known_or_none(layout: &KeyLayout, code: &str) -> Option<String> {
    layout.plain.get(code).cloned()
}

#[test]
fn a_composing_press_teaches_nothing() {
    let mut layout = KeyLayout::default();
    layout.learn(&Press {
        composing: true,
        ..plain("Digit1", "ぬ")
    });
    assert_eq!(known_or_none(&layout, "Digit1"), None);
    // The positive control: the same press, not composing, is learned.
    layout.learn(&plain("Digit1", "ぬ"));
    assert_eq!(known_or_none(&layout, "Digit1").as_deref(), Some("ぬ"));
}

#[test]
fn a_modified_press_teaches_nothing() {
    let mut layout = KeyLayout::default();
    for press in [
        alt_gr("Digit7", "{"),
        Press {
            ctrl: true,
            ..plain("Digit7", "7")
        },
        Press {
            meta: true,
            ..plain("Digit7", "7")
        },
    ] {
        layout.learn(&press);
    }
    assert_eq!(known_or_none(&layout, "Digit7"), None);
}

#[test]
fn shift_alone_teaches_the_shift_level() {
    let mut layout = KeyLayout::default();
    // German Shift+2 types `"` (US: `@`).
    layout.learn(&shifted(plain("Digit2", "\"")));
    assert_eq!(layout.base("Digit2", true).as_deref(), Some("\""));
    assert_eq!(layout.base("Digit2", false).as_deref(), Some("2"));
}

/// German AltGr+Shift+1 types nothing; Chrome on Windows then reports `key`
/// `"!"`, the key's Shift character. Desktop's `text` is empty for that
/// press, so it is the chord there; it is the chord here too, before any
/// press taught the Shift level (the US fallback) and after.
#[test]
fn alt_gr_shift_reporting_the_shift_character_is_the_chord_as_on_desktop() {
    let press = shifted(alt_gr("Digit1", "!"));
    assert!(!desktop(&press, None, "1"), "desktop: the chord");
    let mut layout = KeyLayout::default();
    assert!(!layout.is_alt_graph_text(&press), "fallback: the chord");
    layout.learn(&shifted(plain("Digit1", "!")));
    assert!(!layout.is_alt_graph_text(&press), "learned: the chord");
    // Without the Shift level the comparison was against `"1"`, and `"!"`
    // read as AltGr text: the chord was dropped.
    let unshifted_base = layout.base("Digit1", false);
    assert!(rinch::platform::is_alt_graph_text(
        true,
        rinch::platform::Modifiers {
            shift: true,
            ctrl: true,
            alt: true,
            meta: false,
        },
        Some("!"),
        unshifted_base.as_deref(),
    ));
}

/// An AltGr+Shift press that does type a character is text, on both
/// backends: Polish AltGr+Shift+A types `Ą`.
#[test]
fn alt_gr_shift_typing_a_character_is_text() {
    let press = shifted(alt_gr("KeyA", "Ą"));
    assert!(desktop(&press, Some("Ą"), "a"));
    assert!(KeyLayout::default().is_alt_graph_text(&press));
    // A letter's Shift level is its capital: AltGr+Shift+K reporting `K`
    // typed nothing new.
    assert!(!KeyLayout::default().is_alt_graph_text(&shifted(alt_gr("KeyK", "K"))));
}

/// Lithuanian: the digit row types `ą č ę ė į š ų ū „` and AltGr+N types N.
/// Until a plain press of `Digit1` teaches the page, the US fallback reads
/// AltGr+1 (`"1"`) as the chord. Once any plain digit-row press has shown the
/// row is not digits, the fallback is dropped for the whole row and the press
/// is text: the safer error (see the module docs).
#[test]
fn a_non_digit_row_drops_the_digit_fallback() {
    let mut layout = KeyLayout::default();
    assert!(
        !layout.is_alt_graph_text(&alt_gr("Digit1", "1")),
        "unknown layout: the US fallback, the chord"
    );
    layout.learn(&plain("Digit2", "č"));
    assert_eq!(layout.base("Digit1", false), None);
    assert!(
        layout.is_alt_graph_text(&alt_gr("Digit1", "1")),
        "Lithuanian AltGr+1 types `1`: text"
    );
    assert!(layout.is_alt_graph_text(&shifted(alt_gr("Digit1", "!"))));
    // A learned digit key still answers for itself.
    layout.learn(&plain("Digit1", "ą"));
    assert!(layout.is_alt_graph_text(&alt_gr("Digit1", "1")));
    assert!(!layout.is_alt_graph_text(&alt_gr("Digit1", "ą")));
}

/// The fallback stays for a layout whose digit row is digits: German and
/// Polish right-Alt+1 report `"1"` and are the chord.
#[test]
fn a_digit_row_of_digits_keeps_the_fallback() {
    let mut layout = KeyLayout::default();
    layout.learn(&plain("Digit2", "2"));
    layout.learn(&plain("KeyZ", "y"));
    assert!(!layout.is_alt_graph_text(&alt_gr("Digit1", "1")));
    // German AltGr+7 types `{`: text.
    assert!(layout.is_alt_graph_text(&alt_gr("Digit7", "{")));
}

#[test]
fn a_key_name_is_never_alt_graph_text() {
    let layout = KeyLayout::default();
    assert!(!layout.is_alt_graph_text(&alt_gr("ArrowLeft", "ArrowLeft")));
    assert!(!layout.is_alt_graph_text(&alt_gr("Digit1", "Dead")));
}

/// Windows US-International: `'` is a dead key, and `'` then `a` types `á`,
/// which the `KeyA` keydown reports as its `key` (not composing). AltGr+A
/// types `á` too, so learning `á` as what `KeyA` types would read that AltGr
/// press as typing nothing new: the app's Mod-Alt-A chord would take it and
/// the character would be lost. The press after a dead key teaches nothing,
/// and a dead key itself is never learned.
#[test]
fn the_press_after_a_dead_key_teaches_nothing() {
    let mut layout = KeyLayout::default();
    layout.learn(&plain("Quote", "Dead"));
    layout.learn(&plain("KeyA", "á"));
    assert_eq!(known_or_none(&layout, "KeyA"), None);
    assert_eq!(known_or_none(&layout, "Quote"), None, "a dead key itself");
    assert!(
        layout.is_alt_graph_text(&alt_gr("KeyA", "á")),
        "US-International AltGr+A types `á`: text, not the chord"
    );
    // Only the one press: the key's next plain press is learned (the
    // positive control).
    layout.learn(&plain("KeyA", "a"));
    assert_eq!(known_or_none(&layout, "KeyA").as_deref(), Some("a"));
}

/// A modifier's own keydown between the dead key and the key it composes
/// with (Shift, for `'` then Shift+A, `Á`) leaves the dead key pending.
#[test]
fn a_dead_key_stays_pending_across_a_modifier_keydown() {
    let mut layout = KeyLayout::default();
    layout.learn(&plain("Quote", "Dead"));
    layout.learn(&shifted(plain("ShiftLeft", "Shift")));
    layout.learn(&shifted(plain("KeyA", "Á")));
    assert_eq!(layout.shifted.get("KeyA"), None);
    // The positive control: with no dead key before it, the same press
    // teaches the Shift level.
    layout.learn(&shifted(plain("KeyA", "Á")));
    assert_eq!(layout.shifted.get("KeyA").map(String::as_str), Some("Á"));
}

/// A dead circumflex (Shift+6 on US-International) then `2` composes `²`,
/// reported as the `Digit2` press's `key`. Learned, it would say this
/// layout's digit row is not digits and drop the digit fallback: German and
/// Polish right-Alt+1, which report `"1"`, would stop being the chord.
#[test]
fn a_dead_key_composition_on_the_digit_row_keeps_the_digit_fallback() {
    let mut layout = KeyLayout::default();
    layout.learn(&shifted(plain("Digit6", "Dead")));
    layout.learn(&plain("Digit2", "²"));
    assert_eq!(known_or_none(&layout, "Digit2"), None);
    assert_eq!(layout.base("Digit1", false).as_deref(), Some("1"));
    assert!(
        !layout.is_alt_graph_text(&alt_gr("Digit1", "1")),
        "the chord"
    );
}

/// CapsLock changes what some keys type: on French AZERTY it makes the digit
/// row type digits (and Shift then gives the symbols). A press with CapsLock
/// on would teach the digits over the seeded `&`, and the symbols as the
/// Shift level, so it teaches nothing.
#[test]
fn a_press_with_caps_lock_on_teaches_nothing() {
    let mut layout = KeyLayout::default();
    layout.seed("Digit1", "&");
    let caps = |press: Press<'static>| Press {
        caps_lock: true,
        ..press
    };
    layout.learn(&caps(plain("Digit1", "1")));
    layout.learn(&caps(shifted(plain("Digit1", "&"))));
    assert_eq!(layout.base("Digit1", false).as_deref(), Some("&"));
    assert_eq!(layout.shifted.get("Digit1"), None);
    // So French AltGr+1, reporting the key's own `&`, stays the chord.
    assert!(!layout.is_alt_graph_text(&alt_gr("Digit1", "&")));
    // The positive control: the same plain press, CapsLock off, is learned.
    layout.learn(&plain("Digit1", "1"));
    assert_eq!(known_or_none(&layout, "Digit1").as_deref(), Some("1"));
}

/// A layout map asked before `__forget_key_layout` that resolves after it
/// seeds nothing: the forget started a new generation.
#[test]
fn a_layout_map_from_before_a_forget_seeds_nothing() {
    let mut layout = KeyLayout::default();
    let asked = layout.generation();
    layout.forget();
    layout.seed_from_map(asked, "KeyQ", "a");
    assert_eq!(known_or_none(&layout, "KeyQ"), None);
    // The positive control: a map asked in this generation seeds.
    let asked = layout.generation();
    layout.seed_from_map(asked, "KeyQ", "a");
    assert_eq!(known_or_none(&layout, "KeyQ").as_deref(), Some("a"));
}

/// A forget also drops a pending dead key: a test's first press is learned.
#[test]
fn a_forget_drops_a_pending_dead_key() {
    let mut layout = KeyLayout::default();
    layout.learn(&plain("Quote", "Dead"));
    layout.forget();
    layout.learn(&plain("KeyA", "a"));
    assert_eq!(known_or_none(&layout, "KeyA").as_deref(), Some("a"));
}
