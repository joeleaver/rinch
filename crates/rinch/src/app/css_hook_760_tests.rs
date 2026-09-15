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

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use crate as rinch;
use rinch_core::element::IntoEventHandler;
use rinch_macros::rsx;

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
const WHITE: peniko::Color = peniko::Color::from_rgb8(255, 255, 255);

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
        "and `.rinch-tooltip--opened > .rinch-tooltip__content` reveals it"
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
/// specificity one: `.rinch-tooltip--disabled > .rinch-tooltip__content` and
/// `.rinch-tooltip--opened > .rinch-tooltip__content` are both (0,2,0), so the
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
        "the `.rinch-dropdown-menu--opened` panel rule reveals the panel"
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
    // What this does NOT show: that the label *text* fades. The text lives in
    // the label span, and on desktop a transition on an inherited property
    // reaches no descendant — `tick_transitions` writes only the button's own
    // `computed_style`, so the span resolves the end colour at once and the text
    // snaps (review of #774). In a browser the span inherits the animated value
    // and fades. The assertion is about the button's `ActiveTransition` only.
    assert!(
        running(&app, m.tabs[1]) > 0,
        "the tab button's own `transition: color` is started — it never could \
         be before, because the active colour was written inline on the label \
         span. (On desktop the label text still snaps: an inherited transition \
         does not reach descendants there. On web it fades.)"
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
        "`.rinch-tabs--pills > .rinch-tabs__list > .rinch-tabs__tab[data-active=\"true\"]` fills the pill"
    );
    no_inline_style(&app, &m.tabs);
    assert_ne!(background_of(&app, m.tabs[1]), Some(ACTIVE));
    // The pill's label is white, and that is the `pills` rule's `color: white`
    // and nothing else: the generic active rule alone would make it the tab
    // colour, which is what it reads with that declaration deleted. The inline
    // code this replaced set the white by hand on the label span.
    assert_eq!(
        color_of(&app, m.labels[0]),
        Some(WHITE),
        "`.rinch-tabs--pills … [data-active=\"true\"]` makes the active pill's text white"
    );
    assert_ne!(color_of(&app, m.labels[1]), Some(WHITE));
    fire(&app, m.tabs[1], "data-rid");
    settle(&mut app, 1.0);
    // `.rinch-tabs__tab` declares `transition: background-color`, so read the
    // settled value rather than a frame of it.
    finish_transitions(&mut app);
    assert_eq!(background_of(&app, m.tabs[1]), Some(ACTIVE));
    assert_ne!(background_of(&app, m.tabs[0]), Some(ACTIVE));
    assert_eq!(color_of(&app, m.labels[1]), Some(WHITE));
    assert_ne!(color_of(&app, m.labels[0]), Some(WHITE));
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
        "`.rinch-tabs--outline > .rinch-tabs__list > .rinch-tabs__tab[data-active=\"true\"]` borders it"
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

// ── 5. Nesting: one instance's state is not its descendants' ─────────────
//
// Moving a reveal from an inline write on the instance's own node to a sheet
// rule keyed off a class on its root changes who the rule can reach. A
// *descendant* combinator matches through any ancestor carrying the class, so
// an open `DropdownMenu` would open every closed menu inside its panel, and an
// opened `Tooltip` every tooltip inside its target. The inline writes never
// had that problem — each touched only its own nodes — so the first cut of
// #760 introduced it (review of #774, measured on both backends). Most of the
// rules are child combinators now, which matches exactly the nodes each
// component builds: the backdrop and the tooltip content are direct children
// of their root, and a tab is wired only when it is a direct child of a
// `TabsList` that is a direct child of the `Tabs` root (`Tabs::render` walks
// exactly that). The DropdownMenu **panel** is the exception — a caller's child,
// often behind an rsx wrapper — and its rule is a descendant rule with an
// exclusion instead; section 6 has why.
//
// Every fixture below puts the two instances in **different** states. Nested
// instances in the same state sit on the fixed point where a leaking rule and
// a correct one agree.

fn parent_of(app: &RinchApp, node: usize) -> Option<usize> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).and_then(|n| n.parent)
}

fn is_inside(app: &RinchApp, node: usize, ancestor: usize) -> bool {
    let mut cur = parent_of(app, node);
    while let Some(p) = cur {
        if p == ancestor {
            return true;
        }
        cur = parent_of(app, p);
    }
    false
}

/// The one node carrying `data-probe="{marker}"`.
fn probe(app: &RinchApp, marker: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.attributes.get("data-probe").is_some_and(|v| v == marker))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "expected one `[data-probe={marker}]`");
    found[0]
}

/// The one node with `class` whose parent is `parent`.
fn child_with_class(app: &RinchApp, parent: usize, class: &str) -> usize {
    let found: Vec<usize> = nodes_with_class(app, class)
        .into_iter()
        .filter(|n| parent_of(app, *n) == Some(parent))
        .collect();
    assert_eq!(found.len(), 1, "expected one `{class}` under node {parent}");
    found[0]
}

/// A closed `DropdownMenu` inside an open one's panel stays closed — panel
/// **and** backdrop.
///
/// Kills the panel rule without its `:not(…)` exclusion, and the backdrop rule
/// respelled with a descendant combinator: either way the outer root's class
/// matches the inner panel or backdrop through the outer panel.
#[test]
fn a_closed_dropdown_menu_nested_in_an_open_ones_panel_stays_closed() {
    let mut app = mount(move |scope| {
        let inner_target = DropdownMenuTarget.render(scope, &[]);
        let inner_dropdown = DropdownMenuDropdown.render(scope, &[]);
        let inner = DropdownMenu {
            opened_fn: Some(Rc::new(|| false)),
            on_close: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = DropdownMenuTarget.render(scope, &[]);
        let outer_dropdown = DropdownMenuDropdown.render(scope, &[inner]);
        let outer = DropdownMenu {
            opened_fn: Some(Rc::new(|| true)),
            on_close: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });
    settle(&mut app, 1.0);

    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    assert!(
        is_inside(&app, inner, outer),
        "the fixture really nests them"
    );
    let outer_panel = child_with_class(&app, outer, "rinch-dropdown-menu__dropdown");
    let outer_backdrop = child_with_class(&app, outer, "rinch-dropdown-menu__backdrop");
    let inner_panel = child_with_class(&app, inner, "rinch-dropdown-menu__dropdown");
    let inner_backdrop = child_with_class(&app, inner, "rinch-dropdown-menu__backdrop");

    assert_eq!(
        (
            display_of(&app, outer_panel),
            display_of(&app, outer_backdrop)
        ),
        (DisplayValue::Block, DisplayValue::Block),
        "control: the outer menu is open"
    );
    assert!(!has_class(&app, inner, "rinch-dropdown-menu--opened"));
    assert_eq!(
        display_of(&app, inner_panel),
        DisplayValue::None,
        "the outer menu's `--opened` does not reach the inner menu's panel"
    );
    assert_eq!(
        display_of(&app, inner_backdrop),
        DisplayValue::None,
        "nor its backdrop"
    );
}

/// **The sharp end of the leak.** A tap on the outer menu's item runs the item,
/// not the closed inner menu's `on_close`.
///
/// A leaked inner backdrop is `position: fixed` at `z-index: 99`, hoisted
/// (#545) into the outer panel's stacking context above the outer items, so it
/// covers the whole window inside that context and takes the tap: measured at
/// the first cut of #760 as item 0 / inner `on_close` 1. Driven through
/// `handle_event` with a real press and release at the item's painted centre,
/// so hit testing and stacking decide, not a direct dispatch.
///
/// Kills a revert of `.rinch-dropdown-menu--opened > .rinch-dropdown-menu__backdrop`
/// to a descendant combinator.
#[test]
fn a_tap_on_an_outer_menu_item_is_not_taken_by_a_closed_nested_menus_backdrop() {
    use std::cell::Cell;
    let item_clicks = Rc::new(Cell::new(0));
    let inner_closes = Rc::new(Cell::new(0));
    let (ic, cc) = (item_clicks.clone(), inner_closes.clone());
    let mut app = mount(move |scope| {
        let ic = ic.clone();
        let cc = cc.clone();
        let text = scope.create_text("Outer item");
        let item = DropdownMenuItem {
            onclick: Some(Callback::new(move || ic.set(ic.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[text]);
        item.set_attribute("data-probe", "outer-item");

        let inner_target = DropdownMenuTarget.render(scope, &[]);
        let inner_dropdown = DropdownMenuDropdown.render(scope, &[]);
        let inner = DropdownMenu {
            opened_fn: Some(Rc::new(|| false)),
            on_close: Some(Callback::new(move || cc.set(cc.get() + 1))),
            close_on_item_click: false,
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);

        let outer_target = DropdownMenuTarget.render(scope, &[]);
        let outer_dropdown = DropdownMenuDropdown.render(scope, &[item, inner]);
        DropdownMenu {
            opened_fn: Some(Rc::new(|| true)),
            on_close: Some(Callback::new(|| {})),
            close_on_item_click: false,
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown])
    });
    settle(&mut app, 1.0);

    let item = probe(&app, "outer-item");
    let (x, y) = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (x, y, w, h) = super::hit_testing::painted_element_box(&d.tree, item);
        assert!(w > 0.0 && h > 0.0, "the outer item is laid out and visible");
        (x + w / 2.0, y + h / 2.0)
    };
    for event in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(event, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    }

    assert_eq!(
        inner_closes.get(),
        0,
        "the closed inner menu's backdrop took the tap — it is shown by the \
         outer menu's `--opened` class"
    );
    assert_eq!(item_clicks.get(), 1, "the outer item ran");
}

fn nested_tooltips(outer_opened: bool, outer_disabled: bool, inner_opened: bool) -> RinchApp {
    mount(move |scope| {
        let inner_target = scope.create_element("span");
        let inner = Tooltip {
            label: "inner".to_string(),
            opened: inner_opened,
            ..Default::default()
        }
        .render(scope, &[inner_target]);
        inner.set_attribute("data-probe", "inner");
        let outer = Tooltip {
            label: "outer".to_string(),
            opened: outer_opened,
            disabled: outer_disabled,
            ..Default::default()
        }
        .render(scope, &[inner]);
        outer.set_attribute("data-probe", "outer");
        outer
    })
}

/// Hovering a tooltip shows **its** content, not that of an un-hovered tooltip
/// inside its target.
///
/// Opened by hover (the effect path), not statically, so the fixture exercises
/// the class the effect adds. Kills a revert of
/// `.rinch-tooltip--opened > .rinch-tooltip__content` to a descendant
/// combinator.
#[test]
fn hovering_a_tooltip_does_not_open_a_tooltip_nested_in_it() {
    let mut app = nested_tooltips(false, false, false);
    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    assert!(
        is_inside(&app, inner, outer),
        "the fixture really nests them"
    );
    let outer_content = child_with_class(&app, outer, "rinch-tooltip__content");
    let inner_content = child_with_class(&app, inner, "rinch-tooltip__content");

    fire(&app, outer, "data-onenter");
    settle(&mut app, 1.0);

    assert_eq!(
        display_of(&app, outer_content),
        DisplayValue::Block,
        "control: the hovered tooltip is open"
    );
    assert!(!has_class(&app, inner, "rinch-tooltip--opened"));
    assert_eq!(
        display_of(&app, inner_content),
        DisplayValue::None,
        "the outer tooltip's `--opened` does not reach the inner tooltip's content"
    );
}

/// An `opened` tooltip inside a `disabled` one is still shown.
///
/// Kills a revert of `.rinch-tooltip--disabled > .rinch-tooltip__content` to a
/// descendant combinator — a rule that is older than #760 but had no effect
/// while the reveal was an inline `display: block`, which outranked it.
#[test]
fn a_disabled_tooltip_does_not_hide_an_opened_tooltip_nested_in_it() {
    let app = nested_tooltips(false, true, true);
    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    let outer_content = child_with_class(&app, outer, "rinch-tooltip__content");
    let inner_content = child_with_class(&app, inner, "rinch-tooltip__content");

    assert_eq!(
        display_of(&app, outer_content),
        DisplayValue::None,
        "control: the disabled tooltip is closed"
    );
    assert_eq!(
        display_of(&app, inner_content),
        DisplayValue::Block,
        "the outer tooltip's `--disabled` does not hide the inner tooltip's content"
    );
}

/// A two-tab `Tabs` (value "a") tagged `data-probe="{marker}"`, whose panel "a"
/// holds `panel_a_kids`.
fn probe_tabs(
    scope: &mut RenderScope,
    variant: &str,
    orientation: &str,
    marker: &str,
    panel_a_kids: &[NodeHandle],
) -> NodeHandle {
    fn tab(scope: &mut RenderScope, value: &str) -> NodeHandle {
        let label = scope.create_text(value);
        Tab {
            value: value.to_string(),
            ..Default::default()
        }
        .render(scope, &[label])
    }
    let a = tab(scope, "a");
    let b = tab(scope, "b");
    let list = TabsList::default().render(scope, &[a, b]);
    let panel_a = TabsPanel {
        value: "a".to_string(),
    }
    .render(scope, panel_a_kids);
    let panel_b = TabsPanel {
        value: "b".to_string(),
    }
    .render(scope, &[]);
    let tabs = Tabs {
        value: "a".to_string(),
        variant: variant.to_string(),
        orientation: orientation.to_string(),
        ..Default::default()
    }
    .render(scope, &[list, panel_a, panel_b]);
    tabs.set_attribute("data-probe", marker);
    tabs
}

/// `default` Tabs inside a `pills` or an `outline` Tabs' panel keep the
/// `default` look on their active tab: no pill fill, no white text, no outline
/// box.
///
/// Before #760 the variant rules keyed off `[data-active="true"]` matched
/// nothing, so they could leak nowhere; wiring the hook made them reachable
/// from every nested `Tabs`. Kills a revert of either selector of the `pills`
/// or the `outline` active rule to a descendant combinator (both hooks are set
/// on every active tab, so one leaking selector is enough to show).
#[test]
fn nested_default_tabs_do_not_take_the_outer_variants_active_styling() {
    let body = peniko::Color::from_rgb8(250, 250, 250);
    for (outer_variant, outer_active_bg) in [("pills", ACTIVE), ("outline", body)] {
        let app = mount(move |scope| {
            let inner = probe_tabs(scope, "default", "", "inner", &[]);
            probe_tabs(scope, outer_variant, "", "outer", &[inner])
        });
        let outer = probe(&app, "outer");
        let inner = probe(&app, "inner");
        assert!(
            is_inside(&app, inner, outer),
            "the fixture really nests them"
        );
        let active: Vec<usize> = nodes_with_class(&app, "rinch-tabs__tab")
            .into_iter()
            .filter(|t| attr(&app, *t, "data-active").as_deref() == Some("true"))
            .collect();
        assert_eq!(active.len(), 2, "one active tab per Tabs");
        let outer_active = *active
            .iter()
            .find(|t| !is_inside(&app, **t, inner))
            .unwrap();
        let inner_active = *active.iter().find(|t| is_inside(&app, **t, inner)).unwrap();
        let inner_label = child_with_class(&app, inner_active, "rinch-tabs__tab-label");

        assert_eq!(
            background_of(&app, outer_active),
            Some(outer_active_bg),
            "control: the outer `{outer_variant}` active tab is styled"
        );
        assert_ne!(
            background_of(&app, inner_active),
            Some(outer_active_bg),
            "the outer `{outer_variant}` active rule reached a nested `default` tab"
        );
        assert_eq!(
            color_of(&app, inner_label),
            Some(ACTIVE),
            "the nested active `default` tab's text is the tab colour, not the \
             outer `{outer_variant}` rule's"
        );
    }
}

/// A horizontal `default` Tabs inside a **vertical** `default` Tabs' panel
/// keeps a horizontal underline, not the outer orientation's side bar.
///
/// Kills a revert of `.rinch-tabs--vertical.rinch-tabs--default > … >
/// .rinch-tabs__tab-indicator` to a descendant combinator, which measured a
/// 2 x 46 bar on the inner tab. The control is the outer indicator, which *is*
/// the side bar — so the two readings differ, and neither is a fixed point.
#[test]
fn nested_horizontal_tabs_keep_an_underline_inside_vertical_tabs() {
    let app = mount(move |scope| {
        let inner = probe_tabs(scope, "default", "", "inner", &[]);
        probe_tabs(scope, "default", "vertical", "outer", &[inner])
    });
    let inner = probe(&app, "inner");
    let indicators = nodes_with_class(&app, "rinch-tabs__tab-indicator");
    let size = |node: usize| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let l = d.tree.get(node).unwrap().layout;
        (l.width, l.height)
    };
    let outer_indicator = *indicators
        .iter()
        .find(|n| !is_inside(&app, **n, inner))
        .unwrap();
    let inner_indicator = *indicators
        .iter()
        .find(|n| is_inside(&app, **n, inner))
        .unwrap();

    let (ow, oh) = size(outer_indicator);
    assert!(
        ow == 2.0 && oh > 2.0,
        "control: the vertical Tabs' indicator is a 2px-wide side bar, got {ow}x{oh}"
    );
    let (iw, ih) = size(inner_indicator);
    assert!(
        ih == 2.0 && iw > 2.0,
        "the nested horizontal Tabs' indicator is a 2px-tall underline, got {iw}x{ih}"
    );
}

// ── 6. Wrappers: the panel is not always a direct child ──────────────────
//
// Round 2 answered the nesting leak with a child combinator on the panel too,
// `.rinch-dropdown-menu--opened > .rinch-dropdown-menu__dropdown`, and that
// broke compositions `main` handles, most of them through a wrapper the caller
// never wrote (second review of #774). `rsx!` puts a `display: contents`
// `<div>` between the menu root and the panel for a `{Option<NodeHandle>}`
// child, for every branch of an `if` after the first, for a component with a
// reactive prop inside an `if`, and for a helper component whose body is an
// `if`. Behind any of those the menu never opened.
//
// These fixtures are written with **real `rsx!`**, not `Component::render`:
// every other fixture in this file hand-builds its tree, which is exactly why
// none of them could see a wrapper the macro inserts. Each first asserts the
// wrapper is really there — without one it would be the direct-child control,
// on the fixed point where `>` and the descendant rule agree.

/// Whether `node` is attached to the document.
fn connected(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut cur = Some(node);
    while let Some(c) = cur {
        if c == d.tree.root_id {
            return true;
        }
        cur = d.tree.get(c).and_then(|n| n.parent);
    }
    false
}

/// The one **attached** node with `class` — a branch that was swapped out can
/// leave a detached one in the arena.
fn live_node_with_class(app: &RinchApp, class: &str) -> usize {
    let found: Vec<usize> = nodes_with_class(app, class)
        .into_iter()
        .filter(|n| connected(app, *n))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one attached `{class}`, found {found:?}"
    );
    found[0]
}

/// Whether `node` is rendered: neither it nor any ancestor computes
/// `display: none`. A `display` read on the panel alone is not enough — on
/// `main` the positional inline write landed on the wrapper, not the panel.
fn rendered(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut cur = Some(node);
    while let Some(c) = cur {
        let n = d.tree.get(c).unwrap();
        if n.computed_style.display == DisplayValue::None {
            return false;
        }
        cur = n.parent;
    }
    true
}

/// The menu opens and closes with its panel behind whatever `shape` put between
/// them: hidden, shown, hidden again.
fn assert_the_menu_opens_through(mut app: RinchApp, opened: Signal<bool>, shape: &str) {
    let menu = live_node_with_class(&app, "rinch-dropdown-menu");
    let panel = live_node_with_class(&app, "rinch-dropdown-menu__dropdown");
    assert!(
        is_inside(&app, panel, menu),
        "{shape}: the panel is in the menu"
    );
    assert_ne!(
        parent_of(&app, panel),
        Some(menu),
        "precondition ({shape}): something sits between the menu root and the panel — \
         without a wrapper this fixture is the direct-child control"
    );

    assert!(
        !rendered(&app, panel),
        "{shape}: the closed menu's panel is hidden"
    );
    opened.set(true);
    settle(&mut app, 1.0);
    assert_eq!(
        live_node_with_class(&app, "rinch-dropdown-menu__dropdown"),
        panel,
        "{shape}: opening does not rebuild the panel"
    );
    assert!(
        rendered(&app, panel),
        "{shape}: the open menu's panel is shown — `.rinch-dropdown-menu--opened` has \
         to reach a panel that is not a direct child of the root"
    );
    opened.set(false);
    settle(&mut app, 2.0);
    assert!(
        !rendered(&app, panel),
        "{shape}: and closing hides it again"
    );
}

/// A `DropdownMenuDropdown` passed as `{Option<NodeHandle>}`.
///
/// `IntoNode for Option<NodeHandle>` inserts a `display: contents` wrapper.
/// Killed by the panel rule respelled `--opened > __dropdown`.
#[test]
fn a_dropdown_passed_as_an_option_opens() {
    let opened = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        let dropdown: Option<NodeHandle> =
            Some(rsx! { DropdownMenuDropdown { DropdownMenuItem { "One" } } });
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                {dropdown}
            }
        }
    });
    assert_the_menu_opens_through(app, opened, "{Option<NodeHandle>}");
}

/// A `DropdownMenuDropdown` in the `else if` branch of an `if` — so the second
/// of `if loading { … } else if empty { … }` and every branch after it.
///
/// Killed by the panel rule respelled `--opened > __dropdown`.
#[test]
fn a_dropdown_in_an_else_if_branch_opens() {
    let opened = Signal::new(false);
    let mode = Signal::new(2u32);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                if mode.get() == 1 {
                    span { "loading" }
                } else if mode.get() == 2 {
                    DropdownMenuDropdown { DropdownMenuItem { "One" } }
                }
            }
        }
    });
    assert_the_menu_opens_through(app, opened, "else-if branch");
}

/// A helper component that builds the dropdown, used with a **reactive** prop.
#[derive(Debug, Default)]
struct MenuItems {
    label: String,
}

impl Component for MenuItems {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let label = self.label.clone();
        rsx! { DropdownMenuDropdown { DropdownMenuItem { {label} } } }
    }
}

/// A component with a reactive prop as the body of an `if` branch.
///
/// Killed by the panel rule respelled `--opened > __dropdown`.
#[test]
fn a_reactive_prop_component_in_an_if_opens() {
    let opened = Signal::new(false);
    let label = Signal::new("One".to_string());
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                if true { MenuItems { label: {move || label.get()} } }
            }
        }
    });
    assert_the_menu_opens_through(app, opened, "reactive-prop component in an `if`");
}

/// A helper component whose whole rsx body is an `if`.
#[derive(Debug, Default)]
struct MaybeMenuItems {
    show: bool,
}

impl Component for MaybeMenuItems {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let show = self.show;
        rsx! { if show { DropdownMenuDropdown { DropdownMenuItem { "One" } } } }
    }
}

/// A helper component whose body is control flow.
///
/// Killed by the panel rule respelled `--opened > __dropdown`.
#[test]
fn a_helper_whose_body_is_an_if_opens() {
    let opened = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                MaybeMenuItems { show: true }
            }
        }
    });
    assert_the_menu_opens_through(app, opened, "helper whose body is an `if`");
}

/// A plain author `<div>` around the panel opens too.
///
/// Round 2 documented the opposite ("a panel wrapped in an element of your own
/// stays hidden"), which was true of the `>` rule and is not of this one. Kept
/// as a fixture so that sentence cannot come back without a red test.
#[test]
fn a_dropdown_wrapped_in_an_author_div_opens() {
    let opened = Signal::new(false);
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                div { DropdownMenuDropdown { DropdownMenuItem { "One" } } }
            }
        }
    });
    assert_the_menu_opens_through(app, opened, "author `div`");
}

/// The same wrapper shapes cannot reach `Tooltip`'s content: `Tooltip::render`
/// appends it to the root itself, so the `>` rule is safe there however the
/// tooltip is composed. Here the tooltip is behind an `else if` wrapper and
/// still opens on hover.
#[test]
fn a_tooltip_behind_an_rsx_wrapper_still_opens() {
    let mode = Signal::new(2u32);
    let mut app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                if mode.get() == 1 {
                    span { "none" }
                } else if mode.get() == 2 {
                    Tooltip { label: "hi", button { "b" } }
                }
            }
        }
    });
    let root = live_node_with_class(&app, "rinch-tooltip");
    let content = live_node_with_class(&app, "rinch-tooltip__content");
    let wrapper = parent_of(&app, root).unwrap();
    assert_eq!(
        attr(&app, wrapper, "class"),
        None,
        "precondition: the tooltip sits in rsx's classless `else if` wrapper"
    );
    assert!(
        attr(&app, wrapper, "style").is_some_and(|s| s.contains("contents")),
        "precondition: and that wrapper is `display: contents`"
    );
    assert_eq!(
        parent_of(&app, content),
        Some(root),
        "the content is the root's own child"
    );
    assert!(!rendered(&app, content));
    fire(&app, root, "data-onenter");
    settle(&mut app, 1.0);
    assert!(
        rendered(&app, content),
        "hovering reveals the content through the wrapper"
    );
}

/// An open `DropdownMenu` inside a **closed** one's target shows its panel.
///
/// Kills the exclusion without its leading `.rinch-dropdown-menu--opened`
/// (`:not(.rinch-dropdown-menu:not(.rinch-dropdown-menu--opened) .rinch-dropdown-menu__dropdown)`),
/// which hides a panel behind **any** closed menu root, open ancestor or not —
/// so a menu in a closed menu's trigger never opens. Every other fixture here
/// survives that mutant (third review of #774).
#[test]
fn an_open_dropdown_menu_in_a_closed_ones_target_is_shown() {
    let outer_open = Signal::new(false);
    let inner_open = Signal::new(false);
    let mut app = mount(move |__scope: &mut RenderScope| {
        rsx! {
            DropdownMenu { opened_fn: move || outer_open.get(), on_close: || {},
                DropdownMenuTarget {
                    DropdownMenu { opened_fn: move || inner_open.get(), on_close: || {},
                        DropdownMenuTarget { button { "inner" } }
                        DropdownMenuDropdown { class: "probe-inner", DropdownMenuItem { "i" } }
                    }
                }
                DropdownMenuDropdown { class: "probe-outer", DropdownMenuItem { "o" } }
            }
        }
    });
    let inner = live_node_with_class(&app, "probe-inner");
    let outer = live_node_with_class(&app, "probe-outer");
    assert!(
        !rendered(&app, inner),
        "the closed inner menu's panel is hidden"
    );
    inner_open.set(true);
    settle(&mut app, 1.0);
    assert!(!rendered(&app, outer), "control: the outer menu is closed");
    assert!(
        rendered(&app, inner),
        "a menu in a closed menu's trigger opens"
    );
}

/// A **closed** menu whose own panel sits behind an rsx wrapper, nested in an
/// open menu's panel, stays closed — and opens with its own menu.
///
/// Kills the exclusion spelled with `>` before the panel
/// (`:not(.rinch-dropdown-menu--opened .rinch-dropdown-menu:not(.rinch-dropdown-menu--opened) > .rinch-dropdown-menu__dropdown)`).
/// That spelling is the obvious way to make the known limit exact, and it turns
/// the limit pin red; this fixture is what says it also brings back round 1's
/// leak for a wrapped nested panel. Without it only the limit pin caught that
/// mutant (third review of #774).
#[test]
fn a_closed_menu_with_a_wrapped_panel_nested_in_an_open_ones_panel_stays_closed() {
    let outer_open = Signal::new(false);
    let inner_open = Signal::new(false);
    let mut app = mount(move |__scope: &mut RenderScope| {
        let inner_panel: Option<NodeHandle> =
            Some(rsx! { DropdownMenuDropdown { class: "probe-inner", DropdownMenuItem { "i" } } });
        rsx! {
            DropdownMenu { opened_fn: move || outer_open.get(), on_close: || {},
                DropdownMenuTarget { button { "outer" } }
                DropdownMenuDropdown { class: "probe-outer",
                    DropdownMenu { opened_fn: move || inner_open.get(), on_close: || {},
                        DropdownMenuTarget { button { "inner" } }
                        {inner_panel}
                    }
                }
            }
        }
    });
    outer_open.set(true);
    settle(&mut app, 1.0);
    let inner = live_node_with_class(&app, "probe-inner");
    let outer = live_node_with_class(&app, "probe-outer");
    let wrapper = parent_of(&app, inner).unwrap();
    assert!(
        attr(&app, wrapper, "style").is_some_and(|s| s.contains("contents")),
        "precondition: the inner panel sits behind rsx's `display: contents` wrapper"
    );
    assert!(rendered(&app, outer), "control: the outer menu is open");
    assert!(
        !rendered(&app, inner),
        "the closed inner menu's wrapped panel stays hidden"
    );
    inner_open.set(true);
    settle(&mut app, 2.0);
    assert!(rendered(&app, inner), "and opens with its own menu");
}

/// An **open** menu nested in an open menu's panel shows its panel — a submenu.
///
/// The other half of the exclusion in the panel rule: only a *closed* menu root
/// between an open ancestor and a panel hides it. Killed by an exclusion that
/// forgets the `:not(.rinch-dropdown-menu--opened)` on that intermediate root,
/// which hides every nested panel whatever state its own menu is in — and which
/// every other fixture here survives, because each nests a closed menu or none.
#[test]
fn an_open_dropdown_menu_nested_in_an_open_ones_panel_is_shown() {
    let app = mount(move |scope| {
        let inner_target = DropdownMenuTarget.render(scope, &[]);
        let inner_dropdown = DropdownMenuDropdown.render(scope, &[]);
        let inner = DropdownMenu {
            opened: true,
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");
        let outer_target = DropdownMenuTarget.render(scope, &[]);
        let outer_dropdown = DropdownMenuDropdown.render(scope, &[inner]);
        let outer = DropdownMenu {
            opened: true,
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });
    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    let outer_panel = child_with_class(&app, outer, "rinch-dropdown-menu__dropdown");
    let inner_panel = child_with_class(&app, inner, "rinch-dropdown-menu__dropdown");
    assert!(
        rendered(&app, outer_panel),
        "control: the outer menu is open"
    );
    assert!(
        rendered(&app, inner_panel),
        "an open menu inside an open menu's panel shows its own panel"
    );
}

/// **The known limit of the panel rule, recorded.** The rule shows a panel under
/// an open root unless a *closed* menu root sits between some open ancestor and
/// the panel. The limit, in the words of the note on that rule in
/// `styles/dropdown_menu.rs`: an open menu C inside the *target* (trigger) of a
/// closed menu B that is itself inside an open menu A stays hidden — B is closed
/// and sits between A and C's panel, though C's own root is open.
///
/// "Inside A" means **either** half of A: B in A's panel **or** in A's target.
/// Both shapes are pinned, because a spelling that repaired only one of them
/// would otherwise pass (third review of #774 measured both, and a deeper stack
/// of closed menus in triggers, as this same limit). All of them opened on
/// `main`.
///
/// A menu in the trigger of a menu in a menu is not a composition anything in the
/// repo builds. This fixture is here so a change to the limit cannot land without
/// someone reading the note on the rule first.
#[test]
fn known_limit_an_open_menu_in_a_closed_menus_target_inside_an_open_menu_stays_hidden() {
    for b_in_a_target in [false, true] {
        let app = mount(move |scope| {
            let c_target = DropdownMenuTarget.render(scope, &[]);
            let c_dropdown = DropdownMenuDropdown.render(scope, &[]);
            let c = DropdownMenu {
                opened: true,
                ..Default::default()
            }
            .render(scope, &[c_target, c_dropdown]);
            c.set_attribute("data-probe", "c");
            let b_target = DropdownMenuTarget.render(scope, &[c]);
            let b_dropdown = DropdownMenuDropdown.render(scope, &[]);
            let b = DropdownMenu {
                opened: false,
                ..Default::default()
            }
            .render(scope, &[b_target, b_dropdown]);
            b.set_attribute("data-probe", "b");
            let (a_target, a_dropdown) = if b_in_a_target {
                (
                    DropdownMenuTarget.render(scope, &[b]),
                    DropdownMenuDropdown.render(scope, &[]),
                )
            } else {
                (
                    DropdownMenuTarget.render(scope, &[]),
                    DropdownMenuDropdown.render(scope, &[b]),
                )
            };
            let a = DropdownMenu {
                opened: true,
                ..Default::default()
            }
            .render(scope, &[a_target, a_dropdown]);
            a.set_attribute("data-probe", "a");
            a
        });
        let shape = if b_in_a_target {
            "B in A's target"
        } else {
            "B in A's panel"
        };
        let a = probe(&app, "a");
        let b = probe(&app, "b");
        let c = probe(&app, "c");
        let a_panel = child_with_class(&app, a, "rinch-dropdown-menu__dropdown");
        let b_panel = child_with_class(&app, b, "rinch-dropdown-menu__dropdown");
        let c_panel = child_with_class(&app, c, "rinch-dropdown-menu__dropdown");
        let c_target = child_with_class(&app, b, "rinch-dropdown-menu__target");
        assert!(rendered(&app, a_panel), "control ({shape}): A is open");
        assert!(!rendered(&app, b_panel), "control ({shape}): B is closed");
        assert!(
            rendered(&app, c_target),
            "control ({shape}): B's target, which holds C, is on screen"
        );
        assert!(
            has_class(&app, c, "rinch-dropdown-menu--opened"),
            "C is open"
        );
        assert!(
            !rendered(&app, c_panel),
            "the documented limit ({shape}): C's panel is hidden. If this has gone red, \
             the panel rule changed — and the obvious edit that makes this shape exact \
             (`> .rinch-dropdown-menu__dropdown` inside the `:not()`) also brings back \
             the nesting leak for a closed menu whose panel sits behind an rsx wrapper. \
             Re-check `a_closed_menu_with_a_wrapped_panel_nested_in_an_open_ones_panel_stays_closed` \
             before touching this pin; only if that is still green, flip this and update \
             the limit in `styles/dropdown_menu.rs` and `component-props.md`"
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
