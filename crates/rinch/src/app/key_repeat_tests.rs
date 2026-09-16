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
/// answers `Unknown` — and on Android, which translates no `KeyAction::Up` at
/// all (issue #479), it used to be the *only* thing that ever cleared the
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
