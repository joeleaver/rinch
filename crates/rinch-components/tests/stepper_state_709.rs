//! `Stepper` derives each step's `state` and index from `active` (issue #709).
//!
//! `StepperStep::state` has always been documented as "will be set by parent
//! based on active index" and `StepperStep::step` as "set automatically", and
//! the parent set neither: it published its own `active` as `data-active` on the
//! container and appended the children unchanged. So `active` selected nothing,
//! every step rendered inactive and numbered itself `1`, and the
//! `completed_icon` #707 wired reached a step only when the caller had already
//! written `state: "completed"` by hand.
//!
//! **A parent renders after its children**, so this travels the way every
//! category-C prop in this crate does (`prop_wiring_707`'s module doc): the
//! stepper walks its rendered steps and patches them. The two facts a patch
//! cannot recover are the step's *own* `completed_icon` and `progress_icon` —
//! those are `TablerIcon`s that live in the step's props, not in its DOM, and a
//! step with no `state` of its own drew neither. `StepperStep` therefore leaves
//! them in its icon box as hidden alternates for the parent to promote.
//!
//! **Every fixture here samples off a fixed point.** `active: 0` makes
//! "position" and "constant zero" agree for the first step, and the number a
//! step draws unpatched is `1`, which is also the number the *first* step should
//! draw — so a stepper of one step at `active: 0` passes against code that
//! derives nothing at all. Each fixture below therefore uses three steps and
//! reads all three.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperCompleted, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, Signal};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

// ---------------------------------------------------------------- scaffolding

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
        collect_by_class(&self.root, "rinch-stepper__step", &mut out);
        out
    }
}

fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if has_class(node, class) {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

fn collect_by_class(node: &NodeHandle, class: &str, out: &mut Vec<NodeHandle>) {
    if has_class(node, class) {
        out.push(node.clone());
    }
    for child in node.children() {
        collect_by_class(&child, class, out);
    }
}

/// The one state modifier a step carries, as a bare word.
///
/// A step always carries exactly one of the three, so this is total; it panics
/// rather than returning an `Option` so a step that somehow carries none fails
/// where the state is read instead of where it is compared.
fn state_of(step: &NodeHandle) -> &'static str {
    let completed = has_class(step, "rinch-stepper__step--completed");
    let progress = has_class(step, "rinch-stepper__step--progress");
    let inactive = has_class(step, "rinch-stepper__step--inactive");
    match (completed, progress, inactive) {
        (true, false, false) => "completed",
        (false, true, false) => "progress",
        (false, false, true) => "inactive",
        _ => panic!(
            "a step carries exactly one state class, this one has {:?}",
            step.get_attribute("class")
        ),
    }
}

fn states(tree: &Tree) -> Vec<&'static str> {
    tree.steps().iter().map(state_of).collect()
}

fn indices(tree: &Tree) -> Vec<Option<String>> {
    tree.steps()
        .iter()
        .map(|s| s.get_attribute("data-step"))
        .collect()
}

/// What each step's icon box reads as text — its number, or `""` for a glyph.
///
/// A parked alternate never holds text: a number is a built-in the parent can
/// rebuild from the step's index, so it is discarded rather than parked (issue
/// #716). Reading the box's whole text is therefore still reading the live
/// content.
fn numbers(tree: &Tree) -> Vec<String> {
    tree.steps()
        .iter()
        .map(|s| {
            find_by_class(s, "rinch-stepper__step-icon")
                .expect("every step has an icon box")
                .text_content()
                .unwrap_or_default()
        })
        .collect()
}

/// Every `d` attribute in `node`'s subtree, in document order — what tells one
/// rendered Tabler glyph from another.
fn glyph(node: &NodeHandle) -> Vec<String> {
    fn walk(node: &NodeHandle, out: &mut Vec<String>) {
        if let Some(d) = node.get_attribute("d") {
            out.push(d);
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn glyph_of(icon: TablerIcon) -> Vec<String> {
    let tree = Tree::build(move |scope| render_tabler_icon(scope, icon, TablerIconStyle::Outline));
    let paths = glyph(&tree.root);
    assert!(!paths.is_empty(), "{icon:?} renders no path data");
    paths
}

/// The **live** glyph in step `n`'s icon box — what the step draws.
///
/// The box may also hold parked alternates, hidden, for a state the step could
/// still be moved into when a sibling arrives later (issue #716). Those are not
/// what the step draws, so reading the whole subtree would make every assertion
/// below say something other than it means.
fn glyph_at(tree: &Tree, n: usize) -> Vec<String> {
    glyph(&live_icon_at(tree, n).expect("every step draws something"))
}

/// The one child of step `n`'s icon box that is not a parked alternate.
fn live_icon_at(tree: &Tree, n: usize) -> Option<NodeHandle> {
    let steps = tree.steps();
    let icon_box =
        find_by_class(&steps[n], "rinch-stepper__step-icon").expect("every step has an icon box");
    icon_box
        .children()
        .into_iter()
        .find(|c| !has_class(c, "rinch-stepper__step-icon-alt"))
}

/// The alternates parked in step `n`'s icon box, by the content key each is for.
fn parked_at(tree: &Tree, n: usize) -> Vec<(String, NodeHandle)> {
    let steps = tree.steps();
    let icon_box =
        find_by_class(&steps[n], "rinch-stepper__step-icon").expect("every step has an icon box");
    icon_box
        .children()
        .into_iter()
        .filter(|c| has_class(c, "rinch-stepper__step-icon-alt"))
        .map(|c| (c.get_attribute("data-icon-for").unwrap_or_default(), c))
        .collect()
}

/// The doc example's shape: a stepper of three steps that name no state and no
/// index of their own, which is how a caller writes one.
fn three_steps(stepper: Stepper) -> Tree {
    three_steps_of(stepper, StepperStep::default)
}

fn three_steps_of(stepper: Stepper, step: impl Fn() -> StepperStep) -> Tree {
    Tree::build(move |scope| {
        let steps: Vec<NodeHandle> = (0..3).map(|_| step().render(scope, &[])).collect();
        stepper.render(scope, &steps)
    })
}

// ------------------------------------------------- (a) the derivation itself

#[test]
fn the_doc_example_derives_one_state_per_step() {
    let tree = three_steps(Stepper {
        active: 1,
        ..Default::default()
    });
    assert_eq!(
        states(&tree),
        vec!["completed", "progress", "inactive"],
        "`active: 1` means step 0 is behind it, step 1 is it, and step 2 is \
         ahead — the three states `StepperStep::state` documents"
    );
}

#[test]
fn active_zero_puts_the_first_step_in_progress() {
    let tree = three_steps(Stepper::default());
    assert_eq!(
        states(&tree),
        vec!["progress", "inactive", "inactive"],
        "the default `active: 0` is the first step, not a step before the first"
    );
}

#[test]
fn an_active_past_the_last_step_completes_every_one() {
    let tree = three_steps(Stepper {
        active: 9,
        ..Default::default()
    });
    assert_eq!(
        states(&tree),
        vec!["completed"; 3],
        "a stepper driven past its last step is finished, not stuck with two \
         completed steps and one inactive"
    );
}

// ------------------------------------------------------ (b) the escape hatch

#[test]
fn a_step_that_named_its_own_state_keeps_it() {
    let tree = Tree::build(|scope| {
        let a = StepperStep {
            state: "inactive".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        let b = StepperStep::default().render(scope, &[]);
        let c = StepperStep {
            state: "completed".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &[a, b, c])
    });
    assert_eq!(
        states(&tree),
        vec!["inactive", "progress", "completed"],
        "the child wins: step 0 asked to stay inactive where the derivation \
         would have completed it, step 2 asked to be completed where the \
         derivation would have left it inactive, and step 1 — which asked for \
         nothing — still derives"
    );
}

// --------------------------------------------------------- (c) the index

#[test]
fn the_parent_numbers_the_steps_from_zero() {
    let tree = three_steps(Stepper {
        active: 1,
        ..Default::default()
    });
    assert_eq!(
        indices(&tree),
        vec![
            Some("0".to_string()),
            Some("1".to_string()),
            Some("2".to_string())
        ],
        "`data-step` is the position, 0-indexed — index 0 alone cannot tell \
         that from a constant"
    );
    assert_eq!(
        numbers(&tree),
        vec!["", "2", "3"],
        "and the number a step draws is its index plus one: step 0 is completed \
         so it draws a tick instead, and the other two drew `1` before the \
         parent renumbered them"
    );
}

#[test]
fn a_step_that_named_its_own_index_keeps_it() {
    let tree = three_steps_of(
        Stepper {
            active: 1,
            ..Default::default()
        },
        || StepperStep {
            step: Some(7),
            ..Default::default()
        },
    );
    assert_eq!(
        indices(&tree),
        vec![Some("7".to_string()); 3],
        "an index the step named is the step's, the same way its `state` is"
    );
    assert_eq!(
        numbers(&tree),
        vec!["", "8", "8"],
        "and it is the number it draws — an explicit index changes the label, \
         not the position the state derives from"
    );
    assert_eq!(
        states(&tree),
        vec!["completed", "progress", "inactive"],
        "the state still derives from document order: `allow_next_steps_select` \
         counts positions, so a second notion of index would disagree with it"
    );
}

// ------------------------------------------------ (e) the icons #707 wired

#[test]
fn the_stepper_completed_icon_reaches_a_derived_completed_step() {
    let tree = three_steps(Stepper {
        active: 1,
        completed_icon: Some(TablerIcon::CircleCheck),
        ..Default::default()
    });
    assert_eq!(
        glyph_at(&tree, 0),
        glyph_of(TablerIcon::CircleCheck),
        "the doc example on `Stepper` — three steps that set no `state` and a \
         `completed_icon` — finally draws that icon (issues #474, #709)"
    );
    assert!(
        glyph_at(&tree, 1).is_empty() && glyph_at(&tree, 2).is_empty(),
        "and reaches only the completed step"
    );
}

#[test]
fn the_stepper_progress_icon_reaches_a_derived_progress_step() {
    let tree = three_steps(Stepper {
        active: 1,
        progress_icon: Some(TablerIcon::Home),
        ..Default::default()
    });
    assert_eq!(glyph_at(&tree, 1), glyph_of(TablerIcon::Home));
    assert_eq!(
        numbers(&tree),
        vec!["", "", "3"],
        "the number it replaced is gone, not merely covered"
    );
}

#[test]
fn a_steps_own_completed_icon_survives_the_derivation() {
    assert_ne!(glyph_of(TablerIcon::X), glyph_of(TablerIcon::CircleCheck));
    let tree = three_steps_of(
        Stepper {
            active: 1,
            completed_icon: Some(TablerIcon::CircleCheck),
            ..Default::default()
        },
        || StepperStep {
            completed_icon: Some(TablerIcon::X),
            ..Default::default()
        },
    );
    assert_eq!(
        glyph_at(&tree, 0),
        glyph_of(TablerIcon::X),
        "a step's own `completed_icon` outranks the stepper's default when the \
         parent is the one that decided the step was completed — the step drew \
         no such icon, so the parent has to have kept it"
    );
    assert!(
        glyph_at(&tree, 1).is_empty() && glyph_at(&tree, 2).is_empty(),
        "and the alternates the step left for the parent are gone from the \
         steps that did not need them"
    );
}

#[test]
fn a_steps_own_progress_icon_survives_the_derivation() {
    let tree = three_steps_of(
        Stepper {
            active: 1,
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
        || StepperStep {
            progress_icon: Some(TablerIcon::X),
            ..Default::default()
        },
    );
    assert_eq!(glyph_at(&tree, 1), glyph_of(TablerIcon::X));
    assert!(
        glyph_at(&tree, 0).is_empty(),
        "the completed step took the built-in tick, which draws no path data, \
         not the progress alternate"
    );
    assert!(glyph_at(&tree, 2).is_empty());
}

#[test]
fn a_loading_step_keeps_its_loader_through_the_derivation() {
    let tree = three_steps_of(
        Stepper {
            active: 1,
            completed_icon: Some(TablerIcon::CircleCheck),
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
        || StepperStep {
            loading: true,
            ..Default::default()
        },
    );
    for (n, step) in tree.steps().iter().enumerate() {
        let icon_box = find_by_class(step, "rinch-stepper__step-icon").expect("an icon box");
        assert!(
            find_by_class(&icon_box, "rinch-stepper__loader").is_some(),
            "step {n} is loading, and a spinner is the right thing to draw in \
             every state"
        );
        assert!(glyph(&icon_box).is_empty(), "step {n} drew no icon");
    }
}

// ---------------------------------------------------------- (d) reactivity

/// The generated shape of `Stepper { active: {|| sig.get()} }`: the rsx macro
/// reads the prop closure in the tracked region and renders the whole subtree —
/// children included — inside `untracked`, so the derivation runs again on every
/// change. This drives that by hand rather than through `rsx!`, which cannot be
/// invoked from an integration test of this crate.
fn reactive_stepper(active: Signal<u32>) -> Tree {
    Tree::build(move |scope| {
        let host = scope.create_element("div");
        let h = host.clone();
        rinch_core::dom::reactive_component_dom(scope, &host, move |child_scope| {
            let active = active.get();
            rinch_core::reactive::untracked(|| {
                let steps: Vec<NodeHandle> = (0..3)
                    .map(|_| StepperStep::default().render(child_scope, &[]))
                    .collect();
                Stepper {
                    active,
                    ..Default::default()
                }
                .render(child_scope, &steps)
            })
        });
        h
    })
}

#[test]
fn a_reactive_active_re_derives_every_state() {
    let active = Signal::new(0u32);
    let tree = reactive_stepper(active);
    assert_eq!(states(&tree), vec!["progress", "inactive", "inactive"]);
    assert_eq!(numbers(&tree), vec!["1", "2", "3"]);

    active.set(2);
    assert_eq!(
        states(&tree),
        vec!["completed", "completed", "progress"],
        "a stepper whose `active` is a signal follows it"
    );
    assert_eq!(
        numbers(&tree),
        vec!["", "", "3"],
        "and renumbers the steps it rebuilt"
    );

    active.set(1);
    assert_eq!(
        states(&tree),
        vec!["completed", "progress", "inactive"],
        "in both directions — a derivation that only ever advances would pass a \
         fixture that only ever counts up"
    );
}

// ------------------------------------ the verb: `discard`, not `remove` (#719)

/// Render `build`'s tree, and hand back whatever it stashed on the way.
///
/// The probe has to be taken **before** the stepper renders: the whole question
/// is what became of a node that is no longer anywhere in the tree, so there is
/// nothing to find it by afterwards.
fn tree_with_probe(
    build: impl FnOnce(&mut RenderScope, &mut Option<NodeHandle>) -> NodeHandle,
) -> (Tree, NodeHandle) {
    let probe: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let p = probe.clone();
    let tree = Tree::build(move |scope| {
        let mut slot = None;
        let root = build(scope, &mut slot);
        *p.borrow_mut() = slot;
        root
    });
    let probe = probe.borrow().clone().expect("the builder stashed a node");
    (tree, probe)
}

/// Is `node` still live in its document, or has it been retired?
///
/// There is no predicate for this — `is_valid` only asks whether the *document*
/// is alive — so this asks the document the only way a caller can: a retired id
/// is a silent no-op, so a write to it does not read back. That is the
/// post-condition `discard` buys and `remove` does not (issue #719).
fn still_live(node: &NodeHandle) -> bool {
    node.set_attribute("data-probe-709", "1");
    node.get_attribute("data-probe-709").is_some()
}

#[test]
fn the_glyph_a_step_no_longer_draws_is_parked_against_a_later_move() {
    let (tree, drew) = tree_with_probe(|scope, probe| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| {
                StepperStep {
                    icon: Some(TablerIcon::Home),
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        let icon_box = find_by_class(&steps[0], "rinch-stepper__step-icon").expect("an icon box");
        *probe = icon_box.children().into_iter().next();
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &steps)
    });

    assert_eq!(
        glyph_at(&tree, 0),
        Vec::<String>::new(),
        "precondition: step 0 was moved to completed, so the tick is what it \
         draws — `check_dom` is a polyline and contributes no path data"
    );
    assert!(
        still_live(&drew),
        "the `Home` glyph it stopped drawing is **kept**, hidden. It came from a \
         `TablerIcon` in the step's props, and a step that gains a sibling in \
         front of it moves *back* out of completed (issue #716) — there is \
         nowhere else for the glyph to come from then, so discarding it here \
         would leave that step empty. #709 discarded it, when nothing could \
         move a step twice"
    );
    let parked = parked_at(&tree, 0);
    assert_eq!(
        parked.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        vec!["base"],
        "parked under the content key it serves, which is the key every state \
         but completed resolves to for this step"
    );
    assert_eq!(
        parked[0].1.get_attribute("style").as_deref(),
        Some("display: none"),
        "and hidden, or the step would draw two glyphs at once"
    );
}

#[test]
fn a_built_in_a_step_no_longer_draws_is_discarded_not_merely_removed() {
    // The same shape with no `icon` prop: what step 0 drew is the plain number,
    // which the parent can rebuild from the step's index at any time.
    let (tree, drew) = tree_with_probe(|scope, probe| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        let icon_box = find_by_class(&steps[0], "rinch-stepper__step-icon").expect("an icon box");
        *probe = icon_box.children().into_iter().next();
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &steps)
    });

    assert_eq!(
        numbers(&tree)[0],
        "",
        "precondition: step 0 was moved to completed, so the tick replaced the \
         number it had drawn"
    );
    assert!(
        parked_at(&tree, 0).is_empty(),
        "nothing is parked: a number is not a glyph the step's props supplied"
    );
    assert!(
        !still_live(&drew),
        "so it is gone for good, and it leaves by `discard` and not by `remove` \
         (issue #719). `remove` is a detach that keeps a subtree re-insertable; \
         only `discard` releases `rinch-web`'s strong `web_sys::Node` from its \
         two page-global maps, and `rinch-web` compiles this crate"
    );
}

#[test]
fn an_alternate_the_stepper_does_not_need_yet_is_kept_hidden() {
    let (tree, alternate) = tree_with_probe(|scope, probe| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| {
                StepperStep {
                    // A named index, so the parent has no number to redraw, and
                    // an `icon`, so what it drew is not a number either. Between
                    // them the last step needs no new content at all.
                    step: Some(9),
                    icon: Some(TablerIcon::Home),
                    completed_icon: Some(TablerIcon::X),
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        let icon_box = find_by_class(&steps[2], "rinch-stepper__step-icon").expect("an icon box");
        *probe = icon_box
            .children()
            .into_iter()
            .find(|c| c.get_attribute("data-icon-for").is_some());
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &steps)
    });

    assert_eq!(
        glyph_at(&tree, 2),
        glyph_of(TablerIcon::Home),
        "precondition: the last step is inactive and keeps exactly what it drew"
    );
    assert!(
        still_live(&alternate),
        "the completed alternate it does not need *yet* is kept. #709 discarded \
         it, on the reasoning that a step's position only ever grows — a keyed \
         `for` reorder moves a step backwards with `insert_before`, so this step \
         can be moved into completed later and its own `X` is the only place \
         that glyph exists (issue #716)"
    );
    let parked = parked_at(&tree, 2);
    assert_eq!(
        parked.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        vec!["completed"],
        "one alternate, under the key it serves"
    );
    assert_eq!(
        parked[0].1.get_attribute("style").as_deref(),
        Some("display: none"),
        "hidden, or the step would draw two glyphs at once"
    );
}

#[test]
fn a_named_state_steps_parked_glyph_is_discarded_not_merely_removed() {
    // A step that named its own state is in that state wherever it is moved to,
    // so what the stepper displaces there really is dead — and dead markup
    // leaves by `discard`, not by `remove` (issue #719).
    let (tree, drew) = tree_with_probe(|scope, probe| {
        let step = StepperStep {
            state: "progress".into(),
            step: Some(1),
            icon: Some(TablerIcon::Home),
            ..Default::default()
        }
        .render(scope, &[]);
        let icon_box = find_by_class(&step, "rinch-stepper__step-icon").expect("an icon box");
        *probe = icon_box.children().into_iter().next();
        Stepper {
            active: 0,
            progress_icon: Some(TablerIcon::X),
            ..Default::default()
        }
        .render(scope, &[step])
    });

    assert_eq!(
        glyph_at(&tree, 0),
        glyph_of(TablerIcon::X),
        "precondition: the stepper's `progress_icon` stands in for the one the \
         step did not set, so it displaces the step's plain `icon`"
    );
    assert!(
        parked_at(&tree, 0).is_empty(),
        "nothing is parked for a step whose state is its own"
    );
    assert!(
        !still_live(&drew),
        "and the glyph it displaced is gone for good. `remove` is a detach that \
         keeps a subtree re-insertable; only `discard` releases `rinch-web`'s \
         strong `web_sys::Node` from its two page-global maps, and `rinch-web` \
         compiles this crate"
    );
}

// ------------------ the three things a first pass at this left unpinned (#740)

#[test]
fn a_step_that_named_only_its_state_is_still_numbered_by_its_parent() {
    let tree = three_steps_of(
        Stepper {
            active: 1,
            ..Default::default()
        },
        || StepperStep {
            state: "inactive".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        states(&tree),
        vec!["inactive"; 3],
        "precondition: all three named a state, so none of them derives one"
    );
    assert_eq!(
        indices(&tree),
        vec![
            Some("0".to_string()),
            Some("1".to_string()),
            Some("2".to_string())
        ],
        "the two asks are separate: naming a `state` says nothing about the \
         index, so the parent still numbers a step that named no `step`"
    );
    assert_eq!(
        numbers(&tree),
        vec!["1", "2", "3"],
        "and redraws the number, which each of them had drawn as `1`. Position \
         0 is the fixed point here — it drew `1` and should draw `1` — so the \
         claim rests on the other two"
    );
}

#[test]
fn a_step_the_walk_reaches_twice_ends_up_with_one_state_class() {
    // A `Stepper` inside the `StepperCompleted` of another one. `collect_steps`
    // stops at a step, so it never descends into a step's *content* — but a
    // completed block is a sibling of the steps, not inside one, so the outer
    // stepper does reach these and re-derives them at its own positions. They
    // arrive carrying the class their own stepper already gave them.
    let tree = Tree::build(|scope| {
        let inner: Vec<NodeHandle> = (0..2)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        let inner_stepper = Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &inner);
        let completed = StepperCompleted.render(scope, &[inner_stepper]);
        let outer: Vec<NodeHandle> = (0..2)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        let mut children = outer;
        children.push(completed);
        Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &children)
    });

    // `state_of` panics unless a step carries exactly one of the three, which is
    // the whole assertion; reading every step is what makes it total.
    let all: Vec<&'static str> = states(&tree);
    assert_eq!(
        all.len(),
        4,
        "two steps of the outer stepper plus the two the walk reached inside \
         its completed block"
    );
    assert_eq!(
        all,
        vec!["completed", "progress", "inactive", "inactive"],
        "the outer stepper re-derives all four at its own positions, and each \
         one is left in exactly one state — a swap that took off only \
         `--inactive` left the third step carrying `--progress` as well"
    );
}

#[test]
fn allow_next_steps_select_counts_positions_and_not_declared_indices() {
    // Declared indices that run *backwards*, so "position" and "declared index"
    // pick opposite halves of the list. `four_steps` in `prop_wiring_707` gives
    // step `i` the index `i`, where the two agree and nothing discriminates.
    let tree = Tree::build(|scope| {
        let steps: Vec<NodeHandle> = (0..4)
            .map(|i| {
                StepperStep {
                    step: Some(3 - i),
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        Stepper {
            active: 1,
            allow_next_steps_select: true,
            ..Default::default()
        }
        .render(scope, &steps)
    });

    let clickable: Vec<bool> = tree
        .steps()
        .iter()
        .map(|s| has_class(s, "rinch-stepper__step--clickable"))
        .collect();
    assert_eq!(
        clickable,
        vec![false, false, true, true],
        "the last two steps sit past `active: 1` and are the ones granted the \
         class. Reading their declared indices instead would grant it to the \
         first two — 3 and 2 are the indices above 1 — which is the exact \
         inverse, and is why the state derivation counts positions too"
    );
    assert_eq!(
        states(&tree),
        vec!["completed", "progress", "inactive", "inactive"],
        "and the state follows the same count, so the two never disagree"
    );
}
