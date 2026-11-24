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
    doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    fn build(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(&mut scope);
        Self { doc, scope, root }
    }

    /// Whether the document has let go of `node` (it was discarded).
    fn retired(&self, node: &NodeHandle) -> bool {
        self.doc.borrow().is_retired(node.node_id())
    }

    /// The `.rinch-stepper__step-content` wrapper holding the marker `text`.
    fn content_of(&self, text: &str) -> NodeHandle {
        let mut markers = Vec::new();
        collect_by_class(&self.root, "marker", &mut markers);
        markers
            .into_iter()
            .find(|m| m.text_content().as_deref() == Some(text))
            .and_then(|m| m.parent_node())
            .unwrap_or_else(|| panic!("a content wrapper holding `{text}`"))
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

fn parent_id(node: &NodeHandle) -> Option<rinch_core::dom::NodeId> {
    node.parent_node().map(|p| p.node_id())
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
    // Back where it was: the last child of the step's body, under the label.
    let body = steps[2]
        .children()
        .into_iter()
        .find(|c| has_class(c, "rinch-stepper__step-body"))
        .expect("step two has its body");
    let kids = body.children();
    assert!(
        kids.last()
            .is_some_and(|c| has_class(c, "rinch-stepper__step-content")),
        "the content wrapper went back to the end of the step's body"
    );
    assert!(
        has_class(&kids[0], "rinch-stepper__step-label"),
        "after the label"
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

/// The step a `for` row built is **discarded** when the row goes, not just
/// detached, and while it is the active one its content sits in the region,
/// out of reach of the recursive discard. The content belongs to the step, so
/// it is retired with it — not put back into a step the document no longer
/// holds, alive and attached to nothing.
#[test]
fn discarding_the_active_step_retires_its_content() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let content = tree.content_of("two");
    assert_eq!(
        parent_id(&content),
        Some(tree.region().node_id()),
        "shown at first"
    );

    let two = tree.steps()[1].clone();
    two.discard();

    assert!(tree.retired(&two), "the step is gone");
    assert_eq!(Tree::markers(&tree.region()), ["three"]);
    assert!(
        tree.retired(&content),
        "the discarded step's content is retired with it (parent now {:?})",
        content.parent_node().map(|p| p.node_id())
    );
}

/// The same discard, reached through a wrapper the step sits in — the shape a
/// `for` row has.
#[test]
fn discarding_the_wrapper_of_the_active_step_retires_its_content() {
    let tree = Tree::build(|scope| {
        let wrapper = scope.create_element("div");
        wrapper.set_attribute("class", "row");
        wrapper.append_child(&step(scope, "two"));
        let kids = [step(scope, "one"), wrapper, step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let content = tree.content_of("two");
    let wrapper = find_by_class(&tree.root, "row").expect("the row wrapper");

    wrapper.discard();

    assert_eq!(Tree::markers(&tree.region()), ["three"]);
    assert!(tree.retired(&content), "retired with the row");
}

/// A step that is not the active one holds its own content, so the recursive
/// discard takes both; and content that went back home is retired with its
/// step too.
#[test]
fn discarding_a_step_that_is_not_active_retires_its_content() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let three = tree.content_of("three");
    let two = tree.content_of("two");

    // `two` leaves the region (step one goes, so `three` is at `active`) and is
    // then discarded from its own step.
    tree.steps()[0].discard();
    assert_eq!(Tree::markers(&tree.region()), ["three"]);
    let steps = tree.steps();
    assert_eq!(Tree::markers(&steps[0]), ["two"], "back in its step");
    steps[0].discard();

    assert!(tree.retired(&two), "retired with its step");
    assert!(!tree.retired(&three), "the shown content stays");
    assert_eq!(Tree::markers(&tree.region()), Vec::<String>::new());
}

/// The stepper itself discarded retires the shown content, which is under its
/// region.
#[test]
fn discarding_the_stepper_retires_the_shown_content() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let shown = tree.content_of("two");
    let hidden = tree.content_of("one");
    tree.root.discard();
    assert!(tree.retired(&shown));
    assert!(tree.retired(&hidden));
}

/// A step **removed** (detached, not discarded) keeps its content, and put
/// back at `active` it shows that same content again.
#[test]
fn a_removed_step_put_back_shows_its_own_content_again() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let content = tree.content_of("two");
    let two = tree.steps()[1].clone();
    two.remove();
    assert_eq!(Tree::markers(&tree.region()), ["three"]);
    assert!(!tree.retired(&content), "a removal keeps the content alive");

    let three = tree.steps()[1].clone();
    tree.steps_container().insert_before(&two, &three);

    assert_eq!(Tree::markers(&tree.region()), ["two"]);
    assert_eq!(
        parent_id(&content),
        Some(tree.region().node_id()),
        "the same content node, not a copy"
    );
    assert_eq!(Tree::markers(&tree.steps()[2]), ["three"]);
}

/// Content **the app takes out of the region** is the app's from then on: the
/// stepper forgets it, and does not pull it back into its step when another
/// step becomes the active one.
#[test]
fn content_the_app_takes_out_of_the_region_is_forgotten() {
    let mut tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let content = tree.content_of("two");
    let elsewhere = tree.scope.create_element("div");
    tree.root.append_child(&elsewhere);
    elsewhere.append_child(&content);

    // A pass: step one goes, so `three` is now at `active`.
    tree.steps()[0].remove();

    assert_eq!(Tree::markers(&tree.region()), ["three"]);
    assert_eq!(
        parent_id(&content),
        Some(elsewhere.node_id()),
        "the content stays where the app put it"
    );
    assert_eq!(
        Tree::markers(&tree.steps()[0]),
        Vec::<String>::new(),
        "and is not put back into step two"
    );
}

/// Content the app took out of the region stays the app's when its step is
/// then **discarded**: it is not retired with the step. `settle_content` asks
/// whether the content still sits in the region before it asks whether the
/// step was retired; asked the other way round, this content is discarded
/// from under the app.
#[test]
fn content_the_app_takes_out_survives_its_step_being_discarded() {
    let mut tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        stepper(scope, 1, &kids)
    });
    let content = tree.content_of("two");
    let elsewhere = tree.scope.create_element("div");
    tree.root.append_child(&elsewhere);
    elsewhere.append_child(&content);

    let two = tree.steps()[1].clone();
    two.discard();

    assert!(tree.retired(&two), "the step is gone");
    assert_eq!(Tree::markers(&tree.region()), ["three"], "a pass ran");
    assert!(
        !tree.retired(&content),
        "the content the app moved out is the app's, not discarded with the step"
    );
    assert_eq!(
        parent_id(&content),
        Some(elsewhere.node_id()),
        "and stays where the app put it"
    );
}

/// `content_for_active_only` (issue #2195's escape hatch): on by default, the
/// active step's content is moved into the region; off, every step keeps its
/// own content and the stepper says so with `--content-in-steps`. The
/// completed block comes below the steps either way.
#[test]
fn content_for_active_only_off_keeps_every_steps_content_in_its_step() {
    assert!(Stepper::default().content_for_active_only, "on by default");
    for (only_active, active) in [(true, 1), (false, 1), (false, 3)] {
        let tree = Tree::build(|scope| {
            let kids = [
                step(scope, "one"),
                step(scope, "two"),
                step(scope, "three"),
                completed(scope),
            ];
            Stepper {
                active,
                content_for_active_only: only_active,
                ..Default::default()
            }
            .render(scope, &kids)
        });
        let case = format!("content_for_active_only {only_active}, active {active}");
        assert_eq!(
            has_class(&tree.root, "rinch-stepper--content-in-steps"),
            !only_active,
            "{case}"
        );
        let steps = tree.steps();
        let in_steps: Vec<Vec<String>> = steps.iter().map(Tree::markers).collect();
        let region = Tree::markers(&tree.region());
        if only_active {
            assert_eq!(in_steps, [vec!["one"], vec![], vec!["three"]], "{case}");
            assert_eq!(region, ["two", "done"], "{case}");
        } else {
            assert_eq!(in_steps, [["one"], ["two"], ["three"]], "{case}");
            assert_eq!(region, ["done"], "{case}: only the completed block");
        }
        assert_eq!(
            has_class(&tree.region(), "rinch-stepper__content--completed"),
            active == 3,
            "{case}"
        );
    }
}

/// With the option off, a change to the list moves no content either.
#[test]
fn content_for_active_only_off_moves_nothing_on_a_later_pass() {
    let tree = Tree::build(|scope| {
        let kids = [step(scope, "one"), step(scope, "two"), step(scope, "three")];
        Stepper {
            active: 1,
            content_for_active_only: false,
            ..Default::default()
        }
        .render(scope, &kids)
    });
    tree.steps()[0].remove();
    let in_steps: Vec<Vec<String>> = tree.steps().iter().map(Tree::markers).collect();
    assert_eq!(in_steps, [["two"], ["three"]]);
    assert_eq!(Tree::markers(&tree.region()), Vec::<String>::new());
}
