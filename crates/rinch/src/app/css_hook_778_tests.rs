//! `Popover`'s dropdown reveal, read through the **real** cascade (issue #778).
//!
//! # What was wrong
//!
//! `crates/rinch-components/src/styles/popover.rs` revealed the panel with a
//! plain descendant rule, `.rinch-popover--opened .rinch-popover__dropdown`. A
//! descendant combinator matches through *any* ancestor carrying the class, so
//! a closed `Popover` placed inside an open one's dropdown was revealed with
//! it — measured on desktop (#778's own issue body): the nested closed panel
//! read back `visibility: Visible`, `opacity: 1`, and took pointer input,
//! although its own root carried no `--opened`.
//!
//! Found while reviewing #774 (#760); #774 touched neither `popover.rs` file,
//! so the rule had been descendant since #474 and #774 did not introduce it.
//!
//! # The fix, and why it is not `>`
//!
//! The naive fix is a child combinator, the way `DropdownMenu`'s **backdrop**
//! is spelled (`.rinch-dropdown-menu--opened > .rinch-dropdown-menu__backdrop`)
//! — the dropdown is always a direct child of the root by construction
//! (`Popover::render` appends every child, the panel included, to the root
//! itself). But the round-2 review of #774 measured that `>` breaks exactly
//! this shape for `DropdownMenu`'s **panel**: `rsx!` is not always handing the
//! caller's child over as a direct child. It inserts a `display: contents`
//! wrapper of its own for a `{Option<NodeHandle>}` child, for every branch of
//! an `if` after the first, for a reactive-prop component inside an `if`, and
//! for a helper component whose body is control flow — and a caller may wrap
//! the panel in a plain `div` too. So this fix copies `DropdownMenu`'s *panel*
//! pattern instead: a descendant rule with an exclusion that opens behind any
//! wrapper but stops at a closed `.rinch-popover` root sitting between an open
//! ancestor and the dropdown.
//!
//! # Section map
//!
//! 1. The leak itself, direct child shape — the exact table from #778's issue
//!    body, read back through [`mount`].
//! 2. The positive control: an *open* popover nested in an *open* one's
//!    dropdown is still shown (the fix is an exclusion, not a blanket
//!    descendant cut).
//! 3. Wrapper shapes, mirroring `css_hook_760_tests`'s wrapper suite: the
//!    outer popover's own dropdown opens from behind an `rsx!`-inserted
//!    `display: contents` wrapper, and a closed popover nested behind one
//!    still stays hidden.
//! 4. The #912 animation-pause selector stays the exact complement of the
//!    widened reveal rule: a `Loader` inside the nested closed popover's
//!    dropdown is paused precisely where the panel is hidden, including when
//!    the leak would have shown it.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_macros::rsx;

use rinch_components::{Loader, Popover, PopoverDropdown, PopoverTarget};
use rinch_core::element::IntoEventHandler;
use rinch_core::{Callback, Component, Signal};
use rinch_dom::computed_style::VisibilityValue;
use std::rc::Rc;

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
fn settle(app: &mut RinchApp, nth: f32) {
    app.resolve_and_repaint(VIEWPORT.0 + nth, VIEWPORT.1);
}

/// Complete every running transition and drop it. `opacity`/`transform` carry
/// a 150ms transition on open, so right after `opened.set(true)` the cascade
/// has written the *interpolated* value (at t=0, unchanged) back over the
/// resolved one — reading `opacity` right after `settle` would see a frame of
/// the animation, not the end state a correctness fixture wants.
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

/// The one attached node carrying `class` — attached, because a wrapper
/// fixture's probe is sometimes the only live instance while a stale one from
/// an earlier reconcile would otherwise collide.
fn live_node_with_class(app: &RinchApp, class: &str) -> usize {
    nodes_with_class(app, class)
        .into_iter()
        .find(|n| is_attached(app, *n))
        .unwrap_or_else(|| panic!("expected an attached `{class}`"))
}

fn is_attached(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut cur = Some(node);
    while let Some(c) = cur {
        if Some(c) == Some(d.tree.root_id) {
            return true;
        }
        cur = d.tree.get(c).and_then(|n| n.parent);
    }
    false
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

// ── 1. The leak itself (#778's own measured table) ──────────────────────

/// A closed `Popover` nested directly inside an open one's dropdown stays
/// hidden — panel `visibility`/`opacity` alike.
///
/// Kills the reveal rule without its `:not(…)` exclusion (plain descendant,
/// the pre-fix shape): the outer root's `--opened` then matches through the
/// outer dropdown and reaches the inner panel too.
#[test]
fn a_closed_popover_nested_in_an_open_ones_dropdown_stays_hidden() {
    let mut app = mount(move |scope| {
        let inner_target = PopoverTarget.render(scope, &[]);
        let inner_dropdown = PopoverDropdown.render(scope, &[]);
        let inner = Popover {
            opened_fn: Some(Rc::new(|| false)),
            onclose: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = PopoverTarget.render(scope, &[]);
        let outer_dropdown = PopoverDropdown.render(scope, &[inner]);
        let outer = Popover {
            opened_fn: Some(Rc::new(|| true)),
            onclose: Some(Callback::new(|| {})),
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

    let outer_panel = child_with_class(&app, outer, "rinch-popover__dropdown");
    let inner_panel = child_with_class(&app, inner, "rinch-popover__dropdown");

    assert!(
        revealed(&app, outer_panel),
        "control: the outer popover is open"
    );
    assert!(
        !has_class_778(&app, inner, "rinch-popover--opened"),
        "precondition: the inner popover's own root is closed"
    );
    assert!(
        !revealed(&app, inner_panel),
        "the outer popover's `--opened` must not reach the inner popover's panel \
         through the descendant combinator — visibility {:?}, opacity {}",
        visibility_of(&app, inner_panel),
        opacity_of(&app, inner_panel)
    );
}

fn has_class_778(app: &RinchApp, node: usize, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get("class").cloned())
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

// ── 2. Positive control: open-in-open is still shown ────────────────────

/// An *open* `Popover` nested inside an *open* one's dropdown is shown.
///
/// Kills a mutant that drops `.rinch-popover--opened` from the exclusion's
/// leading compound — which would hide a panel behind **any** closed popover
/// root, open ancestor or not, including this one's own closed-then-opened
/// state (`:not(.rinch-popover:not(.rinch-popover--opened)
/// .rinch-popover__dropdown)` with no leading `.rinch-popover--opened` on the
/// `:not()`'s own ancestor half hides every nested panel whatever its
/// ancestor chain, which this fixture would catch where section 1 alone does
/// not).
#[test]
fn an_open_popover_nested_in_an_open_ones_dropdown_is_shown() {
    let mut app = mount(move |scope| {
        let inner_target = PopoverTarget.render(scope, &[]);
        let inner_dropdown = PopoverDropdown.render(scope, &[]);
        let inner = Popover {
            opened_fn: Some(Rc::new(|| true)),
            onclose: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = PopoverTarget.render(scope, &[]);
        let outer_dropdown = PopoverDropdown.render(scope, &[inner]);
        let outer = Popover {
            opened_fn: Some(Rc::new(|| true)),
            onclose: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });
    settle(&mut app, 1.0);

    let outer = probe(&app, "outer");
    let inner = probe(&app, "inner");
    let inner_panel = child_with_class(&app, inner, "rinch-popover__dropdown");
    assert!(is_inside(&app, inner, outer));
    assert!(
        revealed(&app, inner_panel),
        "an open popover nested in an open one's dropdown is still shown"
    );
}

// ── 3. Wrapper shapes ────────────────────────────────────────────────────

/// `rsx!` inserts a `display: contents` wrapper around an `{Option<NodeHandle>}`
/// child — exactly the shape the review of #774 found breaks a plain `>` fix.
/// The outer popover's own dropdown must still open from behind it.
///
/// Kills the reveal rule respelled `--opened > __dropdown`.
#[test]
fn a_popovers_own_dropdown_behind_an_option_wrapper_still_opens() {
    let opened = Signal::new(false);
    let mut app = mount(move |__scope: &mut RenderScope| {
        let text = __scope.create_text("content");
        let dropdown: Option<NodeHandle> = Some(PopoverDropdown.render(__scope, &[text]));
        rsx! {
            Popover { opened_fn: move || opened.get(), onclose: || {},
                PopoverTarget { button { "t" } }
                {dropdown}
            }
        }
    });

    let root = live_node_with_class(&app, "rinch-popover");
    let panel = live_node_with_class(&app, "rinch-popover__dropdown");
    assert!(is_inside(&app, panel, root));
    assert_ne!(
        parent_of(&app, panel),
        Some(root),
        "precondition: the `Option<NodeHandle>` wrapper sits between the root \
         and the panel"
    );

    assert!(!revealed(&app, panel), "closed: hidden");
    opened.set(true);
    settle(&mut app, 1.0);
    finish_transitions(&mut app);
    assert!(
        revealed(&app, panel),
        "open: `.rinch-popover--opened` reaches a panel that is not a direct \
         child of the root"
    );
}

/// A closed `Popover` nested behind the same kind of wrapper, inside an open
/// one's dropdown, still stays hidden — the exclusion is not defeated by a
/// `display: contents` wrapper sitting between the open ancestor and the
/// closed inner root either.
#[test]
fn a_closed_popover_behind_a_wrapper_nested_in_an_open_ones_dropdown_stays_hidden() {
    let mut app = mount(move |__scope: &mut RenderScope| {
        let inner: Option<NodeHandle> = Some({
            let inner_target = PopoverTarget.render(__scope, &[]);
            let inner_dropdown = PopoverDropdown.render(__scope, &[]);
            let inner = Popover {
                opened_fn: Some(Rc::new(|| false)),
                onclose: Some(Callback::new(|| {})),
                ..Default::default()
            }
            .render(__scope, &[inner_target, inner_dropdown]);
            inner.set_attribute("data-probe", "inner");
            inner
        });
        rsx! {
            Popover { opened_fn: move || true, onclose: || {},
                PopoverTarget { button { "t" } }
                PopoverDropdown { {inner} }
            }
        }
    });
    settle(&mut app, 1.0);

    let inner = probe(&app, "inner");
    let inner_panel = child_with_class(&app, inner, "rinch-popover__dropdown");
    assert_ne!(
        parent_of(&app, inner),
        {
            let outer_dropdown = live_node_with_class(&app, "rinch-popover__dropdown");
            Some(outer_dropdown)
        },
        "precondition: the wrapper sits between the outer dropdown and the \
         inner popover's root"
    );
    assert!(
        !revealed(&app, inner_panel),
        "the wrapper does not let the outer `--opened` through to the inner panel"
    );
}

// ── 4. The #912 pause selector stays the exact complement ───────────────

/// A `Loader` inside the nested closed popover's dropdown is paused precisely
/// where the panel is hidden — including the nested case the old pause
/// selector's complement never had to describe, because the old reveal rule
/// (and therefore its complement) could not distinguish "closed root nested
/// in an open one" from "closed root, full stop".
///
/// Kills either half drifting out of sync: the pause selector left as the old,
/// narrower complement (which would leave the nested-but-closed loader
/// *unpaused*, since the old complement's `:not()` argument is exactly the old,
/// leakier reveal rule and therefore matches the nested panel — the same
/// leak, from the other side), or the reveal rule's exclusion changed without
/// updating the pause rule's.
#[test]
fn a_loader_in_a_closed_nested_popovers_dropdown_is_paused() {
    let mut app = mount(move |scope| {
        let loader = Loader::default().render(scope, &[]);
        let inner_target = PopoverTarget.render(scope, &[]);
        let inner_dropdown = PopoverDropdown.render(scope, &[loader]);
        let inner = Popover {
            opened_fn: Some(Rc::new(|| false)),
            onclose: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = PopoverTarget.render(scope, &[]);
        let outer_dropdown = PopoverDropdown.render(scope, &[inner]);
        let outer = Popover {
            opened_fn: Some(Rc::new(|| true)),
            onclose: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });
    settle(&mut app, 1.0);

    let inner = probe(&app, "inner");
    let inner_panel = child_with_class(&app, inner, "rinch-popover__dropdown");
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
        "the loader inside a closed popover nested in an open one's dropdown \
         must still be paused — it is exactly the panel the reveal rule \
         leaves hidden"
    );
}
