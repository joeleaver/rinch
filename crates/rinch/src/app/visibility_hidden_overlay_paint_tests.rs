//! #829, at the component level: an overlay that closes by `visibility: hidden`
//! draws none of its text.
//!
//! `Drawer` (since #751) and `Popover` hide their closed state with
//! `visibility: hidden`, and both **close instantly while something else is
//! still animating**: the Drawer's panel slides back over 300ms with its root
//! already hidden, the Popover's dropdown fades `opacity` over 150ms with its
//! `visibility` already flipped. So on the frame after a close the panel is
//! still where it was open — on screen, at full or nearly full opacity — and
//! the only thing keeping its content off the screen is paint honouring
//! `visibility`. Desktop paint used to honour it for boxes but not for the
//! text an IFC root lays out, so the title and body went on drawing with no
//! panel behind them.
//!
//! The oracle is local and absolute: the app is `transparent`, nothing else is
//! mounted, so after the close **every** pixel must be unpainted. Each fixture
//! first proves the same frame is inked while open, and that the close really
//! left the panel on screen (the fixed point where an off-screen transform or a
//! zero opacity hides the text for the wrong reason). Both a full frame and the
//! frame `build_pixels` paints next are checked. For these two overlays the
//! close restyles most of the window, so that next frame is usually a full
//! repaint; the dirty-region path proper is pinned by
//! `hiding_a_span_inside_a_line_repaints_it_incrementally`, which asserts the
//! region it took.
//!
//! **Since #759 neither component closes that way by default.** Both now hold
//! their closed state `visible` for the length of the slide or fade (a delayed
//! `visibility` transition), so the frame after a close legitimately paints the
//! panel *and* its text, and by the time the root hides the panel is off
//! screen or transparent — the fixed point this file is built to stay off. So
//! each fixture restores the instant hide with an app stylesheet
//! ([`INSTANT_HIDE_DRAWER`], [`INSTANT_HIDE_POPOVER`]): the shape is still
//! reachable by any app that writes it, and #829's guarantee is about paint,
//! not about the components' defaults. The default close is pinned by
//! `drawer_open_animation_tests` and `overlay_animation_audit_tests`.
//!
//! `#817`'s text context menu is covered by
//! `text_context_menu_tests::closed_panel_against_paint`; it hides its panel
//! with `display: none` and is unaffected either way.

use super::*;

use rinch_components::{Drawer, Popover, PopoverDropdown, PopoverTarget};
use rinch_core::{Component, Signal};
use std::rc::Rc;

const VP: (u32, u32) = (800, 600);

/// Restores the Drawer's pre-#759 close: the root hides on the close pass
/// while the panel slides back behind it.
const INSTANT_HIDE_DRAWER: &str = ".rinch-drawer__root--hidden { transition: none !important; }";

/// Restores the Popover's pre-#759 close, as desktop used to run it: the
/// dropdown hides on the close pass while its `opacity` fades behind it.
const INSTANT_HIDE_POPOVER: &str =
    ".rinch-popover__dropdown { transition: opacity 150ms ease, transform 150ms ease !important; }";

fn mount(extra_css: &str, build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = build(scope);
        root.append_child(&child);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        if !extra_css.is_empty() {
            d.load_css(extra_css);
        }
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

/// Painted pixels in a full, non-incremental, transparent software frame.
fn full_ink(app: &mut RinchApp) -> usize {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, _, _) = app.build_pixels(1.0, VP, true);
    px.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
}

/// Painted pixels in the next frame without forcing a full repaint — what a
/// running app paints after a change (dirty-region or full, as the region
/// decides).
fn incremental_ink(app: &mut RinchApp) -> usize {
    app.scene_dirty = true;
    let (px, _, _) = app.build_pixels(1.0, VP, true);
    px.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
}

fn class_node(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == class))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "one `.{class}`");
    found[0]
}

/// Run every active transition to completion.
fn settle(app: &mut RinchApp) {
    let doc = app.doc.as_ref().unwrap().clone();
    let mut d = doc.borrow_mut();
    let latest = d
        .tree
        .active_transitions
        .values()
        .flat_map(|m| m.values())
        .map(|t| t.start_time_ms + 10_000.0)
        .fold(0.0_f64, f64::max);
    rinch_dom::transition::tick_transitions(&mut d.tree, latest);
}

#[test]
fn a_drawer_closing_over_its_slide_paints_no_text() {
    let opened = Signal::new(false);
    let mut app = mount(INSTANT_HIDE_DRAWER, move |scope| {
        let body = scope.create_element("p");
        body.set_attribute("style", "font-size: 20px; line-height: 24px; color: black");
        let text = scope.create_text("Drawer body text that must not linger");
        body.append_child(&text);
        Drawer {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "left".to_string(),
            title: "Drawer title".to_string(),
            // No overlay: the overlay alone would ink the open frame, and the
            // assertion is about the panel's text.
            with_overlay: false,
            ..Default::default()
        }
        .render(scope, &[body])
    });
    assert_eq!(
        full_ink(&mut app),
        0,
        "precondition: closed, nothing is drawn"
    );

    opened.set(true);
    app.resolve_and_repaint(VP.0 as f32 + 1.0, VP.1 as f32);
    settle(&mut app);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let open = full_ink(&mut app);
    assert!(
        open > 1000,
        "positive control: the open drawer paints ({open})"
    );

    opened.set(false);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let panel = class_node(&app, "rinch-drawer");
    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let t = &d.tree.get(panel).unwrap().computed_style.transform;
        assert_eq!(
            t.pct_translate_w,
            [0.0, 0.0],
            "precondition: at t = 0 of the close the panel is still at its open \
             position, so nothing but `visibility` keeps its text off the screen"
        );
    }
    assert_eq!(
        incremental_ink(&mut app),
        0,
        "the incremental frame after the close draws nothing"
    );
    assert_eq!(
        full_ink(&mut app),
        0,
        "a full frame after the close draws nothing"
    );
}

#[test]
fn a_popover_closing_over_its_fade_paints_no_text() {
    let opened = Signal::new(false);
    let mut app = mount(INSTANT_HIDE_POPOVER, move |scope| {
        let target = PopoverTarget.render(scope, &[]);
        let inner = scope.create_element("div");
        inner.set_attribute("style", "font-size: 20px; line-height: 24px; color: black");
        let text = scope.create_text("Popover text that must not linger");
        inner.append_child(&text);
        let dropdown = PopoverDropdown.render(scope, &[inner]);
        Popover {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "bottom".to_string(),
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });
    assert_eq!(
        full_ink(&mut app),
        0,
        "precondition: closed, nothing is drawn"
    );

    opened.set(true);
    app.resolve_and_repaint(VP.0 as f32 + 1.0, VP.1 as f32);
    settle(&mut app);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let open = full_ink(&mut app);
    assert!(
        open > 500,
        "positive control: the open popover paints ({open})"
    );

    opened.set(false);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let panel = class_node(&app, "rinch-popover__dropdown");
    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let opacity = d.tree.get(panel).unwrap().computed_style.opacity;
        assert!(
            opacity > 0.9,
            "precondition: at t = 0 of the close the fade has not begun \
             (opacity {opacity}), so nothing but `visibility` hides the text"
        );
    }
    assert_eq!(
        incremental_ink(&mut app),
        0,
        "the incremental frame after the close draws nothing"
    );
    assert_eq!(
        full_ink(&mut app),
        0,
        "a full frame after the close draws nothing"
    );
}

/// The rect `compute_dirty_region` hands the incremental frame, if any.
fn region(app: &RinchApp) -> Option<(f64, f64, f64, f64)> {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, VP.0 as f64, VP.1 as f64)
        .map(|r| (r.x0, r.y0, r.x1, r.y1))
}

fn frame_px(app: &mut RinchApp, full: bool) -> Vec<u8> {
    app.scene_dirty = true;
    if full {
        app.has_previous_frame = false;
    }
    app.build_pixels(1.0, VP, true).0.to_vec()
}

fn red_px(px: &[u8]) -> usize {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80 && p[0] as i32 > p[2] as i32 + 80)
        .count()
}

/// A line of blue text with a red span in it, the span's `visibility` bound to
/// `hidden`; and, when `other` is set, a 20x20 box 200px below whose background
/// changes in the same effect — an unrelated paint change in the same frame.
fn line_page(hidden: Signal<bool>, other: bool) -> RinchApp {
    mount("", move |scope| {
        let wrap = scope.create_element("div");
        let line = scope.create_element("div");
        line.set_attribute(
            "style",
            "font-size: 20px; line-height: 24px; color: rgb(0, 0, 255); width: 300px",
        );
        let before = scope.create_text("before ");
        line.append_child(&before);
        let span = scope.create_element("span");
        span.set_attribute("style", "color: rgb(255, 0, 0)");
        let t = scope.create_text("SPAN");
        span.append_child(&t);
        line.append_child(&span);
        let after = scope.create_text(" after");
        line.append_child(&after);
        wrap.append_child(&line);
        let span_in = span.clone();
        rinch_core::reactive::Effect::new(move || {
            span_in.set_style(
                "visibility",
                if hidden.get() { "hidden" } else { "visible" },
            );
        });
        if other {
            let o = scope.create_element("div");
            o.set_attribute("style", "width: 20px; height: 20px; margin-top: 200px");
            wrap.append_child(&o);
            rinch_core::reactive::Effect::new(move || {
                o.set_style(
                    "background-color",
                    if hidden.get() {
                        "rgb(0, 128, 0)"
                    } else {
                        "rgb(0, 0, 128)"
                    },
                );
            });
        }
        wrap
    })
}

/// Toggling a span's visibility — at an **unchanged** viewport, so nothing
/// re-lays out — must reach the dirty region. The span is a flowed inline with
/// a `0x0` box, so its own rect adds nothing; the region has to come from the
/// IFC root that draws its glyphs (PR #844 review, F1).
///
/// Two halves:
/// - alone, the region must be `Some` and smaller than the viewport — so the
///   frame really takes the dirty-region path, which is the positive control
///   that the incremental assertion below is not a full repaint in disguise;
/// - beside an unrelated change elsewhere in the same frame (the review's D4),
///   the region must still cover the span. Before the fix it covered only the
///   other box, and the incremental frame kept 286 red pixels of the hidden
///   span.
///
/// In both, the incremental frame must equal a full frame, hiding and showing.
#[test]
fn hiding_a_span_inside_a_line_repaints_it_incrementally() {
    for other in [false, true] {
        let hidden = Signal::new(false);
        let mut app = line_page(hidden, other);
        assert!(
            red_px(&frame_px(&mut app, true)) > 50,
            "positive control: the span inks red"
        );
        for (step, h) in [("hide", true), ("show", false), ("hide again", true)] {
            hidden.set(h);
            app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            let reg = region(&app);
            let (x0, y0, x1, y1) =
                reg.unwrap_or_else(|| panic!("other={other} {step}: a dirty region"));
            assert!(
                (x1 - x0) * (y1 - y0) < 0.5 * VP.0 as f64 * VP.1 as f64,
                "other={other} {step}: the region {reg:?} is small enough for an \
                 incremental frame, so the next assertion tests the dirty-region path"
            );
            assert!(
                y0 <= 4.0 && x1 >= 100.0,
                "other={other} {step}: the region {reg:?} covers the line"
            );
            let inc = frame_px(&mut app, false);
            let full = frame_px(&mut app, true);
            assert_eq!(
                red_px(&inc),
                red_px(&full),
                "other={other} {step}: the incremental frame's span matches a full frame's"
            );
            assert!(inc == full, "other={other} {step}: incremental == full");
            assert_eq!(red_px(&full) == 0, h, "other={other} {step}");
        }
    }
}

/// The span toggles in the same frame its line **moves** (its root's own
/// `margin-left` changes with the same signal), and an unrelated box 200px
/// below recolours so the frame takes the dirty-region path.
///
/// The root is paint-dirty itself (it moved), so its old-position rect must be
/// in the region; the span's rule adds the root's *current* rect. The first
/// version of that rule marked the root as seen before the root's own entry
/// came round, which skipped the root's old-position rect and left 781 px of
/// the line painted where it used to be (review of PR #844, round 2, `a6b`).
#[test]
fn a_span_toggled_as_its_line_moves_leaves_no_ghost() {
    let hidden = Signal::new(false);
    let mut app = mount("", move |scope| {
        let wrap = scope.create_element("div");
        let line = scope.create_element("div");
        line.set_attribute(
            "style",
            "font-size: 20px; line-height: 24px; color: rgb(0, 0, 255); width: 300px",
        );
        let t = scope.create_text("before ");
        line.append_child(&t);
        let span = scope.create_element("span");
        span.set_attribute("style", "color: rgb(255, 0, 0)");
        let st = scope.create_text("SPAN");
        span.append_child(&st);
        let s = span.clone();
        rinch_core::reactive::Effect::new(move || {
            s.set_style(
                "visibility",
                if hidden.get() { "hidden" } else { "visible" },
            );
        });
        line.append_child(&span);
        let t3 = scope.create_text(" after");
        line.append_child(&t3);
        let l = line.clone();
        rinch_core::reactive::Effect::new(move || {
            l.set_style("margin-left", if hidden.get() { "420px" } else { "0px" });
        });
        wrap.append_child(&line);
        let o = scope.create_element("div");
        o.set_attribute("style", "width: 20px; height: 20px; margin-top: 200px");
        wrap.append_child(&o);
        rinch_core::reactive::Effect::new(move || {
            o.set_style(
                "background-color",
                if hidden.get() {
                    "rgb(0, 128, 0)"
                } else {
                    "rgb(0, 0, 128)"
                },
            );
        });
        wrap
    });
    app.resolve_and_repaint(VP.0 as f32 + 3.0, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(red_px(&frame_px(&mut app, true)) > 50, "positive control");
    for (step, h) in [
        ("hide", true),
        ("show", false),
        ("hide again", true),
        ("show again", false),
    ] {
        hidden.set(h);
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        let reg = region(&app);
        let (x0, y0, x1, y1) = reg.unwrap_or_else(|| panic!("{step}: a dirty region"));
        assert!(
            (x1 - x0) * (y1 - y0) < 0.5 * VP.0 as f64 * VP.1 as f64,
            "{step}: the region {reg:?} is small enough for an incremental frame"
        );
        let inc = frame_px(&mut app, false);
        let full = frame_px(&mut app, true);
        let diff = inc
            .as_chunks::<4>()
            .0
            .iter()
            .zip(full.as_chunks::<4>().0)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            diff, 0,
            "{step}: the incremental frame equals a full one (region {reg:?})"
        );
        assert_eq!(red_px(&full) == 0, h, "{step}");
    }
}
