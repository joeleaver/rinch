//! `Stepper::on_step_click` wires a real handler onto each step it considers
//! clickable — issue #737.
//!
//! Before this, `crates/rinch-components/src/stepper.rs` contained no
//! `data-rid`, no `register_handler` and no callback prop of any kind: the
//! `rinch-stepper__step--clickable` class bought a cursor and a hover opacity
//! and nothing else, so a step could not be clicked *to do anything*.
//!
//! The fix adds [`Stepper::on_step_click`] (`Option<ValueCallback<u32>>`,
//! fired with the step's 0-based position) and wires it during the same
//! `settle_steps` pass that already derives each step's state and index: a
//! step at or before `active` is clickable without `allow_next_steps_select`
//! (Mantine's default), a step past it needs that flag or its own
//! `allow_step_click`/`allow_step_select`, and `StepperStep::disabled` always
//! wins over every grant. The handler reads its step's position back from the
//! DOM **at click time** rather than from a value captured when it was wired
//! (the #714 pattern), because a late insertion or removal can renumber a
//! step after its handler is registered (issues #716, #745).

use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, IntoEventHandler, Signal, ValueCallback};
use rinch_macros::rsx;

mod common;
use common::has_class;

// ------------------------------------------------------------------ helpers

/// A rendered tree, with the document and scope that own it kept alive.
struct Tree {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    fn build(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(&mut scope);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    fn steps(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }
}

fn collect(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
    if has_class(node, "rinch-stepper__step") {
        out.push(node.clone());
        return;
    }
    for child in node.children() {
        collect(&child, out);
    }
}

/// Click `step` the way the runtime would: read its `data-rid`, dispatch it.
///
/// `None` means the step carries no `data-rid` at all — not merely that
/// dispatch found no handler — which is the DOM-level fact the "not
/// clickable" fixtures below rest on: the click path the desktop and web
/// shells both use never reaches a node with none.
fn click(step: &NodeHandle) -> Option<bool> {
    let rid = step.get_attribute("data-rid")?;
    let id: usize = rid.parse().expect("data-rid is a handler id");
    Some(dispatch_event(EventHandlerId(id)))
}

fn three_default_steps(stepper: Stepper) -> Tree {
    Tree::build(move |scope| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        stepper.render(scope, &steps)
    })
}

// --------------------------------------------- (a) a click reaches the caller

#[test]
fn clicking_a_reachable_step_calls_on_step_click_with_its_position() {
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let tree = three_default_steps(Stepper {
        active: 1,
        on_step_click: Some(ValueCallback::new(move |i: u32| sink.borrow_mut().push(i))),
        ..Default::default()
    });

    let steps = tree.steps();
    assert!(
        click(&steps[0]).unwrap_or(false),
        "step 0 sits before `active: 1` (completed), which Mantine's default \
         makes clickable with no `allow_next_steps_select` — it must carry a \
         live `data-rid`"
    );
    assert_eq!(
        seen.borrow().as_slice(),
        &[0],
        "the callback is called with the step's own 0-based position"
    );

    click(&steps[1]).expect("the active step itself is reachable too");
    assert_eq!(
        seen.borrow().as_slice(),
        &[0, 1],
        "the current (`progress`) step is clickable too — Mantine's \"up to \
         active\" is inclusive"
    );
}

// -------------------------------------- (b) not reached yet, not clickable

#[test]
fn a_step_past_active_is_not_clickable_by_default() {
    // Off a fixed point on purpose: `active: 1` with 3 steps makes position 2
    // *two* past active's own position, so a mutant that compares against
    // `position - 1` or drops the boundary by one still fails this.
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let tree = three_default_steps(Stepper {
        active: 1,
        on_step_click: Some(ValueCallback::new(move |i: u32| sink.borrow_mut().push(i))),
        ..Default::default()
    });

    let steps = tree.steps();
    assert!(
        click(&steps[2]).is_none(),
        "step 2 is past `active: 1` with no `allow_next_steps_select` and no \
         own ask — issue #737 calls this exactly the bug: a step with no \
         handler must not even carry `data-rid`, let alone fire one"
    );
    assert!(
        seen.borrow().is_empty(),
        "and the callback was never reached"
    );
    assert!(
        !has_class(&steps[2], "rinch-stepper__step--clickable"),
        "nor does it get the cursor — unreachable means unreachable, not \
         merely unwired"
    );
}

#[test]
fn allow_next_steps_select_makes_the_later_step_reachable() {
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let tree = three_default_steps(Stepper {
        active: 1,
        allow_next_steps_select: true,
        on_step_click: Some(ValueCallback::new(move |i: u32| sink.borrow_mut().push(i))),
        ..Default::default()
    });

    let steps = tree.steps();
    assert!(click(&steps[2]).unwrap_or(false));
    assert_eq!(seen.borrow().as_slice(), &[2]);
}

#[test]
fn with_no_callback_at_all_nothing_is_wired_and_the_old_decorative_behaviour_is_unchanged() {
    // Same shape as `allow_next_steps_select_reaches_the_steps_past_the_active_one`
    // in `prop_wiring_707.rs`, which this must not regress: with no
    // `on_step_click`, `allow_next_steps_select` still grants only the class.
    let tree = three_default_steps(Stepper {
        active: 1,
        allow_next_steps_select: true,
        ..Default::default()
    });
    let steps = tree.steps();
    assert!(
        has_class(&steps[2], "rinch-stepper__step--clickable"),
        "the class is still decorative-only with no callback"
    );
    assert!(
        click(&steps[2]).is_none(),
        "but nothing is wired: no callback means no `data-rid`, exactly as \
         before #737"
    );
}

// ------------------------------------------------------- (c) `disabled` wins

#[test]
fn a_disabled_step_is_never_clickable_whatever_else_grants_it() {
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let tree = Tree::build(move |scope| {
        let steps: Vec<NodeHandle> = vec![
            StepperStep::default().render(scope, &[]),
            StepperStep {
                disabled: true,
                allow_step_click: true,
                ..Default::default()
            }
            .render(scope, &[]),
            StepperStep::default().render(scope, &[]),
        ];
        Stepper {
            active: 1,
            allow_next_steps_select: true,
            on_step_click: Some(ValueCallback::new(move |i: u32| sink.borrow_mut().push(i))),
            ..Default::default()
        }
        .render(scope, &steps)
    });

    let steps = tree.steps();
    assert!(
        click(&steps[1]).is_none(),
        "step 1 is `active` (reachable by default) and asked for its own \
         `allow_step_click` on top of that — two separate grants, and \
         `disabled` must still beat both"
    );
    assert!(seen.borrow().is_empty());
    assert!(
        !has_class(&steps[1], "rinch-stepper__step--clickable"),
        "no cursor either — a disabled step must not even look clickable"
    );
}

// --------------------------------------- (d) a late-arriving step (#716/#745)

#[test]
fn a_late_inserted_step_is_clickable_with_its_own_correct_position() {
    // `active: 9` holds every position in this test reachable, before and
    // after the insertion below — so a step's data-rid never comes and goes,
    // and the only thing that can discriminate correct code from a mutant
    // that bakes a step's position into its handler at *registration* time
    // is the value the handler fires, not whether it fires at all.
    let items = Signal::new(vec!["a"]);
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 9u32, on_step_click: move |i: u32| sink.borrow_mut().push(i),
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    // Before the insertion: one step, at position 0 — its handler is
    // registered here, with `captured_position` (a mutant's stand-in for
    // "baked in at registration") equal to 0.
    let before = tree.steps();
    assert_eq!(before.len(), 1);
    click(&before[0]).expect("every position is reachable under active: 9");
    assert_eq!(seen.borrow().as_slice(), &[0]);
    seen.borrow_mut().clear();

    // Insert a second step *in front of* the first — #716's shape: the
    // insertion renumbers the step that used to be at position 0 to position
    // 1. Both positions stay reachable under `active: 9`, so the already-wired
    // step's `data-rid` is untouched — only its *position* moved, and the
    // handler has to follow it there rather than go on firing the index it
    // saw when it registered (issue #714's lookup-at-click-time pattern is
    // what this fixture pins).
    items.update(|v| v.insert(0, "z"));

    let steps = tree.steps();
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[1].get_attribute("data-step").as_deref(),
        Some("1"),
        "precondition: the old step 0 was renumbered to 1 by the insertion"
    );

    // The *new* step, at position 0 — never registered before, so a mutant
    // that bakes the position in at registration time still reads 0 here by
    // coincidence (it is registering for the first time, at the position it
    // already holds). This assertion alone would not discriminate.
    assert!(
        click(&steps[0]).unwrap_or(false),
        "the new front step is reachable"
    );
    assert_eq!(seen.borrow().as_slice(), &[0]);
    seen.borrow_mut().clear();

    // The step that *moved* — same handler (still reachable, so its data-rid
    // is untouched by the renumbering), new position. Correct code reads
    // `POSITION_ATTR` live and fires 1; a handler that captured its position
    // at registration time fires the stale 0 instead.
    assert!(
        click(&steps[1]).unwrap_or(false),
        "the moved step is still reachable under active: 9"
    );
    assert_eq!(
        seen.borrow().as_slice(),
        &[1],
        "the handler was registered once, back when this step sat at \
         position 0 — firing 1 here is only possible if it reads its current \
         position from the DOM at click time rather than from a value it \
         captured when it registered"
    );
}
