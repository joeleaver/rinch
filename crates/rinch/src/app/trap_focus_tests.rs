//! Tab containment for open overlays (`trap_focus` / `data-trap-focus`, #474).
//!
//! `Modal`, `Drawer` and `Popover` all declared `trap_focus`, documented it, and
//! read it nowhere: Tab walked out of an open dialog and into the page behind
//! it. That is the "a Modal's or Drawer's backdrop does not contain Tab" gap the
//! focus guide and CLAUDE.md both listed as *not yet*.
//!
//! **Why here and not in `rinch-components`.** That crate's harness mounts into
//! `MockDomDocument`, which has no CSS engine and no keyboard dispatch — it can
//! see that an attribute exists and nothing about what Tab does. `RinchApp::doc`
//! and `RinchApp::focus_target` are `pub(crate)`, so a fixture that delivers a
//! real `PlatformEvent` and then reads the arbiter has to live inside this
//! crate. These tests say the wiring is *right*; `no_dead_props` only says a
//! field is read somewhere.
//!
//! **Two independent guards, and each needs its own fixture.** A trap is only
//! honoured if (a) the attribute is *on* — present, and not rinch's `"false"`
//! escape — and (b) the node carrying it is *visible*. Both hold for a closed
//! `Modal`, whose component removes the attribute **and** whose root is
//! `display: none`, so the obvious "a closed modal traps nothing" fixture passes
//! against a build with either guard deleted. The two are therefore separated
//! here: `an_invisible_trap_is_not_a_trap` builds a hidden root that *keeps* the
//! attribute, and `the_false_escape_opts_out` builds a visible one whose value
//! is `"false"`.
//!
//! **Every geometry here is declared, never measured.** Every control carries an
//! explicit `width`/`height`, so no assertion depends on the host's fonts.

use super::*;
use std::cell::Cell;

use rinch_components::Modal;
use rinch_core::{Component, Signal};

const W: f32 = 800.0;
const H: f32 = 600.0;

/// Mount `build`'s tree under the real theme **and** component stylesheets, so
/// a `Modal`'s root gets the `position: fixed` / `display: none` the shipped
/// sheet gives it. Without the theme sheet every bare `var(--rinch-*)` is
/// invalid at computed-value time and the overlay has no box at all.
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

fn key(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (W as u32, H as u32),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        (W as u32, H as u32),
        1.0,
    );
}

fn tab(app: &mut RinchApp) {
    key(app, KeyCode::Tab, Modifiers::default());
}

fn shift_tab(app: &mut RinchApp) {
    key(
        app,
        KeyCode::Tab,
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );
}

/// The node the arbiter currently holds, whichever target kind it is.
fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

/// Tab `n` times, recording where the arbiter lands each time.
fn tab_tour(app: &mut RinchApp, n: usize) -> Vec<Option<usize>> {
    (0..n)
        .map(|_| {
            tab(app);
            focused(app)
        })
        .collect()
}

/// A `<button>` with a declared box, so nothing here is measured from text.
fn button(scope: &mut RenderScope, id: &str) -> NodeHandle {
    let b = scope.create_element("button");
    b.set_attribute("id", id);
    b.set_attribute("style", "display: block; width: 120px; height: 28px");
    b
}

/// A text `<input>` with a live `data-oninput`, which is what makes the text
/// engine claim it — so it lands as `FocusTarget::Input`, the target kind that
/// makes `FocusEntry::on_key` useless for this job.
fn text_input(scope: &mut RenderScope, id: &str) -> NodeHandle {
    let i = scope.create_element("input");
    i.set_attribute("id", id);
    i.set_attribute("type", "text");
    i.set_attribute("style", "display: block; width: 200px; height: 28px");
    let oninput = scope.register_input_handler(|_: String| {});
    i.set_attribute("data-oninput", &oninput.0.to_string());
    i
}

/// Node ids captured at mount, by the name the fixture gave them.
#[derive(Clone, Copy, Default)]
struct Ids {
    before: usize,
    after: usize,
    inside_first: usize,
    inside_middle: usize,
    inside_last: usize,
    root: usize,
}

/// A page with a control either side of a `Modal`, and three controls inside
/// it: a button, a text field, a button.
///
/// The field in the middle is deliberate — it is the `FocusTarget::Input` case,
/// the one a `register_focus_target`-based trap would never see.
fn modal_page(open: Signal<bool>, trap_focus: bool) -> (RinchApp, Ids) {
    let ids: Rc<Cell<Ids>> = Rc::new(Cell::new(Ids::default()));
    let out = ids.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let before = button(scope, "before");
        page.append_child(&before);

        let in_first = button(scope, "in-first");
        let in_middle = text_input(scope, "in-middle");
        let in_last = button(scope, "in-last");
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus,
            // Off so the only Tab stops inside the modal are the three the
            // fixture put there; the close button is a `<button>` and would
            // otherwise join the cycle and blur every index assertion.
            with_close_button: false,
            ..Default::default()
        }
        .render(
            scope,
            &[in_first.clone(), in_middle.clone(), in_last.clone()],
        );
        page.append_child(&modal);

        let after = button(scope, "after");
        page.append_child(&after);

        out.set(Ids {
            before: before.node_id().0,
            after: after.node_id().0,
            inside_first: in_first.node_id().0,
            inside_middle: in_middle.node_id().0,
            inside_last: in_last.node_id().0,
            root: modal.node_id().0,
        });
        page
    });
    let ids = ids.get();
    (app, ids)
}

/// Re-run layout after a signal write, so a `display: none` toggle is in the
/// tree the collector walks. Adding or removing the modal's `--hidden` class
/// dirties style, so this is not the no-op `resolve_layout`'s `!layout_dirty`
/// early-return makes of a re-resolve at an unchanged viewport.
///
/// **Deliberately not a full runtime turn.** Since #695 an overlay's open and
/// close also *park* a focus request, which only a `UserEvent::ReRender` turn
/// applies ([`turn`]). Leaving that out here is what keeps these fixtures about
/// containment alone: the claim is wherever the fixture put it, and Tab's
/// answer is not entangled with the overlay having moved focus itself.
/// `overlay_focus_tests` is where the move and the restore are asserted.
fn settle(app: &mut RinchApp) {
    app.resolve_and_repaint(W, H);
}

/// A full runtime turn: [`settle`] plus applying whatever focus request the
/// effects parked (#695) — the turn a signal write triggers in a running app.
fn turn(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::UserEvent(UserEvent::ReRender),
        (W as u32, H as u32),
        1.0,
    );
}

// ── 1. containment ──────────────────────────────────────────────────────────

/// Tab cycles the open modal's own controls and never leaves it — including
/// the wrap from its last control back to its first.
///
/// **Mutant: the whole-tree walk** (`collect_focusable_nodes()` in `handle_tab`
/// instead of `collect_focusable_nodes_from(trap)`). The very first Tab then
/// lands on `before`, outside the dialog, and every element of this tour is
/// wrong.
///
/// The tour is four Tabs for three stops on purpose: three would end on the
/// last control, which is where the un-trapped build's *fourth* position also
/// happens not to be — the wrap is the assertion that cannot be satisfied by
/// accident.
#[test]
fn tab_cycles_inside_an_open_modal_and_wraps_from_its_last_control_to_its_first() {
    let open = Signal::new(true);
    let (mut app, ids) = modal_page(open, true);

    let tour = tab_tour(&mut app, 4);
    assert_eq!(
        tour,
        vec![
            Some(ids.inside_first),
            Some(ids.inside_middle),
            Some(ids.inside_last),
            Some(ids.inside_first),
        ],
        "Tab must cycle the modal's own three controls; `before` is {} and \
         `after` is {}",
        ids.before,
        ids.after
    );
}

/// Shift+Tab from the first control inside wraps to the last one inside.
///
/// **Mutant: the whole-tree walk.** Shift+Tab from the modal's first control
/// would then step *backwards out* of the dialog onto `before`.
#[test]
fn shift_tab_from_the_first_control_inside_wraps_to_the_last_inside() {
    let open = Signal::new(true);
    let (mut app, ids) = modal_page(open, true);

    tab(&mut app);
    assert_eq!(focused(&app), Some(ids.inside_first), "precondition");

    shift_tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_last),
        "Shift+Tab must wrap to the last control inside the trap, not to {} \
         outside it",
        ids.before
    );
}

/// The text field inside the trap takes `FocusTarget::Input`, and Tab still
/// wraps out of it.
///
/// This is the case that rules out `register_focus_target` + `FocusEntry::on_key`
/// — the arbiter offers a key to a registered target only while it holds
/// `FocusTarget::Node`, so a trap built that way would go dead the moment the
/// dialog contained a text field, which is most dialogs.
#[test]
fn a_focused_text_field_inside_the_trap_still_wraps() {
    let open = Signal::new(true);
    let (mut app, ids) = modal_page(open, true);

    tab(&mut app);
    tab(&mut app);
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.inside_middle),
        "the field must hold the keyboard as an Input target, not a Node one"
    );

    tab(&mut app);
    assert_eq!(focused(&app), Some(ids.inside_last));
    tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "the wrap must still happen with an Input target in the cycle"
    );
}

/// Focus starting *outside* an open trap is drawn into it, at its first control
/// for Tab and its last for Shift+Tab.
///
/// That is the state every dialog opens in — nothing inside it is focused yet —
/// and it is what rule 2 of `tab_trap_root` (the last trap in pre-order) is for,
/// since rule 1 finds nothing to walk up from.
#[test]
fn tab_from_outside_an_open_trap_enters_it() {
    for (shift, expected) in [(false, 0usize), (true, 2usize)] {
        let open = Signal::new(true);
        let (mut app, ids) = modal_page(open, true);
        let stops = [ids.inside_first, ids.inside_middle, ids.inside_last];

        // Claim something outside the dialog first, the way a page that opened
        // the modal from a button leaves it.
        app.focus_element(ids.before);
        assert_eq!(focused(&app), Some(ids.before), "precondition");

        if shift {
            shift_tab(&mut app);
        } else {
            tab(&mut app);
        }
        assert_eq!(
            focused(&app),
            Some(stops[expected]),
            "shift: {shift} — Tab from outside must enter the trap"
        );
    }
}

// ── 2. when there is no trap ────────────────────────────────────────────────

/// `trap_focus: false` leaves Tab to the whole document.
///
/// **Mutant: the attribute written unconditionally** (dropping
/// `arm_trap_focus`'s `if !trap_focus { return }`). The tour then never leaves
/// the modal and `before`/`after` are unreachable — the prop's off switch is
/// what this pins, and without it a "fix" that traps every overlay passes every
/// other test in this file.
#[test]
fn trap_focus_false_leaves_tab_to_the_whole_document() {
    let open = Signal::new(true);
    let (mut app, ids) = modal_page(open, false);

    let tour = tab_tour(&mut app, 5);
    assert_eq!(
        tour,
        vec![
            Some(ids.before),
            Some(ids.inside_first),
            Some(ids.inside_middle),
            Some(ids.inside_last),
            Some(ids.after),
        ],
        "with the prop off, Tab must walk the document in DOM order"
    );
}

/// A closed-but-mounted modal traps nothing, and Tab reaches the page.
///
/// `Modal::render` runs *once* — `opened_fn` only toggles a class — so a closed
/// modal stays mounted with its root in the tree. Note this fixture alone kills
/// no single *guard*, because a closed modal is protected twice over (the
/// attribute is removed *and* the root is `display: none`); the two guards have
/// a fixture each below. What it does pin is the whole round trip, including
/// that Tab does not simply go **dead** while a closed dialog is mounted.
///
/// **The reopen step needs Shift+Tab to say anything at all.** A plain Tab from
/// `before` lands on `inside_first` either way, because `inside_first` is also
/// the next stop in plain *document* order — the classic fixed point, and the
/// reason this fixture used to be one of the four M1 (the whole-tree walk) does
/// not kill. Shift+Tab from `before` separates them: with the trap live there is
/// no current index inside the list, so it enters at the trap's **last** stop,
/// while an untrapped document wraps from index 0 to `after`.
#[test]
fn a_closed_but_mounted_modal_traps_nothing_and_traps_again_when_reopened() {
    let open = Signal::new(false);
    let (mut app, ids) = modal_page(open, true);

    let tour = tab_tour(&mut app, 3);
    assert_eq!(
        tour,
        vec![Some(ids.before), Some(ids.after), Some(ids.before)],
        "a closed modal's own controls have no box, and its root must trap \
         nothing"
    );

    open.set(true);
    settle(&mut app);
    tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_first),
        "and the trap is live again once it opens"
    );

    // The discriminating half: from a claim *outside* the reopened trap,
    // backwards.
    app.focus_element(ids.before);
    shift_tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inside_last),
        "Shift+Tab from outside the reopened trap enters it at its last stop; \
         an untrapped document would have wrapped to {}",
        ids.after
    );

    open.set(false);
    settle(&mut app);
    let tour = tab_tour(&mut app, 2);
    assert_eq!(
        tour,
        vec![Some(ids.before), Some(ids.after)],
        "and gone again when it closes"
    );
}

/// Closing the modal **removes** the attribute rather than writing a falsey
/// value into it.
///
/// **Mutant: `set_attribute` in place of `write_attribute`** in
/// `arm_trap_focus`. The attribute then reads `"false"` on a closed overlay
/// instead of being absent. Both readers honour rinch's `"false"` escape, so
/// nothing else in this file notices — which is exactly why the *shape* of the
/// write is asserted directly. Removal is the contract; the escape is the
/// backstop.
#[test]
fn closing_removes_the_attribute_rather_than_writing_false() {
    let open = Signal::new(true);
    let (mut app, ids) = modal_page(open, true);

    let attr = |app: &RinchApp| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(ids.root)
            .unwrap()
            .attributes
            .get("data-trap-focus")
            .cloned()
    };

    assert_eq!(
        attr(&app).as_deref(),
        Some(""),
        "an open trap carries the bare presence form"
    );

    open.set(false);
    settle(&mut app);
    assert_eq!(
        attr(&app),
        None,
        "a closed trap must carry no attribute at all"
    );
}

// ── 3. the two guards, one fixture each ─────────────────────────────────────

/// A wrapper with a live `data-trap-focus` and no box is not a trap.
///
/// **Mutant: dropping `node_is_visible` from `tab_trap_root`.** Tab then dies
/// inside a `display: none` subtree — the collector finds nothing focusable in
/// it, `handle_tab` returns on the empty list, and the keyboard is stuck for the
/// rest of the session. This is the failure mode a real `Modal` is saved from by
/// its *other* guard, so it needs a fixture that keeps the attribute on purpose.
#[test]
fn an_invisible_trap_is_not_a_trap() {
    let ids: Rc<Cell<(usize, usize)>> = Rc::new(Cell::new((0, 0)));
    let out = ids.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let visible = button(scope, "visible");
        page.append_child(&visible);

        let hidden_trap = scope.create_element("div");
        hidden_trap.set_attribute("data-trap-focus", "");
        hidden_trap.set_attribute("style", "display: none");
        let buried = button(scope, "buried");
        hidden_trap.append_child(&buried);
        page.append_child(&hidden_trap);

        out.set((visible.node_id().0, buried.node_id().0));
        page
    });
    let (visible, buried) = ids.get();

    tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(visible),
        "a boxless trap must be skipped, not entered (buried control is {buried})"
    );
}

/// `data-trap-focus="false"` is not a trap — rinch's own `"false"` escape,
/// spelled once as `rinch_core::dom::data_attr_is_on` and shared with
/// `data-nofocus`.
///
/// **Mutant: reading the attribute by bare `contains_key`**, which is how the
/// HTML boolean attributes are read since #612. That is the right rule for
/// `disabled`; it is the wrong one here, and this is the only value that can
/// tell the two apart.
///
/// `"0"` goes in beside it because it is the one value where the escape and the
/// *writer's* rule `attr_is_truthy` disagree: `"0"` is **on**, matching the web
/// selector, so a reader that drifted to `attr_is_truthy` would fail here and
/// nowhere else.
#[test]
fn the_false_escape_opts_out_and_zero_does_not() {
    for (value, traps) in [("false", false), ("FALSE", false), ("0", true)] {
        let ids: Rc<Cell<(usize, usize)>> = Rc::new(Cell::new((0, 0)));
        let out = ids.clone();
        let mut app = mount(move |scope: &mut RenderScope| {
            let page = scope.create_element("div");
            let outside = button(scope, "outside");
            page.append_child(&outside);

            let trap = scope.create_element("div");
            trap.set_attribute("data-trap-focus", value);
            trap.set_attribute("style", "display: block; width: 300px; height: 100px");
            let inside = button(scope, "inside");
            trap.append_child(&inside);
            page.append_child(&trap);

            out.set((outside.node_id().0, inside.node_id().0));
            page
        });
        let (outside, inside) = ids.get();

        tab(&mut app);
        let expected = if traps { inside } else { outside };
        assert_eq!(
            focused(&app),
            Some(expected),
            "data-trap-focus={value:?} should {} trap",
            if traps { "" } else { "not" }
        );
    }
}

// ── 4. nesting ──────────────────────────────────────────────────────────────

/// Node ids for the two-modal fixture.
#[derive(Clone, Copy, Default)]
struct Nested {
    outer_a: usize,
    outer_b: usize,
    inner_a: usize,
    inner_b: usize,
}

/// An inner `Modal` mounted inside an outer one's body, each with two controls.
fn nested_modals(outer: Signal<bool>, inner: Signal<bool>) -> (RinchApp, Nested) {
    let ids: Rc<Cell<Nested>> = Rc::new(Cell::new(Nested::default()));
    let out = ids.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let inner_a = button(scope, "inner-a");
        let inner_b = button(scope, "inner-b");
        let inner_modal = Modal {
            opened_fn: Some(Rc::new(move || inner.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[inner_a.clone(), inner_b.clone()]);

        let outer_a = button(scope, "outer-a");
        let outer_b = button(scope, "outer-b");
        let outer_modal = Modal {
            opened_fn: Some(Rc::new(move || outer.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(
            scope,
            &[outer_a.clone(), inner_modal.clone(), outer_b.clone()],
        );

        out.set(Nested {
            outer_a: outer_a.node_id().0,
            outer_b: outer_b.node_id().0,
            inner_a: inner_a.node_id().0,
            inner_b: inner_b.node_id().0,
        });
        outer_modal
    });
    let ids = ids.get();
    (app, ids)
}

/// With both open and nothing focused, the **innermost** trap wins.
///
/// **Mutant: `first` instead of `last` in `tab_trap_root`'s pre-order fallback.**
/// Tab then enters the outer dialog while a modal sits on top of it. A modal
/// opened from a modal is rendered deeper, so later, which is what makes DOM
/// order stand in for stacking order here — and why this rule needs no
/// `z-index` read, matching the Escape dismiss stack's LIFO.
#[test]
fn with_two_traps_open_and_nothing_focused_the_innermost_wins() {
    let outer = Signal::new(true);
    let inner = Signal::new(true);
    let (mut app, ids) = nested_modals(outer, inner);

    let tour = tab_tour(&mut app, 3);
    assert_eq!(
        tour,
        vec![Some(ids.inner_a), Some(ids.inner_b), Some(ids.inner_a)],
        "the inner modal's two controls are the whole cycle; outer are {} and {}",
        ids.outer_a,
        ids.outer_b
    );
}

/// Focus inside the inner trap stays inside it — and once the inner closes, the
/// **outer** traps again, with focus left exactly where it was.
///
/// **This test kills no mutant on its own, and that is measured rather than
/// assumed.** Deleting rule 1 — the walk up from the current claim — leaves it
/// green: with the inner open the inner is also the last visible trap, and with
/// the inner closed the outer is, so the pre-order fallback answers both halves
/// correctly by itself. Rule 1's own discriminating case is a *sibling* pair,
/// which `focus_inside_one_of_two_sibling_traps_stays_in_that_one` covers.
///
/// It stays because it is the user-facing round trip #474 is about, and because
/// it pins something no other fixture does: an overlay closing leaves the claim
/// exactly where it was **until the runtime takes its next turn**, so
/// containment has to recover around a claim pointing into a subtree that no
/// longer has a box. Since #695 that turn does arrive (the close parks a focus
/// request), and the last assertion here is what it does when Tab has already
/// moved the claim on: nothing.
#[test]
fn closing_a_nested_trap_hands_containment_back_to_the_outer_one() {
    let outer = Signal::new(true);
    let inner = Signal::new(true);
    let (mut app, ids) = nested_modals(outer, inner);

    tab(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.inner_a),
        "precondition: inside inner"
    );

    inner.set(false);
    settle(&mut app);
    // Focus is still inside the closed subtree until the runtime takes its next
    // turn — the request is parked, not applied — and containment already has
    // to be right in that window.
    assert_eq!(
        focused(&app),
        Some(ids.inner_a),
        "the claim is untouched until the parked request is applied"
    );

    let tour = tab_tour(&mut app, 3);
    assert_eq!(
        tour,
        vec![Some(ids.outer_a), Some(ids.outer_b), Some(ids.outer_a)],
        "the outer modal traps again"
    );

    // And the turn the running app would take on that same signal write (#695)
    // changes nothing here, because Tab has moved the claim since: the release
    // the inner overlay parked names `inner_a`, which no longer holds the
    // keyboard, so it is refused rather than applied to whoever does. Where the
    // claim has *not* moved, it is applied — `overlay_focus_tests` asserts both
    // halves.
    turn(&mut app);
    assert_eq!(
        focused(&app),
        Some(ids.outer_a),
        "a parked release must not take the keyboard off the trap that has it"
    );
}

/// Two **sibling** traps open at once: whichever one holds the keyboard keeps
/// it, and Tab does not jump between them.
///
/// **Mutant: rule 1 deleted** — `tab_trap_root` falling straight through to the
/// pre-order fallback. The fallback answers "the last visible trap", which is
/// the second one, so Tab yanks the user out of the overlay they are typing in
/// and into an unrelated one. This is the **only** shape where the two rules
/// disagree in a way a fixture can see: nested traps agree (the inner is both
/// the nearest ancestor and the last in pre-order), and a single trap agrees
/// trivially. Without this test the ancestor walk is dead weight that no
/// assertion in the file touches — it survived the whole first mutation pass.
///
/// Sibling traps are not exotic: a `Drawer` and a `Popover { trap_focus: true }`
/// elsewhere on the page are siblings, and neither contains the other.
///
/// The second half is the complement: with **nothing** focused there is no
/// claim to walk up from, so the fallback decides and the later trap wins. That
/// is what keeps rule 1 from quietly becoming "the first trap always".
#[test]
fn focus_inside_one_of_two_sibling_traps_stays_in_that_one() {
    #[derive(Clone, Copy, Default)]
    struct Pair {
        a1: usize,
        a2: usize,
        b1: usize,
        b2: usize,
    }

    let build = || {
        let ids: Rc<Cell<Pair>> = Rc::new(Cell::new(Pair::default()));
        let out = ids.clone();
        let app = mount(move |scope: &mut RenderScope| {
            let page = scope.create_element("div");
            let trap = |scope: &mut RenderScope, first: &str, second: &str| {
                let wrapper = scope.create_element("div");
                wrapper.set_attribute("data-trap-focus", "");
                wrapper.set_attribute("style", "display: block; width: 400px; height: 120px");
                let one = button(scope, first);
                let two = button(scope, second);
                wrapper.append_child(&one);
                wrapper.append_child(&two);
                page.append_child(&wrapper);
                (one.node_id().0, two.node_id().0)
            };
            let (a1, a2) = trap(scope, "a1", "a2");
            let (b1, b2) = trap(scope, "b1", "b2");
            out.set(Pair { a1, a2, b1, b2 });
            page
        });
        (app, ids.get())
    };

    // Focus is claimed through the arbiter rather than by a click, because two
    // full-width traps stacked in flow make "which one did the pointer hit"
    // a second question this test is not asking.
    let (mut app, ids) = build();
    app.focus_element(ids.a1);
    assert_eq!(
        focused(&app),
        Some(ids.a1),
        "precondition: inside the first"
    );

    let tour = tab_tour(&mut app, 2);
    assert_eq!(
        tour,
        vec![Some(ids.a2), Some(ids.a1)],
        "Tab must stay in the trap holding the keyboard; the other holds {} and {}",
        ids.b1,
        ids.b2
    );

    let (mut app, ids) = build();
    let tour = tab_tour(&mut app, 2);
    assert_eq!(
        tour,
        vec![Some(ids.b1), Some(ids.b2)],
        "with no claim to walk up from, the later trap wins"
    );
}
