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

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    press_on_layout(app, key, text, None, modifiers);
}

/// The same press with the layout-mapped letter the desktop shell attaches to
/// a real `KeyDown` (`RinchRuntime::winit_logical_letter`).
fn press_on_layout(
    app: &mut RinchApp,
    key: KeyCode,
    text: Option<&str>,
    logical_key: Option<char>,
    modifiers: Modifiers,
) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key,
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
    press_on_layout(&mut app, KeyCode::KeyQ, None, Some('a'), ctrl_down());

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
