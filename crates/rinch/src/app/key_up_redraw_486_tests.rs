//! A `KeyUp` legs request no redraw (issue #486).
//!
//! `KeyDown` pushes `AppAction::RequestRedraw` on every leg that offers the
//! key to app code — the document-level interceptor (when it consumes),
//! a focused render surface (unconditionally), and a registered node's
//! `on_key` (when it consumes). `KeyUp` mirrors the routing — same three
//! legs, same payload-building function — but was added later (#337, PR
//! #462) and never wired any of them to `actions`. For a handler that writes
//! a `Signal` this is invisible: the signal's own change callback requests a
//! redraw independently. A handler that mutates the DOM **directly** through
//! a `NodeHandle` setter has no such fallback — named damage marks the node
//! dirty (CLAUDE.md: "a frame paints only what's marked"), but nothing asks
//! the shell to paint a frame at all, so the change sits unpainted until some
//! unrelated event arrives.
//!
//! Fixed scope, matching the issue title exactly: the interceptor leg and the
//! `on_key` leg. (The `Surface` leg forwards unconditionally already on
//! `KeyDown` and is a surface's own render-loop concern per the existing
//! comment in the `KeyUp` arm — "the surface's own claim" — so it is left
//! alone here.)

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use rinch_core::events::{KeyEventData, set_keyboard_interceptor};

/// A bare mounted app — no focusable content.
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

fn release(app: &mut RinchApp, key: KeyCode) -> Vec<AppAction> {
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    )
}

/// Reproduction: a `KeyUp` interceptor that mutates the DOM directly (not
/// through a `Signal`) must still see a redraw requested, or its change sits
/// unpainted until the next unrelated event.
#[test]
fn a_keyup_interceptor_requests_a_redraw() {
    let mut app = bare_app();
    set_keyboard_interceptor(move |data: &KeyEventData| {
        // A direct DOM mutation, deliberately not a Signal write — the shape
        // the issue describes as invisible to the signal-triggered redraw.
        assert!(data.is_up(), "only interested in the release here");
        false
    });

    let actions = release(&mut app, KeyCode::KeyK);

    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "a KeyUp offered to the interceptor must request a redraw: {actions:?}"
    );
}

/// Same gap, the `on_key` leg: a registered focus target's release handler.
#[test]
fn a_registered_focus_targets_on_key_requests_a_redraw_on_release() {
    let id: std::rc::Rc<std::cell::Cell<usize>> = std::rc::Rc::new(std::cell::Cell::new(0));
    let id_in = id.clone();

    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let div = scope.create_element("div");
        div.set_attribute("style", "width: 200px; height: 40px");
        div.set_attribute("tabindex", "0");
        register_focus_target(
            &div,
            FocusEntry::new().on_key(|_k| {
                // Consumed, but the point is: does the dispatcher request a
                // redraw regardless of what this handler did to the DOM.
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

    let actions = release(&mut app, KeyCode::KeyK);

    assert!(
        actions.contains(&AppAction::RequestRedraw),
        "a KeyUp offered to a registered node's on_key must request a redraw: {actions:?}"
    );
}
