//! Once per physical press, and what happens when the release never arrives
//! (issue #463).
//!
//! Enter/Space on a keyboard-focused `tabindex` node must activate once per
//! press, and OS auto-repeat delivers presses indistinguishable from fresh
//! ones. rinch used to tell them apart by *inferring*: latch the key on the way
//! down, clear it on the matching `KeyUp`. That is the shape #189 (a drag whose
//! `pointerup` was swallowed) and #315's IME half share — **armed by one event,
//! cleared only by a second that may never arrive** — and here it was fatal:
//! alt-tab while Space is held, or a window-manager grab, or a native menu
//! taking the keyboard, and that node's Space was dead for the rest of the
//! session.
//!
//! The cure is the one the class asks for, pushed one step further than a
//! second clearing condition: the press *carries* the answer
//! ([`KeyRepeat`]), so on a backend that reports it nothing has to be cleared
//! at all. The latch stays for [`KeyRepeat::Unknown`] — an embed host
//! hand-building events — with the `WindowFocus(false)` heal it has had since
//! #147 (untested until this file).

use super::*;
use std::cell::Cell;

/// A focusable `<div>` with a click handler, already holding the keyboard.
fn focused_node_app() -> (RinchApp, Rc<Cell<usize>>) {
    let clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let clicks_in = clicks.clone();
    let id: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let div = scope.create_element("div");
        div.set_attribute("style", "width: 200px; height: 40px");
        div.set_attribute("tabindex", "0");
        let rid = scope.register_handler({
            let clicks = clicks_in.clone();
            move || clicks.set(clicks.get() + 1)
        });
        div.set_attribute("data-rid", &rid.0.to_string());
        id_in.set(div.node_id().0);
        root.append_child(&div);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app.set_focus_target(FocusTarget::Node(id.get()));
    (app, clicks)
}

fn press(app: &mut RinchApp, key: KeyCode, repeat: KeyRepeat) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat,
        },
        (800, 600),
        1.0,
    );
}

fn release(app: &mut RinchApp, key: KeyCode) {
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
}

fn window_focus(app: &mut RinchApp, focused: bool) {
    app.handle_event(PlatformEvent::WindowFocus(focused), (800, 600), 1.0);
}

// ── 1. a backend that knows ─────────────────────────────────────────────────

/// **The defect.** The release is swallowed and the window never blurs — a
/// window-manager grab, a same-window native menu, an embed host that reports
/// no focus changes. The latch is therefore still armed when the user presses
/// again, and before #463 that press (and every press after it, for ever) did
/// nothing. A backend that reports `Fresh` activates anyway.
///
/// Kills the mutant `KeyRepeat::Fresh => self.node_activation_held != Some(key)`
/// — which is exactly the pre-#463 code — measured: `left: 1, right: 2`.
#[test]
fn a_fresh_press_activates_though_the_latch_is_stranded() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    assert_eq!(clicks.get(), 1);
    // …release lost, no blur…
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    assert_eq!(
        clicks.get(),
        2,
        "a press the OS calls fresh activates whatever a stale latch believes"
    );
}

/// The other half of the same answer: a held key still activates exactly once,
/// as it does in a browser. Kills `KeyRepeat::Repeat => true`.
#[test]
fn an_auto_repeat_never_reactivates() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    press(&mut app, KeyCode::Space, KeyRepeat::Repeat);
    press(&mut app, KeyCode::Space, KeyRepeat::Repeat);
    press(&mut app, KeyCode::Space, KeyRepeat::Repeat);
    assert_eq!(clicks.get(), 1, "auto-repeat must not re-activate");
    release(&mut app, KeyCode::Space);
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    assert_eq!(clicks.get(), 2, "the next physical press does");
}

/// `Repeat` is **authoritative**, not merely a second opinion — and this is the
/// fixture that says so, because
/// [`an_auto_repeat_never_reactivates`] sits on a fixed point where
/// "believe the backend" and "consult the latch" agree: the first press arms
/// the latch, so both answers suppress the repeats that follow.
///
/// Off that point: hold Space, alt-tab away and back. The blur heal clears the
/// latch (a window that lost the keyboard holds no key down) while the key is
/// genuinely *still held*, so the repeats that resume find an empty latch. Only
/// the flag on the event can refuse them.
///
/// **On Wayland that is the whole sequence; on X11 and Windows it is not.**
/// winit synthesises a press for every key already held when a window gains
/// focus, and builds it with `repeat: false` — `winit-x11`'s
/// `process_key_event(keycode, state, false)` — so those two platforms deliver
/// a `Fresh` press before any of the repeats below and the node activates a
/// second time with no new physical press. That is **not** something #463
/// introduced (the blur heal had already emptied the latch, so the same
/// synthetic press activated before it too) and this fixture is not the place
/// to fix it: nothing else kills the mutant named below, and rinch reads no
/// `is_synthetic` anywhere. Tracked as issue #800.
///
/// Kills `KeyRepeat::Repeat => self.node_activation_held != Some(key)`.
#[test]
fn a_repeat_that_outlives_the_blur_heal_still_does_not_reactivate() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    assert_eq!(clicks.get(), 1);
    window_focus(&mut app, false);
    window_focus(&mut app, true);
    // Space was never let go, so the OS resumes repeating it.
    press(&mut app, KeyCode::Space, KeyRepeat::Repeat);
    press(&mut app, KeyCode::Space, KeyRepeat::Repeat);
    assert_eq!(
        clicks.get(),
        1,
        "a key the OS says is still held must not re-activate on the way back"
    );
}

/// A backend may answer for some presses and not others — rinch's own debug/MCP
/// channel injects `Fresh` presses into a live desktop app, and an embed host
/// may forward winit's flag for hardware keys while synthesizing others. A
/// `Fresh` press therefore still arms the latch, so the `Unknown` that follows
/// is still recognised as the same held key.
///
/// Kills dropping `self.node_activation_held = Some(key)` from the activation.
#[test]
fn a_fresh_press_still_arms_the_latch_for_a_press_that_answers_unknown() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Fresh);
    press(&mut app, KeyCode::Space, KeyRepeat::Unknown);
    assert_eq!(
        clicks.get(),
        1,
        "the latch is still written on a fresh press"
    );
}

// ── 2. a backend that does not ──────────────────────────────────────────────

/// `Unknown` keeps the inference rinch has always used, so a host that never
/// fills the flag in behaves exactly as it did before.
///
/// Kills `KeyRepeat::Unknown => true`.
#[test]
fn an_unknown_backend_still_latches_and_still_releases() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
    press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
    press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
    assert_eq!(clicks.get(), 1, "indistinguishable presses activate once");
    release(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
    assert_eq!(clicks.get(), 2, "a fresh press after the release activates");
}

/// The `WindowFocus(false)` heal, which has been in the runtime since #147 and
/// was pinned by nothing. It is what bounds the damage on a backend that
/// answers `Unknown` — and on Android, which translated no `KeyAction::Up`
/// until issue #479, it used to be the *only* thing that ever cleared the
/// latch.
///
/// Kills deleting the `self.node_activation_held = None` in the
/// `PlatformEvent::WindowFocus` arm.
#[test]
fn a_window_blur_heals_an_unknown_backends_stranded_latch() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Unknown);
    assert_eq!(clicks.get(), 1);
    // The release goes to whatever took the keyboard, not to us.
    window_focus(&mut app, false);
    window_focus(&mut app, true);
    press(&mut app, KeyCode::Space, KeyRepeat::Unknown);
    assert_eq!(clicks.get(), 2, "the blur cleared the stranded latch");
}

/// Two keys, one latch slot. A press of the *other* activation key is a fresh
/// press by either route, and it takes the slot — so the first key is freed as
/// a side effect. Pinned because it is the behaviour, not because it is a
/// design: the slot is a fallback, and the flag is what a real backend uses.
#[test]
fn the_latch_holds_one_key_at_a_time() {
    let (mut app, clicks) = focused_node_app();
    press(&mut app, KeyCode::Space, KeyRepeat::Unknown);
    press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
    assert_eq!(clicks.get(), 2);
    assert_eq!(app.node_activation_held, Some(KeyCode::Enter));
}

// ── 3. the producers ────────────────────────────────────────────────────────

/// The debug/MCP channel is a producer that sends presses and **no releases at
/// all**, so before #463 a second `key_press("Enter")` on a focused node did
/// nothing — the first press armed the latch and nothing on that path could
/// ever clear it short of a window blur. It answers `KeyRepeat::Fresh`, which
/// is the truthful answer: the channel has no way to express a held key
/// (`key_press` is one discrete press per call, `type_text` one per character),
/// so nothing here can repeat where a real keyboard would not.
///
/// Kills either `debug_commands.rs` site reverting to `KeyRepeat::Unknown` —
/// both of which survived the whole 602-test suite when the review of PR #796
/// measured them, because every other fixture supplies its own `KeyRepeat`.
#[cfg(feature = "debug")]
#[test]
fn every_injected_press_activates_though_the_channel_sends_no_releases() {
    use rinch_debug::DebugCommandKind;

    let (mut app, clicks) = focused_node_app();
    let mut actions = Vec::new();
    for _ in 0..2 {
        app.execute_debug_command(
            DebugCommandKind::KeyPress {
                key: "Enter".into(),
                shift: false,
                ctrl: false,
                alt: false,
                modifiers: Vec::new(),
            },
            &mut actions,
            1.0,
            (800, 600),
        );
    }
    assert_eq!(clicks.get(), 2, "two `key_press` calls are two presses");

    // `type_text` is the second site, and it synthesizes one press per
    // character — so two typed spaces are two activations, not one.
    app.execute_debug_command(
        DebugCommandKind::TypeText { text: "  ".into() },
        &mut actions,
        1.0,
        (800, 600),
    );
    assert_eq!(clicks.get(), 4, "each typed Space is its own press");
}

// ── review of PR #1416 (#479): the Android translator's output, through RinchApp ──
mod review_1416 {
    use super::*;
    use crate::shell::android_key::{KeyPhase, KeyTranslator, MapChar, RawKey};

    fn raw(code: u32, phase: KeyPhase, key: Option<KeyCode>, ch: MapChar) -> RawKey {
        RawKey {
            id: (7, code),
            phase,
            key,
            ch,
            modifiers: Modifiers::default(),
        }
    }
    const DOWN: KeyPhase = KeyPhase::Down { repeat_count: 0 };

    /// Focus 4 of the brief: a translated Enter press arms the latch and the
    /// translated release clears it. Red when `KeyPhase::Up` translates to
    /// nothing (the pre-#479 behaviour): the `expect` fails.
    #[test]
    fn a_translated_release_clears_the_activation_latch() {
        let (mut app, clicks) = focused_node_app();
        let mut t = KeyTranslator::new();
        let ev = t
            .translate(
                raw(66, DOWN, Some(KeyCode::Enter), MapChar::None),
                |_, _| None,
            )
            .expect("press");
        app.handle_event(ev, (800, 600), 1.0);
        assert_eq!(clicks.get(), 1);
        assert_eq!(app.node_activation_held, Some(KeyCode::Enter));
        let ev = t
            .translate(
                raw(66, KeyPhase::Up, Some(KeyCode::Enter), MapChar::None),
                |_, _| None,
            )
            .expect("android translates the release");
        app.handle_event(ev, (800, 600), 1.0);
        assert_eq!(app.node_activation_held, None);
        // And an Unknown-backend press after it activates (the latch is what
        // that reads).
        press(&mut app, KeyCode::Enter, KeyRepeat::Unknown);
        assert_eq!(clicks.get(), 2);
    }

    /// FINDING (documents current behaviour): a stranded spelling survives a
    /// run of repeat-only downs. Shift+A down, its release lost; the key is
    /// later seen only as auto-repeats typing "a" (held while the window
    /// regained focus), then released. Every down since was "a"; the release
    /// says "A".
    #[test]
    fn finding_a_stranded_spelling_outlives_repeat_only_downs() {
        let mut t = KeyTranslator::new();
        t.translate(
            raw(29, DOWN, Some(KeyCode::KeyA), MapChar::Unicode('A')),
            |_, _| None,
        );
        // release lost
        for n in 1..4 {
            t.translate(
                raw(
                    29,
                    KeyPhase::Down { repeat_count: n },
                    Some(KeyCode::KeyA),
                    MapChar::Unicode('a'),
                ),
                |_, _| None,
            );
        }
        let up = t.translate(
            raw(29, KeyPhase::Up, Some(KeyCode::KeyA), MapChar::Unicode('a')),
            |_, _| None,
        );
        match up {
            Some(PlatformEvent::KeyUp { logical_key, .. }) => {
                assert_eq!(logical_key.as_deref(), Some("A"), "current behaviour")
            }
            other => panic!("{other:?}"),
        }
    }

    /// FINDING (documents current behaviour): what `on_key`-style pairing by
    /// string sees for dead-´ + held e. The repeat is a down spelled "e" that
    /// no release ever names.
    #[test]
    fn finding_a_repeat_after_a_dead_key_is_a_down_no_release_names() {
        let dead = |a: char, c: char| (a == '\u{b4}' && c == 'e').then_some('é');
        let mut t = KeyTranslator::new();
        let mut held: std::collections::BTreeSet<String> = Default::default();
        let seq = [
            raw(68, DOWN, None, MapChar::CombiningAccent('\u{b4}')),
            raw(68, KeyPhase::Up, None, MapChar::CombiningAccent('\u{b4}')),
            raw(33, DOWN, Some(KeyCode::KeyE), MapChar::Unicode('e')),
            raw(
                33,
                KeyPhase::Down { repeat_count: 1 },
                Some(KeyCode::KeyE),
                MapChar::Unicode('e'),
            ),
            raw(33, KeyPhase::Up, Some(KeyCode::KeyE), MapChar::Unicode('e')),
        ];
        for r in seq {
            match t.translate(r, dead) {
                Some(PlatformEvent::KeyDown { logical_key, .. }) => {
                    held.insert(logical_key.unwrap());
                }
                Some(PlatformEvent::KeyUp { logical_key, .. }) => {
                    held.remove(&logical_key.unwrap());
                }
                _ => {}
            }
        }
        assert_eq!(
            held.into_iter().collect::<Vec<_>>(),
            ["e"],
            "current behaviour: \"e\" looks held for ever"
        );
    }

    /// UNPINNED BRANCH (mutant M1 `if repeat == KeyRepeat::Fresh {` survives
    /// the PR's suite): a key first seen as an auto-repeat (held when the app
    /// came to the front) is remembered by that repeat, so its release pairs
    /// with the downs the app did see.
    #[test]
    fn a_key_first_seen_as_a_repeat_is_released_as_that_repeat_was_spelled() {
        let mut t = KeyTranslator::new();
        t.translate(
            raw(
                29,
                KeyPhase::Down { repeat_count: 5 },
                Some(KeyCode::KeyA),
                MapChar::Unicode('A'),
            ),
            |_, _| None,
        );
        match t.translate(
            raw(29, KeyPhase::Up, Some(KeyCode::KeyA), MapChar::Unicode('a')),
            |_, _| None,
        ) {
            Some(PlatformEvent::KeyUp { logical_key, .. }) => {
                assert_eq!(logical_key.as_deref(), Some("A"))
            }
            other => panic!("{other:?}"),
        }
    }

    /// UNPINNED BRANCH (mutants M2/M4 survive): a press that typed nothing is
    /// remembered as nothing. Its release is not respelled from the map when
    /// the modifier that silenced the press has gone up.
    #[test]
    fn a_press_that_typed_nothing_is_released_with_no_logical_key() {
        let mut t = KeyTranslator::new();
        t.translate(raw(29, DOWN, Some(KeyCode::KeyA), MapChar::None), |_, _| {
            None
        });
        match t.translate(
            raw(29, KeyPhase::Up, Some(KeyCode::KeyA), MapChar::Unicode('A')),
            |_, _| None,
        ) {
            Some(PlatformEvent::KeyUp { logical_key, .. }) => assert_eq!(logical_key, None),
            other => panic!("{other:?}"),
        }
    }
}
