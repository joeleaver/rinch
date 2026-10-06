//! A hardware key event on Android, translated for the shell.
//!
//! `android_runtime` compiles only for `target_os = "android"` and no host
//! test compiles it, so the decision of **what a key event becomes** lives
//! here, over plain data, and is compiled for the host test build too. The run
//! loop keeps only what needs the device: reading the event's fields and
//! asking the key character map.
//!
//! Three rules, each of which was once wrong or absent (issue #479):
//!
//! **A release is an event.** Android reports a key going up as
//! `ACTION_UP`; the loop used to translate `ACTION_DOWN` alone, so no
//! [`PlatformEvent::KeyUp`] existed on this backend and a document-level
//! interceptor or a focused node's `on_key` saw a press with no release.
//!
//! **A release is spelled as its press was.** `logical_key` is the string an
//! app pairs a press with its release by. Resolving the release through the
//! key character map again would spell it from the modifiers held *at the
//! release* and without the dead key the press combined with: Shift+A released
//! Shift first would go down as `"A"` and come up as `"a"`, and a dead `´` then
//! `e` would go down as `"é"` and come up as `"e"`. So the translator remembers
//! what each held key's press was spelled as, and the release takes that. A
//! release whose press it never saw (a key already held when the app came to
//! the front) falls back to the map.
//!
//! **A release leaves the pending dead key alone.** The combining accent is
//! armed by the dead key's *press* and consumed by the *next press*. The dead
//! key's own release arrives between the two, and so can the release of any
//! key held from before; neither is the character the accent is waiting for.
//!
//! The soft keyboard does not come through here at all: it speaks
//! `InputConnection`, which is `android_ime`'s.

use std::collections::HashMap;

use rinch_platform::{KeyCode, KeyRepeat, Modifiers, PlatformEvent};

/// Which half of a keystroke an event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyPhase {
    /// `ACTION_DOWN`. Android reports auto-repeat as further downs with a
    /// rising repeat count, so `0` is the press itself.
    Down { repeat_count: i32 },
    /// `ACTION_UP`.
    Up,
}

/// What the device's key character map answers for a key under the event's
/// meta state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapChar {
    /// The key types this character.
    Unicode(char),
    /// The key is a dead key carrying this accent.
    CombiningAccent(char),
    /// The key types nothing (Enter, an arrow, a modifier), or there is no
    /// map to ask.
    None,
}

/// One hardware key event, as plain data.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RawKey {
    /// Which physical key on which device: `(device id, Android key code)`.
    /// Only ever compared for equality, to find a release's press.
    pub id: (i32, u32),
    pub phase: KeyPhase,
    /// The rinch key code the Android key code maps to, if it has one.
    pub key: Option<KeyCode>,
    /// The key character map's answer for this event.
    pub ch: MapChar,
    pub modifiers: Modifiers,
}

/// Hardware key state that has to survive between events.
#[derive(Debug, Default)]
pub(crate) struct KeyTranslator {
    /// The accent of a dead key that has been pressed and not yet combined.
    combining_accent: Option<char>,
    /// The `logical_key` each held key's press was reported with, by
    /// [`RawKey::id`]. An entry is written by a press that produced an event
    /// and taken by that key's release. One left behind by a release that
    /// never arrived is replaced by the key's next press, so it is bounded by
    /// the number of keys and cannot misspell a later keystroke.
    held: HashMap<(i32, u32), Option<String>>,
}

impl KeyTranslator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Translate one key event. `dead_char(accent, ch)` is the key character
    /// map's combination of a pending accent with the character just typed
    /// (`None` when the two do not combine); it is called only for a press
    /// that has an accent waiting.
    ///
    /// `None` means the event is nothing rinch can name: its key code has no
    /// [`KeyCode`] and it types no character. A press and its release are
    /// dropped or delivered together.
    pub(crate) fn translate(
        &mut self,
        raw: RawKey,
        dead_char: impl FnOnce(char, char) -> Option<char>,
    ) -> Option<PlatformEvent> {
        match raw.phase {
            KeyPhase::Down { repeat_count } => {
                let text = match raw.ch {
                    MapChar::Unicode(ch) => Some(match self.combining_accent.take() {
                        Some(accent) => dead_char(accent, ch).unwrap_or(ch).to_string(),
                        None => ch.to_string(),
                    }),
                    MapChar::CombiningAccent(accent) => {
                        self.combining_accent = Some(accent);
                        None
                    }
                    MapChar::None => None,
                };
                // Android says which downs are repeats, so the runtime never
                // has to infer it from a release (issue #463).
                let repeat = if repeat_count > 0 {
                    KeyRepeat::Repeat
                } else {
                    KeyRepeat::Fresh
                };
                // The key-character-map char doubles as the logical key
                // value: it is the layout-produced, case-accurate (the meta
                // state includes Shift) character — what `KeyboardEvent.key`
                // spells for a printable key. Android hands us no DOM-style
                // *name* for the rest (Enter, arrows, CapsLock), so those stay
                // `None` and resolve through the physical `key`.
                let key = match (raw.key, &text) {
                    (Some(key), _) => key,
                    (None, Some(_)) => KeyCode::Other,
                    (None, None) => return None,
                };
                // The release is spelled as the press was. A repeat of a key
                // that combined with a dead key types the bare character (the
                // accent is spent), so it does not respell the keystroke.
                if repeat == KeyRepeat::Fresh || !self.held.contains_key(&raw.id) {
                    self.held.insert(raw.id, text.clone());
                }
                Some(PlatformEvent::KeyDown {
                    key,
                    logical_key: text.clone(),
                    text,
                    modifiers: raw.modifiers,
                    repeat,
                })
            }
            // HEAD's behaviour, extracted unchanged: a release is not translated.
            KeyPhase::Up => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: i32 = 7;
    // Android key codes, as `AKEYCODE_*` numbers them.
    const A: u32 = 29;
    const E: u32 = 33;
    const ENTER: u32 = 66;
    const SHIFT_LEFT: u32 = 59;
    /// A key code rinch has no `KeyCode` for.
    const UNMAPPED: u32 = 9001;

    const SHIFT: Modifiers = Modifiers {
        ctrl: false,
        shift: true,
        alt: false,
        meta: false,
    };

    /// The combination a real key character map makes of an acute accent and
    /// a vowel; everything else does not combine.
    fn dead(accent: char, ch: char) -> Option<char> {
        match (accent, ch) {
            ('\u{b4}', 'e') => Some('é'),
            ('\u{b4}', 'a') => Some('á'),
            _ => None,
        }
    }

    fn raw(
        code: u32,
        phase: KeyPhase,
        key: Option<KeyCode>,
        ch: MapChar,
        modifiers: Modifiers,
    ) -> RawKey {
        RawKey {
            id: (DEVICE, code),
            phase,
            key,
            ch,
            modifiers,
        }
    }

    fn down(code: u32, key: Option<KeyCode>, ch: MapChar) -> RawKey {
        raw(
            code,
            KeyPhase::Down { repeat_count: 0 },
            key,
            ch,
            Modifiers::default(),
        )
    }

    fn up(code: u32, key: Option<KeyCode>, ch: MapChar) -> RawKey {
        raw(code, KeyPhase::Up, key, ch, Modifiers::default())
    }

    /// `(key, logical_key, text, repeat)` of a press; panics on anything else.
    fn pressed(
        event: Option<PlatformEvent>,
    ) -> (KeyCode, Option<String>, Option<String>, KeyRepeat) {
        match event {
            Some(PlatformEvent::KeyDown {
                key,
                logical_key,
                text,
                repeat,
                ..
            }) => (key, logical_key, text, repeat),
            other => panic!("expected a KeyDown, got {other:?}"),
        }
    }

    /// `(key, logical_key, modifiers)` of a release; panics on anything else.
    fn released(event: Option<PlatformEvent>) -> (KeyCode, Option<String>, Modifiers) {
        match event {
            Some(PlatformEvent::KeyUp {
                key,
                logical_key,
                modifiers,
            }) => (key, logical_key, modifiers),
            other => panic!("expected a KeyUp, got {other:?}"),
        }
    }

    #[test]
    fn a_release_is_translated_into_a_key_up() {
        let mut t = KeyTranslator::new();
        let (key, logical, text, repeat) =
            pressed(t.translate(down(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(key, KeyCode::KeyA);
        assert_eq!(logical.as_deref(), Some("a"));
        assert_eq!(text.as_deref(), Some("a"));
        assert_eq!(repeat, KeyRepeat::Fresh);

        let (key, logical, _) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(key, KeyCode::KeyA);
        assert_eq!(logical.as_deref(), Some("a"));
    }

    /// A key that types nothing is named by its physical key on both halves.
    #[test]
    fn a_named_keys_release_carries_its_key_code_and_no_logical_key() {
        let mut t = KeyTranslator::new();
        let (key, logical, text, _) =
            pressed(t.translate(down(ENTER, Some(KeyCode::Enter), MapChar::None), dead));
        assert_eq!((key, logical, text), (KeyCode::Enter, None, None));
        let (key, logical, _) =
            released(t.translate(up(ENTER, Some(KeyCode::Enter), MapChar::None), dead));
        assert_eq!((key, logical), (KeyCode::Enter, None));
    }

    /// The release reports the modifiers held at the release, not the press's.
    #[test]
    fn a_release_carries_the_modifiers_of_its_own_event() {
        let mut t = KeyTranslator::new();
        t.translate(
            raw(
                A,
                KeyPhase::Down { repeat_count: 0 },
                Some(KeyCode::KeyA),
                MapChar::Unicode('A'),
                SHIFT,
            ),
            dead,
        );
        let (_, _, modifiers) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(modifiers, Modifiers::default());
    }

    /// Shift+A with Shift let go first: the map now says `a` for the key, but
    /// the keystroke went down as `A` and has to come up as `A`.
    #[test]
    fn a_release_is_spelled_as_its_press_was_when_shift_went_up_first() {
        let mut t = KeyTranslator::new();
        t.translate(
            raw(
                SHIFT_LEFT,
                KeyPhase::Down { repeat_count: 0 },
                Some(KeyCode::ShiftLeft),
                MapChar::None,
                SHIFT,
            ),
            dead,
        );
        let (_, down_as, _, _) = pressed(t.translate(
            raw(
                A,
                KeyPhase::Down { repeat_count: 0 },
                Some(KeyCode::KeyA),
                MapChar::Unicode('A'),
                SHIFT,
            ),
            dead,
        ));
        released(t.translate(
            up(SHIFT_LEFT, Some(KeyCode::ShiftLeft), MapChar::None),
            dead,
        ));
        let (_, up_as, _) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(down_as.as_deref(), Some("A"));
        assert_eq!(up_as, down_as);
    }

    /// The whole dead-key sequence, in the order a keyboard sends it: dead key
    /// down, dead key **up**, `e` down, `e` up. The dead key's release sits
    /// between the accent being armed and the press that consumes it.
    #[test]
    fn a_dead_keys_release_leaves_the_accent_for_the_next_press() {
        const DEAD: u32 = 68; // AKEYCODE_GRAVE, a dead key on this layout
        let mut t = KeyTranslator::new();
        // Android's dead keys have no rinch `KeyCode`, and a dead key types
        // nothing, so neither half is an event — but the press arms the accent.
        assert!(
            t.translate(down(DEAD, None, MapChar::CombiningAccent('\u{b4}')), dead)
                .is_none()
        );
        assert!(
            t.translate(up(DEAD, None, MapChar::CombiningAccent('\u{b4}')), dead)
                .is_none()
        );

        let (_, down_as, text, _) =
            pressed(t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        assert_eq!(
            text.as_deref(),
            Some("é"),
            "the accent survived the release"
        );
        assert_eq!(down_as.as_deref(), Some("é"));

        // …and the `e` release is spelled as its press was, not as the map
        // spells the bare key.
        let (_, up_as, _) =
            released(t.translate(up(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        assert_eq!(up_as.as_deref(), Some("é"));
    }

    /// A key held from before the dead key, released after it: a release that
    /// *does* carry a character must not be combined with the accent either.
    #[test]
    fn a_printable_keys_release_does_not_take_the_pending_accent() {
        const DEAD: u32 = 68;
        let mut t = KeyTranslator::new();
        t.translate(down(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead);
        t.translate(down(DEAD, None, MapChar::CombiningAccent('\u{b4}')), dead);
        let (_, up_as, _) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(up_as.as_deref(), Some("a"), "the release is not accented");
        let (_, _, text, _) =
            pressed(t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        assert_eq!(text.as_deref(), Some("é"), "the accent was still pending");
    }

    /// A press consumes the accent exactly once, as before.
    #[test]
    fn the_accent_combines_with_one_press_only() {
        let mut t = KeyTranslator::new();
        t.translate(down(68, None, MapChar::CombiningAccent('\u{b4}')), dead);
        let (_, _, first, _) =
            pressed(t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        t.translate(up(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead);
        let (_, _, second, _) =
            pressed(t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        assert_eq!(first.as_deref(), Some("é"));
        assert_eq!(second.as_deref(), Some("e"));
    }

    /// An accent that does not combine with the character types the character.
    #[test]
    fn an_accent_that_does_not_combine_types_the_bare_character() {
        let mut t = KeyTranslator::new();
        t.translate(down(68, None, MapChar::CombiningAccent('\u{b4}')), dead);
        let (_, _, text, _) =
            pressed(t.translate(down(52, Some(KeyCode::KeyX), MapChar::Unicode('x')), dead));
        assert_eq!(text.as_deref(), Some("x"));
    }

    /// Held `e` after a dead key: the press is `é`, the auto-repeats type `e`
    /// (the accent is spent), and the one release pairs with the press.
    #[test]
    fn a_repeat_does_not_respell_the_release() {
        let mut t = KeyTranslator::new();
        t.translate(down(68, None, MapChar::CombiningAccent('\u{b4}')), dead);
        t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead);
        let (_, repeat_as, _, repeat) = pressed(t.translate(
            raw(
                E,
                KeyPhase::Down { repeat_count: 1 },
                Some(KeyCode::KeyE),
                MapChar::Unicode('e'),
                Modifiers::default(),
            ),
            dead,
        ));
        assert_eq!(repeat, KeyRepeat::Repeat);
        assert_eq!(repeat_as.as_deref(), Some("e"));
        let (_, up_as, _) =
            released(t.translate(up(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead));
        assert_eq!(up_as.as_deref(), Some("é"));
    }

    /// A press replaces whatever an earlier keystroke of the same key left
    /// behind, so a release that never arrived cannot misspell the next one.
    #[test]
    fn a_fresh_press_replaces_a_stranded_spelling() {
        let mut t = KeyTranslator::new();
        // Shift+A goes down; its release never arrives.
        t.translate(
            raw(
                A,
                KeyPhase::Down { repeat_count: 0 },
                Some(KeyCode::KeyA),
                MapChar::Unicode('A'),
                SHIFT,
            ),
            dead,
        );
        t.translate(down(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead);
        let (_, up_as, _) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!(up_as.as_deref(), Some("a"));
    }

    /// A key already held when the app came to the front: no press was seen,
    /// so the release is spelled from the map.
    #[test]
    fn a_release_with_no_press_seen_is_spelled_from_the_map() {
        let mut t = KeyTranslator::new();
        let (key, up_as, _) =
            released(t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead));
        assert_eq!((key, up_as.as_deref()), (KeyCode::KeyA, Some("a")));
    }

    /// The same key code on two devices is two keys.
    #[test]
    fn a_release_takes_the_spelling_of_its_own_devices_press() {
        let mut t = KeyTranslator::new();
        t.translate(
            RawKey {
                id: (1, A),
                phase: KeyPhase::Down { repeat_count: 0 },
                key: Some(KeyCode::KeyA),
                ch: MapChar::Unicode('A'),
                modifiers: SHIFT,
            },
            dead,
        );
        let (_, up_as, _) = released(t.translate(
            RawKey {
                id: (2, A),
                phase: KeyPhase::Up,
                key: Some(KeyCode::KeyA),
                ch: MapChar::Unicode('a'),
                modifiers: Modifiers::default(),
            },
            dead,
        ));
        assert_eq!(up_as.as_deref(), Some("a"));
    }

    /// A key rinch has no `KeyCode` for is delivered as `Other` when it types
    /// something — both halves — and not at all when it does not.
    #[test]
    fn an_unmapped_key_is_delivered_or_dropped_as_a_pair() {
        let mut t = KeyTranslator::new();
        let (key, down_as, _, _) =
            pressed(t.translate(down(UNMAPPED, None, MapChar::Unicode('ß')), dead));
        assert_eq!((key, down_as.as_deref()), (KeyCode::Other, Some("ß")));
        let (key, up_as, _) =
            released(t.translate(up(UNMAPPED, None, MapChar::Unicode('ß')), dead));
        assert_eq!((key, up_as.as_deref()), (KeyCode::Other, Some("ß")));

        assert!(
            t.translate(down(UNMAPPED, None, MapChar::None), dead)
                .is_none()
        );
        assert!(
            t.translate(up(UNMAPPED, None, MapChar::None), dead)
                .is_none()
        );
    }

    /// A release takes its entry with it: the translator holds nothing for a
    /// key that is up.
    #[test]
    fn a_release_forgets_the_press() {
        let mut t = KeyTranslator::new();
        t.translate(down(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead);
        t.translate(down(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead);
        assert_eq!(t.held.len(), 2);
        t.translate(up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')), dead);
        t.translate(up(E, Some(KeyCode::KeyE), MapChar::Unicode('e')), dead);
        assert!(t.held.is_empty());
    }

    /// End to end, through the shared runtime: what a focused node's `on_key`
    /// (issue #337) hears for a translated Shift+A whose Shift went up first.
    /// The app pairs a press with its release by `k.key`, so the two strings
    /// have to be one string.
    #[test]
    fn on_key_hears_a_translated_release_spelled_as_its_press() {
        use crate::app::RinchApp;
        use crate::focus_registry::{FocusEntry, register_focus_target};
        use rinch_core::dom::RenderScope;
        use std::cell::RefCell;
        use std::rc::Rc;

        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let log_in = log.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let div = scope.create_element("div");
            div.set_attribute("style", "width: 200px; height: 40px");
            div.set_attribute("tabindex", "0");
            register_focus_target(
                &div,
                FocusEntry::new().on_key({
                    let log = log_in.clone();
                    move |k| {
                        let phase = if k.is_up() { "up" } else { "down" };
                        log.borrow_mut().push(format!("{phase}:{}", k.key));
                        false
                    }
                }),
            );
            div
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);

        let mut t = KeyTranslator::new();
        let shift_a = raw(
            A,
            KeyPhase::Down { repeat_count: 0 },
            Some(KeyCode::KeyA),
            MapChar::Unicode('A'),
            SHIFT,
        );
        let events = [
            down(61, Some(KeyCode::Tab), MapChar::None), // focus the node
            up(61, Some(KeyCode::Tab), MapChar::None),
            shift_a,
            up(A, Some(KeyCode::KeyA), MapChar::Unicode('a')),
        ];
        for raw in events {
            let event = t
                .translate(raw, dead)
                .expect("every one of these is an event");
            app.handle_event(event, (800, 600), 1.0);
        }
        let heard: Vec<String> = log
            .borrow()
            .iter()
            .filter(|l| !l.ends_with(":Tab"))
            .cloned()
            .collect();
        assert_eq!(heard, ["down:A", "up:A"]);
    }
}
