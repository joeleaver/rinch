//! Issue #482: a focused `RenderSurface` used to swallow every key the host
//! had not claimed — window-level host keybindings (and, on desktop, rinch's
//! own DevTools/inspect-mode toggles) went deaf the moment a canvas took
//! focus, whether or not the surface did anything with the key.
//!
//! `FocusTarget::Surface`'s `KeyDown` arm always forwarded the key to the
//! surface and then unconditionally returned, with no path back to the same
//! global fallback `FocusTarget::Input/None/Node` reach (DevTools, inspect
//! mode, Tab, …). The fix: an additive, opt-in
//! [`RenderSurfaceHandle::set_key_handler`] answers whether a given key
//! should stop at the surface; unset (or returning `false`) now falls through
//! to [`RinchApp::handle_unclaimed_key_fallback`], exactly as an unfocused
//! canvas always has.

use super::*;
use crate::render_surface::{SurfaceEvent, SurfaceKeyData, create_render_surface};
use rinch_core::Component;
use rinch_core::events::{KeyEventData, set_keyboard_interceptor};

/// A mounted app whose only content is a focused `RenderSurface`. Returns the
/// app with `focus_target` already set to the surface, plus the surface
/// handle for registering `set_event_handler`/`set_key_handler` on.
fn app_with_focused_surface() -> (RinchApp, crate::render_surface::RenderSurfaceHandle) {
    let surface = create_render_surface();
    let surface_id = surface.id();
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
    (app, surface)
}

fn press(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) -> Vec<AppAction> {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    )
}

/// The reported defect, directly: F12 toggles DevTools from anywhere in the
/// app *except* a focused surface, which swallowed it outright. With no
/// `set_key_handler` registered — the default, "claims nothing" — F12 must
/// now reach the same global fallback and emit `ToggleDevTools`.
///
/// Reverting the fix (dropping the `else if self.handle_unclaimed_key_fallback`
/// arm and going back to the old unconditional
/// `actions.push(AppAction::RequestRedraw)` after forwarding to the surface)
/// makes this fail: `actions` would contain `RequestRedraw` alone, never
/// `ToggleDevTools`.
#[test]
#[cfg(feature = "desktop")]
fn an_unclaimed_f12_still_toggles_devtools_while_a_surface_is_focused() {
    let (mut app, _surface) = app_with_focused_surface();

    let actions = press(&mut app, KeyCode::F12, Modifiers::default());

    assert!(
        actions.contains(&AppAction::ToggleDevTools),
        "F12 must reach the global fallback and toggle DevTools, even though \
         a RenderSurface holds focus and did not claim it: {actions:?}"
    );
}

/// Same shape, Alt+I / inspect mode — a second built-in that the old code
/// swallowed identically. Pinned separately so a fix that special-cased F12
/// alone (rather than routing through the shared fallback) still gets caught.
#[test]
fn an_unclaimed_alt_i_still_toggles_inspect_mode_while_a_surface_is_focused() {
    let (mut app, _surface) = app_with_focused_surface();

    let actions = press(
        &mut app,
        KeyCode::KeyI,
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );

    assert!(
        actions.contains(&AppAction::ToggleInspectMode),
        "Alt+I must reach the global fallback and toggle inspect mode: {actions:?}"
    );
}

/// A surface that explicitly claims a key via `set_key_handler` must still
/// stop it there — the fix is additive, not "nothing is ever swallowed again".
/// `KeyCode::KeyI` with Alt held would otherwise toggle inspect mode; once the
/// surface claims it, that must not happen.
#[test]
fn a_surface_that_claims_the_key_still_stops_it_there() {
    let (mut app, surface) = app_with_focused_surface();
    surface.set_key_handler(|data| data.key == "i");

    let actions = press(
        &mut app,
        KeyCode::KeyI,
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );

    assert!(
        !actions.contains(&AppAction::ToggleInspectMode),
        "a claimed key must not also run the global fallback: {actions:?}"
    );
}

/// The ordinary `set_event_handler` keeps receiving every `KeyDown` exactly
/// as before `set_key_handler` existed — the two are independent, and an app
/// that never calls `set_key_handler` must see no change to its input.
#[test]
fn the_surface_still_receives_every_keydown_whether_or_not_it_claims_it() {
    let (mut app, surface) = app_with_focused_surface();
    let seen: Rc<RefCell<Vec<SurfaceKeyData>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    surface.set_event_handler(move |event| {
        if let SurfaceEvent::KeyDown(data) = event {
            sink.borrow_mut().push(data);
        }
    });

    press(&mut app, KeyCode::F12, Modifiers::default());

    assert_eq!(
        seen.borrow().len(),
        1,
        "the surface's own event handler must still see the unclaimed key \
         for input purposes, independent of the fallback it also now reaches"
    );
}

/// Desktop already ran the document-level interceptor ahead of
/// `FocusTarget::Surface` before this fix (issue #484's desktop side was
/// already closed); confirm the refactor did not change that ordering. A
/// host's `set_keyboard_interceptor`-registered chord must still win over the
/// surface, and the surface must not see a key the interceptor consumed.
#[test]
fn an_interceptor_that_claims_a_key_still_wins_over_a_focused_surface() {
    let (mut app, surface) = app_with_focused_surface();
    let surface_saw: Rc<std::cell::Cell<usize>> = Rc::new(std::cell::Cell::new(0));
    let sink = surface_saw.clone();
    surface.set_event_handler(move |event| {
        if matches!(event, SurfaceEvent::KeyDown(_)) {
            sink.set(sink.get() + 1);
        }
    });
    set_keyboard_interceptor(move |_: &KeyEventData| true);

    let actions = press(&mut app, KeyCode::KeyK, Modifiers::default());

    assert_eq!(
        surface_saw.get(),
        0,
        "the interceptor claimed the key; the surface must never see it"
    );
    assert!(
        !actions.contains(&AppAction::ToggleDevTools),
        "and the fallback must not run either: {actions:?}"
    );
}

#[test]
fn tab_moves_focus_off_an_unclaimed_focused_surface_when_another_tab_stop_exists() {
    let surface = create_render_surface();
    let surface_id = surface.id();
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = crate::render_surface::RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        root.append_child(&child);
        let btn = scope.create_element("button");
        btn.set_attribute("style", "width: 40px; height: 20px;");
        let label = scope.create_text("ok");
        btn.append_child(&label);
        root.append_child(&btn);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app.focus_target = FocusTarget::Surface(surface_id);

    let _actions = press(&mut app, KeyCode::Tab, Modifiers::default());

    assert!(
        !matches!(app.focus_target, FocusTarget::Surface(_)),
        "Tab should move focus off the surface onto the button when the \
         surface has not claimed Tab: focus_target = {:?}",
        app.focus_target
    );
}
