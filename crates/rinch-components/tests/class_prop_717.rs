//! A reactive effect owns **its own modifier class**, not the whole `class`
//! attribute (issue #717).
//!
//! `Checkbox` and `Switch` rebuilt the entire attribute from a `base_class`
//! string captured during `render`, so the first toggle discarded every class
//! put on the node *after* `render` returned. That is exactly where classes
//! arrive in this crate:
//!
//! - the rsx macro's universal `class:` prop — `generate_class_code` runs
//!   `add_class` on the `NodeHandle` a component **returned** (issue #647), so
//!   `Checkbox { class: "compact", checked_fn: … }` styled correctly until the
//!   user clicked it once;
//! - a parent patching a rendered child, which is how #474's category C props
//!   travel at all (`RadioGroup`'s size class, `Stepper`'s clickable class).
//!
//! #707 fixed `Radio` this way because `RadioGroup::size` tripped over it. This
//! file is the sweep: every effect in the crate that wrote a whole class string
//! now adds and removes the one class it is responsible for.
//!
//! **Each case is asserted in both directions and the caller's class is checked
//! after each.** One direction proves nothing on its own: an effect that only
//! ever *added* would pass a fixture that toggles on and stops, and the
//! whole-string rewrite this file exists to kill is itself correct on the
//! render pass — it only destroys on the **second** write. So every fixture
//! flips the signal on and off again, and reads the caller's class both times.
//!
//! The caller's class is applied here the way `rsx!` applies it: `add_class` on
//! the finished handle, after `render` has returned and after the effect's
//! first run. Applying it *before* would be a fixed point — the effect's
//! initial write happens during render, so a fixture that seeded the class
//! earlier would be measuring the wrong write.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::checkbox::Checkbox;
use rinch_components::color_input::ColorInput;
use rinch_components::drawer::Drawer;
use rinch_components::modal::Modal;
use rinch_components::navlink::NavLink;
use rinch_components::notification::Notification;
use rinch_components::popover::Popover;
use rinch_components::select::{Select, SelectOption};
use rinch_components::switch::Switch;
use rinch_components::tree::{Tree as TreeComponent, TreeNodeData, UseTreeOptions, UseTreeReturn};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, Signal};

mod common;
use common::{find_by_class, has_class};

/// The class a caller puts on the component — `class: "mine"` in rsx.
const CALLER: &str = "mine";

/// A rendered tree, with the document and scope that own it kept alive.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Mounted {
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
}

/// The first `data-rid` in `node`'s subtree, in document order.
fn rid(node: &NodeHandle) -> EventHandlerId {
    fn walk(node: &NodeHandle) -> Option<usize> {
        if let Some(id) = node.get_attribute("data-rid") {
            return id.parse().ok();
        }
        node.children().iter().find_map(walk)
    }
    EventHandlerId(walk(node).expect("the subtree carries no `data-rid`"))
}

/// Drive `flag` off→on→off and assert, at every step, that `modifier` follows
/// the flag **and** that the class the caller put on `node` is still there.
///
/// `on_when` is the flag value that puts `modifier` on: `--checked` and
/// `--opened` follow the flag, `--hidden` opposes it.
fn survives_both_ways(node: &NodeHandle, flag: &Signal<bool>, modifier: &str, on_when: bool) {
    let show = |n: &NodeHandle| n.get_attribute("class").unwrap_or_default();

    assert!(
        has_class(node, CALLER),
        "precondition: the caller's class is on the node before any toggle — got `{}`",
        show(node)
    );
    assert_eq!(
        has_class(node, modifier),
        !on_when,
        "precondition: `{modifier}` starts in the state the initial flag asks for — got `{}`",
        show(node)
    );

    flag.set(on_when);
    assert!(
        has_class(node, modifier),
        "positive control: `{modifier}` must actually toggle on, or this fixture \
         would pass against an effect that writes nothing at all — got `{}`",
        show(node)
    );
    assert!(
        has_class(node, CALLER),
        "#717: the effect kept `{modifier}` and threw `{CALLER}` away — got `{}`",
        show(node)
    );

    flag.set(!on_when);
    assert!(
        !has_class(node, modifier),
        "positive control: `{modifier}` must come off again — got `{}`",
        show(node)
    );
    assert!(
        has_class(node, CALLER),
        "#717: the effect's off branch rewrote the attribute too — got `{}`",
        show(node)
    );
}

// ---------------------------------------------------------------------------
// The two components #717 names. Their effect writes the node `render` returns,
// which is the node the rsx `class:` prop lands on.
// ---------------------------------------------------------------------------

#[test]
fn a_checkbox_keeps_the_callers_class_through_a_toggle() {
    let checked = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Checkbox {
            checked_fn: Some(Rc::new(move || checked.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    survives_both_ways(&mounted.root, &checked, "rinch-checkbox--checked", true);
}

#[test]
fn a_switch_keeps_the_callers_class_through_a_toggle() {
    let checked = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Switch {
            checked_fn: Some(Rc::new(move || checked.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    survives_both_ways(&mounted.root, &checked, "rinch-switch--checked", true);
}

// ---------------------------------------------------------------------------
// The rest of the sweep: every other effect in the crate that wrote a whole
// class string. These five write the returned root too, so the rsx `class:`
// prop reaches them all.
// ---------------------------------------------------------------------------

#[test]
fn a_popover_keeps_the_callers_class_through_a_toggle() {
    let opened = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Popover {
            opened_fn: Some(Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    survives_both_ways(&mounted.root, &opened, "rinch-popover--opened", true);
}

#[test]
fn a_modal_keeps_the_callers_class_through_a_toggle() {
    let opened = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Modal {
            opened_fn: Some(Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    // `--hidden` is the closed state, so it opposes the flag.
    survives_both_ways(&mounted.root, &opened, "rinch-modal__root--hidden", false);
}

#[test]
fn a_drawer_keeps_the_callers_class_on_both_the_root_and_the_panel() {
    let opened = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Drawer {
            opened_fn: Some(Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);
    // The panel is an inner node, so no rsx prop reaches it — but one effect
    // writes both, and a parent patching the panel is the #474 category C
    // shape. Seed it the same way and hold the effect to the same rule.
    let panel = mounted.find("rinch-drawer");
    panel.add_class(CALLER);

    survives_both_ways(&mounted.root, &opened, "rinch-drawer__root--hidden", false);
    assert!(
        has_class(&panel, CALLER),
        "the same effect writes the panel: got `{}`",
        panel.get_attribute("class").unwrap_or_default()
    );
    // And the panel's own modifier still tracks the flag, both ways.
    opened.set(true);
    assert!(has_class(&panel, "rinch-drawer--opened"));
    assert!(has_class(&panel, CALLER));
    opened.set(false);
    assert!(!has_class(&panel, "rinch-drawer--opened"));
    assert!(has_class(&panel, CALLER));
}

#[test]
fn a_notification_keeps_the_callers_class_through_a_toggle() {
    let opened = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Notification {
            opened_fn: Some(Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    survives_both_ways(&mounted.root, &opened, "rinch-notification--hidden", false);
}

#[test]
fn a_color_input_keeps_the_callers_class_when_its_dropdown_opens() {
    // `ColorInput` owns its `opened` signal, so the flip is a click on the
    // input group rather than a prop.
    let mounted = Mounted::build(|scope| ColorInput::default().render(scope, &[]));
    mounted.root.add_class(CALLER);

    let group = mounted.find("rinch-color-input__input-group");
    let toggle = rid(&group);

    assert!(!has_class(&mounted.root, "rinch-color-input--opened"));
    assert!(dispatch_event(toggle), "the toggle handler ran");
    assert!(
        has_class(&mounted.root, "rinch-color-input--opened"),
        "positive control: opening writes the modifier"
    );
    assert!(
        has_class(&mounted.root, CALLER),
        "#717: got `{}`",
        mounted.root.get_attribute("class").unwrap_or_default()
    );

    assert!(dispatch_event(toggle));
    assert!(!has_class(&mounted.root, "rinch-color-input--opened"));
    assert!(
        has_class(&mounted.root, CALLER),
        "#717, closing: got `{}`",
        mounted.root.get_attribute("class").unwrap_or_default()
    );
}

// ---------------------------------------------------------------------------
// Three effects write an **inner** node, so no rsx `class:` prop reaches them.
// The same rule still applies: a parent that patches a rendered child is how
// every #474 category C prop travels (`RadioGroup`'s size, `Stepper`'s
// clickable class), and it patches inner nodes by definition.
// ---------------------------------------------------------------------------

#[test]
fn a_navlink_keeps_a_class_added_to_its_anchor_after_render() {
    let active = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        NavLink {
            href: "#x".into(),
            active_fn: Some(Rc::new(move || active.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let anchor = mounted.find("rinch-navlink");
    anchor.add_class(CALLER);

    survives_both_ways(&anchor, &active, "rinch-navlink--active", true);
}

#[test]
fn a_navlink_button_keeps_a_class_added_after_render() {
    let active = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        NavLink {
            // No `href`, so the inner element is a <button>.
            active_fn: Some(Rc::new(move || active.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let button = mounted.find("rinch-navlink");
    button.add_class(CALLER);

    survives_both_ways(&button, &active, "rinch-navlink--active", true);
}

#[test]
fn a_select_keeps_a_class_added_to_its_display_and_its_options() {
    let value = Signal::new(String::new());
    let mounted = Mounted::build(move |scope| {
        Select {
            placeholder: "Pick one".into(),
            data: vec![SelectOption::new("a", "A"), SelectOption::new("b", "B")],
            value_fn: Some(Rc::new(move || value.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let display = mounted.find("rinch-select__display");
    display.add_class(CALLER);
    let option = mounted.find("rinch-select__option");
    option.add_class(CALLER);

    assert!(
        has_class(&display, "rinch-select__display--placeholder"),
        "precondition: an empty value shows the placeholder"
    );

    value.set("a".into());
    assert!(
        !has_class(&display, "rinch-select__display--placeholder"),
        "positive control: picking a value takes the placeholder class off"
    );
    assert!(
        has_class(&option, "rinch-select__option--selected"),
        "positive control: the first option is the selected one"
    );
    assert!(has_class(&display, CALLER), "#717, display");
    assert!(has_class(&option, CALLER), "#717, option");

    value.set(String::new());
    assert!(has_class(&display, "rinch-select__display--placeholder"));
    assert!(!has_class(&option, "rinch-select__option--selected"));
    assert!(has_class(&display, CALLER), "#717, display, back again");
    assert!(has_class(&option, CALLER), "#717, option, back again");
}

#[test]
fn a_tree_row_keeps_a_class_added_after_render() {
    let state = UseTreeReturn::new(UseTreeOptions::default());
    let mounted = Mounted::build(move |scope| {
        TreeComponent {
            data: vec![TreeNodeData::new("a", "A")],
            tree: Some(state),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let row = mounted.find("rinch-tree__node-content");
    row.add_class(CALLER);

    assert!(!has_class(&row, "rinch-tree__node-content--selected"));
    state
        .selected
        .set(std::iter::once("a".to_string()).collect());
    assert!(
        has_class(&row, "rinch-tree__node-content--selected"),
        "positive control: selection writes the modifier"
    );
    assert!(
        has_class(&row, CALLER),
        "#717: got `{}`",
        row.get_attribute("class").unwrap_or_default()
    );

    state.selected.set(Default::default());
    assert!(!has_class(&row, "rinch-tree__node-content--selected"));
    assert!(has_class(&row, CALLER), "#717, deselecting");
}
/// The other half of "an effect owns one class": it must not own it *twice*.
///
/// An effect re-runs whenever anything it read changes, and `Signal::set`
/// notifies on every write whether or not the value changed (`set_if_changed`
/// is the other method). Measured before `add_class` was made idempotent, three
/// `set(true)` calls left `rinch-checkbox mine rinch-checkbox--checked
/// rinch-checkbox--checked rinch-checkbox--checked` — unbounded growth that
/// healed only when the class next came off.
#[test]
fn a_re_run_that_changes_nothing_does_not_grow_the_attribute() {
    let checked = Signal::new(false);
    let mounted = Mounted::build(move |scope| {
        Checkbox {
            checked_fn: Some(Rc::new(move || checked.get())),
            ..Default::default()
        }
        .render(scope, &[])
    });
    mounted.root.add_class(CALLER);

    checked.set(true);
    let once = mounted.root.get_attribute("class").unwrap_or_default();
    checked.set(true);
    checked.set(true);
    let thrice = mounted.root.get_attribute("class").unwrap_or_default();

    assert_eq!(
        once, thrice,
        "two further writes of the same value must leave the attribute alone"
    );
    assert_eq!(
        thrice
            .split_whitespace()
            .filter(|c| *c == "rinch-checkbox--checked")
            .count(),
        1,
        "got `{thrice}`"
    );
    assert!(has_class(&mounted.root, CALLER));
}

/// The `Tree` **chevron**, which the first pass of this sweep missed.
///
/// It had both faults at once, twenty lines below the row this file already
/// covered: the expand effect wrote the whole `class` attribute, and that write
/// was also the only thing that ever gave the chevron its base class — the
/// static sibling `render_tree_node_static` has always written it at creation.
#[test]
fn a_tree_chevron_keeps_a_class_added_after_render() {
    let state = UseTreeReturn::new(UseTreeOptions::default());
    let mounted = Mounted::build(move |scope| {
        TreeComponent {
            data: vec![
                TreeNodeData::new("a", "A").with_children(vec![TreeNodeData::new("b", "B")]),
            ],
            tree: Some(state),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let chevron = mounted.find("rinch-tree__chevron");
    chevron.add_class(CALLER);

    assert!(!has_class(&chevron, "rinch-tree__chevron--expanded"));
    state
        .expanded
        .set(std::iter::once("a".to_string()).collect());
    assert!(
        has_class(&chevron, "rinch-tree__chevron--expanded"),
        "positive control: expanding writes the modifier"
    );
    assert!(
        has_class(&chevron, CALLER),
        "#717: got `{}`",
        chevron.get_attribute("class").unwrap_or_default()
    );

    state.expanded.set(Default::default());
    assert!(!has_class(&chevron, "rinch-tree__chevron--expanded"));
    assert!(has_class(&chevron, CALLER), "#717, collapsing");
    assert!(
        has_class(&chevron, "rinch-tree__chevron"),
        "the base class is written at creation, not by the effect"
    );
}
