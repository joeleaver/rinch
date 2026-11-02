//! An open overlay inside a hidden one stays hidden (issue #936).
//!
//! `Popover`, `HoverCard` and `LoadingOverlay` showed their panel with
//! `visibility: visible`. `visibility` inherits, and a declared `visible`
//! overrides an inherited `hidden`, so an open `Popover` inside a closed
//! `Drawer` (or inside a closed outer `Popover`'s dropdown) computed
//! `visibility: visible` and desktop painted it while its container was
//! closed — measured: the dropdown read back `Visible` under the closed drawer.
//! A browser does the same with that CSS; the stylesheet was wrong, not the
//! cascade.
//!
//! The shown state now declares `visibility: inherit`: under a visible
//! container it is visible, as before, and under a hidden one it is hidden.
//! (The issue's second half, a closed nested popover revealed by an open
//! outer one, was #778 and is pinned by `css_hook_778_tests`.)
//!
//! Every fixture also checks the container open — the positive control that
//! the shown state still shows — and section 4 that `inherit` is read as
//! *inherited* by the #759 hand-down, so a drawer closing over an open popover
//! keeps the panel on screen for its slide and hides it at the end.

use super::*;

use rinch_components::{
    Drawer, HoverCard, HoverCardDropdown, HoverCardTarget, LoadingOverlay, Popover, PopoverDropdown,
};
use rinch_core::{Component, Signal};
use rinch_dom::computed_style::VisibilityValue;
use std::rc::Rc;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

// ── harness (the css_hook_778_tests one) ─────────────────────────────────

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

fn settle(app: &mut RinchApp, nth: f32) {
    app.resolve_and_repaint(VIEWPORT.0 + nth, VIEWPORT.1);
}

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

fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<u64> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut found: Vec<u64> = d
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

fn one_with_class(app: &RinchApp, class: &str) -> u64 {
    let found = nodes_with_class(app, class);
    assert_eq!(found.len(), 1, "expected one `{class}`");
    found[0]
}

/// The one node carrying `data-probe="{marker}"`.
fn probe(app: &RinchApp, marker: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.attributes.get("data-probe").is_some_and(|v| v == marker))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "expected one `[data-probe={marker}]`");
    found[0]
}

fn visibility_of(app: &RinchApp, node: u64) -> VisibilityValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.visibility
}

fn popover(scope: &mut RenderScope, opened: Signal<bool>, children: &[NodeHandle]) -> NodeHandle {
    let dd = PopoverDropdown.render(scope, children);
    Popover {
        opened_fn: Some(Rc::new(move || opened.get())),
        ..Default::default()
    }
    .render(scope, &[dd])
}

fn drawer(scope: &mut RenderScope, opened: Signal<bool>, children: &[NodeHandle]) -> NodeHandle {
    Drawer {
        opened_fn: Some(Rc::new(move || opened.get())),
        position: "left".to_string(),
        ..Default::default()
    }
    .render(scope, children)
}

// ── 1. Popover inside a closed Drawer ────────────────────────────────────

/// An open `Popover` inside a closed `Drawer` is hidden, and shown once the
/// drawer opens.
///
/// Kills the shown rule's `visibility: inherit` put back to `visible` (the
/// pre-fix sheet): the dropdown then reads `Visible` under the closed drawer.
#[test]
fn an_open_popover_in_a_closed_drawer_is_hidden() {
    let drawer_open = Signal::new(false);
    let pop_open = Signal::new(true);
    let mut app = mount(move |scope| {
        let content = scope.create_element("span");
        content.set_attribute("data-probe", "content");
        let pop = popover(scope, pop_open, &[content]);
        drawer(scope, drawer_open, &[pop])
    });
    finish_transitions(&mut app);
    let dd = one_with_class(&app, "rinch-popover__dropdown");
    let content = probe(&app, "content");
    assert_eq!(
        visibility_of(&app, dd),
        VisibilityValue::Hidden,
        "an open popover's dropdown must not show through a closed drawer"
    );
    assert_eq!(visibility_of(&app, content), VisibilityValue::Hidden);

    // Positive control: the drawer opens, the popover's panel shows.
    drawer_open.set(true);
    settle(&mut app, 1.0);
    finish_transitions(&mut app);
    assert_eq!(visibility_of(&app, dd), VisibilityValue::Visible);
    assert_eq!(visibility_of(&app, content), VisibilityValue::Visible);

    // …and closing the popover alone still hides it (the closed rule wins
    // over the inherited `visible`).
    pop_open.set(false);
    settle(&mut app, 2.0);
    finish_transitions(&mut app);
    assert_eq!(visibility_of(&app, dd), VisibilityValue::Hidden);
}

// ── 2. Popover inside a closed Popover ───────────────────────────────────

/// An open `Popover` nested in a **closed** popover's dropdown is hidden with
/// it, and shown when the outer one opens.
///
/// Kills the same mutant as section 1, through the outer dropdown's own
/// `visibility: hidden` rather than the drawer's `!important` one.
#[test]
fn an_open_popover_in_a_closed_popovers_dropdown_is_hidden() {
    let outer_open = Signal::new(false);
    let inner_open = Signal::new(true);
    let mut app = mount(move |scope| {
        let content = scope.create_element("span");
        content.set_attribute("data-probe", "inner-content");
        let inner = popover(scope, inner_open, &[content]);
        inner.set_attribute("data-probe", "inner-root");
        popover(scope, outer_open, &[inner])
    });
    finish_transitions(&mut app);
    let inner_root = probe(&app, "inner-root");
    let inner_dd = nodes_with_class(&app, "rinch-popover__dropdown")
        .into_iter()
        .find(|&n| {
            let doc = app.doc.as_ref().unwrap();
            let d = doc.borrow();
            d.tree.get(n).and_then(|x| x.parent) == Some(inner_root)
        })
        .expect("the inner dropdown");
    let content = probe(&app, "inner-content");
    assert_eq!(visibility_of(&app, inner_dd), VisibilityValue::Hidden);
    assert_eq!(visibility_of(&app, content), VisibilityValue::Hidden);

    outer_open.set(true);
    settle(&mut app, 1.0);
    finish_transitions(&mut app);
    assert_eq!(visibility_of(&app, inner_dd), VisibilityValue::Visible);
    assert_eq!(visibility_of(&app, content), VisibilityValue::Visible);
}

// ── 3. LoadingOverlay and HoverCard, the same rule ───────────────────────

/// A visible `LoadingOverlay` inside a closed `Drawer` is hidden.
///
/// Kills `.rinch-loading-overlay--visible { visibility: visible }` (pre-fix).
#[test]
fn a_visible_loading_overlay_in_a_closed_drawer_is_hidden() {
    let drawer_open = Signal::new(false);
    let mut app = mount(move |scope| {
        let host = scope.create_element("div");
        host.set_attribute("style", "position: relative; width: 100px; height: 100px");
        let overlay = LoadingOverlay {
            visible: true,
            ..Default::default()
        }
        .render(scope, &[]);
        host.append_child(&overlay);
        drawer(scope, drawer_open, &[host])
    });
    finish_transitions(&mut app);
    let overlay = one_with_class(&app, "rinch-loading-overlay");
    assert_eq!(visibility_of(&app, overlay), VisibilityValue::Hidden);

    drawer_open.set(true);
    settle(&mut app, 1.0);
    finish_transitions(&mut app);
    assert_eq!(visibility_of(&app, overlay), VisibilityValue::Visible);
}

/// A hovered `HoverCard` inside a `visibility: hidden` container keeps its
/// dropdown hidden; in a visible one hovering shows it.
///
/// Kills the first reveal rule in `styles/hover_card.rs`
/// (`.rinch-hover-card:hover .rinch-hover-card__dropdown:not(…)`) put back to
/// `visibility: visible`. The second (`.rinch-hover-card__dropdown:hover`) is
/// changed too but no fixture can see it: it matches only when the first does
/// (a hovered dropdown's card is hovered), and the first has the higher
/// specificity — measured, putting it back alone survives.
#[test]
fn a_hovered_card_in_a_hidden_container_stays_hidden() {
    for hidden in [true, false] {
        let mut app = mount(move |scope| {
            let host = scope.create_element("div");
            if hidden {
                host.set_attribute("style", "visibility: hidden");
            }
            let label = scope.create_text("t");
            let t = HoverCardTarget.render(scope, &[label]);
            let content = scope.create_element("span");
            content.set_attribute("data-probe", "card-content");
            let dd = HoverCardDropdown.render(scope, &[content]);
            let card = HoverCard::default().render(scope, &[t, dd]);
            host.append_child(&card);
            host
        });
        let dd = one_with_class(&app, "rinch-hover-card__dropdown");
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            let mut changed = false;
            // Hover the dropdown itself: every ancestor (the card included)
            // is `:hover`, so both reveal rules match it.
            d.update_hover(Some(dd), &mut changed);
        }
        settle(&mut app, 1.0);
        finish_transitions(&mut app);
        let expected = if hidden {
            VisibilityValue::Hidden
        } else {
            VisibilityValue::Visible
        };
        assert_eq!(
            visibility_of(&app, dd),
            expected,
            "hidden container: {hidden}"
        );
        assert_eq!(
            visibility_of(&app, probe(&app, "card-content")),
            expected,
            "hidden container: {hidden}"
        );
    }
}

// ── 4. A drawer closing over an open popover ─────────────────────────────

/// `visibility: inherit` is *inherited* to the #759 hand-down: while a drawer
/// slides closed its root is held `visible`, and the open popover's panel
/// inside it stays on screen with it, then hides at the end.
///
/// Kills a hand-down that skips the panel (measured: `visibility_is_inherited`
/// with its shared-struct fast path off **and** `inherit` read as the node's
/// own value — the mid-close assertion fails). Either half alone survives: the
/// panel's `InheritedBox` is its parent's (an `inherit` of an inherited
/// property copies nothing), so the fast path answers before the rule walk.
#[test]
fn an_open_popover_stays_shown_while_its_drawer_slides_closed() {
    let drawer_open = Signal::new(true);
    let pop_open = Signal::new(true);
    let mut app = mount(move |scope| {
        let pop = popover(scope, pop_open, &[]);
        drawer(scope, drawer_open, &[pop])
    });
    finish_transitions(&mut app);
    let dd = one_with_class(&app, "rinch-popover__dropdown");
    assert_eq!(visibility_of(&app, dd), VisibilityValue::Visible);

    drawer_open.set(false);
    settle(&mut app, 1.0);
    assert_eq!(
        visibility_of(&app, dd),
        VisibilityValue::Visible,
        "held visible for the drawer's 300ms close"
    );
    finish_transitions(&mut app);
    assert_eq!(visibility_of(&app, dd), VisibilityValue::Hidden);
}
