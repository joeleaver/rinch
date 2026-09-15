//! A container's default reaches a child that arrives **after** the container
//! rendered — issue #716.
//!
//! The five props are #474's cluster C, wired by #707, plus the state/index
//! derivation #740 gave `Stepper`. All of them are parent→child plumbing, and a
//! parent component renders *after* its children (the `rsx!` macro builds them
//! into a `<template>` and hands the finished handles to `Component::render`),
//! so none of them can travel as a prop. They travel as a patch of the rendered
//! tree instead — and that patch used to run exactly once, so a child inserted
//! later by a `for` reconcile, a `show_dom` branch or a hand-rolled
//! `append_child` never got it.
//!
//! Every case here is checked in the two directions that matter: the late child
//! takes the container's default, and a late child that named its own value
//! keeps it. One direction alone proves nothing — a container that overwrote
//! *every* child would pass a fixture that only ever looks at a child which
//! named nothing.

use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::list::{List, ListItem};
use rinch_components::radio::{Radio, RadioGroup};
use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, Signal};
use rinch_macros::rsx;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

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

    fn find_all(&self, class: &str) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.root, class, &mut out);
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

// ------------------------------------------------------- C: List::icon, `for`

#[test]
fn a_row_a_for_loop_adds_later_takes_the_lists_icon() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    for it in items.get() { ListItem { key: it, {it} } }
                }
            }
        }
    });

    assert_eq!(tree.find_all("rinch-list__item").len(), 1);
    assert_eq!(
        tree.find_all("rinch-list__item-icon").len(),
        1,
        "precondition: the row present at the list's render has the icon"
    );

    items.update(|v| v.push("two"));

    assert_eq!(
        tree.find_all("rinch-list__item").len(),
        2,
        "precondition: the reconcile added the row"
    );
    assert_eq!(
        tree.find_all("rinch-list__item-icon").len(),
        2,
        "#716: the row the reconcile added takes the list's default icon too — \
         a list with one iconed row and one bare row is ragged"
    );
}

#[test]
fn a_row_a_for_loop_adds_later_keeps_an_icon_of_its_own() {
    assert_ne!(glyph_of(TablerIcon::Check), glyph_of(TablerIcon::X));

    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    for it in items.get() {
                        ListItem { key: it, icon: TablerIcon::X, {it} }
                    }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let boxes = tree.find_all("rinch-list__item-icon");
    assert_eq!(boxes.len(), 2, "one icon box per row, not two on either");
    for icon_box in &boxes {
        assert_eq!(
            glyph(icon_box),
            glyph_of(TablerIcon::X),
            "the row's own icon wins over the list's default, late or not"
        );
    }
}

// ------------------------------------------------- C: List::icon, hand-rolled

#[test]
fn a_row_appended_by_hand_takes_the_lists_icon() {
    let tree = Tree::build(|scope| {
        let text = scope.create_text("One");
        let item = ListItem::default().render(scope, &[text]);
        List {
            icon: Some(TablerIcon::Check),
            ..Default::default()
        }
        .render(scope, &[item])
    });

    // A second row built and appended long after the list rendered — no macro,
    // no reconcile, the bare `NodeHandle` API an app may use directly.
    {
        let doc = tree._doc.clone();
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc, body);
        let text = scope.create_text("Two");
        let item = ListItem::default().render(&mut scope, &[text]);
        tree.root.append_child(&item);
        std::mem::forget(scope);
    }

    assert_eq!(
        tree.find_all("rinch-list__item-icon").len(),
        2,
        "#716: a hand-rolled `append_child` is a late arrival like any other"
    );
}

// --------------------------------------------- C: RadioGroup::size, `for`

#[test]
fn a_radio_a_for_loop_adds_later_takes_the_groups_size() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                RadioGroup { size: "lg",
                    for it in items.get() { Radio { key: it, value: it } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let radios = tree.find_all("rinch-radio");
    assert_eq!(
        radios.len(),
        2,
        "precondition: the reconcile added the radio"
    );
    for radio in &radios {
        assert!(
            has_class(radio, "rinch-radio--lg"),
            "#716: the late radio takes the group's size; it read \
             {:?}",
            radio.get_attribute("class")
        );
        assert!(
            !has_class(radio, "rinch-radio--md"),
            "a radio carries exactly one size class"
        );
    }
}

#[test]
fn a_radio_a_for_loop_adds_later_keeps_a_size_of_its_own() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                RadioGroup { size: "lg",
                    for it in items.get() { Radio { key: it, value: it, size: "xs" } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let radios = tree.find_all("rinch-radio");
    assert_eq!(radios.len(), 2);
    for radio in &radios {
        assert!(
            has_class(radio, "rinch-radio--xs"),
            "the radio's own size wins over the group's default, late or not"
        );
    }
}

// ------------------------------------ C: the two Stepper icons + #740, `for`

#[test]
fn a_step_a_for_loop_adds_later_takes_the_steppers_completed_icon() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 2u32, completed_icon: TablerIcon::CircleCheck,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let boxes = tree.find_all("rinch-stepper__step-icon");
    assert_eq!(boxes.len(), 2, "precondition: the reconcile added the step");
    for icon_box in &boxes {
        assert_eq!(
            glyph(icon_box),
            glyph_of(TablerIcon::CircleCheck),
            "#716: both steps are before `active`, so both are completed and \
             both draw the stepper's completed icon"
        );
    }
}

#[test]
fn a_step_a_for_loop_adds_later_is_numbered_and_stated_from_its_position() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let steps = tree.find_all("rinch-stepper__step");
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[1].get_attribute("data-step").as_deref(),
        Some("1"),
        "#740 + #716: the late step is numbered from its position"
    );
    assert!(
        has_class(&steps[1], "rinch-stepper__step--progress"),
        "the late step sits at `active`, so it is the in-progress one; it read \
         {:?}",
        steps[1].get_attribute("class")
    );
    let icon_box =
        find_by_class(&steps[1], "rinch-stepper__step-icon").expect("the step has an icon box");
    assert_eq!(
        icon_box.text_content().as_deref(),
        Some("2"),
        "an in-progress step with no icon draws its own number"
    );
}

#[test]
fn a_step_inserted_in_front_renumbers_the_ones_after_it() {
    let items = Signal::new(vec!["b"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    items.update(|v| v.insert(0, "a"));

    let steps = tree.find_all("rinch-stepper__step");
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps
            .iter()
            .map(|s| s.get_attribute("data-step").unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["0".to_string(), "1".to_string()],
        "#716: an insertion at the front moves every step after it"
    );
    assert!(
        has_class(&steps[1], "rinch-stepper__step--progress"),
        "the step that was at 0 is now at 1, which is `active`"
    );
}

#[test]
fn a_late_step_is_made_clickable_by_allow_next_steps_select() {
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 0u32, allow_next_steps_select: true,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));

    let steps = tree.find_all("rinch-stepper__step");
    assert_eq!(steps.len(), 2);
    assert!(
        !has_class(&steps[0], "rinch-stepper__step--clickable"),
        "the active step is not a *next* step"
    );
    assert!(
        has_class(&steps[1], "rinch-stepper__step--clickable"),
        "#716: the late step sits past `active`, so it is selectable"
    );
}

// ---------------------------------------------------- show_dom branch arrival

#[test]
fn a_row_a_branch_reveals_later_takes_the_lists_icon() {
    let shown = Signal::new(false);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    ListItem { "always" }
                    if shown.get() { ListItem { "sometimes" } }
                }
            }
        }
    });

    assert_eq!(tree.find_all("rinch-list__item").len(), 1);
    shown.set(true);

    assert_eq!(
        tree.find_all("rinch-list__item").len(),
        2,
        "precondition: the branch rendered its row"
    );
    assert_eq!(
        tree.find_all("rinch-list__item-icon").len(),
        2,
        "#716: a row a branch reveals is a late arrival like a reconciled one"
    );
}
