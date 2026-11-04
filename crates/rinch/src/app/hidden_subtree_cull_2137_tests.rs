//! A closed `Drawer` is skipped by paint and the stacking collector (#2137),
//! and what a running app paints is unchanged by it.
//!
//! The rinch-dom half (`crates/rinch-dom/tests/hidden_subtree_cull_2137_tests.rs`)
//! pins the documents. This one drives the real component through an open and
//! a close, with its transitions and the desktop loop's partial frames: every
//! incremental frame must hold exactly what a frame painted from scratch holds,
//! and every frame from scratch must be what a paint that walks the hidden
//! subtree paints (`rinch_dom::visibility::set_hidden_subtree_cull`).

use super::perf_expect::*;
use super::*;
use rinch_components::Drawer;
use rinch_core::{Component, Signal};
use rinch_dom::perf::Counter;

/// The page: text rows across the window, under a left `Drawer` whose
/// content is 40 rows, each with a translucent badge.
fn mount(open: Signal<bool>) -> RinchApp {
    mount_with(open, false)
}

/// [`mount`], with each row's badge `in_flow` (a static block after the row's
/// text) rather than absolute: then the text is bare text beside a block, held
/// by an anonymous block box that copies the drawer's `visibility` when layout
/// builds it.
fn mount_with(open: Signal<bool>, in_flow: bool) -> RinchApp {
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = format!(
            "{} body {{ font-family: sans-serif; font-size: 14px; line-height: 20px; }} \
             .page-row {{ height: 20px; background: rgb(220, 230, 250); \
                          border-bottom: 1px solid rgb(120, 120, 160); }} \
             .item {{ position: relative; height: 12px; line-height: 12px; }} \
             .badge {{ position: absolute; right: 4px; top: 2px; width: 8px; height: 8px; \
                       opacity: 0.5; background: rgb(200, 0, 0); }} {}",
            rinch_components::generate_component_css(),
            if in_flow {
                ".item { height: auto; } .badge { position: static; height: 4px; }"
            } else {
                ""
            }
        );
        let text = scope.create_text(&css);
        style.append_child(&text);
        root.append_child(&style);
        for i in 0..20 {
            let row = scope.create_element("div");
            row.set_attribute("class", "page-row");
            let t = scope.create_text(&format!("page row {i}"));
            row.append_child(&t);
            root.append_child(&row);
        }
        let content = scope.create_element("div");
        for i in 0..40 {
            let item = scope.create_element("div");
            item.set_attribute("class", "item");
            let t = scope.create_text(&format!("drawer item {i}"));
            item.append_child(&t);
            let badge = scope.create_element("div");
            badge.set_attribute("class", "badge");
            item.append_child(&badge);
            content.append_child(&item);
        }
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || open.get())),
            position: "left".to_string(),
            size: "xs".to_string(),
            // No full-window backdrop, so the slide damages only the panel
            // and the loop paints partial frames.
            with_overlay: false,
            ..Default::default()
        }
        .render(scope, &[content]);
        root.append_child(&drawer);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    app
}

/// A from-scratch frame of the current state, with or without the skip.
fn full_frame(app: &mut RinchApp, cull: bool) -> Vec<u8> {
    rinch_dom::visibility::set_hidden_subtree_cull(cull);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    rinch_dom::visibility::set_hidden_subtree_cull(true);
    px
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p != q).count()
}

/// Turns of the loop for at least `ms` and until a turn asks for nothing (the
/// transitions are over), each painting what it asked for; after each, the
/// incremental frame is compared with a frame from scratch, and that with a
/// from-scratch frame that walks hidden subtrees. Returns how many turns
/// painted, and how many of those partially.
fn run_for(app: &mut RinchApp, ms: u64, what: &str) -> (usize, usize) {
    let (mut frames, mut partial) = (0, 0);
    let start = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(15));
        if !about_to_wait(app) && !app.scene_dirty {
            if start.elapsed().as_millis() >= ms as u128 {
                break;
            }
            continue;
        }
        if app.has_pending_layout() {
            app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
        }
        let incremental = app.build_pixels(1.0, SIZE, false).0.to_vec();
        let stats = app.end_perf_frame().expect("mounted");
        frames += 1;
        if stats.get(Counter::RepaintPartial) == 1 {
            partial += 1;
        }

        let full = full_frame(app, true);
        let walked = full_frame(app, false);
        assert_eq!(
            differing(&incremental, &full),
            0,
            "{what}: an incremental frame differs from a full one"
        );
        assert_eq!(
            differing(&full, &walked),
            0,
            "{what}: the skip changed the picture"
        );
    }
    (frames, partial)
}

/// Open, then close: through the slide-in, the 300ms close the root is held
/// `visible` for, and the frames after it hides, every frame the loop paints
/// is the picture a from-scratch paint gives, with the skip and without it. A
/// closed drawer still repaints the area it leaves (its rows, its panel).
#[test]
fn a_drawer_opening_and_closing_paints_what_a_full_frame_paints() {
    let open = Signal::new(false);
    let mut app = mount(open);
    let closed = full_frame(&mut app, true);
    assert_eq!(differing(&closed, &full_frame(&mut app, false)), 0);

    open.set(true);
    let (_, partial) = run_for(&mut app, 450, "opening");
    assert!(
        partial > 0,
        "positive control: the slide-in painted partial frames"
    );
    let opened = full_frame(&mut app, true);
    assert!(
        differing(&closed, &opened) > 10_000,
        "positive control: the open drawer shows"
    );

    open.set(false);
    // The slide-out's frames repaint in full here (its damage passes the
    // full-repaint threshold), so what they pin is the picture through the
    // held-visible close and on the frame that hides it.
    let (frames, _) = run_for(&mut app, 550, "closing");
    assert!(
        frames >= 2,
        "positive control: the close painted {frames} frames"
    );
    let after = full_frame(&mut app, true);
    assert_eq!(differing(&after, &closed), 0, "closed again, as it began");
}

/// The skip's whole saving, on the full frame: a closed drawer of 40 rows and
/// 40 translucent badges is entered nowhere — the same visits, layers and
/// stacking sequences as the page without one — and walking it is the cost
/// the skip removes. (`perf_regression_tests` pins the whole frame.)
#[test]
fn a_closed_drawer_costs_a_full_frame_nothing() {
    let frame = |cull: bool| {
        let mut app = mount(Signal::new(false));
        rinch_dom::visibility::set_hidden_subtree_cull(cull);
        app.mark_scene_dirty();
        let s = paint(&mut app);
        rinch_dom::visibility::set_hidden_subtree_cull(true);
        s
    };
    let (on, off) = (frame(true), frame(false));
    for c in [
        Counter::PaintNodesVisited,
        Counter::PaintLayers,
        Counter::StackingOrderBuilds,
    ] {
        assert!(
            on.get(c) < off.get(c),
            "{}: {} skipped against {} walked",
            c.name(),
            on.get(c),
            off.get(c)
        );
    }
    assert_eq!(
        on.get(Counter::PaintLayers),
        0,
        "no layer for a closed drawer"
    );
}

/// A small popover-like panel toggled straight between `visible` and
/// `hidden` — and closed through a held `visibility` transition, as `Popover`
/// does — repaints its own area in **partial** frames: the frame that hides it
/// clears the panel, the one that shows it draws it, and each is the picture
/// a from-scratch frame paints, with the skip and without it.
#[test]
fn toggling_a_small_panel_repaints_its_area_partially() {
    let open = Signal::new(true);
    let mut app = mount_settled(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        let style = __scope.create_element("style");
        let css = __scope.create_text(
            "body { font-family: sans-serif; font-size: 12px; line-height: 14px; } \
             .bg { width: 800px; height: 600px; background: rgb(240, 240, 200); } \
             .pop { position: absolute; left: 100px; top: 80px; width: 120px; \
                    background: rgb(40, 40, 120); color: white; } \
             .pop.shut { visibility: hidden; transition: visibility 0s linear 120ms; } \
             .pop-row { position: relative; height: 14px; } \
             .dot { position: absolute; right: 2px; top: 3px; width: 6px; height: 6px; \
                    opacity: 0.5; background: rgb(255, 200, 0); }",
        );
        style.append_child(&css);
        root.append_child(&style);
        let bg = __scope.create_element("div");
        bg.set_attribute("class", "bg");
        root.append_child(&bg);
        let pop = __scope.create_element("div");
        for i in 0..8 {
            let row = __scope.create_element("div");
            row.set_attribute("class", "pop-row");
            let t = __scope.create_text(&format!("item {i}"));
            row.append_child(&t);
            let dot = __scope.create_element("div");
            dot.set_attribute("class", "dot");
            row.append_child(&dot);
            pop.append_child(&row);
        }
        let handle = pop.clone();
        __scope.create_effect(move || {
            handle.set_attribute("class", if open.get() { "pop" } else { "pop shut" });
        });
        root.append_child(&pop);
        root
    });
    let shown = full_frame(&mut app, true);
    for (state, ms) in [(false, 300), (true, 100), (false, 300)] {
        open.set(state);
        let (frames, partial) = run_for(&mut app, ms, if state { "show" } else { "hide" });
        assert!(
            frames > 0 && partial == frames,
            "positive control: {frames} frames, {partial} of them partial"
        );
        let now = full_frame(&mut app, true);
        if state {
            assert_eq!(differing(&now, &shown), 0, "shown again");
        } else {
            assert!(differing(&now, &shown) > 5000, "hidden: the panel is gone");
        }
    }
}

/// A `Drawer` built closed whose rows hold bare text beside an in-flow block,
/// then opened: the rows' text is in anonymous block boxes built while the
/// drawer was hidden, and opening is paint-only, so their copied `visibility`
/// stays `hidden` (review of #2137). Every label shows once it is open, as a
/// paint that walks the subtree shows it — and is hit there.
#[test]
fn a_drawer_built_closed_shows_its_bare_text_when_it_opens() {
    let open = Signal::new(false);
    let mut app = mount_with(open, true);
    let closed = full_frame(&mut app, true);
    open.set(true);
    run_for(&mut app, 450, "opening");
    let opened = full_frame(&mut app, true);
    let walked = full_frame(&mut app, false);
    assert!(
        differing(&closed, &walked) > 10_000,
        "positive control: the open drawer shows"
    );
    assert_eq!(differing(&opened, &walked), 0, "every label is painted");

    // Hit testing reads the same answer: the first row's text is hit.
    let d = app.doc.as_ref().unwrap().borrow();
    let tree = &d.tree;
    let anon = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.is_anonymous_block_box && n.text_layout.is_some())
        .map(|(id, _)| id)
        .find(|&id| {
            let r = rinch_dom::paint::painted_border_box(tree, id, 1.0);
            r.x0 >= 0.0 && r.y0 >= 0.0 && r.width() > 20.0
        })
        .expect("positive control: the rows' text is in anonymous boxes");
    let r = rinch_dom::paint::painted_border_box(tree, anon, 1.0);
    let hit =
        super::hit_testing::hit_test(tree, (r.x0 + 5.0) as f32, (r.y0 + r.height() / 2.0) as f32);
    assert!(
        hit.is_some_and(|h| h == anon || tree.get(anon).and_then(|n| n.parent) == Some(h)),
        "the opened row's text is hit: {hit:?} for box {anon}"
    );
}
