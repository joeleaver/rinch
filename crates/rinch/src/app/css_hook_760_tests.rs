//! The four CSS hooks `rinch-components` declares for its own stylesheet, each
//! read back through the **real** cascade (issue #760).
//!
//! # What was wrong
//!
//! Four selectors in the shipped sheet matched nothing, because the component
//! that was supposed to set their hook styled itself inline instead:
//!
//! | Hook | Declared in | Set by |
//! |---|---|---|
//! | `.rinch-tooltip--opened` | nothing at all | `class_string`, matched by no rule |
//! | `.rinch-dropdown-menu--opened` | nothing at all | `class_string`, matched by no rule |
//! | `.rinch-tabs__panel[hidden]` | `styles/tabs.rs` | nobody |
//! | `.rinch-tabs__tab[data-active="true"]` / `--active` | `styles/tabs.rs`, 4 rules | nobody |
//!
//! Three of those were invisible — the components worked, by a different
//! mechanism, and only the next person to reach for the documented hook would
//! have found the silence. The fourth was not: the `default` variant's
//! underline is drawn by a `<div>` the component appends with inline styles,
//! while the 150ms `transition` for it sat on a `::after` rule keyed off
//! `data-active`, so **the tab underline did not animate**.
//!
//! # Why these fixtures mount the real sheet
//!
//! A hook is only wired if the *cascade* answers. Asserting that the component
//! writes an attribute proves nothing about whether a rule matches it, which is
//! the whole of what #760 is about — every one of the four hooks was written
//! somewhere and read nowhere. So each fixture goes through
//! [`mount`], which loads `rinch_components::generate_component_css()`, and
//! asserts on **computed** values.
//!
//! # The colours are declared, not inherited from a theme
//!
//! `generate_component_css()` carries no theme variables, so
//! `var(--rinch-tabs-color, var(--rinch-primary-color))` would be invalid at
//! computed-value time and `background-color` would compute to `transparent` in
//! *both* tab states. A transition fixture written that way sits exactly on the
//! fixed point it is meant to measure: nothing changes, nothing animates, and it
//! passes against a build where the hook is still dead. [`THEME`] therefore
//! declares the two variables the tab rules read, so the active and inactive
//! values are concrete and different.

use super::*;

use rinch_components::{
    DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget, Tab, Tabs, TabsList,
    TabsPanel, Tooltip,
};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Callback, Component, Signal};
use rinch_dom::computed_style::DisplayValue;
use std::rc::Rc;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The two custom properties `styles/tabs.rs` reads for its active colour, plus
/// the dimmed colour the inactive tab inherits, given concrete values so a
/// change between the two states is a real change. See the module doc.
const THEME: &str = ":root { --rinch-tabs-color: rgb(20, 90, 200); \
                    --rinch-color-dimmed: rgb(130, 130, 130); \
                    --rinch-color-border: rgb(200, 200, 200); \
                    --rinch-color-body: rgb(250, 250, 250); }";

const ACTIVE: peniko::Color = peniko::Color::from_rgb8(20, 90, 200);

// ── harness ──────────────────────────────────────────────────────────────

/// Mount `build` under the real component stylesheet plus [`THEME`].
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
        d.load_css(THEME);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    // Loading the sheet *after* the first layout is itself a style change, and
    // a property the sheet now declares a `transition` for can start one on that
    // pass. Run those to completion before the fixture looks at anything: while
    // a transition is live the cascade's resolved value is overwritten by the
    // interpolated one, so a colour assertion would read a frame of an animation
    // nothing in the test asked for, and a `running()` count after the switch
    // could not tell a new transition from a left-over one.
    finish_transitions(&mut app);
    app
}

/// Complete every running transition and drop it, so the document is settled.
fn finish_transitions(app: &mut RinchApp) {
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        let far_future = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
            + 10_000.0;
        rinch_dom::transition::tick_transitions(&mut d.tree, far_future);
    }
    app.resolve_and_repaint(VIEWPORT.0 + 0.5, VIEWPORT.1);
}

/// Lay out again at a *different* viewport: `resolve_layout` early-returns on a
/// clean tree, so re-resolving at the same size would measure nothing.
fn settle(app: &mut RinchApp, nth: f32) {
    app.resolve_and_repaint(VIEWPORT.0 + nth, VIEWPORT.1);
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

fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let found = nodes_with_class(app, class);
    assert_eq!(found.len(), 1, "expected one `{class}`, found {found:?}");
    found[0]
}

fn has_class(app: &RinchApp, node: usize, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get("class").cloned())
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn attr(app: &RinchApp, node: usize, name: &str) -> Option<String> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get(name).cloned())
}

fn display_of(app: &RinchApp, node: usize) -> DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}

fn color_of(app: &RinchApp, node: usize) -> Option<peniko::Color> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.color
}

fn background_of(app: &RinchApp, node: usize) -> Option<peniko::Color> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.background_color()
}

/// How many transitions are running on `node`.
fn running(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_transitions
        .get(&node)
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Whether `node` declares any `transition` at all.
fn declares_a_transition(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    !d.tree.get(node).unwrap().transition_specs.is_empty()
}

/// Fire the handler behind a node's `attr`, the way the click/hover paths do.
fn fire(app: &RinchApp, node: usize, name: &str) {
    let rid = attr(app, node, name)
        .unwrap_or_else(|| panic!("node {node} carries no `{name}`"))
        .parse::<usize>()
        .expect("a handler id");
    assert!(
        dispatch_event(EventHandlerId(rid)),
        "the `{name}` handler on node {node} did not dispatch"
    );
}

// ── 1. Tooltip ───────────────────────────────────────────────────────────

fn tooltip_app(opened: bool, disabled: bool) -> RinchApp {
    mount(move |scope| {
        let target = scope.create_element("span");
        Tooltip {
            label: "Hi".to_string(),
            opened,
            disabled,
            ..Default::default()
        }
        .render(scope, &[target])
    })
}

/// Hovering toggles `rinch-tooltip--opened` on the root, and **that class** is
/// what shows the content.
///
/// Both halves are needed and neither implies the other: before #760 the
/// content really did reveal (from an inline write) while the class was absent,
/// and a fix that set the class without adding the rule would leave the content
/// hidden.
#[test]
fn hovering_a_tooltip_toggles_the_opened_class_and_the_cascade_reveals_it() {
    let mut app = tooltip_app(false, false);
    let root = node_with_class(&app, "rinch-tooltip");
    let content = node_with_class(&app, "rinch-tooltip__content");

    assert!(
        !has_class(&app, root, "rinch-tooltip--opened"),
        "an un-hovered tooltip does not carry the class"
    );
    assert_eq!(
        display_of(&app, content),
        DisplayValue::None,
        "and its content is hidden by `.rinch-tooltip__content`'s own rule"
    );

    fire(&app, root, "data-onenter");
    settle(&mut app, 1.0);
    assert!(
        has_class(&app, root, "rinch-tooltip--opened"),
        "hover adds the class the component has always advertised"
    );
    assert_eq!(
        display_of(&app, content),
        DisplayValue::Block,
        "and `.rinch-tooltip--opened .rinch-tooltip__content` reveals it"
    );
    assert_eq!(
        attr(&app, content, "style"),
        None,
        "the reveal is the cascade's, not an inline `style` rewrite"
    );

    fire(&app, root, "data-onleave");
    settle(&mut app, 2.0);
    assert!(
        !has_class(&app, root, "rinch-tooltip--opened"),
        "leaving takes the class off again"
    );
    assert_eq!(display_of(&app, content), DisplayValue::None);
}

/// `opened: true` with no hover: `class_string` carries the class, so the
/// static case reaches the same rule.
///
/// The `style` assertion is what makes this discriminating rather than a fixed
/// point. `class_string` emitted `--opened` before #760 too, and the old inline
/// write also left the content at `display: block` — so the first two
/// assertions held against the broken code as readily as against this one, and
/// only "nobody wrote an inline style" tells the two mechanisms apart.
#[test]
fn a_statically_opened_tooltip_is_shown_by_the_same_rule() {
    let app = tooltip_app(true, false);
    let root = node_with_class(&app, "rinch-tooltip");
    let content = node_with_class(&app, "rinch-tooltip__content");
    assert!(has_class(&app, root, "rinch-tooltip--opened"));
    assert_eq!(display_of(&app, content), DisplayValue::Block);
    assert_eq!(
        attr(&app, content, "style"),
        None,
        "and it is the rule that shows it, not an inline `display: block`"
    );
}

/// `disabled` beats `opened`, which is a **source-order** fact rather than a
/// specificity one: `.rinch-tooltip--disabled .rinch-tooltip__content` and
/// `.rinch-tooltip--opened .rinch-tooltip__content` are both (0,2,0), so the
/// disabled rule only wins because it is later in `styles/tooltip.rs`.
///
/// Moving it above the `--opened` rule is the mutant this kills.
#[test]
fn a_disabled_tooltip_stays_hidden_even_when_opened() {
    let app = tooltip_app(true, true);
    let root = node_with_class(&app, "rinch-tooltip");
    assert!(
        has_class(&app, root, "rinch-tooltip--opened")
            && has_class(&app, root, "rinch-tooltip--disabled"),
        "the fixture really does put both classes on the root, or it proves nothing"
    );
    assert_eq!(
        display_of(&app, node_with_class(&app, "rinch-tooltip__content")),
        DisplayValue::None,
        "`--disabled` is declared after `--opened`, and ties go to the later rule"
    );
}

// ── 2. DropdownMenu ──────────────────────────────────────────────────────

/// Opening toggles `rinch-dropdown-menu--opened` on the root, and that one
/// class reveals **both** the panel and the outside-click backdrop.
#[test]
fn opening_a_dropdown_menu_toggles_the_opened_class_and_reveals_panel_and_backdrop() {
    let opened = Signal::new(false);
    let mut app = mount(move |scope| {
        let target = DropdownMenuTarget.render(scope, &[]);
        let item_text = scope.create_text("One");
        let item = DropdownMenuItem::default().render(scope, &[item_text]);
        let dropdown = DropdownMenuDropdown.render(scope, &[item]);
        DropdownMenu {
            opened_fn: Some(Rc::new(move || opened.get())),
            on_close: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });

    let root = node_with_class(&app, "rinch-dropdown-menu");
    let panel = node_with_class(&app, "rinch-dropdown-menu__dropdown");
    let backdrop = node_with_class(&app, "rinch-dropdown-menu__backdrop");

    assert!(!has_class(&app, root, "rinch-dropdown-menu--opened"));
    assert_eq!(display_of(&app, panel), DisplayValue::None);
    assert_eq!(display_of(&app, backdrop), DisplayValue::None);

    opened.set(true);
    settle(&mut app, 1.0);
    assert!(
        has_class(&app, root, "rinch-dropdown-menu--opened"),
        "the class the component has always advertised is now the open state"
    );
    assert_eq!(
        display_of(&app, panel),
        DisplayValue::Block,
        "`.rinch-dropdown-menu--opened .rinch-dropdown-menu__dropdown` reveals the panel"
    );
    assert_eq!(
        display_of(&app, backdrop),
        DisplayValue::Block,
        "and the same class reveals the backdrop, which used to need an effect of its own"
    );
    for (node, what) in [(panel, "panel"), (backdrop, "backdrop")] {
        assert_eq!(
            attr(&app, node, "style"),
            None,
            "the {what}'s reveal is the cascade's, not an inline `style` rewrite"
        );
    }

    opened.set(false);
    settle(&mut app, 2.0);
    assert!(!has_class(&app, root, "rinch-dropdown-menu--opened"));
    assert_eq!(display_of(&app, panel), DisplayValue::None);
    assert_eq!(display_of(&app, backdrop), DisplayValue::None);
}

/// The licence for revealing that panel with `display` at all: nothing on it
/// declares a `transition`, so css-transitions-1 §3 has nothing to refuse.
///
/// This is the #751 rule read as a precondition rather than an audit. The day
/// `styles/dropdown_menu.rs` grows a fade on the panel, this fails and says to
/// take the panel off `display` first — which is what `Popover` does.
#[test]
fn the_dropdown_panel_declares_no_transition_so_a_display_reveal_is_legitimate() {
    let app = mount(move |scope| {
        let target = DropdownMenuTarget.render(scope, &[]);
        let dropdown = DropdownMenuDropdown.render(scope, &[]);
        DropdownMenu {
            opened: true,
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });
    let panel = node_with_class(&app, "rinch-dropdown-menu__dropdown");
    assert!(
        !declares_a_transition(&app, panel),
        "`.rinch-dropdown-menu__dropdown` declares a transition now — a `display` \
         reveal refuses it (issue #751). Keep the panel rendered \
         (`visibility`/`opacity`) the way `styles/popover.rs` does, then delete \
         this assertion."
    );
}

// ── 3 & 4. Tabs ──────────────────────────────────────────────────────────

struct MountedTabs {
    app: RinchApp,
    tabs: Vec<usize>,
    panels: Vec<usize>,
    labels: Vec<usize>,
    indicators: Vec<usize>,
}

fn tabs_app(variant: &str) -> MountedTabs {
    let variant = variant.to_string();
    let app = mount(move |scope| {
        let label_a = scope.create_text("A");
        let tab_a = Tab {
            value: "a".to_string(),
            ..Default::default()
        }
        .render(scope, &[label_a]);
        let label_b = scope.create_text("B");
        let tab_b = Tab {
            value: "b".to_string(),
            ..Default::default()
        }
        .render(scope, &[label_b]);
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
            variant: variant.clone(),
            ..Default::default()
        }
        .render(scope, &[list, panel_a, panel_b])
    });
    let tabs = nodes_with_class(&app, "rinch-tabs__tab");
    let panels = nodes_with_class(&app, "rinch-tabs__panel");
    let labels = nodes_with_class(&app, "rinch-tabs__tab-label");
    let indicators = nodes_with_class(&app, "rinch-tabs__tab-indicator");
    assert_eq!(tabs.len(), 2, "two tabs");
    assert_eq!(panels.len(), 2, "two panels");
    assert_eq!(labels.len(), 2, "two labels");
    MountedTabs {
        app,
        tabs,
        panels,
        labels,
        indicators,
    }
}

/// Both hooks, in both states, on both tabs — and they move together on a
/// switch.
#[test]
fn a_tab_switch_moves_both_data_active_and_the_active_class() {
    let m = tabs_app("default");
    let mut app = m.app;

    assert_eq!(
        attr(&app, m.tabs[0], "data-active").as_deref(),
        Some("true")
    );
    assert_eq!(
        attr(&app, m.tabs[1], "data-active").as_deref(),
        Some("false"),
        "the inactive tab says so explicitly — `data-active` is a valued \
         attribute, not a boolean one, and `[data-active=\"true\"]` is what the \
         sheet matches"
    );
    assert!(has_class(&app, m.tabs[0], "rinch-tabs__tab--active"));
    assert!(!has_class(&app, m.tabs[1], "rinch-tabs__tab--active"));

    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);

    assert_eq!(
        attr(&app, m.tabs[0], "data-active").as_deref(),
        Some("false")
    );
    assert_eq!(
        attr(&app, m.tabs[1], "data-active").as_deref(),
        Some("true")
    );
    assert!(!has_class(&app, m.tabs[0], "rinch-tabs__tab--active"));
    assert!(has_class(&app, m.tabs[1], "rinch-tabs__tab--active"));
}

/// The colour the `[data-active="true"]` rule declares really reaches the label
/// text, which is what the deleted inline `set_style` on the label span used to
/// do by hand.
///
/// The label is a child of the button the rule matches, so this is also the
/// check that inherited `color` propagates across a restyle driven by an
/// *attribute* — the reason the old code gave for writing the span directly.
#[test]
fn the_active_tabs_colour_comes_from_the_cascade_and_reaches_the_label() {
    let m = tabs_app("default");
    let mut app = m.app;

    assert_eq!(color_of(&app, m.tabs[0]), Some(ACTIVE));
    assert_eq!(
        color_of(&app, m.labels[0]),
        Some(ACTIVE),
        "the label inherits it — the rule is on the button"
    );
    assert_ne!(
        color_of(&app, m.labels[1]),
        Some(ACTIVE),
        "the inactive label is the dimmed colour, not the active one"
    );

    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);
    // The button's `transition: color` is now running, and while it runs the
    // interpolated value is written back over the cascade's — so a colour read
    // here would be a frame of the animation. Let it land first; what this
    // fixture claims is where the cascade *puts* the colour, not how it gets
    // there. (`the_tab_indicator_transition_runs_on_a_switch` is the fixture
    // that claims the animation.)
    finish_transitions(&mut app);

    assert_eq!(color_of(&app, m.labels[1]), Some(ACTIVE));
    assert_ne!(color_of(&app, m.labels[0]), Some(ACTIVE));
    assert_eq!(
        attr(&app, m.labels[0], "style"),
        None,
        "and no inline `color` was written on the label at any point"
    );
}

/// **The one user-visible half of #760.** The `default` variant's underline
/// carries `transition: background-color 150ms ease`, and until the hook was
/// wired it could not fire: the declaration lived on a `::after` rule keyed off
/// `data-active`, and rinch creates no pseudo-element for `content: ''`, so the
/// box that was actually visible was a `<div>` with inline styles and no
/// transition at all.
///
/// Asserted on **both** indicators — the one arriving at the colour and the one
/// leaving it — because a fix that only styled the newly-active tab would pass
/// a one-sided check.
#[test]
fn the_tab_indicator_transition_runs_on_a_switch() {
    let m = tabs_app("default");
    let mut app = m.app;
    assert_eq!(
        m.indicators.len(),
        2,
        "the `default` variant appends one underline per tab"
    );

    for ind in &m.indicators {
        assert!(
            declares_a_transition(&app, *ind),
            "the underline declares its own transition, from the sheet"
        );
    }
    assert_eq!(
        background_of(&app, m.indicators[0]),
        Some(ACTIVE),
        "the active tab's underline is painted before any switch — so the \
         fixture is not measuring a from-nothing reveal"
    );
    assert_eq!(
        running(&app, m.indicators[0]),
        0,
        "and nothing is animating yet"
    );

    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);

    assert_eq!(
        running(&app, m.indicators[1]),
        1,
        "the arriving underline animates its `background-color`"
    );
    assert_eq!(
        running(&app, m.indicators[0]),
        1,
        "and so does the leaving one"
    );
    assert!(
        running(&app, m.tabs[1]) > 0,
        "the tab button's own `transition: color` runs too — it never could \
         before, because the active colour was written inline on the label span"
    );
}

/// The panels: `hidden` by presence, and `.rinch-tabs__panel[hidden]` is what
/// hides them.
///
/// The old mechanism showed a panel by writing `set_style("display", "")`,
/// which serialises the literal declaration `display: ` for the parser to
/// discard. The `style` assertions below are what fails if that comes back.
#[test]
fn only_the_inactive_panel_carries_hidden_and_the_rule_hides_it() {
    let m = tabs_app("default");
    let mut app = m.app;

    assert_eq!(attr(&app, m.panels[0], "hidden"), None, "the active panel");
    assert_eq!(
        attr(&app, m.panels[1], "hidden").as_deref(),
        Some(""),
        "the inactive one carries the bare presence form, as HTML asks"
    );
    assert_ne!(display_of(&app, m.panels[0]), DisplayValue::None);
    assert_eq!(
        display_of(&app, m.panels[1]),
        DisplayValue::None,
        "`.rinch-tabs__panel[hidden]` is live"
    );
    for p in &m.panels {
        assert_eq!(
            attr(&app, *p, "style"),
            None,
            "no inline `display` is written on a panel any more — the old show \
             path wrote the invalid declaration `display: `"
        );
    }

    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);

    assert_eq!(attr(&app, m.panels[0], "hidden").as_deref(), Some(""));
    assert_eq!(attr(&app, m.panels[1], "hidden"), None);
    assert_eq!(display_of(&app, m.panels[0]), DisplayValue::None);
    assert_ne!(display_of(&app, m.panels[1]), DisplayValue::None);
}

/// The two variants whose active styling is not just a colour: `pills` fills the
/// button, `outline` borders it. Both were inline `set_style` calls and both are
/// now the sheet's `[data-active="true"]` rules.
///
/// The `no_inline_style` checks are the discriminating half, for the reason
/// [`a_statically_opened_tooltip_is_shown_by_the_same_rule`] spells out: the old
/// inline writes computed to the **same** colours, so a fixture reading only the
/// computed value sits on a fixed point where both mechanisms agree.
#[test]
fn the_pills_and_outline_variants_are_styled_by_the_same_hook() {
    let m = tabs_app("pills");
    let mut app = m.app;
    assert!(
        m.indicators.is_empty(),
        "`pills` draws no underline, so none is appended"
    );
    assert_eq!(
        background_of(&app, m.tabs[0]),
        Some(ACTIVE),
        "`.rinch-tabs--pills .rinch-tabs__tab[data-active=\"true\"]` fills the pill"
    );
    no_inline_style(&app, &m.tabs);
    assert_ne!(background_of(&app, m.tabs[1]), Some(ACTIVE));
    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);
    // `.rinch-tabs__tab` declares `transition: background-color`, so read the
    // settled value rather than a frame of it.
    finish_transitions(&mut app);
    assert_eq!(background_of(&app, m.tabs[1]), Some(ACTIVE));
    assert_ne!(background_of(&app, m.tabs[0]), Some(ACTIVE));
    no_inline_style(&app, &m.tabs);

    let m = tabs_app("outline");
    let mut app = m.app;
    let body = peniko::Color::from_rgb8(250, 250, 250);
    let border = peniko::Color::from_rgb8(200, 200, 200);
    assert_eq!(background_of(&app, m.tabs[0]), Some(body));
    assert_eq!(
        {
            let doc = app.doc.as_ref().unwrap();
            let d = doc.borrow();
            d.tree
                .get(m.tabs[0])
                .unwrap()
                .computed_style
                .border_top_color
        },
        Some(border),
        "`.rinch-tabs--outline .rinch-tabs__tab[data-active=\"true\"]` borders it"
    );
    no_inline_style(&app, &m.tabs);
    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);
    finish_transitions(&mut app);
    assert_eq!(background_of(&app, m.tabs[1]), Some(body));
    assert_ne!(background_of(&app, m.tabs[0]), Some(body));
    no_inline_style(&app, &m.tabs);
}

/// No tab button carries an inline `style` at all. The active styling used to be
/// four `set_style` calls on the button (and one more on its label span); the
/// cascade owns all of it now.
fn no_inline_style(app: &RinchApp, nodes: &[usize]) {
    for n in nodes {
        assert_eq!(
            attr(app, *n, "style"),
            None,
            "node {n} carries an inline `style` — the active styling is the \
             sheet's, not `set_style`'s"
        );
    }
}

// ── the gap this fix had to work around ──────────────────────────────────

/// rinch creates **no** pseudo-element for `content: ''`, which is why the tab
/// underline is an element and not the `::after` the sheet used to declare.
///
/// `style_resolution/pseudo.rs` extracts the `content` text and returns early
/// when it is empty, so a decorative `::before`/`::after` — the only kind that
/// wants an empty `content` — never exists. Six rules in the shipped component
/// sheet are written that way (Refs #773); the Tabs one is the only one #760
/// moved off it.
///
/// Sampled against a non-empty `content`, which is the positive control: a
/// fixture that only looked at the empty case could not tell "no pseudo-element
/// here" from "no pseudo-elements at all".
#[test]
fn an_empty_content_creates_no_pseudo_element_but_a_non_empty_one_does() {
    fn pseudo_count(content: &str) -> usize {
        let css = format!(".probe::after {{ content: {content}; display: block; height: 2px; }}");
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let d = scope.create_element("div");
            d.set_attribute("class", "probe");
            d
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&css);
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .filter(|(_, n)| n.is_pseudo_element)
            .count()
    }

    assert_eq!(
        pseudo_count("'x'"),
        1,
        "the control: a non-empty `content` does create one"
    );
    assert_eq!(
        pseudo_count("''"),
        0,
        "and an empty one does not — so `.rinch-tabs__tab::after` never existed \
         on desktop, and the underline had to become a real element"
    );
}
