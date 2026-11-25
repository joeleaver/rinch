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
