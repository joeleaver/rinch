//! A wired step moved to another `Stepper` answers to the stepper it is in —
//! issue #1428.
//!
//! `settle_step_clickability` registers a step's click handler once and records
//! it on the step, so a later pass restores `data-rid` from the record instead
//! of registering again. The record used to be a bare handler id, and the
//! handler closed over the `on_step_click` of the stepper that registered it:
//! a step moved from stepper A to stepper B was wired, at its position in B, to
//! A's callback, and B's was never called. A step moved into a stepper with no
//! `on_step_click` lost its `data-rid` but kept the clickable class A's wiring
//! had granted it.
//!
//! Every fixture moves a step to a position other than the one it left, and
//! the two steppers' callbacks record their own name, so "the right callback"
//! and "the right position" are separate facts.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, ValueCallback};

mod common;
use common::{find_by_class, has_class};

const CLICKABLE: &str = "rinch-stepper__step--clickable";

type Hits = Rc<RefCell<Vec<(&'static str, u32)>>>;

struct Fixture {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    hits: Hits,
}

impl Fixture {
    fn new() -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let scope = RenderScope::new(doc.clone(), body);
        Self {
            _doc: doc,
            scope,
            hits: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A stepper of three plain steps. `name` is what its `on_step_click`
    /// records; `None` gives it no callback at all.
    fn stepper(&mut self, name: Option<&'static str>, props: Stepper) -> NodeHandle {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(&mut self.scope, &[]))
            .collect();
        let hits = self.hits.clone();
        Stepper {
            on_step_click: name
                .map(|name| ValueCallback::new(move |p: u32| hits.borrow_mut().push((name, p)))),
            ..props
        }
        .render(&mut self.scope, &steps)
    }

    fn take_hits(&self) -> Vec<(&'static str, u32)> {
        std::mem::take(&mut *self.hits.borrow_mut())
    }
}

fn steps_of(stepper: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        if has_class(node, "rinch-stepper__step") {
            out.push(node.clone());
            return;
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(stepper, &mut out);
    out
}

/// Move `step` into `stepper`, in front of the step now at `at`.
fn move_into(step: &NodeHandle, stepper: &NodeHandle, at: usize) {
    let container = find_by_class(stepper, "rinch-stepper__steps").expect("steps container");
    let before = steps_of(stepper)[at].clone();
    container.insert_before(step, &before);
}

/// Click `step` as the runtime does: `None` when it carries no `data-rid`.
fn click(step: &NodeHandle) -> Option<bool> {
    let rid = step.get_attribute("data-rid")?;
    let id: usize = rid.parse().expect("data-rid is a handler id");
    Some(dispatch_event(EventHandlerId(id)))
}

fn all_reachable() -> Stepper {
    Stepper {
        active: 9,
        ..Default::default()
    }
}

#[test]
fn a_step_moved_to_another_stepper_calls_that_steppers_callback() {
    let mut f = Fixture::new();
    let a = f.stepper(Some("A"), all_reachable());
    let b = f.stepper(Some("B"), all_reachable());

    // Positive control: before the move the step is A's, at position 0.
    let moving = steps_of(&a)[0].clone();
    assert_eq!(click(&moving), Some(true));
    assert_eq!(f.take_hits(), vec![("A", 0)]);

    // A's first step becomes B's third (of four).
    move_into(&moving, &b, 2);
    assert_eq!(
        moving.get_attribute("data-step-position").as_deref(),
        Some("2")
    );

    assert_eq!(click(&moving), Some(true), "still wired, in B");
    assert_eq!(
        f.take_hits(),
        vec![("B", 2)],
        "the step is in B: B's callback, with its position in B, and A's not at all"
    );

    // The steps that did not move still answer to their own steppers.
    assert_eq!(click(&steps_of(&a)[0]), Some(true));
    assert_eq!(click(&steps_of(&b)[0]), Some(true));
    assert_eq!(f.take_hits(), vec![("A", 0), ("B", 0)]);
}

#[test]
fn a_step_moved_back_calls_the_first_stepper_again_with_the_handler_it_had() {
    let mut f = Fixture::new();
    let a = f.stepper(Some("A"), all_reachable());
    let b = f.stepper(Some("B"), all_reachable());

    let moving = steps_of(&a)[0].clone();
    let rid_in_a = moving.get_attribute("data-rid").expect("wired in A");

    move_into(&moving, &b, 2);
    let rid_in_b = moving.get_attribute("data-rid").expect("wired in B");
    assert_ne!(rid_in_a, rid_in_b, "B wires its own handler");

    // Back into A, as its second step, and round again.
    move_into(&moving, &a, 1);
    assert_eq!(click(&moving), Some(true));
    assert_eq!(f.take_hits(), vec![("A", 1)]);
    assert_eq!(
        moving.get_attribute("data-rid").as_deref(),
        Some(rid_in_a.as_str()),
        "A registered a handler for this step already, and uses it again"
    );

    move_into(&moving, &b, 1);
    assert_eq!(click(&moving), Some(true));
    assert_eq!(f.take_hits(), vec![("B", 1)]);
    assert_eq!(
        moving.get_attribute("data-rid").as_deref(),
        Some(rid_in_b.as_str()),
        "and so does B: a step bounced between two steppers costs each one handler"
    );
}

#[test]
fn a_step_moved_into_a_stepper_with_no_callback_stops_being_clickable() {
    let mut f = Fixture::new();
    let a = f.stepper(Some("A"), all_reachable());
    // B has no callback; `active: 9` would make the step reachable if it had.
    let b = f.stepper(None, all_reachable());

    let moving = steps_of(&a)[0].clone();
    assert!(has_class(&moving, CLICKABLE), "control: clickable in A");
    assert!(moving.get_attribute("data-rid").is_some());

    move_into(&moving, &b, 2);

    assert_eq!(click(&moving), None, "no data-rid: nothing to click");
    assert!(f.take_hits().is_empty());
    assert_eq!(moving.get_attribute("tabindex"), None);
    assert_eq!(moving.get_attribute("role"), None);
    assert!(
        !has_class(&moving, CLICKABLE),
        "the class was A's wiring's, and B wires nothing"
    );

    // Back in A it is A's again.
    move_into(&moving, &a, 1);
    assert!(has_class(&moving, CLICKABLE));
    assert_eq!(click(&moving), Some(true));
    assert_eq!(f.take_hits(), vec![("A", 1)]);
}

#[test]
fn a_callbackless_stepper_keeps_the_class_it_grants_itself_on_a_moved_step() {
    let mut f = Fixture::new();
    let a = f.stepper(Some("A"), all_reachable());
    // No callback, but `allow_next_steps_select` grants the class (and only
    // the class) to every step past `active`.
    let b = f.stepper(
        None,
        Stepper {
            active: 0,
            allow_next_steps_select: true,
            ..Default::default()
        },
    );

    let moving = steps_of(&a)[0].clone();
    move_into(&moving, &b, 2);
    assert!(
        has_class(&moving, CLICKABLE),
        "position 2 > active 0 under allow_next_steps_select: B's own decorative grant"
    );
    assert_eq!(click(&moving), None);

    // A step no stepper ever wired is outside this change: B's own third step
    // got the class from B's grant at position 3, and keeps it when the steps
    // in front of it go and it lands on `active` — what a callback-less
    // stepper did before #1428, and still does.
    let never_wired = steps_of(&b)[3].clone();
    assert!(has_class(&never_wired, CLICKABLE));
    for step in steps_of(&b).into_iter().take(3) {
        step.remove();
    }
    assert_eq!(
        never_wired.get_attribute("data-step-position").as_deref(),
        Some("0")
    );
    assert!(
        has_class(&never_wired, CLICKABLE),
        "only a step some stepper wired has the class taken off"
    );

    // And a step that asked for the class itself keeps it wherever it goes.
    let own = StepperStep {
        allow_step_click: true,
        ..Default::default()
    }
    .render(&mut f.scope, &[]);
    let a_container = find_by_class(&a, "rinch-stepper__steps").unwrap();
    a_container.append_child(&own);
    assert_eq!(click(&own), Some(true), "control: wired in A");
    f.take_hits();

    let c = f.stepper(None, Stepper::default());
    move_into(&own, &c, 1);
    assert_eq!(click(&own), None);
    assert!(
        has_class(&own, CLICKABLE),
        "the step's own ask keeps the class"
    );
}
