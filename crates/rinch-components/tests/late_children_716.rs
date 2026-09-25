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
use rinch_tabler_icons::TablerIcon;

mod common;
use common::{collect_by_class, find_by_class, glyph, glyph_of, has_class};

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

// ------------------------------------------- moved between two containers

#[test]
fn a_row_moved_into_another_list_takes_the_new_lists_icon() {
    assert_ne!(glyph_of(TablerIcon::Check), glyph_of(TablerIcon::X));

    let tree = Tree::build(|scope| {
        let text = scope.create_text("One");
        let item = ListItem::default().render(scope, &[text]);
        let first = List {
            icon: Some(TablerIcon::Check),
            ..Default::default()
        }
        .render(scope, &[item]);
        let second = List {
            icon: Some(TablerIcon::X),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&first);
        root.append_child(&second);
        root
    });

    let lists = tree.find_all("rinch-list");
    assert_eq!(lists.len(), 2);
    let item = tree.find_all("rinch-list__item").remove(0);
    assert_eq!(
        glyph(&item),
        glyph_of(TablerIcon::Check),
        "precondition: the row starts in the first list"
    );

    lists[1].append_child(&item);

    assert_eq!(
        glyph(&item),
        glyph_of(TablerIcon::X),
        "#716: the row re-resolves against the list it now belongs to. The icon \
         it was carrying was the first list's default, not its own"
    );
    assert_eq!(
        tree.find_all("rinch-list__item-icon").len(),
        1,
        "and it has one icon box, not two"
    );
}

#[test]
fn a_row_with_its_own_icon_keeps_it_across_a_move() {
    let tree = Tree::build(|scope| {
        let text = scope.create_text("One");
        let item = ListItem {
            icon: Some(TablerIcon::Home),
        }
        .render(scope, &[text]);
        let first = List {
            icon: Some(TablerIcon::Check),
            ..Default::default()
        }
        .render(scope, &[item]);
        let second = List {
            icon: Some(TablerIcon::X),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&first);
        root.append_child(&second);
        root
    });

    let lists = tree.find_all("rinch-list");
    let item = tree.find_all("rinch-list__item").remove(0);
    lists[1].append_child(&item);

    assert_eq!(
        glyph(&item),
        glyph_of(TablerIcon::Home),
        "a child's value wins over a parent's default wherever the child ends up"
    );
}

#[test]
fn a_radio_moved_into_another_group_takes_the_new_groups_size() {
    let tree = Tree::build(|scope| {
        let radio = Radio {
            value: "a".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        let first = RadioGroup {
            size: "lg".into(),
            ..Default::default()
        }
        .render(scope, &[radio]);
        let second = RadioGroup {
            size: "xs".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&first);
        root.append_child(&second);
        root
    });

    let wrappers = tree.find_all("rinch-radio-group__radios");
    let radio = tree.find_all("rinch-radio").remove(0);
    assert!(has_class(&radio, "rinch-radio--lg"), "precondition");

    wrappers[1].append_child(&radio);

    assert!(
        has_class(&radio, "rinch-radio--xs"),
        "#716: the radio re-resolves against the group it now belongs to; it \
         read {:?}",
        radio.get_attribute("class")
    );
    assert!(!has_class(&radio, "rinch-radio--lg"));
}

#[test]
fn a_step_moved_into_another_stepper_is_restated_from_its_new_position() {
    let tree = Tree::build(|scope| {
        let a = StepperStep {
            label: "a".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        let b = StepperStep {
            label: "b".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        // The first stepper is on step 0, so `a` is in progress and `b` is not
        // yet reached.
        let first = Stepper {
            active: 0,
            ..Default::default()
        }
        .render(scope, &[a, b]);
        // The second is past its only step, so anything landing in it at
        // position 0 is completed.
        let second = Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&first);
        root.append_child(&second);
        root
    });

    let containers = tree.find_all("rinch-stepper__steps");
    let steps = tree.find_all("rinch-stepper__step");
    assert!(
        has_class(&steps[1], "rinch-stepper__step--inactive"),
        "precondition: the second step of the first stepper is not reached yet"
    );

    containers[1].append_child(&steps[1]);

    assert_eq!(
        steps[1].get_attribute("data-step").as_deref(),
        Some("0"),
        "#716: the step is renumbered from its position in its new stepper"
    );
    assert!(
        has_class(&steps[1], "rinch-stepper__step--completed"),
        "and restated: position 0 against `active: 1` is completed. It read {:?}",
        steps[1].get_attribute("class")
    );
    assert!(
        !has_class(&steps[1], "rinch-stepper__step--inactive"),
        "a step carries exactly one state class"
    );
}

// -------------------------------- a reactive container prop, then more growth

#[test]
fn a_late_radio_takes_the_groups_current_size_after_a_reactive_change() {
    let size = Signal::new("lg".to_string());
    let items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                RadioGroup { size: {move || size.get()},
                    for it in items.get() { Radio { key: it, value: it } }
                }
            }
        }
    });

    items.update(|v| v.push("two"));
    for radio in tree.find_all("rinch-radio") {
        assert!(
            has_class(&radio, "rinch-radio--lg"),
            "precondition: both radios carry the group's first size"
        );
    }

    // The reactive prop rebuilds the component, children and all — which also
    // retires the observer the old wrapper registered, and installs one for the
    // new group.
    size.set("xs".to_string());
    items.update(|v| v.push("three"));

    let radios = tree.find_all("rinch-radio");
    assert_eq!(radios.len(), 3, "three radios");
    for radio in &radios {
        assert!(
            has_class(radio, "rinch-radio--xs"),
            "#716: the radio added after the prop changed takes the group's \
             *current* size, and so do the ones the rebuild re-rendered. It \
             read {:?}",
            radio.get_attribute("class")
        );
    }
}

// ------------------------------------------------ the nesting boundary

#[test]
fn a_row_added_to_a_nested_list_takes_the_inner_lists_icon() {
    assert_ne!(glyph_of(TablerIcon::Check), glyph_of(TablerIcon::X));

    let inner_items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    ListItem {
                        List { icon: TablerIcon::X,
                            for it in inner_items.get() { ListItem { key: it, {it} } }
                        }
                    }
                }
            }
        }
    });

    inner_items.update(|v| v.push("two"));

    let inner = tree.find_all("rinch-list").remove(1);
    let rows: Vec<NodeHandle> = {
        let mut out = Vec::new();
        collect_by_class(&inner, "rinch-list__item", &mut out);
        out
    };
    assert_eq!(rows.len(), 2, "precondition: the inner list grew");
    for row in &rows {
        let icon_box =
            find_by_class(row, "rinch-list__item-icon").expect("the inner row has an icon box");
        assert_eq!(
            glyph(&icon_box),
            glyph_of(TablerIcon::X),
            "#716: a row of a *nested* list belongs to that list. The outer list \
             is told about the insertion too — two containers of different kinds \
             can nest — and declines because the chain up to it crosses one of \
             its own items"
        );
    }
}

// ------------------------------------ a glyph the step's props supplied comes back

#[test]
fn a_step_displaced_out_of_completed_gets_its_own_glyph_back() {
    let items = Signal::new(vec!["b"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32,
                    for it in items.get() { StepperStep { key: it, label: it, icon: TablerIcon::Home } }
                }
            }
        }
    });

    let live = |n: usize| -> Vec<String> {
        let steps = tree.find_all("rinch-stepper__step");
        let icon_box = find_by_class(&steps[n], "rinch-stepper__step-icon").expect("an icon box");
        let live = icon_box
            .children()
            .into_iter()
            .find(|c| !has_class(c, "rinch-stepper__step-icon-alt"))
            .expect("the step draws something");
        glyph(&live)
    };

    assert_eq!(
        live(0),
        Vec::<String>::new(),
        "precondition: the only step sits before `active`, so it is completed \
         and draws the built-in tick, which contributes no path data"
    );

    items.update(|v| v.insert(0, "a"));

    assert_eq!(
        live(1),
        glyph_of(TablerIcon::Home),
        "#716: the insertion moved the step to `active`, out of completed, and \
         its own `icon` is what it draws there. That glyph came from a \
         `TablerIcon` in the step's props — no patch of the rendered tree could \
         rebuild it, so the stepper had to have parked it rather than discarded \
         it when it moved the step *into* completed"
    );
}

// ------------------------------------------------ a keyed reorder moves backwards

/// The live (unwrapped) glyph in step `n`'s icon box, and the keys the box says
/// it holds a real glyph for.
fn step_icon(tree: &Tree, n: usize) -> (Vec<String>, String) {
    let steps = tree.find_all("rinch-stepper__step");
    let icon_box =
        find_by_class(&steps[n], "rinch-stepper__step-icon").expect("the step has an icon box");
    let live = icon_box
        .children()
        .into_iter()
        .find(|c| !has_class(c, "rinch-stepper__step-icon-alt"))
        .expect("the step draws something");
    (
        glyph(&live),
        icon_box.get_attribute("data-icon-has").unwrap_or_default(),
    )
}

#[test]
fn a_step_a_keyed_reorder_moves_backwards_keeps_its_own_completed_icon() {
    for icon in [TablerIcon::Star, TablerIcon::Bell, TablerIcon::Home] {
        assert!(!glyph_of(icon).is_empty());
    }
    assert_ne!(glyph_of(TablerIcon::Star), glyph_of(TablerIcon::Bell));

    let items = Signal::new(vec!["a", "b", "c"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32, completed_icon: TablerIcon::Bell,
                    for it in items.get() {
                        StepperStep {
                            key: it, label: it,
                            icon: TablerIcon::Home,
                            completed_icon: TablerIcon::Star,
                        }
                    }
                }
            }
        }
    });

    assert_eq!(
        step_icon(&tree, 0).0,
        glyph_of(TablerIcon::Star),
        "precondition: the step at 0 is completed and draws its own completed icon"
    );
    assert_eq!(
        step_icon(&tree, 2).0,
        glyph_of(TablerIcon::Home),
        "precondition: the step at 2 is inactive and draws its own plain icon"
    );

    // A keyed reorder. The reconcile repositions the live node for "c" with
    // `insert_before`, which is one of the four notifying verbs — and it moves
    // that node *backwards*, which the first cut of this pass assumed could not
    // happen.
    items.set(vec!["c", "a", "b"]);

    let (live, has) = step_icon(&tree, 0);
    assert_eq!(
        live,
        glyph_of(TablerIcon::Star),
        "#716: \"c\" is completed now and draws the completed icon *it* set. \
         Pruning its parked alternate left the built-in tick here, or — with a \
         `Stepper::completed_icon` set, as here — the stepper's own {:?}, which \
         inverts the rule that a step's icon wins",
        glyph_of(TablerIcon::Bell)
    );

    let mut tokens: Vec<&str> = has.split_whitespace().collect();
    let before = tokens.len();
    tokens.sort_unstable();
    tokens.dedup();
    assert_eq!(
        tokens.len(),
        before,
        "and `data-icon-has` holds each key once — rendered DOM, so a key twice \
         is a lie about the box. Keeping the alternates is what closed the path \
         that grew it, so this assertion no longer discriminates the `contains` \
         guard in `push_key`; it stands against a future one. It read {has:?}"
    );
}

// ------------------------------------------- a List directly inside a List

#[test]
fn a_list_directly_inside_a_list_keeps_its_own_rows() {
    assert_ne!(glyph_of(TablerIcon::Check), glyph_of(TablerIcon::X));

    let inner_items = Signal::new(vec!["one"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check,
                    List { icon: TablerIcon::X,
                        for it in inner_items.get() { ListItem { key: it, {it} } }
                    }
                }
            }
        }
    });

    let inner_rows = |tree: &Tree| -> Vec<NodeHandle> {
        let inner = tree.find_all("rinch-list").remove(1);
        let mut out = Vec::new();
        collect_by_class(&inner, "rinch-list__item", &mut out);
        out
    };

    for row in inner_rows(&tree) {
        let icon_box = find_by_class(&row, "rinch-list__item-icon").expect("an icon box");
        assert_eq!(
            glyph(&icon_box),
            glyph_of(TablerIcon::X),
            "#716: a `List` placed *directly* inside another — no `ListItem` \
             between them — owns its own rows at the outer list's render. The \
             outer walk reaches them through the inner `<ul>`, so it has to stop \
             at a nested list as well as at its own items"
        );
    }

    inner_items.update(|v| v.push("two"));

    let rows = inner_rows(&tree);
    assert_eq!(rows.len(), 2, "precondition: the inner list grew");
    for row in rows {
        let icon_box = find_by_class(&row, "rinch-list__item-icon").expect("an icon box");
        assert_eq!(
            glyph(&icon_box),
            glyph_of(TablerIcon::X),
            "and again for a row that arrives later: the upward boundary names \
             the same classes the downward walk stops at, or a row gets one \
             answer at render and the other one here"
        );
    }
}

// -------------------------------------------- a step that LEAVES (issue #745)

/// Each step's `data-step`, paired with its state as a bare word.
///
/// Reading both is what makes a removal fixture discriminating: a stepper that
/// renumbered but did not restate, or restated but did not renumber, fails here
/// rather than in half the assertions.
fn steps_now(tree: &Tree) -> Vec<(String, &'static str)> {
    tree.find_all("rinch-stepper__step")
        .iter()
        .map(|s| {
            let index = s.get_attribute("data-step").unwrap_or_default();
            let completed = has_class(s, "rinch-stepper__step--completed");
            let progress = has_class(s, "rinch-stepper__step--progress");
            let inactive = has_class(s, "rinch-stepper__step--inactive");
            let state = match (completed, progress, inactive) {
                (true, false, false) => "completed",
                (false, true, false) => "progress",
                (false, false, true) => "inactive",
                _ => panic!(
                    "a step carries exactly one state class, this one has {:?}",
                    s.get_attribute("class")
                ),
            };
            (index, state)
        })
        .collect()
}

fn at(index: &str, state: &'static str) -> (String, &'static str) {
    (index.to_string(), state)
}

#[test]
fn a_step_a_for_loop_drops_renumbers_and_restates_the_ones_after_it() {
    // Four steps at `active: 2`, so dropping the first moves a step *into*
    // progress and another *out* of it. Three steps at `active: 2` would leave
    // both survivors completed — a state fixed point where a stepper that
    // renumbers without restating passes.
    let items = Signal::new(vec!["a", "b", "c", "d"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 2u32,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "completed"),
            at("2", "progress"),
            at("3", "inactive"),
        ],
        "precondition"
    );

    items.update(|v| {
        v.remove(0);
    });

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "completed"),
            at("2", "progress"),
        ],
        "#745: dropping a step moves every step after it *backwards*, which \
         renumbers it and can restate it. Nothing told the stepper before, so \
         the survivors kept the number and the state they had one position \
         later — 1/2/3 with \"d\" still inactive"
    );
}

#[test]
fn dropping_the_step_that_is_active_hands_progress_to_the_one_behind_it() {
    // The step that leaves is the one `active` names, so no survivor keeps its
    // own state and the stepper cannot answer by doing nothing to them.
    let items = Signal::new(vec!["a", "b", "c"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "progress"),
            at("2", "inactive"),
        ],
        "precondition: \"b\" is the in-progress one"
    );

    items.update(|v| {
        v.remove(1);
    });

    assert_eq!(
        steps_now(&tree),
        vec![at("0", "completed"), at("1", "progress")],
        "#745: \"c\" inherits position 1, which is `active`, so it is the \
         in-progress step now"
    );
}

#[test]
fn the_number_a_surviving_step_draws_follows_it_backwards() {
    // `data-step` is an attribute; the number in the icon box is rendered text.
    // A stepper that renumbered the attribute and left the text alone would pass
    // every assertion above.
    let items = Signal::new(vec!["a", "b", "c"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 0u32,
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });

    let numbers = |tree: &Tree| -> Vec<String> {
        tree.find_all("rinch-stepper__step")
            .iter()
            .map(|s| {
                find_by_class(s, "rinch-stepper__step-icon")
                    .expect("every step has an icon box")
                    .text_content()
                    .unwrap_or_default()
            })
            .collect()
    };

    assert_eq!(
        numbers(&tree),
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "precondition"
    );

    items.update(|v| {
        v.remove(0);
    });

    assert_eq!(
        numbers(&tree),
        vec!["1".to_string(), "2".to_string()],
        "#745: the survivors draw 1 and 2, not the 2 and 3 they drew before"
    );
}

#[test]
fn a_step_removed_and_put_back_is_derived_from_where_it_lands() {
    // Removal then re-insertion, which is the pair a `show_dom` branch toggles.
    // The re-derivation has to be idempotent across both, or the second pass
    // leaves the stepper saying something the first did not.
    let shown = Signal::new(true);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: 1u32,
                    StepperStep { label: "a" }
                    if shown.get() { StepperStep { label: "b" } }
                    StepperStep { label: "c" }
                }
            }
        }
    });

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "progress"),
            at("2", "inactive"),
        ],
        "precondition"
    );

    shown.set(false);
    assert_eq!(
        steps_now(&tree),
        vec![at("0", "completed"), at("1", "progress")],
        "#745: the branch closing is a removal, and \"c\" takes position 1"
    );

    shown.set(true);
    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "progress"),
            at("2", "inactive"),
        ],
        "and the branch opening puts it back exactly where it was — the pass is \
         idempotent across a removal as well as an insertion"
    );
}

#[test]
fn a_step_moved_out_of_a_stepper_renumbers_the_ones_it_left_behind() {
    // A move is an insertion for the receiving container and a removal for the
    // one it came from. The receiving half is
    // `a_step_moved_into_another_stepper_is_restated_from_its_new_position`;
    // this is the other half, and only an *implicit* detach — `append_child` of
    // a node that already has a parent — reaches it.
    let tree = Tree::build(|scope| {
        let steps: Vec<NodeHandle> = ["a", "b", "c"]
            .iter()
            .map(|label| {
                StepperStep {
                    label: (*label).into(),
                    ..Default::default()
                }
                .render(scope, &[])
            })
            .collect();
        let first = Stepper {
            active: 1,
            ..Default::default()
        }
        .render(scope, &steps);
        let second = Stepper {
            active: 0,
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&first);
        root.append_child(&second);
        root
    });

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "progress"),
            at("2", "inactive"),
        ],
        "precondition: three steps in the first stepper, none in the second"
    );

    let receiving = tree.find_all("rinch-stepper__steps").remove(1);
    let leaving = tree.find_all("rinch-stepper__step").remove(0);
    receiving.append_child(&leaving);

    assert_eq!(
        steps_now(&tree),
        vec![
            at("0", "completed"),
            at("1", "progress"),
            at("0", "progress"),
        ],
        "#745: \"b\" and \"c\" slide down to 0 and 1 in the stepper they are \
         still in — so \"b\" leaves progress for completed and \"c\" arrives \
         in it — and \"a\", now last in document order, is position 0 of the \
         second stepper, whose `active: 0` makes it the in-progress one there"
    );
}

#[test]
fn a_reactive_active_survives_late_steps_arriving_and_leaving() {
    let active = Signal::new(0u32);
    let items = Signal::new(vec!["a", "b"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: {move || active.get()},
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });
    let st = |tree: &Tree| -> Vec<String> {
        tree.find_all("rinch-stepper__step")
            .iter()
            .map(|s| {
                let c = s.get_attribute("class").unwrap_or_default();
                ["completed", "progress", "inactive"]
                    .iter()
                    .find(|k| c.contains(&format!("--{k}")))
                    .unwrap()
                    .to_string()
            })
            .collect()
    };
    assert_eq!(st(&tree), vec!["progress", "inactive"]);
    active.set(1);
    assert_eq!(st(&tree), vec!["completed", "progress"]);
    items.update(|v| v.push("c"));
    assert_eq!(st(&tree), vec!["completed", "progress", "inactive"]);
    active.set(2);
    assert_eq!(st(&tree), vec!["completed", "completed", "progress"]);
    items.update(|v| v.insert(0, "z"));
    assert_eq!(
        st(&tree),
        vec!["completed", "completed", "progress", "inactive"]
    );
    items.update(|v| {
        v.remove(0);
    });
    assert_eq!(st(&tree), vec!["completed", "completed", "progress"]);
    active.set(0);
    assert_eq!(st(&tree), vec!["progress", "inactive", "inactive"]);
}
