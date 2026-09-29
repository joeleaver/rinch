//! #468 at the component level: the striped, animated `Progress` bar's
//! stripes move, its fill colour is painted under them, and the animated
//! `Skeleton` pulses — with the component library's own CSS, through the
//! software frame an app paints.
//!
//! `crates/rinch-dom/tests/background_position_tests.rs` is the mechanism's
//! oracle (Chrome 153); this file is the proof that the two shipped
//! `@keyframes` blocks the issue names now reach pixels. Before #468 each
//! animation was registered and "running" and every frame was identical.
//!
//! The time is driven by `rinch_dom::animation::tick_animations` with an
//! explicit clock, from the animation's own start, so no sample depends on
//! the wall clock.

use super::*;

use rinch_components::{Progress, Skeleton};
use rinch_core::Component;

const VP: (u32, u32) = (400, 100);

/// Fixed colours for the theme variables the two components read, so that
/// the sampled pixels do not depend on the theme's palette.
const THEME: &str = "
    :root {
        --rinch-color-default: rgb(200, 200, 200);
        --rinch-color-filled: rgb(40, 40, 40);
        --rinch-primary-color: rgb(0, 0, 200);
        --rinch-radius-default: 0px;
    }
    html, body { margin: 0; }
";

fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 320px; padding: 8px");
        let child = build(scope);
        root.append_child(&child);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.load_css(THEME);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
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

/// The node's border box in window px (static flow: the sum of the
/// parent-relative offsets).
fn window_rect(app: &RinchApp, node: usize) -> (u32, u32, u32, u32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let l = d.tree.nodes[node].layout;
    let (mut x, mut y) = (l.x, l.y);
    let mut p = d.tree.nodes[node].parent;
    while let Some(id) = p {
        x += d.tree.nodes[id].layout.x;
        y += d.tree.nodes[id].layout.y;
        p = d.tree.nodes[id].parent;
    }
    (x as u32, y as u32, l.width as u32, l.height as u32)
}

/// Advance the node's animation to `ms` after its start, and paint a full frame.
fn frame_at(app: &mut RinchApp, node: usize, ms: f64) -> Vec<u8> {
    {
        let doc = app.doc.as_ref().unwrap().clone();
        let mut d = doc.borrow_mut();
        let start = d.tree.active_animations[&node][0].start_time_ms;
        rinch_dom::animation::tick_animations(&mut d.tree, start + ms);
    }
    app.scene_dirty = true;
    app.has_previous_frame = false;
    app.build_pixels(1.0, VP, true).0.to_vec()
}

fn px(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * VP.0 + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
}

/// `rinch-progress-stripes` runs `background-position` from `1rem 0` to `0 0`
/// over 1s on a 16px tile, so a quarter of the way the stripes have moved
/// 4px left: the later frame at `x` is the first frame at `x + 4`.
#[test]
fn an_animated_striped_progress_bar_moves_its_stripes() {
    let mut app = mount(|scope| {
        Progress {
            value: Some(100.0),
            size: "xl".to_string(),
            striped: true,
            animated: true,
            ..Default::default()
        }
        .render(scope, &[])
    });
    let bar = class_node(&app, "rinch-progress__bar");
    let (x0, y0, w, h) = window_rect(&app, bar);
    assert!(w > 100 && h >= 16, "the bar is laid out: {w}x{h}");
    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert_eq!(d.tree.active_animations.get(&bar).map(Vec::len), Some(1));
    }

    let first = frame_at(&mut app, bar, 0.0);
    let later = frame_at(&mut app, bar, 250.0);
    let y = y0 + h / 2;
    let xs = (x0 + 8)..(x0 + w - 16);

    let moved = xs
        .clone()
        .filter(|&x| px(&first, x, y) != px(&later, x, y))
        .count();
    assert!(
        moved > 20,
        "the stripes moved: {moved} pixels of the row changed"
    );

    // Which horizontal shift explains the later frame? The stripes are 45deg
    // hard stops, so the pixels on a stop line are anti-aliased by float
    // noise either way and no shift matches exactly; the right one matches
    // best, by a wide margin.
    let mismatches = |shift: u32| {
        xs.clone()
            .filter(|&x| px(&later, x, y) != px(&first, x + shift, y))
            .count()
    };
    let by_shift: Vec<usize> = (0..16).map(mismatches).collect();
    let best = (0..16).min_by_key(|&s| by_shift[s as usize]).unwrap();
    assert_eq!(
        best, 4,
        "a quarter of 1rem is a 4px shift left: {by_shift:?}"
    );
    assert!(
        by_shift[4] * 4 < by_shift[0],
        "and it explains the frame far better than no motion: {by_shift:?}"
    );

    // The bar's own colour is under the translucent white stripes: every
    // pixel is mostly blue, none is the grey track.
    for x in xs {
        let [r, g, b, _] = px(&first, x, y);
        assert!(
            b > 150 && r < 80 && g < 80,
            "({x}, {y}) is the fill: {r},{g},{b}"
        );
    }
}

/// `rinch-skeleton-pulse` runs `background-position` from `200% 0` to
/// `-200% 0` on a 200%-wide tile, a gradient default → filled → default.
/// Mid-pulse the dark centre of the gradient is somewhere on the box; it
/// moves between two samples.
#[test]
fn an_animated_skeleton_pulses() {
    let mut app = mount(|scope| {
        Skeleton {
            width: "300px".to_string(),
            height: "20px".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let sk = class_node(&app, "rinch-skeleton");
    let (x0, y0, w, h) = window_rect(&app, sk);
    let y = y0 + h / 2;

    // At 0.125 of the (ease-in-out) cycle and at 0.375 the offset differs,
    // so the row's darkest column must move.
    let darkest = |frame: &[u8]| {
        (x0..x0 + w)
            .min_by_key(|&x| px(frame, x, y)[0] as u32)
            .expect("a non-empty row")
    };
    let a = frame_at(&mut app, sk, 1500.0 * 0.125);
    let b = frame_at(&mut app, sk, 1500.0 * 0.375);
    let (da, db) = (darkest(&a), darkest(&b));
    assert_ne!(da, db, "the pulse's dark band moved between samples");
    assert!(
        (x0..x0 + w).any(|x| px(&a, x, y) != px(&b, x, y)),
        "two samples of a running pulse paint differently"
    );
}
