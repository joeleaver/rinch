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

/// Focus the user moved out of the overlay before it closed is theirs, and
/// closing neither restores over it nor releases it.
///
/// **Mutant: `restore`'s `is_self_or_descendant` guard deleted**, so the close
/// blurs whatever holds the keyboard. Here that is a control on the page, and
/// the user loses it for no reason. (The restore arm cannot cover this: it is
/// only reached when there *is* a connected opener, and this fixture opens with
/// nothing focused so there is none.)
#[test]
fn closing_does_not_release_a_claim_outside_the_overlay() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);

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
        "a claim outside the closing overlay is left alone"
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

/// A parked blur is refused if the keyboard moved on before it was applied.
///
/// Desktop applies a focus request a layout *after* it is posted, and focus can
/// change in between by a route that never touches the request slot — a
/// pointer press claims its node immediately. An overlay that closed with
/// nothing to restore must not then take the keyboard off whatever the user
/// clicked next.
///
/// **Mutant: `blur_node`'s `holds` check deleted.** The parked blur then
/// releases a claim that is no longer the one it was posted for, and the click
/// silently loses its focus one turn later.
#[test]
fn a_parked_blur_is_refused_if_focus_moved_since() {
    let open = Signal::new(false);
    let Page { mut app, ids, .. } = modal_page(open, true, false);

    open.set(true);
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "precondition: inside"
    );

    // Nothing was focused when it opened, so closing releases rather than
    // restores — that is the blur this fixture is about.
    open.set(false);
    // …and the keyboard moves on before the runtime looks, the way a pointer
    // press claims its node without going through the request slot.
    app.focus_element(ids.elsewhere);
    settle(&mut app);

    assert_eq!(
        focused(&app),
        Some(ids.elsewhere),
        "a blur parked for {} must not release a claim held by someone else",
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
