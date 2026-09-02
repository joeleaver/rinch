//! The key data a document-level consumer receives is the key that was
//! actually pressed (issue #336).
//!
//! Two defects, one path. The `KeyDown` arm builds three `KeyEventData`-shaped
//! payloads — one for the interceptor, one for a focused render surface, one
//! for a focused node's `on_key` — and two of the three used to hardcode
//! `meta: false`, so an app could not tell `Cmd+K` from `K`. Separately,
//! `hook_key_str` only named the twelve Ctrl+letter combos rinch itself binds,
//! and a modifier suppresses the event's `text`, so every *other* chord
//! resolved to `None` and the interceptor was never invoked for it at all.

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use crate::render_surface::{SurfaceEvent, SurfaceKeyData, create_render_surface};
use rinch_core::Component;
use rinch_core::events::{KeyEventData, set_keyboard_interceptor};

fn meta_down() -> Modifiers {
    Modifiers {
        meta: true,
        ..Default::default()
    }
}

fn ctrl_down() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Default::default()
    }
}

/// A bare mounted app — no focusable content, so every key reaches the
/// interceptor and then falls through to the global handlers.
fn bare_app() -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let text = scope.create_text("hello");
        root.append_child(&text);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

/// A press that carries the layout letter, as winit supplies it — the shape a
/// chord has, where a modifier has suppressed `text`.
fn press_logical(
    app: &mut RinchApp,
    key: KeyCode,
    logical_key: Option<&str>,
    modifiers: Modifiers,
) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: logical_key.map(str::to_string),
            text: None,
            modifiers,
        },
        (800, 600),
        1.0,
    );
}

fn release(app: &mut RinchApp, key: KeyCode, logical_key: Option<&str>, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: logical_key.map(str::to_string),
            modifiers,
        },
        (800, 600),
        1.0,
    );
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    press_on_layout(app, key, text, None, modifiers);
}

/// The same press with the layout-mapped letter the desktop shell attaches to
/// a real `KeyDown` (`RinchRuntime::winit_logical_key_str`).
fn press_on_layout(
    app: &mut RinchApp,
    key: KeyCode,
    text: Option<&str>,
    logical_key: Option<&str>,
    modifiers: Modifiers,
) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: logical_key.map(str::to_string),
            text: text.map(str::to_string),
            modifiers,
        },
        (800, 600),
        1.0,
    );
}

/// Install an interceptor that records every event it is offered and consumes
/// none of them.
fn recording_interceptor() -> Rc<RefCell<Vec<KeyEventData>>> {
    let seen: Rc<RefCell<Vec<KeyEventData>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    set_keyboard_interceptor(move |data: &KeyEventData| {
        sink.borrow_mut().push(data.clone());
        false
    });
    seen
}

// ── 1. the meta modifier survives the trip ──────────────────────────────────

/// The reported defect: the interceptor's payload hardcoded `meta: false`, so
/// a Cmd chord on macOS (or a Super chord on Linux) was indistinguishable from
/// the bare key.
#[test]
fn the_interceptor_sees_the_meta_modifier() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::KeyK, None, meta_down());

    let seen = seen.borrow();
    let ev = seen.last().expect("the interceptor was offered the key");
    assert!(ev.meta, "Meta/Cmd was held: {ev:?}");
    assert_eq!(ev.key, "k");
    assert_eq!(ev.code, "KeyK");
}

/// The same payload with no Meta held must not claim one — the fix is "report
/// the modifier", not "report `true`".
#[test]
fn the_interceptor_reports_no_meta_when_none_is_held() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::KeyK, Some("k"), Modifiers::default());

    let seen = seen.borrow();
    let ev = seen.last().expect("the interceptor was offered the key");
    assert!(!ev.meta, "no modifier was held: {ev:?}");
}

/// The second hardcoded literal: a focused render surface's `KeyDown`. The
/// surface's `KeyUp` arm right beside it already passed `modifiers.meta`, so
/// the same path disagreed with itself about the same modifier.
#[test]
fn a_focused_render_surface_sees_the_meta_modifier() {
    let surface = create_render_surface();
    let surface_id = surface.id();
    let seen: Rc<RefCell<Vec<SurfaceKeyData>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    surface.set_event_handler(move |event| {
        if let SurfaceEvent::KeyDown(data) = event {
            sink.borrow_mut().push(data);
        }
    });

    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = crate::render_surface::RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        root.append_child(&child);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app.focus_target = FocusTarget::Surface(surface_id);

    press(&mut app, KeyCode::KeyK, None, meta_down());

    let seen = seen.borrow();
    let ev = seen.last().expect("the surface was forwarded the key");
    assert!(ev.meta, "Meta/Cmd was held: {ev:?}");
    assert_eq!(ev.key, "k", "and it is named, not empty: {ev:?}");
}

// ── 2. a chord reaches the interceptor at all ───────────────────────────────

/// `Ctrl+S` — the canonical "save" chord, and the one the issue names. A
/// modifier suppresses `text`, and `KeyS` was not among the twelve letters the
/// old table knew, so `hook_key_str` returned `None` and the `if let Some(ks)`
/// guard skipped the interceptor entirely. The key was unobservable.
#[test]
fn a_ctrl_chord_outside_the_bound_set_reaches_the_interceptor() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::KeyS, None, ctrl_down());

    let seen = seen.borrow();
    let ev = seen
        .last()
        .expect("Ctrl+S must reach the interceptor (issue #336)");
    assert_eq!(ev.key, "s");
    assert_eq!(ev.code, "KeyS");
    assert!(ev.ctrl);
}

/// Function keys insert no text either, so they were invisible for the same
/// reason. F5 is the one every app wants.
#[test]
fn a_function_key_reaches_the_interceptor() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::F5, None, Modifiers::default());

    let seen = seen.borrow();
    let ev = seen.last().expect("F5 must reach the interceptor");
    assert_eq!(ev.key, "F5");
    assert_eq!(ev.code, "F5");
}

/// An interceptor that consumes still consumes — the widened spelling table
/// must not change who wins, only what they are told.
#[test]
fn a_consuming_interceptor_still_swallows_the_key() {
    let mut app = bare_app();
    let hits = Rc::new(std::cell::Cell::new(0usize));
    let sink = hits.clone();
    set_keyboard_interceptor(move |_| {
        sink.set(sink.get() + 1);
        true
    });

    press(&mut app, KeyCode::F5, None, Modifiers::default());
    press(&mut app, KeyCode::KeyS, None, ctrl_down());

    assert_eq!(hits.get(), 2);
}

/// A chord on a non-QWERTY layout must name the **keycap**, not the physical
/// position under it. The editor's own keymap already reads `logical_key` for
/// exactly this reason, so an interceptor that read the physical table instead
/// would disagree with the editor about which letter the user just pressed —
/// and would still fail to see `Ctrl+A` by its own name on AZERTY, which is
/// the bug #336 is about.
#[test]
fn a_chord_on_a_non_qwerty_layout_names_the_keycap() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    // AZERTY: the key labelled A sits at the physical QWERTY-Q position, and
    // Ctrl suppresses the text.
    press_on_layout(&mut app, KeyCode::KeyQ, None, Some("a"), ctrl_down());

    let seen = seen.borrow();
    let ev = seen.last().expect("the interceptor was offered the chord");
    assert_eq!(
        ev.key, "a",
        "the keycap letter, not the physical one: {ev:?}"
    );
    assert_eq!(
        ev.code, "KeyQ",
        "the physical key is still reported as code"
    );
}

// ── 3. the third payload: a registered focus target ─────────────────────────

/// The `KeyDown` arm builds three payloads and only two carried the reported
/// defect, so the third — a registered node's `on_key` (issue #147) — had no
/// regression guard at all. Pin it too: a `meta: false` literal here would be
/// the same bug in the one place nothing was watching.
#[test]
fn a_registered_focus_target_sees_the_meta_modifier() {
    let seen: Rc<RefCell<Vec<KeyEventData>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let id: Rc<std::cell::Cell<usize>> = Rc::new(std::cell::Cell::new(0));
    let id_in = id.clone();

    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let div = scope.create_element("div");
        div.set_attribute("style", "width: 200px; height: 40px");
        div.set_attribute("tabindex", "0");
        let sink = sink.clone();
        register_focus_target(
            &div,
            FocusEntry::new().on_key(move |k| {
                sink.borrow_mut().push(k.clone());
                true
            }),
        );
        id_in.set(div.node_id().0);
        root.append_child(&div);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app.set_focus_target(FocusTarget::Node(id.get()));

    press(&mut app, KeyCode::KeyK, None, meta_down());

    let seen = seen.borrow();
    let ev = seen
        .last()
        .expect("the registered target was offered the key");
    assert!(ev.meta, "Meta/Cmd was held: {ev:?}");
    assert_eq!(ev.key, "k");
}

// ── 3. a release reaches the app at all (issue #337) ────────────────────────

/// The whole of #337. `KeyUp` cleared the activation latch and forwarded to a
/// focused render surface, and did nothing else — the document-level
/// interceptor and a focused node's `on_key` never saw a release. A consumer
/// could see a key go down and never see it come up.
#[test]
fn a_release_reaches_the_interceptor() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::KeyK, Some("k"), Modifiers::default());
    release(&mut app, KeyCode::KeyK, None, Modifiers::default());

    let seen = seen.borrow();
    assert_eq!(seen.len(), 2, "one press, one release: {seen:?}");
    assert!(seen[0].is_down());
    assert!(seen[1].is_up(), "the release is reported as one");
}

/// The reason the release carries `logical_key`. A consumer pairs a press with
/// its release by comparing `key` — "is W still held" is the use case `KeyUp`
/// primarily exists for. A release carries no `text`, so without the layout
/// letter it would resolve through the *physical* table while its press
/// resolved through the *layout* one: on AZERTY a press of `"a"` would come up
/// as `"q"`, the pairing would silently never match, and the key would look
/// held for ever.
#[test]
fn a_press_and_its_release_agree_on_a_non_qwerty_layout() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    // AZERTY: the key at the physical QWERTY-Q position types 'a'.
    press(&mut app, KeyCode::KeyQ, Some("a"), Modifiers::default());
    release(&mut app, KeyCode::KeyQ, Some("a"), Modifiers::default());

    let seen = seen.borrow();
    assert_eq!(seen[0].key, "a", "the press reports the keycap letter");
    assert_eq!(
        seen[1].key, seen[0].key,
        "and so does the release — by construction, not coincidence: {seen:?}"
    );
    assert_eq!(seen[1].code, seen[0].code, "the physical code matches too");
}

/// The same, under a modifier — where the press has no `text` either, so both
/// phases resolve through `logical_key`.
#[test]
fn a_chord_and_its_release_agree_too() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    let ctrl = Modifiers {
        ctrl: true,
        ..Default::default()
    };
    press_logical(&mut app, KeyCode::KeyQ, Some("a"), ctrl);
    release(&mut app, KeyCode::KeyQ, Some("a"), ctrl);

    let seen = seen.borrow();
    assert_eq!(seen[0].key, "a");
    assert_eq!(seen[1].key, "a");
    assert!(
        seen[0].ctrl && seen[1].ctrl,
        "and the modifier survives both"
    );
}

/// Without a layout letter — the debug channel, an injected or embedded event
/// — both phases fall to the physical table, so they still agree. The point is
/// that they agree *whatever* the source, not that one source is favoured.
#[test]
fn without_a_logical_key_both_phases_use_the_physical_table() {
    let mut app = bare_app();
    let seen = recording_interceptor();

    press(&mut app, KeyCode::KeyS, None, Modifiers::default());
    release(&mut app, KeyCode::KeyS, None, Modifiers::default());

    let seen = seen.borrow();
    assert_eq!(seen[0].key, "s");
    assert_eq!(seen[1].key, "s");
}

/// A release's return value is ignored: there is nothing downstream to
/// suppress, and the activation latch **must** clear whatever a handler thinks
/// — a consumed release that stranded the latch would swallow the next press.
#[test]
fn a_consuming_interceptor_does_not_strand_the_activation_latch() {
    let mut app = bare_app();
    set_keyboard_interceptor(|_| true);

    press(&mut app, KeyCode::Space, None, Modifiers::default());
    release(&mut app, KeyCode::Space, None, Modifiers::default());

    assert_eq!(
        app.node_activation_held, None,
        "the latch cleared even though the release was consumed"
    );
}

// ── the invariant the feature exists for ────────────────────────────────────

/// **A press and its release report the same `key`.** Nothing asserted this
/// across the shift regime before: the two agreement tests above sampled only
/// lowercase / no-`text` inputs, and their claim of "by construction" was
/// coincidence there — `Shift+A` went down as `"A"` (the press spelled itself
/// from `text`, which keeps the capital) and came up as `"a"` (the release has
/// no text and the old `logical_key` was a *lowercased* single ASCII letter).
/// A shifted non-letter was worse: `'!'` failed the letter filter outright, so
/// the release fell to the physical table and `Shift+1` came up as `"1"` after
/// going down as `"!"`.
///
/// The cure is the widened field (issue #337): `logical_key` now carries the
/// full case-accurate `KeyboardEvent.key` value, so both phases resolve to the
/// same string whatever the regime. Table-driven over the shapes that reach
/// different `hook_key_str` steps, so a future change to any one step has to
/// keep the pairing.
#[test]
fn a_press_and_its_release_always_agree() {
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    for (label, key, text, logical, mods) in [
        (
            "unshifted letter",
            KeyCode::KeyA,
            Some("a"),
            Some("a"),
            Modifiers::default(),
        ),
        ("shifted letter", KeyCode::KeyA, Some("A"), Some("A"), shift),
        ("ctrl chord", KeyCode::KeyS, None, Some("s"), ctrl_down()),
        (
            "ctrl+shift chord",
            KeyCode::KeyS,
            None,
            Some("S"),
            ctrl_shift,
        ),
        (
            "shifted non-letter",
            KeyCode::Digit1,
            Some("!"),
            Some("!"),
            shift,
        ),
        (
            "non-QWERTY layout letter",
            KeyCode::KeyQ,
            Some("a"),
            Some("a"),
            Modifiers::default(),
        ),
        (
            "named key",
            KeyCode::Enter,
            None,
            Some("Enter"),
            Modifiers::default(),
        ),
        (
            "dead key",
            KeyCode::Other,
            None,
            Some("Dead"),
            Modifiers::default(),
        ),
        (
            "no layout value at all",
            KeyCode::KeyS,
            None,
            None,
            Modifiers::default(),
        ),
    ] {
        let mut app = bare_app();
        let seen = recording_interceptor();
        press_on_layout(&mut app, key, text, logical, mods);
        release(&mut app, key, logical, mods);

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "{label}: one press, one release");
        assert_eq!(
            seen[0].key, seen[1].key,
            "{label}: press reported {:?} and release reported {:?} — a \
             consumer pairing them by `key` would never match, and the key \
             would look held for ever",
            seen[0].key, seen[1].key
        );
    }
}

/// And the spelling matches what a browser reports — measured in Chromium
/// rather than assumed: `Shift+A` is `"A"`, `Ctrl+Shift+S` is `"S"`, `Ctrl+S`
/// is `"s"`, `Shift+1` is `"!"` on a US layout. `rinch-web` passes
/// `event.key()` straight through, so pinning desktop to the browser's
/// spelling is what makes the two backends agree on the same keystroke.
#[test]
fn the_key_spelling_matches_the_browser() {
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    for (key, text, logical, mods, expected) in [
        (KeyCode::KeyA, Some("A"), Some("A"), shift, "A"),
        (KeyCode::KeyS, None, Some("S"), ctrl_shift, "S"),
        (KeyCode::KeyS, None, Some("s"), ctrl_down(), "s"),
        (
            KeyCode::KeyA,
            Some("a"),
            Some("a"),
            Modifiers::default(),
            "a",
        ),
        (KeyCode::Digit1, Some("!"), Some("!"), shift, "!"),
    ] {
        let mut app = bare_app();
        let seen = recording_interceptor();
        press_on_layout(&mut app, key, text, logical, mods);
        assert_eq!(
            seen.borrow()
                .last()
                .expect("the key reached the interceptor")
                .key,
            expected,
            "{key:?} with shift={} ctrl={}",
            mods.shift,
            mods.ctrl
        );
    }
}
