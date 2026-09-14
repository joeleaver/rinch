//! The nine props #707 wired: #474's category **C** (a parent prop whose
//! child's twin works) and category **D** (content and input props).
//!
//! `tests/no_dead_props.rs` is the ratchet that says a prop is *read*; these say
//! the read has the effect its doc comment promises. They are different claims,
//! and only the second one notices a parent that reads its prop into a value it
//! then throws away.
//!
//! **Category C is parent→child plumbing, and a parent renders after its
//! children.** The rsx macro builds a component's children into a `<template>`
//! and hands the finished `NodeHandle`s to `Component::render`, so nothing a
//! parent knows can reach a child as a *prop*. Each of these six therefore
//! travels as the crate already does it — `AccordionItem` has found its own
//! control button among `children` since long before #474 — by patching the
//! rendered tree, or by publishing an attribute the cascade carries down.
//!
//! **Every "parent default" case is checked twice**, because one direction
//! proves nothing on its own: a parent that overwrote *every* child would pass a
//! fixture that only ever looks at a child which set nothing. So each prop has a
//! child-wins twin, and the two icons or sizes involved are always different
//! ones — a default of `md` against a child of `md`, or one icon against itself,
//! is a fixed point where the wired and the unwired code agree.
//!
//! The two props whose effect is a **cascade** rather than a DOM edit —
//! `Accordion::disable_chevron_rotation` and `Textarea::max_rows` — are pinned
//! here only as far as the markup and the stylesheet go. What a real Stylo
//! cascade computes from them lives in `rinch`'s `app::component_prop_707_tests`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::accordion::{Accordion, AccordionControl, AccordionItem, AccordionPanel};
use rinch_components::checkbox::Checkbox;
use rinch_components::list::{List, ListItem};
use rinch_components::radio::{Radio, RadioGroup};
use rinch_components::stepper::{Stepper, StepperStep};
use rinch_components::switch::Switch;
use rinch_components::textarea::Textarea;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, Signal};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

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

    fn find(&self, class: &str) -> NodeHandle {
        find_by_class(&self.root, class)
            .unwrap_or_else(|| panic!("the tree carries no `{class}` node"))
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

/// Every `d` attribute in `node`'s subtree, in document order.
///
/// This is what tells one rendered Tabler glyph from another: the two icons in
/// a parent-default/child-wins pair are the same `<svg>` box with different
/// path data, so nothing shallower discriminates them.
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

/// The glyph a given icon renders to, rendered on its own for comparison.
fn glyph_of(icon: TablerIcon) -> Vec<String> {
    let tree = Tree::build(move |scope| render_tabler_icon(scope, icon, TablerIconStyle::Outline));
    let paths = glyph(&tree.root);
    assert!(
        !paths.is_empty(),
        "{icon:?} renders no path data, so it cannot stand for itself in a \
         comparison — pick another icon"
    );
    paths
}

/// Every component's CSS, as the runtime loads it.
fn sheet() -> String {
    rinch_components::styles::generate_all_component_styles()
}

// ============================================================ C: List::icon

/// A list and one item, each optionally carrying an icon.
fn list_of_one(list_icon: Option<TablerIcon>, item_icon: Option<TablerIcon>) -> Tree {
    Tree::build(move |scope| {
        let text = scope.create_text("One");
        let item = ListItem { icon: item_icon }.render(scope, &[text]);
        List {
            icon: list_icon,
            ..Default::default()
        }
        .render(scope, &[item])
    })
}

#[test]
fn the_list_icon_reaches_an_item_that_set_none() {
    let tree = list_of_one(Some(TablerIcon::Check), None);
    let item = tree.find("rinch-list__item");

    assert!(
        has_class(&item, "rinch-list__item--with-icon"),
        "an item given the list's default icon takes the same class an item \
         with its own icon does — the stylesheet has one rule for both"
    );

    let icon_box = find_by_class(&item, "rinch-list__item-icon").expect("the item has an icon box");
    assert_eq!(
        glyph(&icon_box),
        glyph_of(TablerIcon::Check),
        "the icon in the box is the list's"
    );

    let content =
        find_by_class(&item, "rinch-list__item-content").expect("the item has a content box");
    assert_eq!(
        content.text_content().as_deref(),
        Some("One"),
        "the item's original content moved into the content box rather than \
         being dropped"
    );

    let kids = item.children();
    assert_eq!(
        kids.len(),
        2,
        "the item holds the icon box and the content box"
    );
    assert!(
        has_class(&kids[0], "rinch-list__item-icon"),
        "the icon comes first, as it does when the item builds the layout itself"
    );
}

#[test]
fn a_list_item_keeps_an_icon_of_its_own() {
    assert_ne!(
        glyph_of(TablerIcon::Check),
        glyph_of(TablerIcon::X),
        "precondition: the two icons this test distinguishes are distinguishable"
    );

    let tree = list_of_one(Some(TablerIcon::Check), Some(TablerIcon::X));
    let boxes = tree.find_all("rinch-list__item-icon");
    assert_eq!(
        boxes.len(),
        1,
        "the list did not add a second icon beside the item's own"
    );
    assert_eq!(
        glyph(&boxes[0]),
        glyph_of(TablerIcon::X),
        "the item's icon wins over the list's default"
    );
}

#[test]
fn a_list_with_no_icon_leaves_a_plain_item_plain() {
    let tree = list_of_one(None, None);
    assert!(
        tree.find_all("rinch-list__item-icon").is_empty(),
        "nothing to default from, so nothing is added"
    );
    assert!(!has_class(
        &tree.find("rinch-list__item"),
        "rinch-list__item--with-icon"
    ));
}

// ================================================= C: the two Stepper icons

/// A stepper of one step: the step's own props, then the stepper's.
fn stepper_of_one(step: StepperStep, stepper: Stepper) -> Tree {
    Tree::build(move |scope| {
        let step = step.render(scope, &[]);
        stepper.render(scope, &[step])
    })
}

#[test]
fn the_stepper_completed_icon_reaches_a_step_that_set_none() {
    let plain = stepper_of_one(
        StepperStep {
            state: "completed".into(),
            ..Default::default()
        },
        Stepper::default(),
    );
    let default_glyph = glyph(&plain.find("rinch-stepper__step-icon"));
    assert_ne!(
        default_glyph,
        glyph_of(TablerIcon::CircleCheck),
        "precondition: the built-in completed tick is not the icon this test \
         asks for, so seeing that icon means the prop reached the step"
    );

    let tree = stepper_of_one(
        StepperStep {
            state: "completed".into(),
            ..Default::default()
        },
        Stepper {
            completed_icon: Some(TablerIcon::CircleCheck),
            ..Default::default()
        },
    );
    assert_eq!(
        glyph(&tree.find("rinch-stepper__step-icon")),
        glyph_of(TablerIcon::CircleCheck),
        "the doc example on `Stepper` — `completed_icon: TablerIcon::CircleCheck` \
         — has to do something"
    );
}

#[test]
fn a_step_keeps_a_completed_icon_of_its_own() {
    let tree = stepper_of_one(
        StepperStep {
            state: "completed".into(),
            completed_icon: Some(TablerIcon::X),
            ..Default::default()
        },
        Stepper {
            completed_icon: Some(TablerIcon::CircleCheck),
            ..Default::default()
        },
    );
    assert_eq!(
        glyph(&tree.find("rinch-stepper__step-icon")),
        glyph_of(TablerIcon::X),
        "the step's icon wins over the stepper's default"
    );
}

#[test]
fn the_stepper_progress_icon_replaces_the_step_number() {
    let plain = stepper_of_one(
        StepperStep {
            state: "progress".into(),
            step: Some(1),
            ..Default::default()
        },
        Stepper::default(),
    );
    assert_eq!(
        plain
            .find("rinch-stepper__step-icon")
            .text_content()
            .as_deref(),
        Some("2"),
        "precondition: with nothing set, an in-progress step draws its number"
    );

    let tree = stepper_of_one(
        StepperStep {
            state: "progress".into(),
            step: Some(1),
            ..Default::default()
        },
        Stepper {
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
    );
    let icon_box = tree.find("rinch-stepper__step-icon");
    assert_eq!(glyph(&icon_box), glyph_of(TablerIcon::Home));
    assert_eq!(
        icon_box.text_content().as_deref().unwrap_or(""),
        "",
        "the number it replaced is gone, not merely covered"
    );
}

#[test]
fn the_stepper_progress_icon_outranks_a_steps_plain_icon() {
    let tree = stepper_of_one(
        StepperStep {
            state: "progress".into(),
            icon: Some(TablerIcon::X),
            ..Default::default()
        },
        Stepper {
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
    );
    assert_eq!(
        glyph(&tree.find("rinch-stepper__step-icon")),
        glyph_of(TablerIcon::Home),
        "the stepper's `progress_icon` stands in for the *`progress_icon`* the \
         step did not set, and that outranks the step's plain `icon` exactly as \
         the step's own would have"
    );
}

#[test]
fn a_step_keeps_a_progress_icon_of_its_own() {
    let tree = stepper_of_one(
        StepperStep {
            state: "progress".into(),
            progress_icon: Some(TablerIcon::X),
            ..Default::default()
        },
        Stepper {
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
    );
    assert_eq!(
        glyph(&tree.find("rinch-stepper__step-icon")),
        glyph_of(TablerIcon::X)
    );
}

#[test]
fn neither_stepper_icon_touches_an_inactive_step() {
    let tree = stepper_of_one(
        StepperStep {
            step: Some(2),
            ..Default::default()
        },
        Stepper {
            completed_icon: Some(TablerIcon::CircleCheck),
            progress_icon: Some(TablerIcon::Home),
            ..Default::default()
        },
    );
    let icon_box = tree.find("rinch-stepper__step-icon");
    assert!(
        glyph(&icon_box).is_empty(),
        "a step that is neither completed nor in progress is in neither \
         default's scope"
    );
    assert_eq!(icon_box.text_content().as_deref(), Some("3"));
}

// ========================================= C: Stepper::allow_next_steps_select

/// Four steps under a stepper whose `active` is 1, so the boundary this gates
/// is off both ends of the list — a fixture at `active: 0` agrees with a
/// mutant that counts from the active step rather than past it.
fn four_steps(allow_next_steps_select: bool, third_step_asks: bool) -> Tree {
    Tree::build(move |scope| {
        let mut steps = Vec::new();
        for i in 0..4u32 {
            steps.push(
                StepperStep {
                    step: Some(i),
                    allow_step_click: third_step_asks && i == 3,
                    ..Default::default()
                }
                .render(scope, &[]),
            );
        }
        Stepper {
            active: 1,
            allow_next_steps_select,
            ..Default::default()
        }
        .render(scope, &steps)
    })
}

fn clickable(tree: &Tree) -> Vec<bool> {
    tree.find_all("rinch-stepper__step")
        .iter()
        .map(|s| has_class(s, "rinch-stepper__step--clickable"))
        .collect()
}

#[test]
fn allow_next_steps_select_reaches_the_steps_past_the_active_one() {
    assert_eq!(
        clickable(&four_steps(true, false)),
        vec![false, false, true, true],
        "`active: 1`, so steps 2 and 3 are the next ones — step 1 is the active \
         one and is not past it"
    );
}

#[test]
fn a_stepper_that_does_not_allow_it_grants_nothing() {
    assert_eq!(
        clickable(&four_steps(false, false)),
        vec![false; 4],
        "off is the default, and off grants nothing"
    );
}

#[test]
fn a_step_that_asked_to_be_clickable_stays_clickable() {
    assert_eq!(
        clickable(&four_steps(false, true)),
        vec![false, false, false, true],
        "the stepper's prop only ever grants: a step's own `allow_step_click` is \
         not something it takes away"
    );
    assert_eq!(
        clickable(&four_steps(true, true)),
        vec![false, false, true, true],
        "and granting to a step that already asked does not double the class"
    );
    let tree = four_steps(true, true);
    let last = tree
        .find_all("rinch-stepper__step")
        .pop()
        .expect("four steps");
    let class = last.get_attribute("class").unwrap_or_default();
    assert_eq!(
        class
            .split_whitespace()
            .filter(|c| *c == "rinch-stepper__step--clickable")
            .count(),
        1,
        "`add_class` does not deduplicate, so the grant has to check first"
    );
}

// ====================================================== C: RadioGroup::size

fn group_of_one(group_size: &str, radio_size: &str) -> Tree {
    let group_size = group_size.to_string();
    let radio_size = radio_size.to_string();
    Tree::build(move |scope| {
        let radio = Radio {
            size: radio_size,
            ..Default::default()
        }
        .render(scope, &[]);
        RadioGroup {
            size: group_size,
            ..Default::default()
        }
        .render(scope, &[radio])
    })
}

fn size_classes(tree: &Tree) -> Vec<String> {
    tree.find("rinch-radio")
        .get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .filter(|c| c.starts_with("rinch-radio--") && !c.contains("checked"))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_group_size_reaches_a_radio_that_asked_for_none() {
    assert_eq!(
        size_classes(&group_of_one("lg", "")),
        vec!["rinch-radio--lg".to_string()],
        "the radio's defaulted `--md` was replaced, not joined — a radio \
         carrying two size classes is styled by whichever the sheet lists last"
    );
}

#[test]
fn a_radio_keeps_a_size_of_its_own() {
    assert_eq!(
        size_classes(&group_of_one("lg", "xs")),
        vec!["rinch-radio--xs".to_string()]
    );
}

#[test]
fn a_radio_that_asked_for_the_default_step_keeps_it() {
    assert_eq!(
        size_classes(&group_of_one("lg", "md")),
        vec!["rinch-radio--md".to_string()],
        "`md` is what an unset `size` produces too, so the class cannot say \
         which happened — this is the whole reason a radio records its ask as \
         `data-size`"
    );
}

#[test]
fn a_group_with_no_size_leaves_its_radios_alone() {
    assert_eq!(
        size_classes(&group_of_one("", "")),
        vec!["rinch-radio--md".to_string()]
    );
}

#[test]
fn a_reactive_radio_keeps_the_group_size_through_a_toggle() {
    let checked = Signal::new(false);
    let tree = Tree::build(move |scope| {
        let radio = Radio {
            checked_fn: Some(Rc::new(move || checked.get())),
            ..Default::default()
        }
        .render(scope, &[]);
        RadioGroup {
            size: "lg".into(),
            ..Default::default()
        }
        .render(scope, &[radio])
    });

    let radio = tree.find("rinch-radio");
    assert!(has_class(&radio, "rinch-radio--lg"), "precondition");

    checked.set(true);
    assert!(
        has_class(&radio, "rinch-radio--checked"),
        "precondition: the effect ran"
    );
    assert!(
        has_class(&radio, "rinch-radio--lg"),
        "the checked effect must add and remove one class rather than rewrite \
         the whole attribute — a rewrite drops every class put on the node after \
         render, which is the group's size and the rsx `class:` prop alike"
    );
}

// ========================================= C: Accordion::disable_chevron_rotation

/// One accordion item with a control and a panel, under an accordion that
/// either disables chevron rotation or does not.
fn accordion_of_one(disable_chevron_rotation: bool) -> Tree {
    Tree::build(move |scope| {
        let label = scope.create_text("Section");
        let control = AccordionControl::default().render(scope, &[label]);
        let panel = AccordionPanel.render(scope, &[]);
        let item = AccordionItem {
            value: "one".into(),
        }
        .render(scope, &[control, panel]);
        Accordion {
            disable_chevron_rotation,
            ..Default::default()
        }
        .render(scope, &[item])
    })
}

#[test]
fn disable_chevron_rotation_is_published_on_the_root() {
    assert_eq!(
        accordion_of_one(true)
            .root
            .get_attribute("data-disable-chevron-rotation")
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        accordion_of_one(false)
            .root
            .get_attribute("data-disable-chevron-rotation"),
        None,
        "the attribute is written only when asked for, so the sheet's \
         countermand cannot fire by accident"
    );
}

#[test]
fn an_open_item_rotates_its_chevron_with_a_class_not_an_inline_style() {
    let tree = accordion_of_one(false);
    let control = tree.find("rinch-accordion__control");
    let chevron = tree.find("rinch-accordion__chevron");

    assert!(!has_class(&chevron, "rinch-accordion__chevron--rotated"));

    let rid: usize = control
        .get_attribute("data-rid")
        .expect("the item wired the control")
        .parse()
        .expect("a handler id");
    assert!(dispatch_event(EventHandlerId(rid)), "the handler ran");

    assert!(
        has_class(&chevron, "rinch-accordion__chevron--rotated"),
        "open is a class. An inline `transform` is the top of the cascade, so \
         `disable_chevron_rotation` — read by a component that renders after \
         this one — would have nothing left to countermand it with"
    );
    assert!(
        !chevron
            .get_attribute("style")
            .unwrap_or_default()
            .contains("transform"),
        "and no inline transform is left behind to outrank the sheet"
    );

    assert!(dispatch_event(EventHandlerId(rid)));
    assert!(!has_class(&chevron, "rinch-accordion__chevron--rotated"));
}

#[test]
fn the_sheet_carries_the_rotation_and_its_countermand() {
    let css = sheet();
    assert!(
        css.contains(".rinch-accordion__chevron--rotated {\n    transform: rotate(180deg);"),
        "the class the item toggles has to mean something"
    );
    assert!(
        css.contains(
            ".rinch-accordion[data-disable-chevron-rotation=\"true\"] \
             .rinch-accordion__chevron--rotated {\n    transform: none;"
        ),
        "and the attribute the accordion publishes has to be spent"
    );
}

// ============================ D: Checkbox::description / Switch::description

#[test]
fn a_checkbox_description_renders_below_its_label() {
    let tree = Tree::build(|scope| {
        Checkbox {
            label: "Ship it".into(),
            description: "Deploys to production".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    assert!(has_class(&tree.root, "rinch-checkbox--with-description"));
    let body = tree.find("rinch-checkbox__body");
    let kids = body.children();
    assert_eq!(kids.len(), 2, "the body is the label and the description");
    assert_eq!(
        tree.find("rinch-checkbox__label").text_content().as_deref(),
        Some("Ship it")
    );
    assert_eq!(
        tree.find("rinch-checkbox__description")
            .text_content()
            .as_deref(),
        Some("Deploys to production")
    );
}

#[test]
fn a_checkbox_without_a_description_does_not_change_its_alignment() {
    let tree = Tree::build(|scope| {
        Checkbox {
            label: "Ship it".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert!(
        !has_class(&tree.root, "rinch-checkbox--with-description"),
        "centring a box on a one-line body is right, and stays"
    );
    assert!(find_by_class(&tree.root, "rinch-checkbox__description").is_none());
    assert_eq!(tree.find("rinch-checkbox__body").children().len(), 1);
}

#[test]
fn a_switch_description_renders_below_its_label() {
    let tree = Tree::build(|scope| {
        Switch {
            label: "Dark mode".into(),
            description: "Follows the system at startup".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    assert!(has_class(&tree.root, "rinch-switch--with-description"));
    assert_eq!(tree.find("rinch-switch__body").children().len(), 2);
    assert_eq!(
        tree.find("rinch-switch__label").text_content().as_deref(),
        Some("Dark mode")
    );
    assert_eq!(
        tree.find("rinch-switch__description")
            .text_content()
            .as_deref(),
        Some("Follows the system at startup")
    );
}

#[test]
fn a_switch_without_a_description_does_not_change_its_alignment() {
    let tree = Tree::build(|scope| {
        Switch {
            label: "Dark mode".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert!(!has_class(&tree.root, "rinch-switch--with-description"));
    assert!(find_by_class(&tree.root, "rinch-switch__description").is_none());
}

#[test]
fn the_sheet_styles_both_descriptions() {
    let css = sheet();
    for component in ["checkbox", "switch"] {
        assert!(
            css.contains(&format!(".rinch-{component}__description {{")),
            "a description nothing styles is the same bug one layer down"
        );
        assert!(
            css.contains(&format!(".rinch-{component}__body {{")),
            "the column the label and description sit in"
        );
        assert!(
            css.contains(&format!(
                ".rinch-{component}--with-description {{\n    align-items: flex-start;"
            )),
            "and the alignment a two-line body needs"
        );
    }
}

// ================================================= D: Textarea::max_rows

#[test]
fn max_rows_publishes_the_count_the_sheet_spends() {
    let tree = Tree::build(|scope| {
        Textarea {
            max_rows: Some(6),
            ..Default::default()
        }
        .render(scope, &[])
    });

    assert!(has_class(&tree.root, "rinch-textarea--max-rows"));
    let style = tree.root.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("--rinch-textarea-max-rows: 6"),
        "the count itself travels as a custom property, not as a class per \
         value: got {style:?}"
    );
}

#[test]
fn a_textarea_with_no_max_rows_publishes_nothing() {
    let tree = Tree::build(|scope| Textarea::default().render(scope, &[]));
    assert!(!has_class(&tree.root, "rinch-textarea--max-rows"));
    assert!(
        !tree
            .root
            .get_attribute("style")
            .unwrap_or_default()
            .contains("--rinch-textarea-max-rows"),
        "an unset cap must leave `max-height` alone entirely"
    );
}

#[test]
fn the_sheet_turns_the_row_count_into_a_height() {
    let css = sheet();
    assert!(
        css.contains(".rinch-textarea__input {") && css.contains("line-height: 1.5;"),
        "a row is only a known height while the line-height is declared"
    );
    assert!(
        css.contains(
            "max-height: calc(var(--rinch-textarea-max-rows) * 1.5em + \
             2 * var(--rinch-spacing-sm) + 2px);"
        ),
        "the cap counts rows at the declared line-height, then adds the padding \
         and border the theme's `box-sizing: border-box` folds into the box it \
         caps"
    );
}
