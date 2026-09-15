//! Moving focus **into** an opening overlay and giving it **back** on close
//! (issue #695) — the half `trap_focus` (#474) deferred.
//!
//! Containment made an open dialog usable from the keyboard; it did not make it
//! *behave* like one. An overlay opened with the claim still on whatever opened
//! it (the user had to press Tab once for something `showModal()` does for
//! them), and closed with the claim pointing into a subtree that no longer has
//! a box — the state `trap_focus_tests` had to document as the thing
//! containment recovers *around*.
//!
//! **Why these fixtures pump an event.** Desktop resolves a focus request after
//! the next layout, not at the instant the effect posts it: an overlay opening
//! is a class removal in the same effect flush, so at that moment every node
//! inside it still has the zero-size box its `display: none` ancestor gave it,
//! and `collect_focusable_nodes_from`'s visibility filter would reject the lot.
//! [`settle`] is therefore a real `UserEvent::ReRender` turn — resolve, *then*
//! apply — which is exactly the turn a signal write triggers in a running app
//! (`set_on_signal_change` → `RinchNativeEvent::ReRender`,
//! `shell/rinch_runtime.rs`). A fixture that only called `resolve_and_repaint`
//! would leave every request parked and assert nothing.
//!
//! **Every geometry here is declared, never measured** — every control carries
//! an explicit `width`/`height`, so no assertion depends on the host's fonts.

use super::*;
use std::cell::{Cell, RefCell};

use rinch_components::{Modal, Popover};
use rinch_core::{Component, Signal};

const W: f32 = 800.0;
const H: f32 = 600.0;

/// Mount under the real theme **and** component stylesheets, so a `Modal`'s
/// root gets the `position: fixed` / `display: none` the shipped sheet gives it.
/// Without the theme sheet every bare `var(--rinch-*)` is invalid at
/// computed-value time and the overlay has no box at all.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(W, H);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(W, H);
    app
}

/// One runtime turn: re-resolve layout, then apply whatever focus request the
/// effects parked — see the module note for why both halves are needed.
fn settle(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::UserEvent(UserEvent::ReRender),
        (W as u32, H as u32),
        1.0,
    );
}

/// The node the arbiter currently holds, whichever target kind it is.
fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

/// A `<button>` with a declared box, so nothing here is measured from text.
fn button(scope: &mut RenderScope, id: &str) -> NodeHandle {
    let b = scope.create_element("button");
    b.set_attribute("id", id);
    b.set_attribute("style", "display: block; width: 120px; height: 28px");
    b
}

/// Node ids captured at mount, by the name the fixture gave them.
#[derive(Clone, Copy, Default)]
struct Ids {
    opener: usize,
    elsewhere: usize,
    inside_first: usize,
    inside_last: usize,
}

/// A page with an opener button, a `Modal` holding two controls, and a second
/// page button to move focus to.
///
/// `autofocus_last` puts HTML's `autofocus` on the **last** control inside, so
/// "the first focusable" and "the `autofocus` one" are different nodes — the
/// fixed point that a fixture putting it on the first would sit on.
struct Page {
    app: RinchApp,
    ids: Ids,
    /// The opener's handle, so a fixture can take it out of the tree.
    opener_handle: Rc<RefCell<Option<NodeHandle>>>,
}

fn modal_page(open: Signal<bool>, trap_focus: bool, autofocus_last: bool) -> Page {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let opener_handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out = ids.clone();
    let out_handle = opener_handle.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        page.append_child(&opener);

        let in_first = button(scope, "in-first");
        let in_last = button(scope, "in-last");
        if autofocus_last {
            in_last.set_attribute("autofocus", "");
        }
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus,
            // Off so the only stops inside are the two the fixture put there;
            // the close button is a `<button>` and would otherwise be the first
            // focusable and blur every assertion below.
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[in_first.clone(), in_last.clone()]);
        page.append_child(&modal);

        let elsewhere = button(scope, "elsewhere");
        page.append_child(&elsewhere);

        out.set(Ids {
            opener: opener.node_id().0,
            elsewhere: elsewhere.node_id().0,
            inside_first: in_first.node_id().0,
            inside_last: in_last.node_id().0,
        });
        *out_handle.borrow_mut() = Some(opener);
        page
    });
    let ids = ids.get();
    Page {
        app,
        ids,
        opener_handle,
    }
}

// ── 1. moving focus in ──────────────────────────────────────────────────────

/// Opening a modal takes the keyboard off the opener and puts it on the first
/// control inside.
///
/// **Mutant: `arm_overlay_focus` not calling `focus_into`** (the #474 state).
/// Focus then stays on `opener`, which is where a browser's `showModal()` would
/// not have left it.
#[test]
fn opening_a_modal_moves_focus_to_its_first_control() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);
    app.focus_element(ids.opener);
    assert_eq!(focused(&app), Some(ids.opener), "precondition: the opener");

    open.set(true);
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "an opening modal must claim its first control, not leave the keyboard \
         on the opener ({})",
        ids.opener
    );
}

/// An `autofocus` control inside wins over the first one, wherever it sits.
///
/// **Mutant: `focus_into_subtree` taking `candidates.first()` unconditionally**
/// — i.e. the `autofocus` lookup deleted. Focus then lands on `in-first`, and
/// HTML's own "focus *this* one" is silently ignored. The attribute is on the
/// **last** stop precisely so the two answers differ.
#[test]
fn an_autofocus_control_inside_wins_over_the_first_one() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, true);

    open.set(true);
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.inside_last),
        "`autofocus` must win over the first stop ({})",
        ids.inside_first
    );
}

/// `trap_focus: false` moves nothing — the prop is rinch's "this overlay is
/// modal", and a browser's non-modal `show()` leaves the keyboard alone.
///
/// **Mutant: `arm_overlay_focus`'s `if !trap_focus { return }` deleted.** Every
/// overlay would then steal the keyboard on open, with no way to decline.
#[test]
fn trap_focus_false_moves_no_focus_and_restores_none() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, false, false);
    app.focus_element(ids.opener);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "with the prop off the keyboard stays where the app put it"
    );

    // And the close half is off with it: nothing was remembered, so nothing is
    // restored and nothing is released.
    app.focus_element(ids.inside_first);
    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "nor may closing move a claim this overlay never took"
    );
}

// ── 2. giving it back ───────────────────────────────────────────────────────

/// Closing hands the keyboard back to whatever held it when the modal opened.
///
/// **Mutant: the `(false, Some(_))` arm of the effect deleted** (open captured,
/// close does nothing). The claim then stays on `in-first`, inside a subtree
/// that is `display: none` — the #474 state this issue is about.
#[test]
fn closing_a_modal_restores_the_opener() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);
    app.focus_element(ids.opener);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "closing must give the keyboard back to the opener"
    );
}

/// An opener the dialog itself removed is **not** focused back into: the claim
/// is released instead.
///
/// **Mutant: `restore`'s `is_connected` guard deleted.** `try_focus_input` does
/// not check whether a node is still attached — a `<button>` keeps its
/// tag-implied tabindex wherever it sits — so the claim lands on a detached
/// node, where it swallows Enter/Space and anchors Tab. Measured: with the
/// guard removed this fixture reports `Some(opener)`.
///
/// It is also the shape of the #304 recycled-slot hazard, and the guard is a
/// liveness test rather than an identity one: a slot freed (desktop frees ids
/// through `NodeTree::remove_subtree`) and handed to a different *attached*
/// node would still be focused. That is pre-existing and named in
/// `DomDocument::is_connected`.
#[test]
fn an_opener_removed_while_the_modal_was_open_is_not_restored() {
    let open = Signal::new(false);
    let Page {
        mut app,
        ids,
        opener_handle,
    } = modal_page(open, true, false);
    app.focus_element(ids.opener);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    // The dialog deletes the row it was opened from — the ordinary shape of an
    // edit dialog that saves a removal.
    opener_handle
        .borrow()
        .as_ref()
        .expect("the opener handle")
        .remove();

    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        None,
        "a detached opener ({}) must not be focused; the claim is released",
        ids.opener
    );
}

/// Focus the user moved out of the overlay before it closed is theirs: the
/// close neither restores over it nor releases it.
///
/// HTML's dialog rule — focus is returned only when the dialog contained it, or
/// when nothing did — and it is rule 1 of `apply_focus_restore`.
///
/// **Mutant: that rule deleted** (`apply_focus_restore` skipping the
/// `node_is_self_or_descendant` gate). There *is* a restorable opener here, so
/// the close yanks the keyboard back to it off a control the user had chosen —
/// which is why the opener is focused first rather than leaving it `None` as
/// this fixture used to: with no opener the deleted gate only ever caused a
/// spurious blur, and half the rule went untested.
#[test]
fn closing_does_not_release_a_claim_outside_the_overlay() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);
    app.focus_element(ids.opener);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    // The app moved focus somewhere else while the overlay was open.
    app.focus_element(ids.elsewhere);
    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.elsewhere),
        "a claim outside the closing overlay is left alone — not released, and          not restored to {}",
        ids.opener
    );
}

/// Unmounting while still open restores too.
///
/// **Mutant: the `on_cleanup` deleted.** `if show { Modal { … } }` is the
/// ordinary way to mount an overlay, and it disposes the component *instead of*
/// toggling `opened` — so the effect that would have restored never runs again
/// and the keyboard stays claimed by a node that is no longer in the tree. This
/// is the "armed by one event, cleared only by a second that may never arrive"
/// shape, and the cleanup is the independent second clearing condition.
#[test]
fn unmounting_an_open_modal_restores_the_opener() {
    let inner_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let modal_root: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let open = Signal::new(false);

    let scope_out = inner_scope.clone();
    let out = ids.clone();
    let root_out = modal_root.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        page.append_child(&opener);

        // A child scope, which is what `show_dom` gives an `if` branch and the
        // only way to unmount the overlay and not the page.
        let doc = scope.doc_weak().upgrade().expect("document alive");
        let mut child = RenderScope::new(doc, page.node_id());
        let (modal, in_first) = {
            let _owner = child.push_owner();
            let in_first = button(&mut child, "in-first");
            let modal = Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                with_close_button: false,
                ..Default::default()
            }
            .render(&mut child, std::slice::from_ref(&in_first));
            (modal, in_first)
        };
        page.append_child(&modal);
        root_out.set(modal.node_id().0);
        *scope_out.borrow_mut() = Some(child);

        out.set(Ids {
            opener: opener.node_id().0,
            inside_first: in_first.node_id().0,
            ..Default::default()
        });
        page
    });
    let ids = ids.get();

    app.focus_element(ids.opener);
    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: the open modal took the keyboard"
    );

    // `show_dom`'s own order: dispose first, remove after — so the restore runs
    // while the overlay is still in the tree, which is the harsher case for
    // anything that looks its own node up.
    inner_scope
        .borrow_mut()
        .take()
        .expect("mounted once")
        .dispose();
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut()
            .remove_node(rinch_core::dom::NodeId(modal_root.get()));
    }
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "an overlay unmounted while open must still give the keyboard back"
    );
}

/// A parked restore is refused if the keyboard moved on before it was applied.
///
/// Desktop answers a focus request a layout *after* it is posted, and focus can
/// change in between by a route that never touches the request slot — a pointer
/// press claims its node immediately. An overlay that closed with nothing to
/// restore must not then take the keyboard off whatever the user clicked next.
///
/// **Mutant: `apply_focus_restore`'s containment gate deleted.** The release
/// then reaches a claim that is no longer inside the overlay it was posted for,
/// and the click silently loses its focus one turn later. This differs from
/// `closing_does_not_release_a_claim_outside_the_overlay` in *when* the claim
/// moves: there, before the close; here, after the request is parked and before
/// it is applied — the window only the deferral opens.
#[test]
fn a_parked_restore_is_refused_if_focus_moved_since() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    // Nothing was focused when it opened, so closing has nothing to hand back.
    open.set(false);
    // …and the keyboard moves on before the runtime looks, the way a pointer
    // press claims its node without going through the request slot.
    app.focus_element(ids.elsewhere);
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.elsewhere),
        "a restore parked for the overlay holding {} must not release a claim          held by someone else",
        ids.inside_first
    );
}

// ── 3. nesting ──────────────────────────────────────────────────────────────

/// Closing an inner overlay restores **into the outer one**, and closing the
/// outer restores to the page.
///
/// Nothing in `arm_overlay_focus` knows about nesting: the inner overlay
/// remembers whatever the outer one had focused, because the memory is
/// per-overlay. This is the fixture that says so.
///
/// **Mutant: one shared memory** (a single `static`/thread-local "previous
/// focus" instead of one cell per armed overlay). The inner's capture would
/// overwrite the outer's, and closing the outer would restore to `inner-a`
/// rather than to `opener`.
#[test]
fn a_nested_overlay_restores_into_the_one_that_contains_it() {
    #[derive(Clone, Copy, Default)]
    struct Nested {
        opener: usize,
        outer_a: usize,
        inner_a: usize,
    }

    let ids: Rc<Cell<Nested>> = Rc::new(Cell::new(Nested::default()));
    let out = ids.clone();
    let outer = Signal::new(false);
    let inner = Signal::new(false);
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        page.append_child(&opener);

        let inner_a = button(scope, "inner-a");
        let inner_modal = Modal {
            opened_fn: Some(Rc::new(move || inner.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, std::slice::from_ref(&inner_a));

        let outer_a = button(scope, "outer-a");
        let outer_modal = Modal {
            opened_fn: Some(Rc::new(move || outer.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[outer_a.clone(), inner_modal.clone()]);
        page.append_child(&outer_modal);

        out.set(Nested {
            opener: opener.node_id().0,
            outer_a: outer_a.node_id().0,
            inner_a: inner_a.node_id().0,
        });
        page
    });
    let ids = ids.get();

    app.focus_element(ids.opener);
    outer.set(true);
    settle(&mut app);
    assert_eq!(focused(&app), Some(ids.outer_a), "the outer took it");

    inner.set(true);
    settle(&mut app);
    assert_eq!(focused(&app), Some(ids.inner_a), "the inner took it");

    inner.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.outer_a),
        "closing the inner restores into the outer, not out to the page"
    );

    outer.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "and closing the outer restores to the page"
    );
}

// ── 4. the popover policy ───────────────────────────────────────────────────

/// A `Popover` does **not** take the keyboard on open — unless something inside
/// asks for it with `autofocus`.
///
/// That is the HTML popover API's rule and not the dialog's: an `auto` popover
/// runs its focusing steps only for an `autofocus` element, where
/// `showModal()` focuses the first stop regardless. `Modal` and `Drawer` pass
/// `FirstFocusable`; `Popover` passes `AutofocusOnly`.
///
/// **Mutant: `Popover` passing `FocusIntoPolicy::FirstFocusable`.** The first
/// half then fails — a popover opening would yank the keyboard out of whatever
/// the user was typing in, which is why the two policies exist at all.
#[test]
fn a_popover_takes_focus_only_for_an_autofocus_child() {
    for autofocus in [false, true] {
        let ids: Rc<Cell<(usize, usize)>> = Rc::new(Cell::new((0, 0)));
        let out = ids.clone();
        let open = Signal::new(false);
        let mut app = mount(move |scope: &mut RenderScope| {
            let page = scope.create_element("div");
            let outside = button(scope, "outside");
            page.append_child(&outside);

            let inside = button(scope, "inside");
            if autofocus {
                inside.set_attribute("autofocus", "");
            }
            let popover = Popover {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                ..Default::default()
            }
            .render(scope, std::slice::from_ref(&inside));
            page.append_child(&popover);

            out.set((outside.node_id().0, inside.node_id().0));
            page
        });
        let (outside, inside) = ids.get();

        app.focus_element(outside);
        open.set(true);
        settle(&mut app);

        let expected = if autofocus { inside } else { outside };
        assert_eq!(
            focused(&app),
            Some(expected),
            "autofocus: {autofocus} — a popover moves focus only when asked"
        );
    }
}

// ── 5. how overlays are actually opened ─────────────────────────────────────

/// The centre of a node's painted box, in the logical space `click()` speaks.
fn centre(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = crate::app::hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h / 2.0)
}

/// A real left press and release on a node, so the `data-rid` dispatch runs on
/// the path a pointer takes.
fn click_node(app: &mut RinchApp, id: usize) {
    let (x, y) = centre(app, id);
    for ev in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(ev, (W as u32, H as u32), 1.0);
    }
}

/// A page whose opener button carries a `data-rid` that opens the modal, which
/// is how an overlay is opened outside a test.
fn modal_page_with_live_opener(open: Signal<bool>) -> Page {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let opener_handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out = ids.clone();
    let out_handle = opener_handle.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        let handler = scope.register_handler(move || open.set(true));
        opener.set_attribute("data-rid", &handler.0.to_string());
        page.append_child(&opener);

        let in_first = button(scope, "in-first");
        let in_last = button(scope, "in-last");
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[in_first.clone(), in_last.clone()]);
        page.append_child(&modal);

        let elsewhere = button(scope, "elsewhere");
        page.append_child(&elsewhere);

        out.set(Ids {
            opener: opener.node_id().0,
            elsewhere: elsewhere.node_id().0,
            inside_first: in_first.node_id().0,
            inside_last: in_last.node_id().0,
        });
        *out_handle.borrow_mut() = Some(opener);
        page
    });
    let ids = ids.get();
    Page {
        app,
        ids,
        opener_handle,
    }
}

/// Clicking the opener moves focus into the modal it opens.
///
/// **This is how overlays are opened**, and the first ten fixtures in this file
/// all miss it: they write `opened` from outside any dispatch, so the parked
/// request is answered by a `UserEvent::ReRender` turn, which is one of the two
/// consumers that runs a layout first. The pointer path
/// (`click_handling.rs`) and the Enter path (`activate_focused_node`) drain the
/// slot **synchronously after `dispatch_event`**, with no layout in between.
///
/// **Mutant: `FocusRequest::needs_layout` returning `false`** (equivalently,
/// the two synchronous consumers calling `apply_focus_request` again). The
/// request is then resolved against the pre-open tree, where every child of the
/// modal still has the zero box its `display: none` ancestor gave it, so
/// nothing is focused — and because the slot has been emptied the later
/// post-layout turns find nothing to do. Measured: focus stays on the opener
/// through two further `settle`s. The move is lost, not delayed.
#[test]
fn clicking_the_opener_moves_focus_into_the_modal() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page_with_live_opener(open);

    click_node(&mut app, ids.opener);
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "a click-opened modal must take the keyboard; it is still on the \
         opener ({})",
        ids.opener
    );
}

/// Enter on the focused opener does the same, through the other synchronous
/// consumer.
///
/// **Mutant: as above.** Two fixtures rather than one because the two drains
/// are two call sites — `click_handling.rs` and `activate_focused_node` — and a
/// fix applied to only one of them leaves the other silently broken, which is
/// how this survived the first round.
#[test]
fn enter_on_the_opener_moves_focus_into_the_modal() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page_with_live_opener(open);
    app.focus_element(ids.opener);
    assert_eq!(focused(&app), Some(ids.opener), "precondition");

    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Enter,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
        },
        (W as u32, H as u32),
        1.0,
    );
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "an Enter-opened modal must take the keyboard too"
    );
}

// ── 6. an opener that is there but cannot take it ───────────────────────────

/// An opener that went `disabled` while the dialog was open is not restored,
/// and the claim is released rather than left inside the closed overlay.
///
/// The realistic shape is a dialog that disables the control that opened it
/// while it works ("Saving…"), or a form section that goes disabled underneath.
///
/// **Mutant: the `node_is_disabled_in_tree` arm of `node_can_take_focus_now`
/// deleted.** The claim then stays on `in-first`, inside a `display: none`
/// subtree, which is precisely the pre-#695 state this feature removes.
///
/// It does **not** pin the verify-after, which an earlier version of this
/// comment claimed: with the predicate intact the restore never attempts the
/// focus, so nothing reaches the verify to be checked. The verify has its own
/// fixture below, and it needed a constructed one.
#[test]
fn a_disabled_opener_is_not_restored_and_the_claim_is_released() {
    let open = Signal::new(false);
    let Page {
        mut app,
        ids,
        opener_handle,
    } = modal_page(open, true, false);
    app.focus_element(ids.opener);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    opener_handle
        .borrow()
        .as_ref()
        .expect("the opener handle")
        .set_attribute("disabled", "");

    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        None,
        "a disabled opener ({}) cannot take the keyboard, so it is released \
         rather than left on {}",
        ids.opener,
        ids.inside_first
    );
}

/// A restore the arbiter **refuses** releases the claim rather than stranding
/// it — the verify-after, which `node_can_take_focus_now` cannot stand in for.
///
/// The two are not the same question. The predicate asks what the tree says
/// (attached, has a box, not disabled, focusable); `try_focus_input` asks what
/// the *engine* can install, and it has one silent refusal the tree cannot
/// show: an `<input>` whose `data-oninput` does not parse. It takes the text
/// branch on its tag, fails the parse, and returns having claimed nothing —
/// while the predicate, reading the same node, says yes.
///
/// **Mutant: the verify deleted** (`apply_focus_restore` returning
/// unconditionally after `focus_element`). The restore then believes a focus
/// that never happened and skips the release, leaving the keyboard on
/// `in-first` inside the closed modal. Measured: the whole `rinch` lib suite
/// passes with the verify gone, and this is the fixture that stops it.
///
/// **Constructed, and deliberately so.** The handler is broken *while the
/// dialog is open*, because the opener has to be focusable at open time to be
/// remembered at all. No in-tree writer produces an unparseable
/// `data-oninput`, so nothing natural opens this gap — but the gap is between
/// two predicates that will go on drifting, and something has to hold them
/// together.
#[test]
fn a_restore_the_arbiter_refuses_releases_the_claim() {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let open = Signal::new(false);
    let out = ids.clone();
    let out_handle = handle.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");

        // A real text control, so the focus routes through the text engine.
        let opener = scope.create_element("input");
        opener.set_attribute("id", "opener");
        opener.set_attribute("type", "text");
        opener.set_attribute("style", "display: block; width: 200px; height: 28px");
        let oninput = scope.register_input_handler(|_: String| {});
        opener.set_attribute("data-oninput", &oninput.0.to_string());
        page.append_child(&opener);

        let in_first = button(scope, "in-first");
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, std::slice::from_ref(&in_first));
        page.append_child(&modal);

        out.set(Ids {
            opener: opener.node_id().0,
            inside_first: in_first.node_id().0,
            ..Default::default()
        });
        *out_handle.borrow_mut() = Some(opener);
        page
    });
    let ids = ids.get();

    app.focus_element(ids.opener);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.opener),
        "precondition: the opener holds the keyboard as a text target"
    );

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: the modal took it, remembering {}",
        ids.opener
    );

    // The opener's handler id stops parsing while the dialog is open. Every
    // check `node_can_take_focus_now` makes still passes — it is attached, it
    // has a box, it is not disabled, and `<input>` is focusable by tag.
    handle
        .borrow()
        .as_ref()
        .expect("the opener handle")
        .set_attribute("data-oninput", "not-a-handler-id");

    open.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        None,
        "the focus was refused, so the claim is released rather than left on {}",
        ids.inside_first
    );
}

/// Closing the **outer** of two open overlays restores the page, and closing
/// the inner afterwards does not put the keyboard back inside the closed outer.
///
/// The order an app closes everything in, or a route change.
///
/// **Mutant: the restore deleted** (`arm_overlay_focus` never calling
/// `restore_focus`). Both steps then leave the claim where it was, inside a
/// dialog that is gone. What this fixture does **not** discriminate is the
/// *reason* the second step is safe: the outer's own restore has already moved
/// the claim out to the page, so the inner's rule 1 returns before rule 2 is
/// consulted at all. The fixture below removes that cover.
#[test]
fn closing_the_outer_overlay_first_still_restores_the_page() {
    #[derive(Clone, Copy, Default)]
    struct Nested {
        opener: usize,
        outer_a: usize,
        inner_a: usize,
    }

    let ids: Rc<Cell<Nested>> = Rc::new(Cell::new(Nested::default()));
    let out = ids.clone();
    let outer = Signal::new(false);
    let inner = Signal::new(false);
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        page.append_child(&opener);

        let inner_a = button(scope, "inner-a");
        let inner_modal = Modal {
            opened_fn: Some(Rc::new(move || inner.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, std::slice::from_ref(&inner_a));

        let outer_a = button(scope, "outer-a");
        let outer_modal = Modal {
            opened_fn: Some(Rc::new(move || outer.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[outer_a.clone(), inner_modal.clone()]);
        page.append_child(&outer_modal);

        out.set(Nested {
            opener: opener.node_id().0,
            outer_a: outer_a.node_id().0,
            inner_a: inner_a.node_id().0,
        });
        page
    });
    let ids = ids.get();

    app.focus_element(ids.opener);
    outer.set(true);
    settle(&mut app);
    inner.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inner_a),
        "precondition: inner has it"
    );

    outer.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "the outer restores to the page even though the inner is still open"
    );

    inner.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.opener),
        "and the inner must not hand the keyboard to {} inside the closed outer",
        ids.outer_a
    );
}

/// An opener that is still attached but sits inside an overlay that has
/// **already** closed is not restored: the keyboard is released instead.
///
/// **Mutant: the `node_is_visible` arm of `node_can_take_focus_now` deleted.**
/// `try_focus_input` has no visibility test at all, so the focus *succeeds* and
/// the claim ends up inside a closed dialog — the failure the verify-after
/// cannot catch, because the arbiter really did take it.
///
/// **The outer traps nothing on purpose.** With `trap_focus` on it, the outer's
/// own restore moves the claim out to the page first, and the inner's rule 1
/// then returns before rule 2 is reached — so the visibility arm is never
/// consulted and the mutant survives. Measured: it did. Turning the outer's
/// focus management off leaves the claim inside the inner, which is inside the
/// closed outer, which is the only state where "connected but unreachable" is
/// the deciding fact.
#[test]
fn an_opener_inside_an_already_closed_overlay_is_not_restored() {
    #[derive(Clone, Copy, Default)]
    struct Nested {
        outer_a: usize,
        inner_a: usize,
    }

    let ids: Rc<Cell<Nested>> = Rc::new(Cell::new(Nested::default()));
    let out = ids.clone();
    let outer = Signal::new(false);
    let inner = Signal::new(false);
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");

        let inner_a = button(scope, "inner-a");
        let inner_modal = Modal {
            opened_fn: Some(Rc::new(move || inner.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, std::slice::from_ref(&inner_a));

        let outer_a = button(scope, "outer-a");
        let outer_modal = Modal {
            opened_fn: Some(Rc::new(move || outer.get())),
            // Off, so the outer neither takes the keyboard nor gives it back —
            // see the note above.
            trap_focus: false,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[outer_a.clone(), inner_modal.clone()]);
        page.append_child(&outer_modal);

        out.set(Nested {
            outer_a: outer_a.node_id().0,
            inner_a: inner_a.node_id().0,
        });
        page
    });
    let ids = ids.get();

    outer.set(true);
    settle(&mut app);
    app.focus_element(ids.outer_a);
    assert_eq!(
        focused(&app),
        Some(ids.outer_a),
        "precondition: in the outer"
    );

    inner.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inner_a),
        "precondition: the inner took it, remembering {}",
        ids.outer_a
    );

    // The outer closes without restoring anything, leaving the inner's
    // remembered opener attached and boxless.
    outer.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inner_a),
        "precondition: the claim is still inside the inner"
    );

    inner.set(false);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        None,
        "{} is connected but has no box, so the keyboard is released rather \
         than handed into a closed dialog",
        ids.outer_a
    );
}
