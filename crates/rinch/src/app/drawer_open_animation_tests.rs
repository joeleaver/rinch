//! The `Drawer`'s 300ms slide-in **runs**, and the closed drawer is still out of
//! the way (issue #751).
//!
//! # What was wrong
//!
//! `styles/drawer.rs` used to put `display: none !important` on the drawer's
//! **root** while `transition: transform 300ms ease` sat on the **panel**
//! underneath it, and `drawer.rs`' one reactive effect removes the root's hidden
//! class and adds the panel's `--opened` class in a single batch. So the panel's
//! ancestor stopped being `display: none` and the panel's own `transform`
//! retargeted in **one** style pass.
//!
//! css-transitions-1 §3 starts no transition there: the before-change style of
//! an element that was not being rendered is its after-change style, so there is
//! nothing to animate from. rinch used to start one anyway; #703 stopped it,
//! which is what made the missing animation visible on desktop. A browser
//! refuses the same shape — that is the whole reason `@starting-style` and
//! `transition-behavior: allow-discrete` exist — and `rinch-components` ships
//! **one** stylesheet to both backends, so the Drawer had never animated on
//! `rinch-web` at all.
//!
//! # The cure, and why `visibility`
//!
//! The root's hidden state is now `visibility: hidden !important`. A
//! `visibility: hidden` element **is** being rendered — it keeps its box and it
//! transitions (`display_none_transition_tests::visibility_hidden_is_rendered_and_still_transitions`
//! is rinch-dom's pin on that) — while still being excluded from paint, from hit
//! testing and from the Tab order. `Popover` arrives at the same place from the
//! other side: it animates `opacity`/`visibility` and has never touched
//! `display` on the panel it animates.
//!
//! # What this file pins
//!
//! 1. The slide **runs**, and the transform really interpolates over
//!    `tick_transitions` rather than merely appearing in `active_transitions`.
//! 2. The four things `display: none` used to buy, which `visibility: hidden`
//!    now has to buy instead: not painted, not hit-testable, not focusable, and
//!    no live focus trap.
//! 3. That closing slides the panel **out** (#413, #759) and then puts all four
//!    back — reached by the effect's `else` branch rather than by the initial
//!    render, which is a different path through a class attribute that has
//!    been rewritten twice by then — and that reopening mid-slide cancels the
//!    pending hide.
//!
//! The un-hide pass is audited for every other overlay in
//! `overlay_animation_audit_tests`, which shares the "a transitioned property
//! that changes on the un-hide pass must animate" rule with this file and is
//! what catches a *future* overlay growing the same shape.

use super::*;

use rinch_components::Drawer;
use rinch_core::{Component, Signal};
use rinch_dom::transition::TransitionProperty;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The panel that carries `transition: transform 300ms ease`.
const PANEL: &str = "rinch-drawer";
/// The root that carries the hidden state while closed.
const ROOT: &str = "rinch-drawer__root";
/// The drawer's close button — the focusable thing inside a closed drawer.
const CLOSE: &str = "rinch-drawer__close";

/// The one node carrying `class` exactly, as a raw node id.
fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
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
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    matches[0]
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

fn display_of(app: &RinchApp, node: usize) -> rinch_dom::computed_style::DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}

fn visibility_of(app: &RinchApp, node: usize) -> rinch_dom::computed_style::VisibilityValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.visibility
}

/// The transitioned property, as the three arrays that carry it.
///
/// `TransformValue` has no `PartialEq`, and its `is_identity` flag describes the
/// **matrix** only — a `translateX(-100%)` leaves the matrix identity and lives
/// entirely in `pct_translate_w`, which is exactly the Drawer's closed state. So
/// the comparison has to be on the fields, and `pct_translate_w` is the one that
/// moves here.
fn transform_of(app: &RinchApp, node: usize) -> ([f64; 6], [f64; 2], [f64; 2]) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let t = &d.tree.get(node).unwrap().computed_style.transform;
    (t.matrix, t.pct_translate_w, t.pct_translate_h)
}

/// How many `transition` declarations the cascade found on the node. Zero would
/// mean the fixture is measuring a component that declares no animation at all,
/// which is the fixed point every assertion here has to be kept off.
fn transition_specs(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().transition_specs.len()
}

/// A closed `Drawer` under the real component stylesheet, with the signal that
/// opens it.
fn mount_closed() -> (RinchApp, Signal<bool>) {
    let opened = Signal::new(false);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&drawer);
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
    (app, opened)
}

/// Opening the drawer starts the panel's 300ms slide, and the slide really
/// moves.
///
/// Two halves, because either alone is weak. `active_transitions` says a
/// transition was *started*; ticking it to its midpoint says the value it
/// carries is interpolable and lands strictly between the endpoints — the
/// fixed-point trap here is that `-100%` lives in `pct_translate_w` and leaves
/// the 3x2 matrix at the identity, so a build that interpolated only the matrix
/// would produce a "running" transition that never moves the panel a pixel.
#[test]
fn opening_the_drawer_runs_its_300ms_slide() {
    let (mut app, opened) = mount_closed();

    let panel = node_with_class(&app, PANEL);
    let root = node_with_class(&app, ROOT);

    // Preconditions. Each keeps this fixture off a fixed point where the
    // assertions below would hold for an uninteresting reason.
    assert_ne!(
        display_of(&app, root),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition (#751): the closed drawer's root is RENDERED — `display: \
         none` is what refused the transition, and the cure is that the closed \
         state no longer uses it"
    );
    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Hidden,
        "precondition: it is hidden the other way — `visibility: hidden`"
    );
    assert!(
        transition_specs(&app, panel) > 0,
        "precondition: the panel really does declare a `transition` — without \
         this, a running transition would say nothing"
    );
    let closed_transform = transform_of(&app, panel);
    assert_eq!(
        closed_transform.1,
        [-1.0, 0.0],
        "precondition: closed, the panel is `translateX(-100%)` — off-screen to \
         the left — so opening it really is a change to the transitioned property"
    );
    assert_eq!(running(&app, panel), 0, "precondition: nothing running yet");

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);

    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "the root is visible now"
    );
    assert_eq!(
        running(&app, panel),
        1,
        "the 300ms slide-in runs. It did not before #751: the root's \
         `display: none` made the panel's before-change style its after-change \
         style, so css-transitions-1 §3 started nothing and the panel arrived \
         at its open position instantly"
    );

    // At t = 0 the cascade has written the *interpolated* value back over the
    // resolved one, so the panel still reads as closed. That is the signature
    // of a running transition, and its absence is the signature of a refused
    // one: measured against the `display: none` spelling, this reads `[0.0,
    // 0.0]` — the open position, arrived at in one frame.
    assert_eq!(
        transform_of(&app, panel).1,
        closed_transform.1,
        "at t = 0 the panel is still at its closed position, because the slide \
         has not advanced yet"
    );

    // Advance to the midpoint of the declared 300ms and read the value the
    // engine wrote. `ease` at input 0.5 outputs 0.8024, so the panel is past
    // half way — what matters is only that it is strictly between the
    // endpoints, which is what neither a refused transition nor a
    // matrix-only interpolation can produce.
    let start_ms = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.active_transitions[&panel][&TransitionProperty::Transform].start_time_ms
    };
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, start_ms + 150.0);
    }
    let midway = transform_of(&app, panel).1[0];
    assert!(
        midway > -1.0 && midway < 0.0,
        "half way through the slide the panel is between its closed (-1.0) and \
         open (0.0) positions, not at either: got {midway}"
    );
}

/// The closed drawer takes no clicks.
///
/// Its root is `position: fixed` over the whole viewport with an overlay at 75%
/// black inside it, so this is the assertion that `visibility: hidden` really
/// replaces what `display: none` was doing. `hit_test` skips a hidden subtree
/// (`hit_testing.rs`); the point of the fixture is that the Drawer's own closed
/// state reaches that skip.
#[test]
fn the_closed_drawer_takes_no_clicks() {
    let (mut app, opened) = mount_closed();

    let root = node_with_class(&app, ROOT);
    let panel = node_with_class(&app, PANEL);

    // Off the fixed point: the closed root is a real box covering the click
    // point, so "the hit misses it" cannot be true merely because there is
    // nothing there. Under `display: none` it was 0x0 and this would pass for
    // the wrong reason.
    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let l = &d.tree.get(root).unwrap().layout;
        assert!(
            l.width >= VIEWPORT.0 && l.height >= VIEWPORT.1,
            "precondition: the closed root still covers the viewport ({} x {})",
            l.width,
            l.height
        );
    }

    let hit_closed = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        hit_test(&d.tree, 40.0, 300.0)
    };
    assert!(
        hit_closed != Some(root) && hit_closed != Some(panel),
        "a click at (40, 300) must not land on the closed drawer, got {hit_closed:?}"
    );

    // The positive control: opened, the same point *does* land inside it.
    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    let hit_open = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        hit_test(&d.tree, 40.0, 300.0)
    };
    let inside = hit_open.is_some_and(|n| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let mut cur = Some(n);
        while let Some(id) = cur {
            if id == root {
                return true;
            }
            cur = d.tree.get(id).and_then(|x| x.parent);
        }
        false
    });
    assert!(
        inside,
        "positive control: the *open* drawer does take the click, got {hit_open:?}"
    );
}

/// Tab skips the closed drawer.
///
/// `collect_focusable_nodes` rejects a node that is zero-size **or**
/// `visibility: hidden`/`collapse`. `display: none` satisfied the first clause;
/// the new closed state has to satisfy the second, and the drawer's close
/// `<button>` is the focusable thing that would otherwise leak into the tab
/// order of every page that mounts a closed drawer.
#[test]
fn the_closed_drawer_is_not_in_the_tab_order() {
    let (mut app, opened) = mount_closed();
    let close = node_with_class(&app, CLOSE);

    assert!(
        !app.collect_focusable_nodes().contains(&close),
        "the closed drawer's close button is not focusable"
    );

    // Positive control: it is focusable once the drawer opens, so the assertion
    // above is about the closed state and not about the button being
    // unfocusable in principle.
    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    assert!(
        app.collect_focusable_nodes().contains(&close),
        "positive control: the open drawer's close button IS focusable"
    );
}

/// A closed drawer holds no focus trap.
///
/// `data-trap-focus` is a boolean attribute written by presence, so the closed
/// state removes it (`overlay_focus::arm_trap_focus`). That is unchanged by
/// #751 — this pins it alongside the rest of the closed-state contract, because
/// `visibility: hidden` is the one thing here that a trap search could have been
/// fooled by if the attribute had been left behind.
#[test]
fn a_closed_drawer_traps_no_focus() {
    let (mut app, opened) = mount_closed();
    let root = node_with_class(&app, ROOT);

    let traps = |app: &RinchApp| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(root)
            .unwrap()
            .attributes
            .contains_key("data-trap-focus")
    };

    assert!(!traps(&app), "closed: no `data-trap-focus`");

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    assert!(
        traps(&app),
        "positive control: the open drawer does declare a focus trap"
    );
}

/// One pixel out of a full software frame, through the shell's own rasteriser —
/// `transparent: true` so an unpainted pixel is a zero alpha rather than a
/// window background.
#[cfg(software_shell)]
fn pixel(app: &mut RinchApp, x: u32, y: u32) -> [u8; 4] {
    app.scene_dirty = true;
    let (px, w, _h) = app.build_pixels(1.0, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), true);
    let idx = ((y * w + x) * 4) as usize;
    [px[idx], px[idx + 1], px[idx + 2], px[idx + 3]]
}

/// The closed drawer paints nothing.
///
/// A local pixel oracle, which is the only kind that can see this: the drawer's
/// overlay is `rgba(0, 0, 0, 0.75)` over the entire viewport, so a closed drawer
/// that painted would darken **every** pixel. The `visibility: hidden` root has
/// a real box now, so "no ink" is a claim about paint's visibility skip rather
/// than about an empty layout.
///
/// It samples one pixel away from the panel's 380px, so it cannot see the
/// panel's own text — which a closed drawer drew until #829, whenever the
/// panel was on screen. `visibility_hidden_overlay_paint_tests` pins that half.
#[cfg(software_shell)]
#[test]
fn the_closed_drawer_paints_nothing() {
    let (mut app, opened) = mount_closed();

    // Sampled away from the panel's own 380px width so the only thing that can
    // ink this pixel is the overlay.
    let closed_px = pixel(&mut app, 600, 300);

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    let open_px = pixel(&mut app, 600, 300);

    assert_ne!(
        closed_px, open_px,
        "positive control: the open drawer's overlay inks this pixel, so the \
         comparison discriminates"
    );
    assert!(
        closed_px[3] == 0,
        "the closed drawer paints nothing at (600, 300): got {closed_px:?}, \
         while the open one paints {open_px:?}"
    );
}

/// Closing slides the panel out, and **then** puts everything back: hidden,
/// unpainted, unclickable, out of the Tab order (#413, #759).
///
/// **Not a restatement of the four fixtures above.** Those measure the state the
/// component *renders into* — `Drawer::render` bakes
/// `rinch-drawer__root--hidden` straight into the root's class string when
/// `opened_fn()` is false at mount. This measures the state the component
/// *returns to*, which is reached by a different path: the effect's `else`
/// branch, `root.add_class(HIDDEN)` + `drawer.remove_class(OPENED)`, on a node
/// whose class attribute has been rewritten twice since. An effect that removed
/// the panel's `--opened` class but forgot to re-add the root's hidden one
/// would leave a fully opaque 800x600 backdrop over the app with every one of
/// the other fixtures still green.
///
/// Measured, not argued: delete `root_clone.add_class(ROOT_HIDDEN_CLASS)` from
/// that `else` branch in `rinch-components/src/drawer.rs` and this is the
/// **only** test in the file that fails.
///
/// # The slide-out
///
/// Until #759 the root went `visibility: hidden` on the close pass itself, and
/// the panel's 300ms `transform` transition ran behind it with nothing painted
/// — about eighteen frames interpolated and thrown away per close, and a close
/// that snapped where the open slid. The hidden state now carries `transition:
/// visibility 0s linear 300ms`, so for the panel's 300ms the root — and the
/// panel under it, which inherits the held value — stays visible and the slide
/// is on screen. The four hidden-state assertions are the same four; they hold
/// once the 300ms are up rather than on the close pass.
#[test]
fn closing_the_drawer_slides_out_and_then_puts_it_back_out_of_the_way() {
    let (mut app, opened) = mount_closed();

    let root = node_with_class(&app, ROOT);
    let panel = node_with_class(&app, PANEL);
    let close = node_with_class(&app, CLOSE);

    // Open, and check it really did open — otherwise "closed again" is the
    // fixed point where the drawer simply never moved and every assertion below
    // holds for nothing.
    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "precondition: it opened"
    );
    assert!(
        app.collect_focusable_nodes().contains(&close),
        "precondition: its close button is reachable while open"
    );
    let hit_open = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        hit_test(&d.tree, 40.0, 300.0)
    };
    assert!(
        hit_open.is_some(),
        "precondition: the open drawer takes clicks"
    );

    // Let the slide finish, so the close is a retarget of a settled box rather
    // than a reversal — the ordinary case.
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        let start = d.tree.active_transitions[&panel][&TransitionProperty::Transform].start_time_ms;
        rinch_dom::transition::tick_transitions(&mut d.tree, start + 400.0);
    }
    assert_eq!(
        transform_of(&app, panel).1,
        [0.0, 0.0],
        "precondition: the slide finished, so the panel is at its open position"
    );

    opened.set(false);
    app.resolve_and_repaint(VIEWPORT.0 + 2.0, VIEWPORT.1);

    // ── the slide-out ──
    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "the closing root is held visible for the slide (#759) — it used to go \
         hidden on this very pass"
    );
    assert_eq!(
        visibility_of(&app, panel),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "and so is the panel, which inherits the held value: Stylo computes it \
         from the root's after-change `hidden`"
    );
    let (vis_start, slide_start) = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        (
            d.tree.active_transitions[&root][&TransitionProperty::Visibility].start_time_ms,
            d.tree.active_transitions[&panel][&TransitionProperty::Transform].start_time_ms,
        )
    };
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, slide_start.max(vis_start) + 150.0);
    }
    let midway = transform_of(&app, panel).1[0];
    assert!(
        midway > -1.0 && midway < 0.0,
        "half way through the close the panel is between its open (0.0) and \
         closed (-1.0) positions: got {midway}"
    );
    assert_eq!(
        visibility_of(&app, panel),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "and still visible, so the slide is on screen"
    );
    #[cfg(software_shell)]
    {
        let px = pixel(&mut app, 600, 300);
        assert_ne!(
            px[3], 0,
            "mid-slide the overlay still inks (600, 300): got {px:?}"
        );
    }

    // ── then out of the way ──
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, slide_start.max(vis_start) + 350.0);
    }
    assert_eq!(running(&app, root), 0, "the root's hide has run");
    assert_eq!(running(&app, panel), 0, "and the slide has finished");
    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Hidden,
        "the root is hidden again"
    );
    assert_eq!(
        visibility_of(&app, panel),
        rinch_dom::computed_style::VisibilityValue::Hidden,
        "and the panel under it"
    );
    assert_ne!(
        display_of(&app, root),
        rinch_dom::computed_style::DisplayValue::None,
        "and still rendered — the closed state is `visibility`, not `display`, \
         whichever path reached it"
    );

    let hit_closed = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        hit_test(&d.tree, 40.0, 300.0)
    };
    assert!(
        hit_closed != Some(root) && hit_closed != Some(panel),
        "it takes no clicks again, got {hit_closed:?}"
    );
    assert!(
        !app.collect_focusable_nodes().contains(&close),
        "its close button has left the Tab order again"
    );
    assert!(
        !app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .get(root)
            .unwrap()
            .attributes
            .contains_key("data-trap-focus"),
        "and it traps no focus again"
    );

    #[cfg(software_shell)]
    {
        let px = pixel(&mut app, 600, 300);
        assert_eq!(
            px[3], 0,
            "and nothing is painted: (600, 300) is empty again, got {px:?}"
        );
    }
}

/// Reopening before the slide-out ends cancels the pending hide: the drawer
/// stays visible after the 300ms the close would have taken.
///
/// css-transitions-1 §3 item 3 (#693): the open state declares no `visibility`
/// transition, so the running one no longer matches and is cancelled. Without
/// that, the delayed hide stayed in the map and hid the reopened drawer.
#[test]
fn reopening_during_the_slide_out_keeps_the_drawer_open() {
    let (mut app, opened) = mount_closed();
    let root = node_with_class(&app, ROOT);

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    opened.set(false);
    app.resolve_and_repaint(VIEWPORT.0 + 2.0, VIEWPORT.1);
    let vis_start = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.active_transitions[&root][&TransitionProperty::Visibility].start_time_ms
    };

    opened.set(true);
    app.resolve_and_repaint(VIEWPORT.0 + 3.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, vis_start + 1000.0);
    }
    assert_eq!(
        visibility_of(&app, root),
        rinch_dom::computed_style::VisibilityValue::Visible,
        "the reopened drawer is still visible after the close would have ended"
    );
}
