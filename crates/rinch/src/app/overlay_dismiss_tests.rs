//! Escape, outside clicks and auto-close, driven through the real event path
//! (issue #474).
//!
//! `close_on_escape`, `Popover::close_on_click_outside` and
//! `Notification::auto_close` were declared, documented and read by nothing.
//! These are what say the wiring is *right*; `rinch-components`'
//! `no_dead_props` only says a field is read somewhere.
//!
//! **Why here and not in `rinch-components`.** That crate's own harness mounts
//! into `MockDomDocument`, which has no CSS engine and no keyboard dispatch, so
//! it can see that an element exists and nothing about what a key does to it.
//! `RinchApp::doc` is `pub(crate)`, so a fixture that delivers a real
//! `PlatformEvent` and then reads the tree has to live inside this crate.
//!
//! **Escape goes in as a `KeyDown` *and* a `KeyUp`**, the way a keyboard
//! delivers it. Both reach `dispatch_keyboard_event`, and only the press may
//! dismiss — a release that also dismissed would close two nested overlays per
//! keystroke, and every count here would read `2`.
//!
//! **"Unmounted" means the component's `RenderScope` was disposed**, which is
//! what `show_dom` does to an `if` branch that stops matching. The nesting
//! fixtures therefore build the inner overlay in a child scope they hold, so
//! they can dispose exactly it.

use super::*;
use std::cell::Cell;

use rinch_components::{Modal, Notification, Popover, PopoverDropdown, PopoverTarget};
use rinch_core::events::dismiss_handler_count;
use rinch_core::{Callback, Component, Signal};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount `build`'s tree under the real theme **and** component stylesheets, so
/// a backdrop's `position`, `z-index` and `display` are the ones the shipped
/// sheet gives it.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// A counter a component's `onclose` bumps.
fn recorder() -> (Rc<Cell<usize>>, Callback) {
    let hits: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let h = hits.clone();
    (hits, Callback::new(move || h.set(h.get() + 1)))
}

fn reactive(sig: Signal<bool>) -> Rc<dyn Fn() -> bool> {
    Rc::new(move || sig.get())
}

/// One Escape keystroke: press and release, as a keyboard delivers it.
fn escape(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Escape,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
}

fn tap(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

/// The one node carrying `class`, by name rather than by append order.
fn find_by_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node carries the class {class}"))
}

fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let n = d.tree.get(node_id).expect("node still in the tree");
    assert!(
        n.layout.width > 0.0 && n.layout.height > 0.0,
        "node {node_id} has no box to aim at: {:?}",
        n.layout
    );
    let (x, y, w, h) = painted_element_box(&d.tree, node_id);
    (x + w / 2.0, y + h / 2.0)
}

// ── 1. Escape, the ordinary case ─────────────────────────────────────────────

/// The whole of `close_on_escape`, and its off switch beside it.
///
/// The off half is not decoration: without it the test passes against a fix
/// that ignores the prop and dismisses unconditionally, which is a different
/// bug with the same headline.
#[test]
fn escape_closes_an_open_modal_and_close_on_escape_false_leaves_the_key_alone() {
    for (close_on_escape, expected) in [(true, 1), (false, 0)] {
        let open = Signal::new(true);
        let (closes, onclose) = recorder();
        let mut app = mount(move |scope: &mut RenderScope| {
            Modal {
                opened_fn: Some(reactive(open)),
                close_on_escape,
                onclose: Some(onclose),
                ..Default::default()
            }
            .render(scope, &[])
        });

        escape(&mut app);
        assert_eq!(
            closes.get(),
            expected,
            "close_on_escape: {close_on_escape} should have closed {expected} time(s)"
        );
    }
}

/// A press dismisses; the release that follows it must not dismiss again.
///
/// `KeyUp` reaches `dispatch_keyboard_event` too. Drop the `is_down` gate and
/// one keystroke closes two nested overlays.
#[test]
fn a_release_does_not_dismiss() {
    let open = Signal::new(true);
    let (closes, onclose) = recorder();
    let mut app = mount(move |scope: &mut RenderScope| {
        Modal {
            opened_fn: Some(reactive(open)),
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[])
    });

    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Escape,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
    assert_eq!(closes.get(), 0, "a bare release must dismiss nothing");

    escape(&mut app);
    assert_eq!(closes.get(), 1, "a whole keystroke dismisses exactly once");
}

/// **The open check is at dispatch, not at registration.**
///
/// `Modal::render` runs once and `opened_fn` only toggles a class, so a closed
/// modal stays mounted. A handler armed "because it was open at mount" would
/// swallow Escape for the rest of the session; one armed only when open at
/// mount would never fire for a modal that starts closed — which is every
/// modal. Both mutants die here, because the fixture starts **closed**.
#[test]
fn a_closed_but_mounted_modal_leaves_escape_to_the_app_and_answers_it_once_opened() {
    let open = Signal::new(false);
    let (closes, onclose) = recorder();
    let mut app = mount(move |scope: &mut RenderScope| {
        Modal {
            opened_fn: Some(reactive(open)),
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[])
    });

    escape(&mut app);
    assert_eq!(closes.get(), 0, "a closed modal must not swallow Escape");

    open.set(true);
    escape(&mut app);
    assert_eq!(closes.get(), 1, "and answers it once it is open");

    open.set(false);
    escape(&mut app);
    assert_eq!(closes.get(), 1, "and stops again when it closes");
}

/// Escape must reach the modal while an `<input>` *inside* it holds the
/// keyboard — the ordinary shape of a dialog with a form in it.
///
/// This is why the dismiss stack is dispatched from inside
/// `dispatch_keyboard_event`, which the desktop runtime calls in step 1, ahead
/// of the focus arbiter. Move it after the arbiter and the input's claim wins
/// and nothing closes.
#[test]
fn escape_closes_the_modal_while_an_input_inside_it_holds_focus() {
    let open = Signal::new(true);
    let (closes, onclose) = recorder();
    let field: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let field_in = field.clone();

    let mut app = mount(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("type", "text");
        input.set_attribute("style", "width: 200px; height: 30px");
        // A live `data-oninput`, as every real text field has: it is what makes
        // the text engine claim the press, so the precondition below is a
        // focused *editor* rather than a bare tab stop.
        let oninput = scope.register_input_handler(|_: String| {});
        input.set_attribute("data-oninput", &oninput.0.to_string());
        field_in.set(Some(input.node_id().0));
        Modal {
            opened_fn: Some(reactive(open)),
            // Off, so the click that focuses the field cannot be answered by
            // the overlay instead and leave the assertion ambiguous.
            close_on_click_outside: false,
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[input])
    });

    let id = field.get().expect("the input's node id");
    let (x, y) = centre(&app, id);
    tap(&mut app, x, y);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(id),
        "precondition: the field owns the keyboard"
    );

    escape(&mut app);
    assert_eq!(closes.get(), 1, "Escape reached the modal, not the field");
}

/// `Drawer` and `Popover` carry the same prop and must answer the same key.
///
/// They share `arm_close_on_escape` with `Modal`, and a test that only covered
/// `Modal` is exactly how a shared helper comes to be called from two of three
/// call sites. Each is checked with the prop on and off.
#[test]
fn drawer_and_popover_answer_escape_too_and_honour_their_own_off_switch() {
    for close_on_escape in [true, false] {
        let expected = usize::from(close_on_escape);

        let open = Signal::new(true);
        let (closes, onclose) = recorder();
        let mut drawer = mount(move |scope: &mut RenderScope| {
            rinch_components::Drawer {
                opened_fn: Some(reactive(open)),
                close_on_escape,
                onclose: Some(onclose),
                ..Default::default()
            }
            .render(scope, &[])
        });
        escape(&mut drawer);
        assert_eq!(
            closes.get(),
            expected,
            "Drawer with close_on_escape: {close_on_escape}"
        );
        drop(drawer);

        let open = Signal::new(true);
        let (closes, onclose) = recorder();
        let mut popover = mount(move |scope: &mut RenderScope| {
            Popover {
                opened_fn: Some(reactive(open)),
                close_on_escape,
                onclose: Some(onclose),
                ..Default::default()
            }
            .render(scope, &[])
        });
        escape(&mut popover);
        assert_eq!(
            closes.get(),
            expected,
            "Popover with close_on_escape: {close_on_escape}"
        );
    }
}

/// An overlay with no `onclose` has nowhere to send the request, so it must
/// leave Escape alone rather than register a handler that swallows it and does
/// nothing. `Popover` is the one that makes this concrete: it had no close
/// callback at all until #474.
#[test]
fn an_overlay_with_no_onclose_does_not_swallow_escape() {
    let open = Signal::new(true);
    let mut app = mount(move |scope: &mut RenderScope| {
        Popover {
            opened_fn: Some(reactive(open)),
            ..Default::default()
        }
        .render(scope, &[])
    });
    escape(&mut app);
    assert_eq!(
        dismiss_handler_count(),
        0,
        "nothing to invoke, so nothing is registered"
    );
}

// ── 2. Nesting ───────────────────────────────────────────────────────────────

/// Two modals, the inner one in a child scope the test can dispose.
struct Nested {
    app: RinchApp,
    outer_closes: Rc<Cell<usize>>,
    inner_closes: Rc<Cell<usize>>,
    inner_scope: Rc<RefCell<Option<RenderScope>>>,
    inner_root: Rc<Cell<Option<usize>>>,
}

impl Nested {
    fn mount(outer_open: Signal<bool>, inner_open: Signal<bool>) -> Self {
        let (outer_closes, outer_cb) = recorder();
        let (inner_closes, inner_cb) = recorder();
        let inner_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));
        let inner_root: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let scope_in = inner_scope.clone();
        let root_in = inner_root.clone();

        let app = mount(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");

            let outer = Modal {
                opened_fn: Some(reactive(outer_open)),
                onclose: Some(outer_cb),
                ..Default::default()
            }
            .render(scope, &[]);
            root.append_child(&outer);

            // The inner modal in a scope of its own — what `show_dom` gives an
            // `if` branch, and the only way to unmount one overlay and not the
            // other.
            let doc = scope.doc_weak().upgrade().expect("document alive");
            let mut child = RenderScope::new(doc, root.node_id());
            let inner = {
                let _owner = child.push_owner();
                Modal {
                    opened_fn: Some(reactive(inner_open)),
                    onclose: Some(inner_cb),
                    ..Default::default()
                }
                .render(&mut child, &[])
            };
            root_in.set(Some(inner.node_id().0));
            root.append_child(&inner);
            *scope_in.borrow_mut() = Some(child);

            root
        });

        Self {
            app,
            outer_closes,
            inner_closes,
            inner_scope,
            inner_root,
        }
    }

    /// Unmount the inner modal: dispose its scope and take its subtree out of
    /// the document, which is what `show_dom` does to a branch that stops
    /// matching.
    fn unmount_inner(&mut self) {
        let id = self.inner_root.get().expect("the inner modal's root");
        {
            let doc = self.app.doc.as_ref().unwrap();
            doc.borrow_mut().remove_node(rinch_core::dom::NodeId(id));
        }
        let scope = self.inner_scope.borrow_mut().take().expect("mounted once");
        scope.dispose();
        self.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
}

/// The innermost open overlay takes Escape, and the one beneath it does not
/// also close. LIFO, not broadcast.
#[test]
fn nested_modals_close_innermost_first() {
    let outer_open = Signal::new(true);
    let inner_open = Signal::new(true);
    let mut n = Nested::mount(outer_open, inner_open);

    escape(&mut n.app);
    assert_eq!(n.inner_closes.get(), 1, "the inner modal took the key");
    assert_eq!(
        n.outer_closes.get(),
        0,
        "and the outer one must not close with it"
    );

    // The app answers by closing the inner modal. The next Escape falls through
    // it — it is still mounted — to the outer one.
    inner_open.set(false);
    escape(&mut n.app);
    assert_eq!(n.inner_closes.get(), 1);
    assert_eq!(n.outer_closes.get(), 1, "the key falls through when closed");
}

/// **The fixture the single-slot interceptor fails.**
///
/// `set_keyboard_interceptor` is one slot per document: the inner modal's
/// registration replaces the outer's, and its cleanup *removes* the slot rather
/// than restoring what it displaced — so once an inner modal has been shown and
/// unmounted, the outer one's Escape is dead for the rest of the session.
///
/// The inner modal is still **open** when it unmounts, which is the stronger
/// spelling: a stack that only skipped *closed* handlers would still be
/// blocked by this one.
#[test]
fn after_the_inner_modal_unmounts_the_outer_still_answers_escape() {
    let outer_open = Signal::new(true);
    let inner_open = Signal::new(true);
    let mut n = Nested::mount(outer_open, inner_open);

    let before = dismiss_handler_count();
    assert_eq!(before, 2, "precondition: both modals are on the stack");

    n.unmount_inner();
    assert_eq!(
        dismiss_handler_count(),
        1,
        "unmounting releases the inner modal's handler eagerly — without this \
         the stack keeps a dead entry the scan has to walk past"
    );

    escape(&mut n.app);
    assert_eq!(
        n.outer_closes.get(),
        1,
        "the outer modal's Escape must survive the inner one's whole lifetime"
    );
    assert_eq!(
        n.inner_closes.get(),
        0,
        "and the unmounted modal's callback must never run"
    );
}

// ── 3. Two documents on one thread ───────────────────────────────────────────

/// #134/#139: two `RinchApp`s on one thread share every thread-local here.
/// One document's Escape must not close the other's modal.
#[test]
fn one_documents_escape_does_not_close_another_documents_modal() {
    let a_open = Signal::new(true);
    let b_open = Signal::new(true);
    let (a_closes, a_cb) = recorder();
    let (b_closes, b_cb) = recorder();

    let mut app_a = mount(move |scope: &mut RenderScope| {
        Modal {
            opened_fn: Some(reactive(a_open)),
            onclose: Some(a_cb),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let mut app_b = mount(move |scope: &mut RenderScope| {
        Modal {
            opened_fn: Some(reactive(b_open)),
            onclose: Some(b_cb),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(dismiss_handler_count(), 2, "one entry per document");

    escape(&mut app_b);
    assert_eq!(b_closes.get(), 1, "B's Escape closes B's modal");
    assert_eq!(
        a_closes.get(),
        0,
        "B's Escape must not reach A's modal — B was mounted last, so a stack \
         with no document check would close A's on the way past"
    );

    escape(&mut app_a);
    assert_eq!(a_closes.get(), 1, "A's own Escape still works");
}

// ── 4. Precedence that already existed ───────────────────────────────────────

/// A drag in progress eats Escape before anything else sees it, and that is
/// correct — cancelling the drag is what the user means. Pinned so nobody
/// "fixes" the dismiss stack into taking it.
#[test]
fn a_drag_in_progress_still_takes_escape_before_the_dismiss_stack() {
    let open = Signal::new(true);
    let (closes, onclose) = recorder();
    let row: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let row_in = row.clone();

    // The handle lives *inside* the modal. An open modal's root is a
    // full-viewport `position: fixed` box at z-index 200, so a draggable
    // sibling of it is not somewhere a press can land.
    let mut app = mount(move |scope: &mut RenderScope| {
        let handle = scope.create_element("div");
        handle.set_attribute("style", "width: 200px; height: 40px");
        handle.set_attribute("draggable", "true");
        row_in.set(Some(handle.node_id().0));

        Modal {
            opened_fn: Some(reactive(open)),
            close_on_click_outside: false,
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[handle])
    });

    let (x, y) = centre(&app, row.get().expect("the drag handle"));
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );

    escape(&mut app);
    assert_eq!(
        closes.get(),
        0,
        "the pending drag consumed Escape, as it did before the dismiss stack"
    );

    // And with nothing dragging, the very same keystroke closes the modal —
    // the positive control that says the fixture is not simply inert.
    escape(&mut app);
    assert_eq!(closes.get(), 1);
}

// ── 4b. A native <select> inside an overlay ──────────────────────────────────

/// A `Modal` holding a native `<select>`, with the select's node id.
fn mount_select_in_modal(open: Signal<bool>) -> (RinchApp, usize, Rc<Cell<usize>>) {
    let (closes, onclose) = recorder();
    let sel: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let sel_in = sel.clone();

    let app = mount(move |scope: &mut RenderScope| {
        let select = scope.create_element("select");
        select.set_attribute("style", "width: 200px; height: 30px");
        for label in ["one", "two"] {
            let opt = scope.create_element("option");
            opt.set_attribute("value", label);
            let t = scope.create_text(label);
            opt.append_child(&t);
            select.append_child(&opt);
        }
        sel_in.set(Some(select.node_id().0));
        Modal {
            opened_fn: Some(reactive(open)),
            close_on_escape: true,
            // Off, so a click in this fixture cannot be answered by the overlay
            // and leave an assertion ambiguous.
            close_on_click_outside: false,
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[select])
    });

    let id = sel.get().expect("the select's node id, captured at mount");
    (app, id, closes)
}

/// **Issue #671.** Escape closes the `<select>` popup, not the modal under it.
///
/// This is the one shape where the dismiss stack made a working behaviour
/// wrong. The popup is handled by the focus arbiter, which the runtime reaches
/// *after* `dispatch_keyboard_event` — so once `close_on_escape` started
/// working, a modal swallowed the key that belonged to the popup inside it, and
/// the popup was left open and orphaned. A browser closes the popup.
///
/// The repair needs no precedence special case: the popup opens **after** the
/// modal mounted, so it joins the stack above it and wins by exactly the LIFO
/// rule that makes nested modals work.
#[test]
fn escape_closes_an_open_select_and_leaves_the_modal_around_it_standing() {
    let open = Signal::new(true);
    let (mut app, id, closes) = mount_select_in_modal(open);

    app.open_select_popup(id, VIEWPORT.0, VIEWPORT.1);
    assert!(app.is_select_open(), "precondition: the popup is open");

    escape(&mut app);
    assert!(
        !app.is_select_open(),
        "#671: Escape must close the <select> popup"
    );
    assert_eq!(
        closes.get(),
        0,
        "#671: and must NOT close the modal underneath it"
    );

    // Positive control: with the popup closed, the very same keystroke closes
    // the modal. Without this the fixture passes against a build where Escape
    // does nothing at all.
    escape(&mut app);
    assert_eq!(closes.get(), 1, "the modal still answers its own Escape");
}

/// The ordering the LIFO rule promises, driven end to end: **one Escape each,
/// innermost first**.
///
/// The case above proves the popup wins. This proves the modal is not merely
/// skipped but still *there*, and that the popup's entry left the stack when it
/// closed rather than going on answering for the rest of the session — which is
/// what a handle that was never released would do.
///
/// **The discriminating assertion is the *second* keystroke, not the third.**
/// Against a build that never releases the handle, a popup entry still on the
/// stack consumes Escape #2 as well and the modal never hears it, so the
/// fixture dies on `"second Escape: the modal closes"`. The third keystroke
/// pins something weaker and true — with everything closed, nothing dismisses —
/// and is worth keeping, but it is not what catches the leak. (Measured; the
/// comment used to credit the third, which is the "killed by the wrong
/// assertion" shape.)
#[test]
fn a_select_opened_inside_a_modal_takes_the_first_escape_and_the_modal_the_second() {
    let open = Signal::new(true);
    let (mut app, id, closes) = mount_select_in_modal(open);
    app.open_select_popup(id, VIEWPORT.0, VIEWPORT.1);

    escape(&mut app);
    assert!(!app.is_select_open(), "first Escape: the select closes");
    assert_eq!(closes.get(), 0, "and the modal does not");

    escape(&mut app);
    assert_eq!(closes.get(), 1, "second Escape: the modal closes");

    // A third keystroke with nothing open must reach neither again — the
    // popup's entry is gone from the stack, not merely inert.
    open.set(false);
    escape(&mut app);
    assert_eq!(closes.get(), 1, "nothing left to dismiss");
}

/// A `<select>` with no dismissible overlay above it is untouched: the stack is
/// empty, nothing consumes, and the key reaches the arbiter exactly as it did
/// before any of this existed.
#[test]
fn a_bare_select_still_closes_on_escape_with_no_overlay_involved() {
    let sel: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let sel_in = sel.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let select = scope.create_element("select");
        select.set_attribute("style", "width: 200px; height: 30px");
        let opt = scope.create_element("option");
        opt.set_attribute("value", "one");
        let t = scope.create_text("one");
        opt.append_child(&t);
        select.append_child(&opt);
        sel_in.set(Some(select.node_id().0));
        root.append_child(&select);
        root
    });

    let id = sel.get().expect("the select's node id");
    app.open_select_popup(id, VIEWPORT.0, VIEWPORT.1);
    assert!(app.is_select_open(), "precondition");
    escape(&mut app);
    assert!(
        !app.is_select_open(),
        "Escape closes a bare select as before"
    );
}

// ── 5. Popover: outside clicks ───────────────────────────────────────────────

struct Pop {
    app: RinchApp,
    closes: Rc<Cell<usize>>,
    inside_clicks: Rc<Cell<usize>>,
    /// A node inside the popover's own dropdown, to aim a click at.
    inside: usize,
}

fn mount_popover(open: Signal<bool>) -> Pop {
    let (closes, onclose) = recorder();
    let inside_clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let inside_in = inside_clicks.clone();
    let inside_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let inside_id_in = inside_id.clone();

    let app = mount(move |scope: &mut RenderScope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 120px; height: 32px");
        let target = PopoverTarget.render(scope, &[trigger]);

        let content = scope.create_element("div");
        content.set_attribute("style", "width: 160px; height: 60px");
        let clicks = inside_in.clone();
        let rid = scope.register_handler(move || clicks.set(clicks.get() + 1));
        content.set_attribute("data-rid", &rid.0.to_string());
        inside_id_in.set(Some(content.node_id().0));
        let dropdown = PopoverDropdown.render(scope, &[content]);

        Popover {
            opened_fn: Some(reactive(open)),
            onclose: Some(onclose),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });

    Pop {
        app,
        closes,
        inside_clicks,
        inside: inside_id.get().expect("the popover's content node"),
    }
}

/// A click outside an open popover closes it; a click on its own content does
/// not. Both halves matter — a backdrop painted *above* the panel satisfies the
/// first and breaks the second, which is exactly what #317 found on
/// `DropdownMenu`.
#[test]
fn a_click_outside_an_open_popover_closes_it_and_a_click_inside_does_not() {
    let open = Signal::new(true);
    let mut pop = mount_popover(open);

    let (ix, iy) = centre(&pop.app, pop.inside);
    tap(&mut pop.app, ix, iy);
    assert_eq!(
        pop.inside_clicks.get(),
        1,
        "the click reached the popover's own content"
    );
    assert_eq!(pop.closes.get(), 0, "and did not dismiss it");

    // Far from the popover, which sits at the top left of an 800x600 viewport.
    tap(&mut pop.app, 700.0, 520.0);
    assert_eq!(pop.closes.get(), 1, "a click outside dismisses");
}

/// The backdrop is only there while the popover is open: a click anywhere on a
/// closed popover's page must reach the page.
#[test]
fn a_closed_popovers_backdrop_catches_nothing() {
    let open = Signal::new(false);
    let mut pop = mount_popover(open);

    tap(&mut pop.app, 700.0, 520.0);
    assert_eq!(
        pop.closes.get(),
        0,
        "a closed popover's backdrop must not swallow the page's clicks"
    );

    open.set(true);
    pop.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    tap(&mut pop.app, 700.0, 520.0);
    assert_eq!(pop.closes.get(), 1, "and catches them once it opens");
}

/// The backdrop is `position: fixed`, one z-level under the dropdown, and
/// hidden while closed — read off the **shipped** stylesheet rather than
/// asserted from the component's source.
#[test]
fn the_popover_backdrop_takes_its_geometry_from_the_stylesheet() {
    let open = Signal::new(true);
    let pop = mount_popover(open);
    let backdrop = find_by_class(&pop.app, "rinch-popover__backdrop");
    let dropdown = find_by_class(&pop.app, "rinch-popover__dropdown");

    let doc = pop.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let style = &d.tree.get(backdrop).expect("in the tree").computed_style;
    assert_eq!(
        style.position,
        rinch_dom::computed_style::values::PositionValue::Fixed,
        "`absolute` would be clipped by whatever clips the popover, so \
         \"outside\" would stop at the enclosing panel"
    );
    let panel = &d.tree.get(dropdown).expect("in the tree").computed_style;
    assert_eq!(
        style.z_index.map(|z| z + 1),
        panel.z_index,
        "the backdrop sits exactly one level under the panel, so a click on \
         the popover's own content still reaches the content"
    );
}

// ── 6. Notification auto-close ───────────────────────────────────────────────

mod auto_close {
    //! `Notification::auto_close`, driven by a manual timer backend.
    //!
    //! [`rinch_core::set_timer_backend`] writes a process-global static, so
    //! these serialize against each other with a mutex. Nothing else in this
    //! crate's test binary arms a timeout, so the installed backend is only
    //! ever seen by the tests below.

    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    thread_local! {
        static SCHEDULED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    }

    fn manual(id: u64, _delay_ms: u32) {
        SCHEDULED.with(|s| s.borrow_mut().push(id));
    }

    /// Serialize the tests that own the global timer backend, and start each
    /// one from an empty schedule.
    fn timer_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rinch_core::set_timer_backend(manual);
        SCHEDULED.with(|s| s.borrow_mut().clear());
        guard
    }

    /// Fire everything armed so far — the deadline arriving, with no waiting.
    fn deadline_arrives() {
        let ids: Vec<u64> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
        for id in ids {
            rinch_core::fire_timeout(id);
        }
    }

    fn armed() -> usize {
        SCHEDULED.with(|s| s.borrow().len())
    }

    fn notification(open: Signal<bool>, auto_close: u32, onclose: Callback) -> RinchApp {
        mount(move |scope: &mut RenderScope| {
            Notification {
                opened_fn: Some(reactive(open)),
                auto_close,
                onclose: Some(onclose),
                ..Default::default()
            }
            .render(scope, &[])
        })
    }

    /// Opening arms one timeout; its deadline closes the notification, once.
    ///
    /// Sampled with the notification starting **closed** and opened afterwards,
    /// which is how a toast actually appears. Starting it open would pass
    /// against a fix that only ever arms at mount.
    #[test]
    fn opening_arms_the_timer_and_the_deadline_closes_it_once() {
        let _lock = timer_lock();
        let open = Signal::new(false);
        let (closes, onclose) = recorder();
        let app = notification(open, 4000, onclose);
        assert_eq!(armed(), 0, "a closed notification arms nothing");

        open.set(true);
        assert_eq!(armed(), 1, "opening arms exactly one timeout");

        deadline_arrives();
        assert_eq!(closes.get(), 1, "the deadline closed it");
        deadline_arrives();
        assert_eq!(closes.get(), 1, "one-shot: it does not fire again");
        drop(app);
    }

    /// `auto_close: 0` is off, as `component-props.md` has always said.
    ///
    /// The zero here is the *documented* off switch and not a fixed point the
    /// fixture is parked on: the test above samples a non-zero delay, and
    /// nothing in either depends on the delay's value, only on whether a
    /// timeout was armed at all.
    #[test]
    fn auto_close_zero_arms_nothing() {
        let _lock = timer_lock();
        let open = Signal::new(true);
        let (closes, onclose) = recorder();
        let app = notification(open, 0, onclose);

        assert_eq!(armed(), 0, "0 means no auto-close");
        deadline_arrives();
        assert_eq!(closes.get(), 0);
        drop(app);
    }

    /// Closing it by hand cancels the pending timeout. Without the
    /// `clear_timeout`, `onclose` fires a second time when the original
    /// deadline arrives — at which point the app has already moved on, and a
    /// toast queue pops the *next* toast.
    #[test]
    fn dismissing_it_before_the_deadline_cancels_the_timer() {
        let _lock = timer_lock();
        let open = Signal::new(false);
        let (closes, onclose) = recorder();
        let app = notification(open, 4000, onclose);

        open.set(true);
        assert_eq!(armed(), 1);
        open.set(false); // the user hit the close button
        deadline_arrives();
        assert_eq!(
            closes.get(),
            0,
            "a cancelled timeout must not fire onclose behind the app's back"
        );

        // And a fresh showing gets a fresh deadline rather than nothing.
        open.set(true);
        assert_eq!(armed(), 1, "re-opening re-arms");
        deadline_arrives();
        assert_eq!(closes.get(), 1);
        drop(app);
    }

    /// Unmounting while the timeout is pending cancels it. Otherwise the
    /// callback fires into a component whose signals have been freed — the
    /// #141 shape, and the reason `set_timeout` records its owner at all.
    #[test]
    fn unmounting_before_the_deadline_cancels_the_timer() {
        let _lock = timer_lock();
        let open = Signal::new(true);
        let (closes, onclose) = recorder();
        let app = notification(open, 4000, onclose);
        assert_eq!(armed(), 1, "precondition: a timeout is pending");

        drop(app);
        deadline_arrives();
        assert_eq!(
            closes.get(),
            0,
            "an unmounted notification must not close itself later"
        );
    }
}
