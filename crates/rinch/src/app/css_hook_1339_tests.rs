//! `HoverCard`'s dropdown reveal, read through the **real** cascade (issue
//! #1339) — the same nested-reveal leak as #778, but via `:hover` rather than
//! a toggled class.
//!
//! # What was wrong
//!
//! `crates/rinch-components/src/styles/hover_card.rs` reveals the panel with a
//! plain descendant rule:
//!
//! ```css
//! .rinch-hover-card:hover .rinch-hover-card__dropdown {
//!     opacity: 1;
//!     visibility: visible;
//! }
//! ```
//!
//! A descendant combinator matches through *any* ancestor carrying `:hover`,
//! and `:hover` itself propagates up the whole DOM ancestor chain from
//! whatever the pointer physically sits over (`RinchDocument::update_hover`
//! marks the hovered node *and every one of its ancestors*). So hovering
//! anything inside an outer `HoverCard`'s dropdown — its own target, or plain
//! content beside a nested card — marks the outer root `:hover`, and the
//! descendant selector then reaches a **closed** `HoverCard` nested inside
//! that dropdown too, even though the inner card's own root is never on the
//! hovered node's ancestor chain.
//!
//! `HoverCard` is nestable (`HoverCardDropdown`/`HoverCardTarget` both take
//! arbitrary `children: &[NodeHandle]`), so this is unremarkable markup, not a
//! constructed worst case.
//!
//! # The fix
//!
//! The same `:not()`-exclusion pattern #778 applies to `Popover`
//! (`css_hook_778_tests.rs`), respelled for a pseudo-class instead of a
//! toggled class: reveal a dropdown under a hovered ancestor UNLESS a
//! *closed* (`:not(:hover)`) `.rinch-hover-card` root sits between that
//! ancestor and the dropdown.
//!
//! ```css
//! .rinch-hover-card:hover .rinch-hover-card__dropdown:not(.rinch-hover-card:hover .rinch-hover-card:not(:hover) .rinch-hover-card__dropdown) { … }
//! ```
//!
//! plus widening the #912 animation-pause selector to the exact complement,
//! as #778 does for Popover's.
//!
//! # The second reveal rule, `.rinch-hover-card__dropdown:hover`
//!
//! This rule (keeps the panel open while the pointer moves from the target to
//! the panel itself) needs **no** matching `:not()` exclusion, and
//! [`a_dropdown_hovered_directly_always_implies_its_own_card_is_hovered`]
//! is why: for *this* selector to match a dropdown `D` at all, `D` itself (or
//! something inside it) must be the genuinely-hovered node, and
//! `update_hover` marks the hovered node's **entire** ancestor chain —
//! including `D`'s own direct parent card — as `:hover`. So whenever this
//! rule matches `D`, `D`'s own card is unconditionally hovered too (zero gap,
//! the nearest possible ancestor), which can never be the "closed card
//! between a hovered ancestor and the dropdown" the widened rule above
//! excludes. The rule cannot leak a closed nested card's panel because
//! matching it already proves that card is open.
//!
//! # Section map
//!
//! 1. The leak itself — hovering outer content (not the inner card) must not
//!    reveal a closed card nested in the outer dropdown.
//! 2. The positive control: hovering the *inner* card's own target (open-in-
//!    open) still reveals it.
//! 3. The `.rinch-hover-card__dropdown:hover` proof above, made concrete.
//! 4. The #912 pause selector stays the exact complement.

use super::*;
use rinch_components::{HoverCard, HoverCardDropdown, HoverCardTarget, Loader};
use rinch_core::Component;
use rinch_dom::computed_style::VisibilityValue;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

// ── harness ──────────────────────────────────────────────────────────────

/// Mount `build` under the real component stylesheet.
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

/// Lay out again at a *different* viewport: `resolve_layout` early-returns on
/// a clean tree, so re-resolving at the same size would measure nothing.
/// (`update_hover` itself sets `styles_dirty`, so this is belt-and-braces
/// rather than load-bearing for the hover fixtures below — kept for parity
/// with `css_hook_778_tests::settle` and because it costs nothing.)
fn settle(app: &mut RinchApp, nth: f32) {
    app.resolve_and_repaint(VIEWPORT.0 + nth, VIEWPORT.1);
}

/// Complete every running transition and drop it. `opacity`/`visibility` carry
/// a transition on both reveal rules, so right after a hover change the
/// cascade has written the *interpolated* value (at t=0, unchanged) back over
/// the resolved one — reading `opacity`/`visibility` right after `settle`
/// would see a frame of the animation, not the end state a correctness
/// fixture wants.
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

/// Simulate the pointer genuinely sitting over `node` — marks `node` and
/// every one of its ancestors `:hover`, clearing whatever was hovered before,
/// exactly as `event_dispatch.rs`'s real hit-test-driven call does.
fn hover(app: &mut RinchApp, node: Option<usize>) {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    let mut changed = false;
    d.update_hover(node, &mut changed);
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

fn has_class(app: &RinchApp, node: usize, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get("class").cloned())
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn visibility_of(app: &RinchApp, node: usize) -> VisibilityValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.visibility
}

fn opacity_of(app: &RinchApp, node: usize) -> f32 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.opacity
}

/// Whether the dropdown is actually revealed: `visibility: visible` AND
/// `opacity` above 0 (the sheet sets both together; checking only one would
/// pass against a mutant that drops the other half of the pair).
fn revealed(app: &RinchApp, node: usize) -> bool {
    visibility_of(app, node) == VisibilityValue::Visible && opacity_of(app, node) > 0.0
}

/// How many animations are registered on a node.
fn animations(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_animations
        .get(&node)
        .map(|v| v.len())
        .unwrap_or(0)
}

fn is_paused(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_animations
        .get(&node)
        .and_then(|v| v.first())
        .map(|a| a.play_state == rinch_dom::animation::AnimationPlayState::Paused)
        .unwrap_or(false)
}

/// Build:
/// ```text
/// outer HoverCard (data-probe="outer")
///   HoverCardTarget (data-probe="outer-target")
///   HoverCardDropdown
///     div (data-probe="outer-content")          <- plain sibling content
///     inner HoverCard (data-probe="inner")
///       HoverCardTarget (data-probe="inner-target")
///       HoverCardDropdown
/// ```
fn nested_app() -> RinchApp {
    mount(move |scope| {
        let inner_target = HoverCardTarget.render(scope, &[]);
        inner_target.set_attribute("data-probe", "inner-target");
        let inner_dropdown = HoverCardDropdown.render(scope, &[]);
        let inner = HoverCard::default().render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = HoverCardTarget.render(scope, &[]);
        outer_target.set_attribute("data-probe", "outer-target");
        let outer_content = scope.create_element("div");
        outer_content.set_attribute("data-probe", "outer-content");
        let outer_dropdown = HoverCardDropdown.render(scope, &[outer_content, inner]);
        let outer = HoverCard::default().render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    })
}

// ── 1. The leak itself ───────────────────────────────────────────────────

/// Hovering plain content inside the outer dropdown — not the inner card at
/// all — marks the outer `.rinch-hover-card` root `:hover` (ancestor
/// propagation) and therefore opens the outer panel; it must NOT also reveal
/// the closed inner card's panel.
///
/// Kills the reveal rule without its `:not(…)` exclusion (plain descendant,
/// the pre-fix shape): the outer root's `:hover` then matches through the
/// outer dropdown and reaches the inner panel too.
#[test]
fn hovering_outer_content_does_not_reveal_a_closed_card_nested_in_its_dropdown() {
    let mut app = nested_app();

    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    let outer_content = probe(&app, "outer-content");
    assert!(
        is_inside(&app, inner, outer),
        "the fixture really nests them"
    );

    let outer_panel = child_with_class(&app, outer, "rinch-hover-card__dropdown");
    let inner_panel = child_with_class(&app, inner, "rinch-hover-card__dropdown");

    assert!(
        !revealed(&app, outer_panel),
        "precondition: nothing is hovered yet"
    );

    hover(&mut app, Some(outer_content));
    settle(&mut app, 1.0);
    finish_transitions(&mut app);

    assert!(
        has_class(&app, outer, "rinch-hover-card")
            && visibility_of(&app, outer) != VisibilityValue::Hidden,
        "sanity: the outer card root exists and is itself rendered"
    );
    assert!(
        revealed(&app, outer_panel),
        "control: hovering content inside the outer dropdown opens the outer panel"
    );
    assert!(
        !revealed(&app, inner_panel),
        "the outer card's `:hover` must not reach the inner card's closed panel \
         through the descendant combinator — visibility {:?}, opacity {}",
        visibility_of(&app, inner_panel),
        opacity_of(&app, inner_panel)
    );
}

// ── 2. Positive control: open-in-open is still shown ────────────────────

/// Hovering the *inner* card's own target marks both the inner root AND the
/// outer root `:hover` (the inner target is itself inside the outer
/// dropdown, so ancestor propagation reaches both) — the inner panel must
/// still open.
///
/// Kills a mutant that drops `.rinch-hover-card:hover` from the exclusion's
/// leading compound — which would hide a panel behind **any** closed card
/// root, hovered ancestor or not, including this one once it stops being the
/// nearest ancestor.
#[test]
fn hovering_the_inner_cards_own_target_still_reveals_it() {
    let mut app = nested_app();

    let inner = probe(&app, "inner");
    let inner_target = probe(&app, "inner-target");
    let inner_panel = child_with_class(&app, inner, "rinch-hover-card__dropdown");

    hover(&mut app, Some(inner_target));
    settle(&mut app, 1.0);
    finish_transitions(&mut app);

    assert!(
        revealed(&app, inner_panel),
        "hovering the inner card's own target still opens its panel"
    );
}

// ── 3. The second reveal rule cannot leak ───────────────────────────────

/// `.rinch-hover-card__dropdown:hover` matching a dropdown proves that
/// dropdown's own direct parent card is hovered too (propagation up from the
/// genuinely-hovered node), so this rule can never reveal a panel whose own
/// card is closed — it needs no exclusion of its own.
///
/// The only way to make `D:hover` true at all is to hover `D` or something
/// inside it, and `update_hover` marks the **entire** ancestor chain, so `D`'s
/// own parent card is hovered as a side effect, every time.
#[test]
fn a_dropdown_hovered_directly_always_implies_its_own_card_is_hovered() {
    let mut app = nested_app();

    let inner = probe(&app, "inner");
    let inner_panel = child_with_class(&app, inner, "rinch-hover-card__dropdown");

    // Hover the inner panel itself directly (as if the pointer moved from the
    // inner target onto the inner panel).
    hover(&mut app, Some(inner_panel));
    settle(&mut app, 1.0);
    finish_transitions(&mut app);

    assert!(
        has_class(&app, inner, "rinch-hover-card"),
        "sanity: inner is really a `.rinch-hover-card` root"
    );
    // The proof: the inner card root is on the hovered node's ancestor chain,
    // so it is unconditionally `:hover`-true whenever the panel itself is.
    let doc = app.doc.as_ref().unwrap();
    let is_hovered = {
        let d = doc.borrow();
        d.tree.get(inner).unwrap().is_hovered
    };
    assert!(
        is_hovered,
        "the inner panel being hovered implies its own card root is hovered too"
    );
    assert!(
        revealed(&app, inner_panel),
        "and the panel is therefore revealed — not via a leak, but because \
         its own card is genuinely open"
    );
}

// ── 4. The #912 pause selector stays the exact complement ───────────────

/// A `Loader` inside the nested closed card's dropdown is paused precisely
/// where the panel is hidden, including when the un-widened pause complement
/// would have left it running (because the un-widened complement's `:not()`
/// argument is exactly the old, leakier reveal rule, and therefore matches
/// the nested panel too — the same leak, read from the other side).
#[test]
fn a_loader_in_a_closed_nested_cards_dropdown_is_paused() {
    let mut app = mount(move |scope| {
        let loader = Loader::default().render(scope, &[]);
        let inner_target = HoverCardTarget.render(scope, &[]);
        inner_target.set_attribute("data-probe", "inner-target");
        let inner_dropdown = HoverCardDropdown.render(scope, &[loader]);
        let inner = HoverCard::default().render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = HoverCardTarget.render(scope, &[]);
        outer_target.set_attribute("data-probe", "outer-target");
        let outer_content = scope.create_element("div");
        outer_content.set_attribute("data-probe", "outer-content");
        let outer_dropdown = HoverCardDropdown.render(scope, &[outer_content, inner]);
        let outer = HoverCard::default().render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });

    let outer_content = probe(&app, "outer-content");
    hover(&mut app, Some(outer_content));
    settle(&mut app, 1.0);
    finish_transitions(&mut app);

    let inner = probe(&app, "inner");
    let inner_panel = child_with_class(&app, inner, "rinch-hover-card__dropdown");
    assert!(
        !revealed(&app, inner_panel),
        "precondition: the nested closed panel stays hidden (section 1)"
    );
    let oval = {
        let found = nodes_with_class(&app, "rinch-loader__oval");
        assert_eq!(found.len(), 1, "expected one `rinch-loader__oval`");
        found[0]
    };
    assert!(
        is_inside(&app, oval, inner_panel),
        "precondition: the oval is inside the nested closed panel"
    );
    assert_eq!(animations(&app, oval), 1, "precondition: registered");
    assert!(
        is_paused(&app, oval),
        "the loader inside a closed card nested in an open one's dropdown \
         must still be paused — it is exactly the panel the reveal rule \
         leaves hidden"
    );
}

/// A `Loader` inside an **open** card's own dropdown must keep running: the
/// pause selector's widened `:not()` must not over-match a panel that is
/// genuinely revealed.
#[test]
fn a_loader_in_an_open_cards_own_dropdown_keeps_running() {
    let mut app = mount(move |scope| {
        let loader = Loader::default().render(scope, &[]);
        let target = HoverCardTarget.render(scope, &[]);
        target.set_attribute("data-probe", "target");
        let dropdown = HoverCardDropdown.render(scope, &[loader]);
        HoverCard::default().render(scope, &[target, dropdown])
    });

    let target = probe(&app, "target");
    hover(&mut app, Some(target));
    settle(&mut app, 1.0);
    finish_transitions(&mut app);

    let panel = nodes_with_class(&app, "rinch-hover-card__dropdown")[0];
    assert!(revealed(&app, panel), "precondition: the panel is open");

    let oval = {
        let found = nodes_with_class(&app, "rinch-loader__oval");
        assert_eq!(found.len(), 1, "expected one `rinch-loader__oval`");
        found[0]
    };
    assert_eq!(animations(&app, oval), 1, "precondition: registered");
    assert!(
        !is_paused(&app, oval),
        "a loader inside an open card's own (revealed) dropdown must not be \
         paused by the widened #912 selector"
    );
}
