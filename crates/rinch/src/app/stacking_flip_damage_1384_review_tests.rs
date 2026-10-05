//! The fixtures the review of PR #1399 (#1384) added: context-only flips
//! (group opacity, clip membership of hoisted descendants), what the damage
//! of a flip is clipped by, and a seeded differential.
use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const SIZE: (u32, u32) = (600, 400);

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}
fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}
fn diff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| (0..4).any(|k| (x[k] as i32 - y[k] as i32).abs() > 2))
        .count()
}
fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

/// (parent index or None for the outer root, style)
pub(super) type Spec = Vec<(Option<usize>, String)>;

fn build(spec: &Spec) -> (RinchApp, Vec<NodeHandle>) {
    let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let slot_in = slot.clone();
    let spec = spec.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let mut hs: Vec<NodeHandle> = Vec::new();
        for (parent, style) in &spec {
            let e = scope.create_element("div");
            e.set_attribute("style", style);
            match parent {
                Some(i) => hs[*i].append_child(&e),
                None => outer.append_child(&e),
            }
            hs.push(e);
        }
        *slot_in.borrow_mut() = hs;
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let hs = slot.borrow().clone();
    (app, hs)
}

struct Outcome {
    changed: usize,
    stale: usize,
    full: u64,
    repainted: u64,
}

fn probe(spec: &Spec, target: usize, to: &str) -> Outcome {
    let (mut app, hs) = build(spec);
    let before = full_frame(&mut app);
    hs[target].set_attribute("style", to);
    resolve(&mut app);
    let (inc, stats) = incremental_frame(&mut app);
    let full = full_frame(&mut app);
    Outcome {
        changed: diff(&before, &full),
        stale: diff(&inc, &full),
        full: stats.get(Counter::RepaintFull),
        repainted: stats.get(Counter::RepaintedPx),
    }
}

fn s(parent: Option<usize>, style: &str) -> (Option<usize>, String) {
    (parent, style.to_string())
}

fn report(name: &str, o: &Outcome) {
    eprintln!(
        "PROBE {name}: changed={} stale={} full={} repainted={}",
        o.changed, o.stale, o.full, o.repainted
    );
}

const RED: &str = "width: 100px; height: 100px; background: rgb(200, 0, 0)";
const BLUE: &str = "width: 100px; height: 100px; background: rgb(0, 0, 200)";

/// A: positioned z-auto box crosses opacity 1 (context-only flip), in-flow
/// child overflows it. Group opacity now reaches the child.
#[test]
fn a_positioned_box_crossing_opacity_one_with_overflowing_flow_child() {
    for (from, to) in [("", "opacity: 0.5;"), ("opacity: 0.5;", "")] {
        let p = "position: relative; width: 100px; height: 100px;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(Some(0), &format!("{p} {from}")),
            s(Some(1), &format!("margin-left: 150px; {RED}")),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("A relative opacity {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0, "stale after incremental");
    }
}

/// A': the same with a static box (the PR's Subtree arm) as control.
#[test]
fn a_static_box_crossing_opacity_one_with_overflowing_flow_child_control() {
    let p = "width: 100px; height: 100px;";
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(Some(0), p),
        s(Some(1), &format!("margin-left: 150px; {RED}")),
    ];
    let o = probe(&spec, 1, &format!("{p} opacity: 0.5"));
    report("A' static opacity", &o);
    assert!(o.changed > 9000);
    assert_eq!(o.stale, 0);
}

/// A'': filter starting on a positioned z-auto box.
#[test]
fn a_positioned_box_starting_a_filter_with_overflowing_flow_child() {
    let p = "position: relative; width: 100px; height: 100px;";
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(Some(0), p),
        s(Some(1), &format!("margin-left: 150px; {RED}")),
    ];
    let o = probe(&spec, 1, &format!("{p} filter: brightness(0.5)"));
    report("A'' relative filter", &o);
    assert!(o.changed > 9000);
    assert_eq!(o.full, 0);
    assert_eq!(o.stale, 0);
}

/// A''': positioned z-auto box crossing opacity with a positioned z-auto
/// descendant overflowing (abs child).
#[test]
fn a_positioned_box_crossing_opacity_one_with_overflowing_abs_child() {
    let p = "position: relative; width: 100px; height: 100px;";
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(Some(0), p),
        s(
            Some(1),
            &format!("position: absolute; left: 150px; top: 0; {RED}"),
        ),
    ];
    let o = probe(&spec, 1, &format!("{p} opacity: 0.5"));
    report("A''' relative opacity abs child", &o);
    assert!(o.changed > 9000);
    assert_eq!(o.full, 0);
    assert_eq!(o.stale, 0);
}

/// B: clipper > relative z-auto <-> z:0 > fixed child outside the clipper
/// (#549: a fixed box under a stacking context under a plain clipper is
/// clipped away).
#[test]
fn a_context_only_flip_over_a_fixed_descendant_under_a_clipper() {
    for (from, to) in [("", "z-index: 0;"), ("z-index: 0;", "")] {
        let p = "position: relative; width: 100px; height: 100px;";
        let spec = vec![
            s(None, "width: 100px; height: 100px; overflow: hidden"),
            s(Some(0), &format!("{p} {from}")),
            s(
                Some(1),
                &format!("position: fixed; left: 300px; top: 200px; {RED}"),
            ),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("B fixed under clipper {from:?}->{to:?}"), &o);
        if o.changed > 0 {
            assert_eq!(o.stale, 0, "stale after incremental (full={})", o.full);
        }
    }
}

/// C: a zero-height box (w > 0) whose z child overflows it.
#[test]
fn a_zero_height_box_that_becomes_a_context() {
    for (from, to) in [("", "opacity: 0.99;"), ("opacity: 0.99;", "")] {
        let p = "width: 100px; height: 0px;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(Some(0), &format!("{p} {from}")),
            s(
                Some(1),
                &format!("position: relative; left: 150px; z-index: 5; {RED}"),
            ),
            s(
                Some(0),
                &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
            ),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("C zero-height {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000);
        assert_eq!(o.stale, 0);
    }
}

/// D: a static flex item with a z-index (#542) whose PARENT stops being a
/// flex container: the item's key flips though its own style did not.
#[test]
fn a_flex_item_context_whose_container_stops_being_flex() {
    for (from, to) in [
        ("display: flex;", "display: block;"),
        ("display: block;", "display: flex;"),
    ] {
        let f = "width: 100px; height: 100px;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(Some(0), &format!("{f} {from}")),
            s(
                Some(1),
                "width: 100px; height: 100px; z-index: 0; flex: none",
            ),
            s(
                Some(2),
                &format!("position: relative; left: 150px; z-index: 5; {RED}"),
            ),
            s(
                Some(0),
                &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
            ),
        ];
        let o = probe(&spec, 1, &format!("{f} {to}"));
        report(&format!("D flex item {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000, "positive control {}", o.changed);
        assert_eq!(o.stale, 0);
    }
}

/// E1: a flip with nothing reaching past the box, scrolled out of a
/// clipper's view, repaints nothing (the box's clip chain still applies).
#[test]
fn e1_a_flip_with_no_reach_below_a_clippers_viewport_repaints_nothing() {
    let p = "width: 100px; height: 100px;";
    let spec = vec![
        s(None, "width: 300px; height: 100px; overflow: hidden"),
        s(Some(0), "height: 150px"),
        s(Some(0), p),
        s(Some(2), RED),
    ];
    let o = probe(&spec, 2, &format!("{p} opacity: 0.99"));
    report("E1 clipped-out flip (no reach)", &o);
    assert_eq!(o.repainted, 0);
}

/// E2: the same flip with an in-flow child that overflows the box (still
/// wholly inside the clipper's hidden part): the reach is not clipped.
#[test]
fn e2_a_flip_with_reach_below_a_clippers_viewport_repaints_nothing() {
    let p = "width: 100px; height: 100px;";
    let spec = vec![
        s(None, "width: 300px; height: 100px; overflow: hidden"),
        s(Some(0), "height: 150px"),
        s(Some(0), p),
        s(Some(2), &format!("margin-left: 150px; {RED}")),
    ];
    let o = probe(&spec, 2, &format!("{p} opacity: 0.99"));
    report("E2 clipped-out flip (reach)", &o);
    assert_eq!(o.changed, 0, "nothing visible changes");
    assert_eq!(
        o.repainted, 0,
        "a flip clipped out of view repaints nothing"
    );
}

/// E3: a visible row in a 300x100 clipper flips; its overflowing child is cut
/// by the clipper. The damage should stay inside the clipper.
#[test]
fn e3_a_flip_inside_a_clipper_stays_inside_the_clipper() {
    let p = "width: 100px; height: 100px;";
    let spec = vec![
        s(None, "width: 120px; height: 100px; overflow: hidden"),
        s(Some(0), p),
        s(
            Some(1),
            "margin-left: 20px; width: 400px; height: 100px; background: rgb(200,0,0)",
        ),
    ];
    let o = probe(&spec, 1, &format!("{p} opacity: 0.99"));
    report("E3 flip inside clipper", &o);
    assert_eq!(o.stale, 0);
    // The clipper's 120 x 100, exactly: the 400px child is cut by it.
    assert_eq!(o.repainted, 120 * 100);
}

/// M7: a static box whose transform stops: the content it moved is repainted
/// where it WAS painted (40px away), not only where it is now.
#[test]
fn a_static_box_whose_transform_stops_repaints_where_its_content_was() {
    for (from, to) in [
        ("transform: translateY(250px);", ""),
        ("", "transform: translateY(250px);"),
    ] {
        let p = "width: 100px; height: 100px;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 400px"),
            s(Some(0), &format!("{p} {from}")),
            s(Some(1), &format!("margin-left: 150px; {RED}")),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("static transform {from:?}->{to:?}"), &o);
        assert!(o.changed > 3000);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0);
    }
}

/// R: a flowed inline span that becomes a context, with a hoisted abs z child.
#[test]
fn r_a_flowed_inline_that_becomes_a_context() {
    let (mut app, hs) = {
        let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
        let slot_in = slot.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let outer = scope.create_element("div");
            outer.set_attribute("style", "width: 600px; height: 400px");
            let c = scope.create_element("div");
            c.set_attribute("style", "position: relative; width: 300px; height: 100px");
            outer.append_child(&c);
            let p = scope.create_element("div");
            p.set_attribute(
                "style",
                "width: 100px; height: 100px; line-height: 20px; font-size: 16px",
            );
            c.append_child(&p);
            let span = scope.create_element("span");
            span.set_attribute("style", "position: relative");
            span.append_child(&scope.create_text("x"));
            p.append_child(&span);
            let k = scope.create_element("div");
            k.set_attribute(
                "style",
                &format!("position: absolute; left: 150px; top: 0; z-index: 5; {RED}"),
            );
            span.append_child(&k);
            let sib = scope.create_element("div");
            sib.set_attribute(
                "style",
                &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
            );
            c.append_child(&sib);
            *slot_in.borrow_mut() = vec![span];
            outer
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        resolve(&mut app);
        let hs = slot.borrow().clone();
        (app, hs)
    };
    for to in [
        "position: relative; z-index: 0",
        "position: relative",
        "position: relative; opacity: 0.99",
        "position: relative",
    ] {
        let before = full_frame(&mut app);
        hs[0].set_attribute("style", to);
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        let full = full_frame(&mut app);
        eprintln!(
            "PROBE R span -> {to:?}: changed={} stale={} full={}",
            diff(&before, &full),
            diff(&inc, &full),
            stats.get(Counter::RepaintFull)
        );
        assert_eq!(diff(&inc, &full), 0, "span -> {to:?}");
    }
}

// ── Seeded differential ────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.next() as usize % xs.len()]
    }
    fn range(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// The order-relevant part of a node's style, toggled by the flip. No
/// `opacity` (its value changes pixels: #1395's class) and no `absolute`
/// (toggling it reflows siblings: #880 J2's class).
fn order_style(r: &mut Rng, abs: bool) -> String {
    let pos = if abs {
        ""
    } else {
        r.pick(&["", "", "position: relative;"])
    };
    let z = r.pick(&[
        "",
        "",
        "z-index: 0;",
        "z-index: 1;",
        "z-index: 3;",
        "z-index: -1;",
    ]);
    let tf = r.pick(&["", "", "", "transform: translateX(0);"]);
    format!("{pos}{z}{tf}")
}

fn gen_case(seed: u64) -> (Spec, Vec<String>, usize, String) {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xD1B54A32D192ED03);
    let n = 4 + r.range(5) as usize;
    let colors = [
        "rgb(200,0,0)",
        "rgb(0,160,0)",
        "rgb(0,0,200)",
        "rgb(200,160,0)",
        "rgb(0,160,200)",
        "rgb(200,0,200)",
        "rgb(90,90,90)",
        "rgb(240,120,60)",
        "rgb(20,220,140)",
    ];
    let mut spec: Spec = vec![s(None, "position: relative; width: 500px; height: 300px")];
    let mut fixed: Vec<String> = vec!["position: relative; width: 500px; height: 300px;".into()];
    let mut is_abs = vec![false];
    for i in 1..=n {
        let parent = r.range(i as u32) as usize;
        let w = 60 + 20 * r.range(4);
        let h = 60 + 20 * r.range(3);
        let ml = 30 * r.range(5) as i32 - 30;
        let mt = 20 * r.range(4) as i32 - 60;
        let abs = r.range(4) == 0;
        let lt = if abs {
            format!(
                "position: absolute; left: {}px; top: {}px;",
                20 * r.range(8),
                20 * r.range(6)
            )
        } else {
            String::new()
        };
        let clip = if r.range(6) == 0 {
            "overflow: hidden;"
        } else {
            ""
        };
        let geom = format!(
            "width: {w}px; height: {h}px; margin-left: {ml}px; margin-top: {mt}px; {lt}{clip} background: {};",
            colors[i % colors.len()]
        );
        let ord = order_style(&mut r, abs);
        spec.push(s(Some(parent), &format!("{geom}{ord}")));
        fixed.push(geom);
        is_abs.push(abs);
    }
    let target = 1 + r.range(n as u32) as usize;
    let to = format!("{}{}", fixed[target], order_style(&mut r, is_abs[target]));
    (spec, fixed, target, to)
}

#[test]
#[ignore = "a search tool: its remaining hits are the pre-existing `g_` class above"]
fn seeded_differential_of_order_flips() {
    let seeds: u64 = std::env::var("REVIEW_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let start: u64 = std::env::var("REVIEW_SEED_START")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut bad = 0;
    let mut changed_cases = 0;
    let mut fulls = 0;
    for seed in start..start + seeds {
        let (spec, _fixed, target, to) = gen_case(seed);
        if spec[target].1 == to {
            continue;
        }
        let o = probe(&spec, target, &to);
        if o.changed > 0 {
            changed_cases += 1;
        }
        if o.full > 0 {
            fulls += 1;
        }
        if o.stale > 0 {
            bad += 1;
            eprintln!(
                "SEED {seed}: stale={} changed={} target={target}\n   from: {}\n   to:   {}",
                o.stale, o.changed, spec[target].1, to
            );
            if std::env::var("REVIEW_DUMP").is_ok() {
                for (i, (p, st)) in spec.iter().enumerate() {
                    eprintln!("     [{i}] parent={p:?} {st}");
                }
            }
        }
    }
    eprintln!(
        "DIFFERENTIAL: {bad} stale of {seeds} seeds ({changed_cases} changed pixels, {fulls} full repaints)"
    );
    assert_eq!(bad, 0);
}

/// F2: a filter on a box with overflowing content, then further incremental
/// frames (does the filter overlay stay inside the damage?).
#[test]
fn f2_a_filter_box_repainted_incrementally() {
    let p = "position: relative; width: 100px; height: 100px;";
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(Some(0), &format!("{p} filter: brightness(0.5)")),
        s(Some(1), &format!("margin-left: 150px; {RED}")),
    ];
    let (mut app, hs) = build(&spec);
    let _ = full_frame(&mut app);
    for i in 0..3 {
        hs[1].set_attribute(
            "style",
            &format!(
                "{p} filter: brightness(0.5); background: rgb(0, {}, 0)",
                50 + i * 50
            ),
        );
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        eprintln!(
            "PROBE F2 frame {i}: repainted={} sample(200,50)={:?}",
            stats.get(Counter::RepaintedPx),
            &inc[(50 * 600 + 200) * 4..(50 * 600 + 200) * 4 + 4]
        );
        if i == 2 {
            let full = full_frame(&mut app);
            eprintln!(
                "PROBE F2 after 3 incremental frames: stale={}",
                diff(&inc, &full)
            );
            assert_eq!(diff(&inc, &full), 0);
        }
    }
}

/// The PR's own "must not grow" pair (`FlowChild`, relative -> relative +
/// opacity) at an opacity that is visible past the oracle's tolerance.
#[test]
fn the_prs_non_flip_pair_at_a_visible_opacity() {
    let p = "position: relative; width: 100px; height: 100px;";
    for op in ["0.99", "0.9", "0.5"] {
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(
                Some(0),
                &format!("position: absolute; left: 150px; top: 0; {BLUE}"),
            ),
            s(Some(0), p),
            s(Some(2), &format!("margin-left: 150px; {RED}")),
        ];
        let o = probe(&spec, 2, &format!("{p} opacity: {op}"));
        report(&format!("PR non-flip pair at opacity {op}"), &o);
    }
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(
            Some(0),
            &format!("position: absolute; left: 150px; top: 0; {BLUE}"),
        ),
        s(Some(0), p),
        s(Some(2), &format!("margin-left: 150px; {RED}")),
    ];
    let o = probe(&spec, 2, &format!("{p} opacity: 0.5"));
    assert_eq!(o.stale, 0);
}

/// Context-only flip with a NEGATIVE z descendant (released behind the
/// parent's in-flow content).
#[test]
fn a_context_only_flip_over_a_negative_z_descendant() {
    for (from, to) in [("", "z-index: 0;"), ("z-index: 0;", "")] {
        let p = "position: relative; width: 100px; height: 100px;";
        let spec = vec![
            s(
                None,
                "position: relative; z-index: 0; width: 300px; height: 100px",
            ),
            s(Some(0), &format!("margin-left: 150px; {BLUE}")),
            s(Some(0), &format!("{p} margin-top: -100px; {from}")),
            s(
                Some(2),
                &format!("position: relative; left: 150px; z-index: -1; {RED}"),
            ),
        ];
        let o = probe(&spec, 2, &format!("{p} margin-top: -100px; {to}"));
        report(&format!("neg-z {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000, "positive control {}", o.changed);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0);
    }
}

/// Context-only flip with the z descendant two levels down, through a
/// non-context wrapper.
#[test]
fn a_context_only_flip_over_a_nested_z_descendant() {
    for (from, to) in [("", "z-index: 0;"), ("z-index: 0;", "")] {
        let p = "position: relative; width: 100px; height: 100px;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(Some(0), &format!("{p} {from}")),
            s(Some(1), "width: 100px; height: 100px"),
            s(
                Some(2),
                &format!("position: relative; left: 150px; z-index: 5; {RED}"),
            ),
            s(
                Some(0),
                &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
            ),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("nested z {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0);
    }
}

/// A transform that STOPS applying on a positioned box whose content it moved
/// (the PR's fixture, sampled at each direction separately).
#[test]
fn a_transform_stopping_on_a_positioned_box() {
    let p = "position: relative; width: 100px; height: 100px;";
    let spec = vec![
        s(None, "position: relative; width: 300px; height: 200px"),
        s(Some(0), &format!("{p} transform: translateY(40px)")),
        s(Some(1), &format!("margin-left: 150px; {RED}")),
    ];
    let o = probe(&spec, 1, p);
    report("transform stop", &o);
    assert!(o.changed > 3000);
    assert_eq!(o.stale, 0);
}

/// G: the flipping box is itself a clipper and starts/stops being the abs
/// containing block (#550/#961 class): its absolute child is clipped in, or
/// escapes.
#[test]
#[ignore = "pre-existing, not #1384: the #550/#961 composition (issue draft of the #1399 review)"]
fn g_a_clipper_that_stops_being_the_containing_block() {
    for (from, to) in [("position: relative;", ""), ("", "position: relative;")] {
        let p = "width: 100px; height: 100px; overflow: hidden;";
        let spec = vec![
            s(None, "position: relative; width: 300px; height: 100px"),
            s(Some(0), &format!("{p} {from}")),
            s(
                Some(1),
                &format!("position: absolute; left: 150px; top: 0; {RED}"),
            ),
        ];
        let o = probe(&spec, 1, &format!("{p} {to}"));
        report(&format!("G clipper CB {from:?}->{to:?}"), &o);
        if o.changed > 0 {
            assert_eq!(o.stale, 0);
        }
    }
}

/// T: an opacity transition reversed mid-run, painted at each step.
#[test]
fn t_an_opacity_transition_reversed_mid_run() {
    use rinch_dom::transition::TransitionProperty;
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        let style = scope.create_element("style");
        style.append_child(
            &scope.create_text(".p { transition: opacity 1000ms linear; } .p.on { opacity: 0.5; }"),
        );
        outer.append_child(&style);
        let c = scope.create_element("div");
        c.set_attribute("style", "position: relative; width: 300px; height: 100px");
        outer.append_child(&c);
        let p = scope.create_element("div");
        p.set_attribute("style", "width: 100px; height: 100px");
        p.set_attribute("class", "p");
        c.append_child(&p);
        let k = scope.create_element("div");
        k.set_attribute(
            "style",
            &format!("position: relative; left: 150px; z-index: 5; {RED}"),
        );
        p.append_child(&k);
        let sib = scope.create_element("div");
        sib.set_attribute(
            "style",
            &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
        );
        c.append_child(&sib);
        *slot_in.borrow_mut() = Some(p);
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let p = slot.borrow().clone().unwrap();
    let _ = full_frame(&mut app);
    let start = |app: &RinchApp| {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.active_transitions[&p.node_id().0][&TransitionProperty::Opacity].start_time_ms
    };
    let tick = |app: &mut RinchApp, ms: f64| {
        {
            let doc = app.doc.as_ref().unwrap().clone();
            let mut d = doc.borrow_mut();
            rinch_dom::transition::tick_transitions(&mut d.tree, ms);
        }
        app.scene_dirty = true;
        resolve(app);
    };
    let check = |app: &mut RinchApp, what: &str| {
        let (inc, stats) = incremental_frame(app);
        // A second app would be the clean oracle; a full frame of this one is
        // the PR's own.
        let full = full_frame(app);
        eprintln!(
            "PROBE T {what}: stale={} full={}",
            diff(&inc, &full),
            stats.get(Counter::RepaintFull)
        );
        assert_eq!(diff(&inc, &full), 0, "{what}");
    };
    p.set_attribute("class", "p on");
    resolve(&mut app);
    let s0 = start(&app);
    tick(&mut app, s0 + 400.0);
    check(&mut app, "mid-run in");
    p.set_attribute("class", "p");
    resolve(&mut app);
    let s1 = start(&app);
    tick(&mut app, s1 + 1.0);
    check(&mut app, "just reversed");
    tick(&mut app, s1 + 5000.0);
    check(&mut app, "reversal finished at 1");
    p.set_attribute("class", "p on");
    resolve(&mut app);
    let s2 = start(&app);
    tick(&mut app, s2);
    check(&mut app, "restart, first frame at 1");
    tick(&mut app, s2 + 5000.0);
    check(&mut app, "finished at 0.5");
}

/// H (seed 2477, minimised): an absolute box that escapes a static clipper
/// (its containing block is above it) holds an absolute child. Giving the
/// outer box an identity transform is a context-only flip with no `z`
/// descendant — and it changes whether the child is clipped by the clipper.
#[test]
fn h_a_context_only_flip_changes_an_absolute_grandchilds_clip() {
    for (from, to) in [
        ("transform: translateX(0);", ""),
        ("", "transform: translateX(0);"),
    ] {
        let p = "position: absolute; left: 150px; top: 50px; width: 80px; height: 80px; background: rgb(0,0,200);";
        let spec = vec![
            s(None, "position: relative; width: 500px; height: 300px"),
            s(
                Some(0),
                "width: 80px; height: 100px; overflow: hidden; background: rgb(0,160,0)",
            ),
            s(Some(1), &format!("{p} {from}")),
            s(
                Some(2),
                &format!("position: absolute; left: 110px; top: 40px; {RED}"),
            ),
        ];
        let o = probe(&spec, 2, &format!("{p} {to}"));
        report(&format!("H abs-in-abs {from:?}->{to:?}"), &o);
        assert_eq!(o.stale, 0, "changed={}", o.changed);
    }
}

// ── What the damage of a flip is clipped by (round 1 of the #1399 review) ──

/// `clipper(100x100, overflow: hidden) > p > …` under a positioned holder,
/// with a blue `z-index: 1` sibling at +150px. `inner` is what sits under
/// `p`; returns the spec and `p`'s index.
fn clipped_scene(p_style: &str, inner: &[(usize, &str)]) -> (Spec, usize) {
    let mut spec = vec![
        s(None, "position: relative; width: 300px; height: 100px"),
        s(Some(0), "width: 100px; height: 100px; overflow: hidden"),
        s(Some(1), p_style),
    ];
    for (parent, style) in inner {
        spec.push(s(Some(*parent), style));
    }
    spec.push(s(
        Some(0),
        &format!("position: absolute; left: 150px; top: 0; z-index: 1; {BLUE}"),
    ));
    (spec, 2)
}

/// The escaping absolute is two levels under the flipping box, through a
/// static wrapper: the escape walk has to recurse.
#[test]
fn an_escaping_absolute_under_a_wrapper_is_repainted() {
    let p = "width: 100px; height: 100px;";
    let k = format!("position: absolute; left: 150px; top: 0; z-index: 5; {RED}");
    for (from, to) in [("", "opacity: 0.5;"), ("opacity: 0.5;", "")] {
        let (spec, t) = clipped_scene(
            &format!("{p} {from}"),
            &[(2, "width: 100px; height: 100px"), (3, &k)],
        );
        let o = probe(&spec, t, &format!("{p} {to}"));
        assert!(o.changed > 9000, "positive control {}", o.changed);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0);
    }
}

/// The absolute under the flipping box is contained by a `relative` box
/// between the two, so it escapes nothing: the clipper cuts it, and cuts the
/// damage. Exactly the clipper's 100 x 100 (its left and top edges are the
/// window's; the damage margin is clipped away on the other two).
#[test]
fn a_contained_absolute_keeps_the_damage_inside_the_clipper() {
    let p = "width: 100px; height: 100px;";
    let k = format!("position: absolute; left: 150px; top: 0; z-index: 5; {RED}");
    let (spec, t) = clipped_scene(
        p,
        &[
            (2, "position: relative; width: 100px; height: 100px"),
            (3, &k),
        ],
    );
    let o = probe(&spec, t, &format!("{p} opacity: 0.5"));
    assert_eq!(o.stale, 0);
    assert_eq!(o.full, 0);
    assert_eq!(o.repainted, 100 * 100);
}

/// The flipping box becomes the containing block of an absolute child that
/// used to escape the clipper above it: the child's old pixels are outside
/// the clipper, so the rect it was painted in is not cut by it — and the
/// reverse.
#[test]
fn a_box_that_starts_containing_an_escaping_absolute_clears_where_it_was() {
    let p = "width: 100px; height: 100px;";
    let k = format!("position: absolute; left: 150px; top: 0; z-index: 5; {RED}");
    for (from, to) in [("", "position: relative;"), ("position: relative;", "")] {
        let (spec, t) = clipped_scene(&format!("{p} {from}"), &[(2, &k)]);
        let o = probe(&spec, t, &format!("{p} {to}"));
        report(&format!("starts containing {from:?}->{to:?}"), &o);
        assert!(o.changed > 9000, "positive control {}", o.changed);
        assert_eq!(o.full, 0);
        assert_eq!(o.stale, 0);
    }
}
