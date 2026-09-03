//! Two follow-ups from issue #316.
//!
//! **Item 2 — a blurred window must not blink.** The caret blink is the
//! runtime's *only* `WaitUntil` arm, and it drove straight off
//! `focused_editor_id()` with no window-focus gate, so an app in the background
//! woke twice a second for ever to animate a caret in a window that does not
//! have the keyboard.
//!
//! **Item 3 — one press, one answer.** Two ancestor walks answered "is this
//! press on the focused node?" with different rules: the mousedown claim
//! resolved the *nearest focusable* ancestor, while `handle_click`'s release
//! check accepted *any* ancestor. Under the second, a press on a nested
//! focusable inside the focused node read as "still inside it" and the outer
//! node kept the keyboard while the user interacted with something else.

use super::*;

// ── item 2: the blink gate ──────────────────────────────────────────────────

#[cfg(feature = "desktop")]
mod blink {
    use super::*;

    fn app_with_focused_editor() -> RinchApp {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let text = scope.create_text("hello");
            root.append_child(&text);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        // The blink gate reads the arbiter and the window-focus flag; which
        // container it names is immaterial here.
        app.focus_target = FocusTarget::Editor(1);
        app
    }

    fn window_focus(app: &mut RinchApp, focused: bool) {
        app.handle_event(PlatformEvent::WindowFocus(focused), (800, 600), 1.0);
    }

    #[test]
    fn a_blurred_window_blinks_nothing() {
        let mut app = app_with_focused_editor();
        assert_eq!(
            app.blinking_editor_id(),
            Some(1),
            "a focused window blinks the focused editor"
        );

        window_focus(&mut app, false);

        assert_eq!(
            app.blinking_editor_id(),
            None,
            "a blurred window arms no wake"
        );
    }

    #[test]
    fn refocusing_the_window_resumes_the_blink() {
        let mut app = app_with_focused_editor();
        window_focus(&mut app, false);
        window_focus(&mut app, true);

        assert_eq!(app.blinking_editor_id(), Some(1));
    }

    /// The gate must **not** move into `focused_editor_id`: that one also feeds
    /// the post-layout `update_all_carets` pass, which draws the selection
    /// highlight as well as the caret. A blurred window keeps its claim
    /// (browser semantics, issue #226) and must keep showing its selection.
    #[test]
    fn a_blurred_window_keeps_its_editor_claim() {
        let mut app = app_with_focused_editor();
        window_focus(&mut app, false);

        assert_eq!(app.focus_target, FocusTarget::Editor(1));
        assert_eq!(
            app.focused_editor_id(),
            Some(1),
            "the overlay pass still has a target; only the blink stops"
        );
    }

    /// The other half of the contract, from the blink's own side: handed no
    /// target, `caret_blink_tick` reports nothing to schedule — which is what
    /// returns the event loop to `Wait`.
    #[test]
    fn no_blink_target_schedules_no_wake() {
        let app = app_with_focused_editor();
        assert!(crate::editor::caret_blink_tick(app.doc_key(), None).is_none());
    }
}

// ── item 3: one press, one answer ───────────────────────────────────────────

/// A focusable outer node with a focusable *inner* node inside it, plus a
/// plain child of the outer. Returns (app, outer, inner, plain_child).
fn nested_fixture() -> (RinchApp, usize, usize, usize) {
    let ids: Rc<std::cell::Cell<Option<(usize, usize, usize)>>> =
        Rc::new(std::cell::Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 400px; height: 200px");
        outer.set_attribute("tabindex", "0");
        let plain = scope.create_element("div");
        plain.set_attribute("style", "width: 300px; height: 40px");
        let inner = scope.create_element("div");
        inner.set_attribute("style", "width: 300px; height: 40px");
        inner.set_attribute("tabindex", "0");
        outer.append_child(&plain);
        outer.append_child(&inner);
        root.append_child(&outer);
        ids_in.set(Some((
            outer.node_id().0,
            inner.node_id().0,
            plain.node_id().0,
        )));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (outer, inner, plain) = ids.get().expect("node ids captured at mount");
    (app, outer, inner, plain)
}

fn abs_center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, w, h) = painted_element_box(&d.tree, id);
    (ax + w / 2.0, ay + h / 2.0)
}

fn press(app: &mut RinchApp, id: usize, button: MouseButton) {
    let (x, y) = abs_center(app, id);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, (800, 600), 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, (800, 600), 1.0);
}

/// The resolution itself, before any event plumbing: a plain child resolves to
/// its focusable ancestor, a nested focusable resolves to *itself*.
#[test]
fn a_press_resolves_to_the_nearest_focusable() {
    let (app, outer, inner, plain) = nested_fixture();
    let d = app.doc.as_ref().unwrap().borrow();

    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(plain)),
        PressFocus::Node(outer),
        "a plain child belongs to the focusable that encloses it"
    );
    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(inner)),
        PressFocus::Node(inner),
        "a nested focusable is its own answer, not its ancestor's"
    );
    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, None),
        PressFocus::Release
    );
}

/// A left press on a plain child keeps the outer node's claim.
#[test]
fn a_left_press_on_a_plain_child_keeps_the_claim() {
    let (mut app, outer, _inner, plain) = nested_fixture();

    press(&mut app, outer, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Node(outer));

    press(&mut app, plain, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Node(outer));
}

/// A left press on the nested focusable moves the claim to it — the claim walk
/// has always done this, and it is the answer the release check now agrees
/// with.
#[test]
fn a_left_press_on_a_nested_focusable_moves_the_claim() {
    let (mut app, outer, inner, _plain) = nested_fixture();

    press(&mut app, outer, MouseButton::Left);
    press(&mut app, inner, MouseButton::Left);

    assert_eq!(app.focus_target, FocusTarget::Node(inner));
}

/// The divergence, and the only behaviour this half of the PR changes. A
/// right/middle press never runs the mousedown claim, so the release check is
/// live there — and under the old any-ancestor rule the *inner* press counted
/// as "inside the outer node", leaving the outer node holding the keyboard.
/// It now resolves to the inner node, which is not the claim holder, so the
/// claim is released.
#[test]
fn a_right_press_on_a_nested_focusable_releases_the_outer_claim() {
    let (mut app, outer, inner, _plain) = nested_fixture();

    press(&mut app, outer, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Node(outer));

    press(&mut app, inner, MouseButton::Right);

    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "the press did not resolve to the claim holder, so it lost the keyboard"
    );
}

/// …while a right press on a *plain* child still resolves to the outer node,
/// so the claim survives. Both buttons now answer the question the same way.
#[test]
fn a_right_press_on_a_plain_child_keeps_the_claim() {
    let (mut app, outer, _inner, plain) = nested_fixture();

    press(&mut app, outer, MouseButton::Left);
    press(&mut app, plain, MouseButton::Right);

    assert_eq!(app.focus_target, FocusTarget::Node(outer));
}
