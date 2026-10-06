//! What a change to a `Stepper`'s step list costs, as a count (issue #748).
//!
//! A step that arrives or leaves renumbers the steps behind it and can restate
//! them, so `Stepper` re-derives on every such change (issues #716, #745). It
//! used to re-derive **every** step each time: about twenty attribute reads and
//! two `class` writes per step per change, whether or not the step had moved,
//! which made growing or shrinking a stepper one step at a time quadratic in
//! everything it did. A step this stepper has already settled at the position
//! it still holds now costs [`READS_PER_SETTLED_STEP`] attribute reads and no
//! write; only the steps a change actually moves are derived again.
//!
//! **Counts, not timings**, read off `MockDomDocument`'s per-kind call counters,
//! and each one is taken at two sizes: a cost that does not depend on the
//! stepper's size is the same number at both, and one that is linear in the
//! untouched steps differs by exactly the per-step figure.
//!
//! **Off the fixed points.** `active` sits in the middle of every stepper here,
//! so the untouched steps are a mix of completed, in progress and inactive; the
//! steppers carry an `on_step_click`, so the clickability half of the pass runs
//! too; and each fixture checks what the steps *say* after the change, so a pass
//! that got cheap by deriving nothing fails beside the count.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{
    NodeHandle, RenderScope,
    mock::{MockDomDocument, MockOpCounts},
};
use rinch_core::{Component, ValueCallback};

mod common;
use common::{collect_by_class, find_by_class, has_class};

/// What re-deriving costs for a step that is already settled where it stands:
/// its `class` (is this a step?), who settled it, and at which position.
const READS_PER_SETTLED_STEP: usize = 3;

struct Tree {
    doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    /// A stepper of `n` plain steps with `active` in the middle.
    fn stepper(n: usize) -> Self {
        Self::stepper_with(n, (n / 2) as u32)
    }

    fn stepper_with(n: usize, active: u32) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
        let steps: Vec<NodeHandle> = (0..n)
            .map(|_| StepperStep::default().render(&mut scope, &[]))
            .collect();
        let root = Stepper {
            active,
            on_step_click: Some(ValueCallback::new(|_: u32| {})),
            ..Default::default()
        }
        .render(&mut scope, &steps);
        Self { doc, scope, root }
    }

    fn container(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-stepper__steps").expect("the stepper has a steps row")
    }

    fn steps(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.root, "rinch-stepper__step", &mut out);
        out
    }

    fn new_step(&mut self) -> NodeHandle {
        StepperStep::default().render(&mut self.scope, &[])
    }

    /// The calls `change` makes.
    fn cost(&self, change: impl FnOnce()) -> MockOpCounts {
        let before = self.doc.borrow().__op_counts();
        change();
        self.doc.borrow().__op_counts().since(before)
    }

    /// `(data-step, state, drawn number or "-")` per step, in order.
    fn says(&self) -> Vec<(String, &'static str, String)> {
        self.steps()
            .iter()
            .map(|step| {
                let state = if has_class(step, "rinch-stepper__step--completed") {
                    "completed"
                } else if has_class(step, "rinch-stepper__step--progress") {
                    "progress"
                } else {
                    "inactive"
                };
                let icon_box = find_by_class(step, "rinch-stepper__step-icon").expect("icon box");
                let drawn = icon_box.text_content().unwrap_or_default();
                (
                    step.get_attribute("data-step").unwrap_or_default(),
                    state,
                    if drawn.is_empty() { "-".into() } else { drawn },
                )
            })
            .collect()
    }
}

/// What a stepper of `n` plain steps on `active` should say.
fn expected(n: usize, active: u32) -> Vec<(String, &'static str, String)> {
    (0..n as u32)
        .map(|i| {
            let (state, drawn) = match i.cmp(&active) {
                // A completed step with no icon of its own draws the tick, which
                // is an SVG and has no text.
                std::cmp::Ordering::Less => ("completed", "-".to_string()),
                std::cmp::Ordering::Equal => ("progress", (i + 1).to_string()),
                std::cmp::Ordering::Greater => ("inactive", (i + 1).to_string()),
            };
            (i.to_string(), state, drawn)
        })
        .collect()
}

const SMALL: usize = 10;
const LARGE: usize = 40;

#[test]
fn appending_a_step_writes_nothing_to_the_steps_it_does_not_move() {
    let mut costs = Vec::new();
    for n in [SMALL, LARGE] {
        let mut tree = Tree::stepper(n);
        let step = tree.new_step();
        let container = tree.container();
        let cost = tree.cost(|| container.append_child(&step));
        assert_eq!(
            tree.says(),
            expected(n + 1, (n / 2) as u32),
            "the appended step is numbered and stated from where it landed"
        );
        costs.push(cost);
    }

    assert_eq!(
        costs[0].writes(),
        costs[1].writes(),
        "#748: an appended step moves no other step, so what the pass writes is \
         what the one new step needs, whatever the stepper's size: {SMALL} steps \
         {:?}, {LARGE} steps {:?}",
        costs[0],
        costs[1]
    );
    assert!(
        costs[0].writes() > 0,
        "positive control: the new step itself is written"
    );
    assert_eq!(
        costs[1].reads() - costs[0].reads(),
        (LARGE - SMALL) * READS_PER_SETTLED_STEP,
        "#748: each step already settled where it stands costs \
         {READS_PER_SETTLED_STEP} reads: {SMALL} steps {:?}, {LARGE} steps {:?}",
        costs[0],
        costs[1]
    );
}

#[test]
fn removing_the_last_step_writes_nothing_at_all() {
    let mut costs = Vec::new();
    for n in [SMALL, LARGE] {
        let tree = Tree::stepper(n);
        let last = tree.steps().pop().expect("a last step");
        let cost = tree.cost(|| last.remove());
        assert_eq!(tree.says(), expected(n - 1, (n / 2) as u32));
        costs.push(cost);
    }

    for cost in &costs {
        assert_eq!(
            cost.writes(),
            0,
            "#748: no step is behind the one that left, so none moved: {cost:?}"
        );
    }
    assert_eq!(
        costs[1].reads() - costs[0].reads(),
        (LARGE - SMALL) * READS_PER_SETTLED_STEP,
        "#748: {SMALL} steps {:?}, {LARGE} steps {:?}",
        costs[0],
        costs[1]
    );
}

#[test]
fn a_step_inserted_in_the_middle_derives_only_the_steps_behind_it() {
    // The insertion point is fixed and the stepper grows *in front of* it, so
    // the steps the insertion moves are the same four at both sizes and the
    // difference between the two costs is the untouched steps alone.
    const BEHIND: usize = 4;
    let mut costs = Vec::new();
    for n in [SMALL, LARGE] {
        let active = (n - 2) as u32;
        let mut tree = Tree::stepper_with(n, active);
        let step = tree.new_step();
        let container = tree.container();
        let reference = tree.steps()[n - BEHIND].clone();
        let cost = tree.cost(|| container.insert_before(&step, &reference));
        assert_eq!(
            tree.says(),
            expected(n + 1, active),
            "every step behind the insertion is renumbered and restated"
        );
        costs.push(cost);
    }

    assert_eq!(
        costs[0].writes(),
        costs[1].writes(),
        "#748: the same four steps moved at both sizes: {:?} against {:?}",
        costs[0],
        costs[1]
    );
    assert_eq!(
        costs[1].reads() - costs[0].reads(),
        (LARGE - SMALL) * READS_PER_SETTLED_STEP,
        "#748: {:?} against {:?}",
        costs[0],
        costs[1]
    );
}

#[test]
fn a_step_inserted_in_front_still_derives_every_step_behind_it() {
    // The control for the three fixtures above: a pass that skipped a step
    // because it had been settled *once*, rather than because it still stands
    // where it was settled, is as cheap as they ask and wrong here.
    let n = SMALL;
    let active = (n / 2) as u32;
    let mut tree = Tree::stepper_with(n, active);
    let step = tree.new_step();
    let container = tree.container();
    let first = tree.steps()[0].clone();
    let cost = tree.cost(|| container.insert_before(&step, &first));

    assert_eq!(tree.says(), expected(n + 1, active));
    assert!(
        cost.writes() >= n,
        "every one of the {n} steps moved, and each is at least renumbered: {cost:?}"
    );
}

/// What renumbering costs a plain step that keeps its state: its position, its
/// index, and the number it draws.
const WRITES_PER_RENUMBERED_STEP: usize = 3;

#[test]
fn a_step_renumbered_without_being_restated_keeps_its_class() {
    // On step 1, a step put in front moves every step back by one; all but the
    // two around `active` were inactive and still are. Those are the steps the
    // two sizes differ by.
    let mut costs = Vec::new();
    for n in [SMALL, LARGE] {
        let mut tree = Tree::stepper_with(n, 1);
        let step = tree.new_step();
        let container = tree.container();
        let first = tree.steps()[0].clone();
        let cost = tree.cost(|| container.insert_before(&step, &first));
        assert_eq!(tree.says(), expected(n + 1, 1));
        costs.push(cost);
    }

    assert_eq!(
        costs[1].writes() - costs[0].writes(),
        (LARGE - SMALL) * WRITES_PER_RENUMBERED_STEP,
        "#748: a step that is only renumbered has its `class` left alone: \
         {SMALL} steps {:?}, {LARGE} steps {:?}",
        costs[0],
        costs[1]
    );
}

#[test]
fn a_step_moved_in_from_another_stepper_at_the_same_position_is_derived_again() {
    // Position alone does not say a step is settled: this one stood at position
    // 1 of a stepper on step 0, where it was inactive, and lands at position 1
    // of a stepper on step 3, where it is completed.
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
    let stepper = |active: u32, scope: &mut RenderScope| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        Stepper {
            active,
            ..Default::default()
        }
        .render(scope, &steps)
    };
    let from = stepper(0, &mut scope);
    let into = stepper(3, &mut scope);

    let steps_of = |root: &NodeHandle| {
        let mut out = Vec::new();
        collect_by_class(root, "rinch-stepper__step", &mut out);
        out
    };
    let moving = steps_of(&from)[1].clone();
    assert!(
        has_class(&moving, "rinch-stepper__step--inactive"),
        "precondition: inactive where it starts"
    );
    let reference = steps_of(&into)[1].clone();
    let container = find_by_class(&into, "rinch-stepper__steps").expect("steps row");
    container.insert_before(&moving, &reference);

    assert_eq!(
        steps_of(&into)[1].node_id(),
        moving.node_id(),
        "precondition: it landed at position 1, the position it left"
    );
    assert!(
        has_class(&moving, "rinch-stepper__step--completed")
            && !has_class(&moving, "rinch-stepper__step--inactive"),
        "the receiving stepper is on step 3, so its step 1 is completed; it reads {:?}",
        moving.get_attribute("class")
    );
}

#[test]
fn growing_a_stepper_one_step_at_a_time_writes_in_proportion_to_its_size() {
    let grow = |n: usize| {
        let mut tree = Tree::stepper_with(0, 3);
        let container = tree.container();
        let mut total = MockOpCounts::default();
        for _ in 0..n {
            let step = tree.new_step();
            let cost = tree.cost(|| container.append_child(&step));
            total.attribute_writes += cost.attribute_writes;
            total.text_writes += cost.text_writes;
        }
        assert_eq!(tree.says(), expected(n, 3));
        total.writes()
    };

    // The first four steps are the completed and in-progress ones, which cost
    // more than an inactive one; past them every appended step costs the same.
    let (a, b, c) = (grow(10), grow(20), grow(40));
    assert_eq!(
        c - b,
        2 * (b - a),
        "#748: twenty more steps cost twice what ten more do — {a}, {b}, {c} \
         writes for 10, 20 and 40 steps"
    );
}
