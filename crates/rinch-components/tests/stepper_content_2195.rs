//! `Stepper` shows the active step's content and, past the last step, its
//! `StepperCompleted` block (issue #2195).
//!
//! Before: every `StepperStep`'s children sat in a `.rinch-stepper__step-content`
//! that the stylesheet hid unconditionally, so no step's content was ever seen,
//! and a `StepperCompleted` rendered whatever `active` was. Mantine shows the
//! content of the step at `active` below the row of steps, and the completed
//! block only once `active` is past the last step. The stepper now moves the
//! active step's content into a `.rinch-stepper__content` region below the
//! steps, and gives that region a `--completed` modifier past the end; these
//! fixtures read the structure, and `crates/rinch/src/app/stepper_window_2195_tests.rs`
//! reads what the real stylesheet makes of it.
//!
//! **Off the fixed points:** three steps with `active: 1`, so neither the first
//! nor the last step is the answer, and every step carries content.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::SectionRenderer;
use rinch_components::stepper::{Stepper, StepperCompleted, StepperStep};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};

mod common;
use common::{collect_by_class, find_by_class, has_class};

struct Tree {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
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
            scope,
            root,
        }
    }

    fn region(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-stepper__content").expect("a content region")
    }

    fn steps_container(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-stepper__steps").expect("a steps container")
    }

    fn steps(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.steps_container(), "rinch-stepper__step", &mut out);
        out
    }

    /// The text of every marker under `node`, in document order.
    fn markers(node: &NodeHandle) -> Vec<String> {
        let mut out = Vec::new();
        collect_by_class(node, "marker", &mut out);
        out.iter()
            .map(|m| m.text_content().unwrap_or_default())
            .collect()
    }
}

/// A step whose content is one `.marker` holding `text`.
fn step(scope: &mut RenderScope, text: &str) -> NodeHandle {
    let marker = scope.create_element("div");
    marker.set_attribute("class", "marker");
    marker.append_child(&scope.create_text(text));
    StepperStep {
        label: text.to_string(),
        ..Default::default()
    }
    .render(scope, &[marker])
}

fn completed(scope: &mut RenderScope) -> NodeHandle {
    let marker = scope.create_element("div");
    marker.set_attribute("class", "marker");
    marker.append_child(&scope.create_text("done"));
    StepperCompleted.render(scope, &[marker])
}

fn stepper(scope: &mut RenderScope, active: u32, children: &[NodeHandle]) -> NodeHandle {
    Stepper {
        active,
        ..Default::default()
    }
    .render(scope, children)
}

#[test]
fn the_active_steps_content_is_shown_below_the_steps() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });

    let region = tree.region();
    assert_eq!(
        Tree::markers(&region),
        ["two"],
        "the region shows step 2's content"
    );

    // Below the steps: the region is the stepper's next child after them.
    let children = tree.root.children();
    let at = |class: &str| {
        children
            .iter()
            .position(|c| has_class(c, class))
            .unwrap_or_else(|| panic!("the stepper has a `{class}` child"))
    };
    assert_eq!(
        at("rinch-stepper__content"),
        at("rinch-stepper__steps") + 1,
        "the region comes right after the steps"
    );

    // The other steps keep theirs, inside their own (hidden) content wrapper.
    let steps = tree.steps();
    assert_eq!(Tree::markers(&steps[0]), ["one"]);
    assert_eq!(Tree::markers(&steps[1]), Vec::<String>::new());
    assert_eq!(Tree::markers(&steps[2]), ["three"]);
}

#[test]
fn a_step_inserted_in_front_hands_the_region_to_the_new_step_at_active() {
    let mut tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });

    let new = step(&mut tree.scope, "zero");
    let container = tree.steps_container();
    let first = tree.steps()[0].clone();
    container.insert_before(&new, &first);

    // `zero, one, two, three` with `active: 1` is `one`.
    assert_eq!(Tree::markers(&tree.region()), ["one"]);
    let steps = tree.steps();
    assert_eq!(
        Tree::markers(&steps[2]),
        ["two"],
        "step 2 got its content back"
    );
    // Back where it was: in front of the separator, after the body.
    let kids = steps[2].children();
    let content = kids
        .iter()
        .position(|c| has_class(c, "rinch-stepper__step-content"))
        .expect("step two has its content wrapper back");
    assert!(
        has_class(&kids[content + 1], "rinch-stepper__separator"),
        "the content wrapper went back in front of the separator"
    );
}

#[test]
fn removing_the_active_step_takes_its_content_with_it() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });

    let two = tree.steps()[1].clone();
    two.remove();

    assert_eq!(
        Tree::markers(&tree.region()),
        ["three"],
        "the step now at `active` is shown"
    );
    assert_eq!(
        Tree::markers(&two),
        ["two"],
        "the removed step carried its own content away"
    );
}

/// An app moves the active step **into its own shown content** (the extended
/// differential test's seed 7 found this). The content cannot go back into a
/// step that sits inside it: that made the content its own ancestor, and every
/// later tree walk spun for ever. It stays where it is, the step with it, and
/// the step now at `active` gets the region.
#[test]
fn a_step_moved_into_its_own_shown_content_makes_no_cycle() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });

    let two = tree.steps()[1].clone();
    let shown_marker = find_by_class(&tree.region(), "marker").expect("step two's marker");
    shown_marker.append_child(&two);

    // No node is its own ancestor.
    let mut seen = 0;
    let mut cur = Some(two.clone());
    while let Some(n) = cur {
        seen += 1;
        assert!(seen < 64, "the step's ancestor chain loops");
        cur = n.parent_node();
    }

    let region = tree.region();
    let in_region = |class: &str| {
        let mut out = Vec::new();
        collect_by_class(&region, class, &mut out);
        out.len()
    };
    assert_eq!(
        in_region("rinch-stepper__step-content"),
        2,
        "step two's content stays (holding step two), and `three`, now at `active`, is shown"
    );
    let steps = tree.steps();
    assert_eq!(steps.len(), 2, "step two left the stepper");
    assert_eq!(Tree::markers(&steps[0]), ["one"]);
    assert_eq!(Tree::markers(&steps[1]), Vec::<String>::new());
}

#[test]
fn the_completed_block_shows_only_past_the_last_step() {
    for (active, past_the_end) in [(1, false), (2, false), (3, true)] {
        let tree = Tree::build(|scope| {
            let kids = [
                step(scope, "one"),
                step(scope, "two"),
                step(scope, "three"),
                completed(scope),
            ];
            stepper(scope, active, &kids)
        });
        let region = tree.region();
        assert_eq!(
            has_class(&region, "rinch-stepper__content--completed"),
            past_the_end,
            "active {active}: the region says whether the completed block shows"
        );
        // The block lives in the region either way (it is not left as the
        // last step's sibling), and the region holds a step's content only
        // while a step is active.
        let expected: &[&str] = match active {
            1 => &["two", "done"],
            2 => &["three", "done"],
            _ => &["done"],
        };
        assert_eq!(Tree::markers(&region), expected, "active {active}");
        assert!(
            find_by_class(&tree.steps_container(), "rinch-stepper__completed").is_none(),
            "active {active}: no completed block is left among the steps"
        );
    }
}

#[test]
fn label_description_and_icon_take_nodes() {
    let node = |text: &'static str| -> SectionRenderer {
        Rc::new(move |scope: &mut RenderScope| {
            let el = scope.create_element("b");
            el.set_attribute("class", text);
            el
        })
    };
    let tree = Tree::build(|scope| {
        let kids: Vec<NodeHandle> = (0..3)
            .map(|_| {
                StepperStep {
                    label: "ignored".into(),
                    label_section: Some(node("custom-label")),
                    description_section: Some(node("custom-description")),
                    icon_section: Some(node("custom-icon")),
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        stepper(scope, 1, &kids)
    });
    let steps = tree.steps();
    for (i, s) in steps.iter().enumerate() {
        let label = find_by_class(s, "rinch-stepper__step-label").expect("a label");
        assert!(find_by_class(&label, "custom-label").is_some(), "step {i}");
        assert_eq!(
            label.text_content().unwrap_or_default(),
            "",
            "step {i}: the node replaces the text"
        );
        let desc = find_by_class(s, "rinch-stepper__step-description").expect("a description");
        assert!(
            find_by_class(&desc, "custom-description").is_some(),
            "step {i}"
        );
    }
    // The icon fills the number's slot: drawn live by the progress and the
    // inactive step, parked (hidden) behind the tick by the completed one.
    let live_icon = |s: &NodeHandle| {
        let icon_box = find_by_class(s, "rinch-stepper__step-icon").unwrap();
        icon_box
            .children()
            .into_iter()
            .find(|c| !has_class(c, "rinch-stepper__step-icon-alt"))
            .unwrap()
    };
    assert!(
        !has_class(&live_icon(&steps[0]), "custom-icon"),
        "completed draws the tick"
    );
    assert!(
        has_class(&live_icon(&steps[1]), "custom-icon"),
        "progress draws the icon"
    );
    assert!(
        has_class(&live_icon(&steps[2]), "custom-icon"),
        "inactive draws the icon"
    );
    assert_eq!(
        live_icon(&steps[2]).text_content().unwrap_or_default(),
        "",
        "no number is written into it"
    );
}
