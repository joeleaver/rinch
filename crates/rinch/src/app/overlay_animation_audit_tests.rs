//! Every overlay's un-hide pass, audited against one rule (issue #751).
//!
//! # The rule
//!
//! **A property a node declares a `transition` for, whose computed value changes
//! across the pass that reveals the overlay, must have a transition running on
//! that node afterwards.** [`assert_nothing_was_refused`] is that sentence in
//! code, and it is the exact negation of the Drawer's bug: the panel's
//! `transform` changed on the un-hide pass, the panel declared
//! `transition: transform`, and nothing ran, so the panel teleported.
//!
//! # Why this rule and not "zero transitions ran"
//!
//! Because "zero ran" is what the *bug* looks like. An overlay that correctly
//! has nothing to animate and an overlay that was refused an animation are both
//! at zero, so an audit written that way would pass for `Modal` today and go on
//! passing after somebody gave `Modal` the Drawer's exact shape. The
//! discriminator is whether anything the node transitions *moved*.
//!
//! The rule is satisfied vacuously by a correct overlay, and that is fine: it is
//! a tripwire for a future change, not a measurement of today. Its value is in
//! the failure it produces the moment a transitioned property starts changing on
//! an un-hide pass. **That it really does fail then is measured, twice**, and
//! both probes are worth re-running before trusting a change to this file:
//!
//! - Put `styles/drawer.rs` back on `display: none !important` and
//!   [`the_drawer_refuses_nothing`] fails with
//!   `Transform on rinch-drawer rinch-drawer--left rinch-drawer--md rinch-drawer--opened`.
//!   That is the original bug, caught by the rule.
//! - Add the obvious next feature to `styles/modal.rs` — `.rinch-modal__overlay
//!   { opacity: 1; transition: opacity 200ms ease; }` plus
//!   `.rinch-modal__root--hidden .rinch-modal__overlay { opacity: 0; }` — and
//!   [`modal_refuses_nothing`] fails with `Opacity on rinch-modal__overlay`,
//!   although that fixture is entirely vacuous today. **That is the important
//!   one**: it shows the tripwire fires on a component where the rule currently
//!   has nothing to check, which is what an audit is for.
//!
//! # Reading a value while a transition runs
//!
//! At t = 0 the cascade has written the **interpolated** value back over the
//! resolved one, so a node that *is* animating reads as unchanged and the rule
//! is vacuous for it too. That is why the positive controls
//! ([`the_drawer_refuses_nothing`], [`popover_animates_its_dropdown`],
//! [`select_animates_its_chevron`]) assert `active_transitions` directly
//! alongside the rule, rather than leaning on it.
//!
//! # The overlays that cannot be opened by flipping a signal
//!
//! `Modal`, `Notification`, `DropdownMenu`, `Popover` and `Drawer` all take an
//! `opened_fn` and toggle a class from one effect. `Select`,
//! `Tabs` and `Tooltip` keep their open state in a `Signal` created *inside*
//! `render` and never handed out, so a fixture has to go in through the handler
//! the component registered — `dispatch_event` on the node's `data-rid` /
//! `data-onenter`, which is the same call the click path makes. `Stepper` has no
//! reveal at all (its `__step-content` is unconditionally `display: none` and
//! nothing ever shows it) and derives state only on a fresh `render`, so its
//! entry swaps the state class on the live node — see
//! [`stepper_refuses_nothing_when_a_step_changes_state`].

use super::*;

use rinch_components::{
    Drawer, DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget, Modal,
    Notification, Popover, PopoverDropdown, PopoverTarget, Select, SelectOption, Stepper,
    StepperStep, Tab, Tabs, TabsList, TabsPanel, Tooltip,
};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Component, Signal};
use rinch_dom::computed_style::ComputedStyle;
use rinch_dom::transition::{diff_animatable, find_matching_spec};
use std::collections::HashMap;
use std::rc::Rc;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

// ── harness ──────────────────────────────────────────────────────────────

/// Mount `build` under the real component stylesheet and lay it out once.
fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = build(scope);
        root.append_child(&child);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// Every node's computed style, by raw id.
fn snapshot(app: &RinchApp) -> HashMap<usize, ComputedStyle> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .map(|(id, n)| (id, n.computed_style.clone()))
        .collect()
}

/// A node's `class` attribute, for a readable failure message.
fn class_of(app: &RinchApp, node: usize) -> String {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get("class").cloned())
        .unwrap_or_else(|| format!("<node {node}, no class>"))
}

fn running(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_transitions
        .get(&node)
        .map(|m| m.len())
        .unwrap_or(0)
}

/// The one node carrying `class` exactly. Panics unless exactly one does.
fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let found = nodes_with_class(app, class);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        found.len()
    );
    found[0]
}

fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<usize> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut found: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    found.sort_unstable();
    found
}

/// Fire the handler behind a node's `attr` (`data-rid`, `data-onenter`, …), the
/// way the click and hover paths do.
fn fire(app: &RinchApp, node: usize, attr: &str) {
    let rid = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(node)
            .and_then(|n| n.attributes.get(attr).cloned())
            .unwrap_or_else(|| panic!("node {node} carries no `{attr}`"))
            .parse::<usize>()
            .expect("a handler id")
    };
    assert!(
        dispatch_event(EventHandlerId(rid)),
        "the `{attr}` handler on node {node} did not dispatch"
    );
}

/// How many nodes were **revealed** by this pass — `display: none` before and
/// something else after, or `visibility: hidden` before and visible after.
///
/// The vacuity guard for every entry below: a fixture whose overlay never
/// un-hid measured nothing at all, and its "refused nothing" verdict would be
/// about an idle restyle.
fn revealed(app: &RinchApp, before: &HashMap<usize, ComputedStyle>) -> usize {
    use rinch_dom::computed_style::{DisplayValue, VisibilityValue};
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(id, n)| {
            let Some(old) = before.get(id) else {
                return false;
            };
            let was_gone = matches!(old.display, DisplayValue::None)
                || matches!(
                    old.visibility,
                    VisibilityValue::Hidden | VisibilityValue::Collapse
                );
            let is_here = !matches!(n.computed_style.display, DisplayValue::None)
                && matches!(n.computed_style.visibility, VisibilityValue::Visible);
            was_gone && is_here
        })
        .count()
}

/// **The audit rule.** Every animatable property that (a) changed across this
/// pass and (b) the node declares a `transition` for, has a transition running.
///
/// Returns how many nodes in the document declare at least one `transition`,
/// which the callers assert is non-zero: a document that declares none could
/// not fail this rule however broken it was.
#[must_use]
fn assert_nothing_was_refused(
    app: &RinchApp,
    before: &HashMap<usize, ComputedStyle>,
    what: &str,
) -> usize {
    let mut declaring = 0usize;
    let mut refused: Vec<String> = Vec::new();

    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        for (id, node) in d.tree.nodes.iter() {
            if node.transition_specs.is_empty() {
                continue;
            }
            declaring += 1;
            let Some(old) = before.get(&id) else {
                // A node created by this pass has no before-change style at
                // all, which is the one case §3 also refuses. Not this rule's
                // business.
                continue;
            };
            let live = d.tree.active_transitions.get(&id);
            for change in diff_animatable(old, &node.computed_style) {
                if find_matching_spec(&node.transition_specs, change.property).is_none() {
                    continue;
                }
                if live.is_some_and(|m| m.contains_key(&change.property)) {
                    continue;
                }
                refused.push(format!(
                    "  {:?} on `{}` (node {id}) changed with no transition running",
                    change.property,
                    node.attributes
                        .get("class")
                        .cloned()
                        .unwrap_or_else(|| "<no class>".into()),
                ));
            }
        }
    }

    assert!(
        refused.is_empty(),
        "{what}: a declared transition was refused on the reveal pass — this is \
         the #751 shape, where a property retargets in the same style pass its \
         ancestor stops being `display: none` and css-transitions-1 §3 starts \
         nothing. Stay rendered (`visibility`/`opacity`, as `Drawer` and \
         `Popover` do) or move the transition off the revealed subtree.\n{}",
        refused.join("\n")
    );

    declaring
}

/// The vacuity guard, asserted by every caller but `Tooltip`: a document in
/// which nothing declares a `transition` could not have failed the rule however
/// broken it was.
fn assert_something_declares_a_transition(declaring: usize, what: &str) {
    assert!(
        declaring > 0,
        "{what}: no node in this document declares a `transition` at all, so \
         `assert_nothing_was_refused` could not have failed. The fixture is \
         vacuous — check the component stylesheet actually loaded."
    );
}

// ── the overlays that open from a signal ─────────────────────────────────

/// The Drawer, through the shared rule — and the proof that the rule bites.
///
/// This is the one entry that was **failing** before #751, naming `Transform` on
/// `.rinch-drawer`. Everything else in this file is a tripwire; this one is the
/// tripwire's calibration.
#[test]
fn the_drawer_refuses_nothing() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        Drawer {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let before = snapshot(&app);
    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the drawer did open");
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Drawer"),
        "Drawer",
    );

    // The positive half the rule cannot give: the panel is animating, not
    // merely "not refused".
    let panel = node_with_class(&app, "rinch-drawer");
    assert_eq!(running(&app, panel), 1, "the panel's slide is running");
}

/// `Modal` hides its root with `display: none !important`, exactly as the
/// Drawer used to. It is safe today because its only `transition` sits on
/// `.rinch-modal__close` and names `background-color`/`color`, which change on
/// `:hover` and not on the open flip. This fixture is what fails if that stops
/// being true.
#[test]
fn modal_refuses_nothing() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        Modal {
            opened_fn: Some(Rc::new(move || opened.get())),
            title: "T".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, node_with_class(&app, "rinch-modal__root")),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed modal's root is `display: none`, so this pass \
         is a real un-hide — the exact shape #751 was about"
    );

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the modal did open");
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Modal"),
        "Modal",
    );
}

/// `Notification` hides **itself** (not a wrapper) with
/// `display: none !important`. Same audit, one level shallower.
#[test]
fn notification_refuses_nothing() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        Notification {
            opened_fn: Some(Rc::new(move || opened.get())),
            title: "T".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, node_with_class(&app, "rinch-notification")),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed notification is `display: none`"
    );

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the notification did open");
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Notification"),
        "Notification",
    );
}

/// `DropdownMenu` reveals its panel — and its backdrop — by toggling
/// `rinch-dropdown-menu--opened` on the root, which the sheet turns into
/// `display: block` on both (issue #760; it was an inline `style` rewrite per
/// panel before that). The `display` route is the one §3 refuses, whether it
/// arrives from a class or an inline write, which is why moving it into the
/// sheet changes nothing this fixture measures.
///
/// It is legitimate only while nothing on the panel declares a `transition`;
/// `css_hook_760_tests::the_dropdown_panel_declares_no_transition_so_a_display_reveal_is_legitimate`
/// is the assertion that says so, and fails the day one is added.
#[test]
fn dropdown_menu_refuses_nothing() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        let target = DropdownMenuTarget.render(scope, &[]);
        let item_text = scope.create_text("One");
        let item = DropdownMenuItem::default().render(scope, &[item_text]);
        let dropdown = DropdownMenuDropdown.render(scope, &[item]);
        DropdownMenu {
            opened_fn: Some(Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, node_with_class(&app, "rinch-dropdown-menu__dropdown")),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed panel is `display: none`"
    );

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the menu did open");
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "DropdownMenu"),
        "DropdownMenu",
    );
}

/// **The control.** `Popover` is the component the #751 cure was modelled on:
/// its dropdown is never `display: none`, so it is rendered throughout and its
/// `opacity` really transitions.
///
/// Without this, every "refuses nothing" verdict above would also hold in a
/// build where transitions never start at all.
#[test]
fn popover_animates_its_dropdown() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        let target = PopoverTarget.render(scope, &[]);
        let inner = scope.create_element("div");
        let dropdown = PopoverDropdown.render(scope, &[inner]);
        Popover {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "bottom".to_string(),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });

    let panel = node_with_class(&app, "rinch-popover__dropdown");
    let before = snapshot(&app);
    assert_ne!(
        display_of(&app, panel),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed popover panel is RENDERED — that is the whole \
         technique"
    );

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(
        running(&app, panel) > 0,
        "the popover's opacity transition runs, because its panel was rendered \
         all along"
    );
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Popover"),
        "Popover",
    );
}

/// `Popover` fades **out** as well as in (#759): closing holds the dropdown —
/// and the content under it, which inherits the held value — visible for the
/// 150ms fade, then hides it.
///
/// It declared `visibility 150ms ease` all along, which a browser honoured and
/// rinch dropped, so the dropdown vanished on the close pass on desktop only.
#[test]
fn popover_fades_its_dropdown_out() {
    use rinch_dom::computed_style::VisibilityValue;
    use rinch_dom::transition::TransitionProperty;

    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        let target = PopoverTarget.render(scope, &[]);
        let inner = scope.create_element("div");
        inner.set_attribute("class", "fixture-popover-content");
        let dropdown = PopoverDropdown.render(scope, &[inner]);
        Popover {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "bottom".to_string(),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });
    let panel = node_with_class(&app, "rinch-popover__dropdown");
    let content = node_with_class(&app, "fixture-popover-content");
    let vis = |app: &RinchApp, n: usize| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.get(n).unwrap().computed_style.visibility
    };

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    assert_eq!(vis(&app, panel), VisibilityValue::Visible, "precondition: open");

    opened.set(false);
    app.resolve_and_repaint(VIEWPORT.0 + 2.0, VIEWPORT.1);
    assert_eq!(
        vis(&app, panel),
        VisibilityValue::Visible,
        "the closing dropdown is held visible for its fade"
    );
    assert_eq!(
        vis(&app, content),
        VisibilityValue::Visible,
        "and so is its content, which inherits the held value"
    );

    let start = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.active_transitions[&panel][&TransitionProperty::Visibility].start_time_ms
    };
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, start + 200.0);
    }
    assert_eq!(vis(&app, panel), VisibilityValue::Hidden, "hidden once the fade is done");
    assert_eq!(vis(&app, content), VisibilityValue::Hidden, "content too");
}

// ── the overlays whose open state is internal ────────────────────────────

/// `Select` keeps `opened` in a `Signal` created inside `render`, so the fixture
/// goes in through the trigger's own `data-rid`.
///
/// It is also a second positive control: the chevron's `transform` rotates on
/// the same flip that reveals the dropdown, and the chevron sits in the
/// **trigger**, outside the revealed subtree — so it is rendered throughout and
/// its 200ms rotation really runs. That is the shape the Drawer should have had
/// and did not.
#[test]
fn select_animates_its_chevron() {
    let mut app = mount(move |scope| {
        Select {
            data: vec![SelectOption::new("a", "A"), SelectOption::new("b", "B")],
            ..Default::default()
        }
        .render(scope, &[])
    });

    let trigger = node_with_class(&app, "rinch-select__input");
    let chevron = node_with_class(&app, "rinch-select__chevron");
    let dropdown = node_with_class(&app, "rinch-select__dropdown");

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, dropdown),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the closed dropdown is `display: none`"
    );
    assert_ne!(
        display_of(&app, chevron),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the chevron is OUTSIDE the hidden dropdown — it is \
         rendered while the select is closed, which is why its rotation can \
         transition at all"
    );

    fire(&app, trigger, "data-rid");
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the dropdown did open");
    assert!(
        running(&app, chevron) > 0,
        "the chevron's 200ms rotation runs: `{}`",
        class_of(&app, chevron)
    );
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Select"),
        "Select",
    );
}

/// `Tabs` reveals a panel by taking the `hidden` attribute off it, driven by an
/// internal signal, so the fixture dispatches the tab button's handler. (It was
/// an inline `display` write until #760, which is when `.rinch-tabs__panel[hidden]`
/// stopped being a rule nothing matched.)
///
/// The panel itself declares no `transition`. The rule is what catches one being
/// added later — a fade-in on `.rinch-tabs__panel` would be refused exactly as
/// the Drawer's slide was.
///
/// This entry stopped being vacuous with #760: the same pass now moves the tab
/// button's `color` and its underline's `background-color`, both of which the
/// sheet declares a `transition` for, so the rule has something real to check.
/// The positive half — that those transitions actually *run* — is
/// `css_hook_760_tests::the_tab_indicator_transition_runs_on_a_switch`.
#[test]
fn tabs_refuses_nothing_when_a_panel_is_revealed() {
    let mut app = mount(move |scope| {
        let tab_a = Tab {
            value: "a".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        let tab_b = Tab {
            value: "b".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        let list = TabsList::default().render(scope, &[tab_a, tab_b]);
        let panel_a = TabsPanel {
            value: "a".to_string(),
        }
        .render(scope, &[]);
        let panel_b = TabsPanel {
            value: "b".to_string(),
        }
        .render(scope, &[]);
        Tabs {
            value: "a".to_string(),
            ..Default::default()
        }
        .render(scope, &[list, panel_a, panel_b])
    });

    // The second panel is the hidden one; `nodes_with_class` is document order.
    let panels = nodes_with_class(&app, "rinch-tabs__panel");
    assert_eq!(panels.len(), 2, "two panels");
    let hidden_panel = panels[1];
    let tabs = nodes_with_class(&app, "rinch-tabs__tab");
    assert_eq!(tabs.len(), 2, "two tabs");

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, hidden_panel),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the inactive panel is `display: none`"
    );

    fire(&app, tabs[1], "data-rid");
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the second panel was revealed");
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Tabs"),
        "Tabs",
    );
}

/// `Tooltip` shows its content by adding `rinch-tooltip--opened` to its root,
/// driven by an internal `hovered` signal that only its own `data-onenter`
/// handler writes. (It was an inline `display: block` on the content until
/// #760, when the class stopped being one nothing matched.)
///
/// `styles/tooltip.rs` declares **no** `transition` at all today, so the audit
/// leans on the rest of the document for its vacuity guard and on the reveal
/// count for its relevance. A `transition: opacity` added to
/// `.rinch-tooltip__content` — the obvious next feature — would be refused, and
/// this is what says so.
#[test]
fn tooltip_refuses_nothing_when_it_is_revealed() {
    let mut app = mount(move |scope| {
        let target = scope.create_element("span");
        Tooltip {
            label: "Hi".to_string(),
            ..Default::default()
        }
        .render(scope, &[target])
    });

    let root = node_with_class(&app, "rinch-tooltip");
    let content = node_with_class(&app, "rinch-tooltip__content");

    let before = snapshot(&app);
    assert_eq!(
        display_of(&app, content),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: an un-hovered tooltip's content is `display: none`"
    );

    fire(&app, root, "data-onenter");
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(revealed(&app, &before) > 0, "the tooltip was revealed");

    // The rule still runs — it is what fails if a transition is added and
    // refused — but Tooltip cannot assert the usual vacuity guard, because the
    // honest answer today is that it declares none at all. So it asserts *that*
    // instead, which is the same tripwire read from the other side.
    let declaring = assert_nothing_was_refused(&app, &before, "Tooltip");
    assert_eq!(
        declaring, 0,
        "`styles/tooltip.rs` declares no `transition`, and this fixture records \
         it so that adding one is a decision rather than an accident. If you are \
         adding one — a fade on `.rinch-tooltip__content` is the obvious \
         candidate — the content must stop being revealed with `display` first \
         (`styles/tooltip.rs` hides it and `.rinch-tooltip--opened` shows it), \
         or css-transitions-1 §3 will refuse it exactly as it refused the \
         Drawer's slide in #751. Then swap this assertion for \
         `assert_something_declares_a_transition`."
    );
}

/// `Stepper` has **no** reveal: `.rinch-stepper__step-content` is
/// unconditionally `display: none` and nothing ever shows it, and the step's
/// state is derived only on a fresh `render`. So there is no un-hide pass to
/// audit, and the shape worth pinning is the opposite one — the step icon and
/// the separator declare `transition: background-color`, and the class that
/// changes their colour lands on a node that was rendered all along.
///
/// The state class is swapped here rather than by the component, because
/// `settle_steps` runs only from `render`; what the fixture pins is that the
/// *declared* transitions sit on rendered nodes, so the day `Stepper` grows a
/// reactive `active` it animates instead of teleporting.
#[test]
fn stepper_refuses_nothing_when_a_step_changes_state() {
    let mut app = mount(move |scope| {
        let one = StepperStep {
            label: "One".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        let two = StepperStep {
            label: "Two".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        Stepper {
            active: 0,
            ..Default::default()
        }
        .render(scope, &[one, two])
    });

    let steps = nodes_with_class(&app, "rinch-stepper__step");
    assert_eq!(steps.len(), 2, "two steps");
    let icons = nodes_with_class(&app, "rinch-stepper__step-icon");
    assert_eq!(icons.len(), 2, "two step icons");

    let before = snapshot(&app);

    // Advance step two from `inactive` to `progress`, which is exactly what
    // `settle_steps` writes — on the live node, so the icon keeps its
    // before-change style.
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.set_attribute(
            rinch_core::dom::NodeId(steps[1]),
            "class",
            "rinch-stepper__step rinch-stepper__step--progress",
        );
    }
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert!(
        running(&app, icons[1]) > 0,
        "the step icon's colour transition runs — the icon was rendered before \
         and after the state change, so §3 has a before-change style"
    );
    assert_something_declares_a_transition(
        assert_nothing_was_refused(&app, &before, "Stepper"),
        "Stepper",
    );

    // And the fact that makes this component different from the seven above:
    // its step content is never revealed, in either state.
    let content = nodes_with_class(&app, "rinch-stepper__step-content");
    for c in content {
        assert_eq!(
            display_of(&app, c),
            rinch_dom::computed_style::DisplayValue::None,
            "`.rinch-stepper__step-content` is unconditionally hidden, so \
             Stepper has no un-hide pass to get wrong"
        );
    }
}

fn display_of(app: &RinchApp, node: usize) -> rinch_dom::computed_style::DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}
