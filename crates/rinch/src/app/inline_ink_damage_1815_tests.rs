//! What an inline element draws past its paragraph's box is cleared when it
//! changes (#1815, #1817).
//!
//! An inline element owns no box: its background, border, `vertical-align`
//! shifted text and wavy underline are drawn by the IFC root it flows in, so
//! a change to it damages that root's box grown by the root's ink
//! (`paint::ink_outsets_in`), now and as last painted (`Node::painted`). That
//! ink left out everything the root draws past its unshifted line boxes, so a
//! pill's vertical padding, raised or lowered text and a wave below the line
//! were neither cleared nor repainted on a partial frame: stale pixels above
//! and below the paragraph.
//!
//! Each fixture is a local pixel oracle: the band just above and below the
//! paragraph's box, where the old frame (positive control) held ink, must hold
//! exactly what a from-scratch frame of the new state holds — at scales 1 and
//! 2, and with the frame proven incremental by the perf counters.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const W: u32 = 300;
const H: u32 = 200;
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The paragraph's box (CSS px): left, top, width, height.
const BOX: (f64, f64, f64, f64) = (40.0, 80.0, 200.0, 18.0);

fn size(scale: f64) -> (u32, u32) {
    ((W as f64 * scale) as u32, (H as f64 * scale) as u32)
}

fn full_frame(app: &mut RinchApp, scale: f64) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(scale, (W, H), false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn incremental_frame(app: &mut RinchApp, scale: f64) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(scale, (W, H), false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

/// The bands above and below the paragraph's box, in device px.
fn bands(scale: f64) -> [(i32, i32, i32, i32); 2] {
    let s = |v: f64| (v * scale).round() as i32;
    let (l, t, w, h) = BOX;
    [
        (s(l - 10.0), s(t - 30.0), s(l + w + 10.0), s(t)),
        (s(l - 10.0), s(t + h), s(l + w + 10.0), s(t + h + 30.0)),
    ]
}

fn diff_in(a: &[u8], b: &[u8], scale: f64, rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = size(scale);
    let (w, h) = (w as i32, h as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 2) {
                n += 1;
            }
        }
    }
    n
}

fn ink_in(px: &[u8], scale: f64, rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = size(scale);
    let (w, h) = (w as i32, h as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if px[i] < 250 || px[i + 1] < 250 || px[i + 2] < 250 {
                n += 1;
            }
        }
    }
    n
}

/// A paragraph at [`BOX`] (not clipping), holding `x ` and a span styled
/// `span` around `Agy`, then a box styled `after` (if any) painted after it.
/// Returns the app, the span and that box.
fn paragraph_with(span: &str, after: Option<&str>) -> (RinchApp, NodeHandle, Option<NodeHandle>) {
    let span = span.to_string();
    let after = after.map(str::to_string);
    type Slot = Option<(NodeHandle, Option<NodeHandle>)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute(
            "style",
            "width: 300px; height: 200px; background: rgb(255, 255, 255)",
        );
        let (l, t, w, h) = BOX;
        let p = scope.create_element("div");
        p.set_attribute(
            "style",
            &format!(
                "position: absolute; left: {l}px; top: {t}px; width: {w}px; height: {h}px; \
                 white-space: nowrap; font-family: sans-serif; font-size: 14px; \
                 line-height: 18px; color: rgb(0, 0, 0)"
            ),
        );
        outer.append_child(&p);
        let lead = scope.create_text("x ");
        p.append_child(&lead);
        let s = scope.create_element("span");
        s.set_attribute("style", &span);
        p.append_child(&s);
        let t = scope.create_text("Agy");
        s.append_child(&t);
        let a = after.as_ref().map(|style| {
            let a = scope.create_element("div");
            a.set_attribute("style", style);
            outer.append_child(&a);
            a
        });
        *slot_in.borrow_mut() = Some((s, a));
        outer
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    let (s, a) = slot.borrow().clone().unwrap();
    (app, s, a)
}

fn paragraph(span: &str) -> (RinchApp, NodeHandle) {
    let (app, s, _) = paragraph_with(span, None);
    (app, s)
}

fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(W as f32, H as f32);
}

/// Paint `from` whole, restyle the span to each of `steps` in turn (an
/// incremental frame after each), and require the last incremental frame to
/// match a from-scratch one in the bands, where `from` held ink.
fn assert_clean(name: &str, from: &str, steps: &[&str], scale: f64) -> Vec<String> {
    let mut failures = Vec::new();
    let (mut app, span) = paragraph(from);
    let before = full_frame(&mut app, scale);
    let held: usize = bands(scale).iter().map(|&r| ink_in(&before, scale, r)).sum();
    if held == 0 {
        failures.push(format!(
            "{name} scale {scale}: positive control — nothing drawn past the box"
        ));
    }
    let mut last = None;
    for step in steps {
        span.set_attribute("style", step);
        resolve(&mut app);
        last = Some(incremental_frame(&mut app, scale));
    }
    let (inc, stats) = last.expect("a step");
    if stats.get(Counter::RepaintFull) != 0 || stats.get(Counter::RepaintPartial) != 1 {
        failures.push(format!("{name} scale {scale}: not incremental: {stats:?}"));
    }
    let full = full_frame(&mut app, scale);
    let ghosts: usize = bands(scale)
        .iter()
        .map(|&r| diff_in(&inc, &full, scale, r))
        .sum();
    if ghosts != 0 {
        failures.push(format!("{name} scale {scale}: {ghosts} stale pixels"));
    }
    failures
}

const PILL: &str = "background-color: rgb(0, 128, 0)";

#[test]
fn a_change_to_what_an_inline_draws_past_its_paragraph_is_cleared() {
    let big_pill = format!("{PILL}; padding: 11px 3px");
    let small_pill = format!("{PILL}; padding: 1px 3px");
    let sup_pill = format!("vertical-align: super; {PILL}; padding: 1px 3px");
    let cases: Vec<(&str, String, Vec<String>)> = vec![
        ("pill shrinks", big_pill.clone(), vec![small_pill.clone()]),
        (
            "pill grows then shrinks",
            small_pill.clone(),
            vec![big_pill.clone(), small_pill.clone()],
        ),
        ("pill goes", big_pill.clone(), vec![String::new()]),
        (
            "raised text comes down",
            "vertical-align: 12px".into(),
            vec![String::new()],
        ),
        (
            "lowered text comes up",
            "vertical-align: -12px".into(),
            vec![String::new()],
        ),
        ("sup pill comes down", sup_pill.clone(), vec![small_pill.clone()]),
        (
            "sub border goes",
            "vertical-align: sub; border: 2px solid rgb(0, 0, 200)".into(),
            vec!["vertical-align: sub".into()],
        ),
        (
            "a big wave goes",
            "font-size: 26px; text-decoration: underline wavy rgb(200, 0, 0)".into(),
            vec!["font-size: 26px".into()],
        ),
        (
            "wave goes",
            "text-decoration: underline wavy rgb(200, 0, 0); text-underline-offset: 0px".into(),
            vec![String::new()],
        ),
    ];
    let mut failures = Vec::new();
    for scale in [1.0, 2.0] {
        for (name, from, steps) in &cases {
            let steps: Vec<&str> = steps.iter().map(String::as_str).collect();
            failures.extend(assert_clean(name, from, &steps, scale));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Damage that reaches only what the paragraph draws past its box — a box
/// above it changes — repaints that ink: the paint prune grows the
/// paragraph's box by the same ink (#1815). It used to skip the paragraph,
/// and the damage's clear left the pill's padding white there.
#[test]
fn damage_over_only_a_pill_s_padding_repaints_it() {
    let (l, t, _, _) = BOX;
    let neighbour = |c: &str| {
        format!(
            "position: absolute; left: {}px; top: {}px; width: 60px; height: 9px; \
             border-top: 1px solid {c}",
            l - 5.0,
            t - 10.0
        )
    };
    let mut failures = Vec::new();
    for scale in [1.0, 2.0] {
        let (mut app, _span, after) = paragraph_with(
            &format!("{PILL}; padding: 11px 3px"),
            Some(&neighbour("rgb(0, 0, 200)")),
        );
        let after = after.unwrap();
        let before = full_frame(&mut app, scale);
        let s = |v: f64| (v * scale).round() as i32;
        // The pill's padding under the neighbour's box, below its border.
        let probe = (s(l), s(t - 8.0), s(l + 40.0), s(t));
        if ink_in(&before, scale, probe) == 0 {
            failures.push(format!("scale {scale}: positive control — no pill under the box"));
        }
        after.set_attribute("style", &neighbour("rgb(200, 0, 200)"));
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app, scale);
        if stats.get(Counter::RepaintPartial) != 1 {
            failures.push(format!("scale {scale}: not incremental: {stats:?}"));
        }
        let full = full_frame(&mut app, scale);
        let ghosts = diff_in(&inc, &full, scale, probe);
        if ghosts != 0 {
            failures.push(format!("scale {scale}: {ghosts} pill pixels lost"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
