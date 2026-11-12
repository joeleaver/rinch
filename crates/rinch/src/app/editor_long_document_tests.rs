//! A long document in an editor that scrolls inside an ancestor.
//!
//! The shape is an app's note pane: a scroller (`overflow-y: auto`) whose
//! child is the editor, which never sets its own `overflow-y` and grows with
//! its content. Documents are plain paragraphs of about sixty words with a
//! heading every twenty, loaded into an editor that is already mounted (an
//! app's editor outlives the notes it shows).
//!
//! `long_document_timings` prints the open, keystroke and scroll timings at
//! 500 and 3,000 paragraphs (`cargo test --release -p rinch --lib
//! long_document_timings -- --ignored --nocapture`).
#![cfg(all(software_shell, feature = "desktop"))]
use super::perf_expect::*;
use super::*;
use rinch_dom::perf::Counter::{self, *};
use rinch_dom::perf::FrameStats;
use rinch_editor_core::{Pos, Selection};

const WORDS: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod \
    tempor incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud \
    exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat duis aute irure dolor \
    in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur excepteur \
    sint occaecat cupidatat non proident sunt in culpa qui officia deserunt mollit anim";

/// `paragraphs` paragraphs of about sixty words, a heading before every
/// twentieth.
fn long_html(paragraphs: usize) -> String {
    let mut html = String::new();
    for i in 0..paragraphs {
        if i % 20 == 0 {
            html.push_str(&format!("<h2>Section {}</h2>", i / 20));
        }
        html.push_str(&format!("<p>{i} {WORDS}</p>"));
    }
    html
}

pub(super) struct Pane {
    pub app: RinchApp,
    pub handle: crate::editor::EditorHandle,
    pub scroller: u64,
}

/// An 800x600 window: a scroller filling it, holding an empty editor that
/// does not scroll itself. Settled, counters zeroed.
pub(super) fn pane() -> Pane {
    let slot: Rc<RefCell<Option<(crate::editor::EditorHandle, u64)>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; display: flex");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "flex: 1; overflow-y: auto; display: flex; flex-direction: column; \
             position: relative; z-index: 0",
        );
        let (container, handle) = crate::editor::mount_editor(scope);
        container.set_attribute(
            "style",
            "flex: 1 1 auto; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        scroller.append_child(&container);
        root.append_child(&scroller);
        *slot_in.borrow_mut() = Some((handle, scroller.node_id().0));
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    let (handle, scroller) = slot.borrow_mut().take().expect("captured at mount");
    Pane {
        app,
        handle,
        scroller,
    }
}

/// Load a long document into the mounted editor; the frame that shows it.
// Not inlined: a profiler attributes each frame kind to its own function.
#[inline(never)]
pub(super) fn open(pane: &mut Pane, paragraphs: usize) -> FrameStats {
    assert!(pane.handle.load_html(&long_html(paragraphs)));
    about_to_wait(&mut pane.app);
    paint(&mut pane.app)
}

pub(super) fn scroll_of(pane: &Pane) -> f64 {
    pane.app.doc.as_ref().unwrap().borrow().tree.nodes[pane.scroller]
        .scroll_offset
        .1
}

// Not inlined: a profiler attributes each frame kind to its own function.
#[inline(never)]
pub(super) fn wheel(pane: &mut Pane, dy: f64) -> FrameStats {
    interaction(&mut pane.app, |app| {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x: 400.0,
                y: 300.0,
                delta_x: 0.0,
                delta_y: dy,
            },
            SIZE,
            1.0,
        );
    })
}

// Not inlined: a profiler attributes each frame kind to its own function.
#[inline(never)]
pub(super) fn type_x(pane: &mut Pane) -> FrameStats {
    interaction(&mut pane.app, |app| {
        app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::KeyX,
                logical_key: None,
                text: Some("x".into()),
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Fresh,
            },
            SIZE,
            1.0,
        );
        app.handle_event(
            PlatformEvent::KeyUp {
                key: KeyCode::KeyX,
                logical_key: None,
                modifiers: Modifiers::default(),
            },
            SIZE,
            1.0,
        );
    })
}

/// Focus the editor with a press in its first paragraph's text.
pub(super) fn focus(pane: &mut Pane) {
    let (x, y, h) = pane
        .app
        .editor_caret_point(&pane.handle, Pos(12))
        .expect("the first block has a caret");
    interaction(&mut pane.app, |app| {
        let button = MouseButton::Left;
        app.handle_event(
            PlatformEvent::MouseDown {
                x: x + 1.0,
                y: y + h / 2.0,
                button,
            },
            SIZE,
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseUp {
                x: x + 1.0,
                y: y + h / 2.0,
                button,
            },
            SIZE,
            1.0,
        );
    });
    assert!(
        matches!(pane.app.focus_target, FocusTarget::Editor(_)),
        "precondition: the editor holds the keyboard"
    );
    settle(&mut pane.app);
}

fn ms(s: &FrameStats, c: Counter) -> f64 {
    s.get(c) as f64 / 1e6
}

fn line(label: &str, s: &FrameStats) -> String {
    format!(
        "{label:<28} style {:>7.2} ms  layout {:>7.2} ms  paint {:>6.2} ms  \
         measures {:>5} shapes {:>5} paint-visits {:>6} [setup {:.2} taffy {:.2} build {:.2}]",
        ms(s, TimeStyleNs),
        ms(s, TimeLayoutNs),
        ms(s, TimePaintNs),
        s.get(TaffyMeasureCalls),
        s.get(ShapeMeasureIfc) + s.get(ShapeIfcBuild),
        s.get(PaintNodesVisited),
        ms(s, TimeIfcSetupNs),
        ms(s, TimeTaffyComputeNs),
        ms(s, TimeBuildIfcNs),
    )
}

/// The best of `n` runs of `f`, by layout and paint time (the machine these
/// run on is shared; the best run is the one least disturbed).
fn best(n: usize, mut f: impl FnMut() -> FrameStats) -> FrameStats {
    (0..n)
        .map(|_| f())
        .min_by_key(|s| s.get(TimeLayoutNs) + s.get(TimePaintNs))
        .expect("n > 0")
}

#[test]
#[ignore = "timings: run in release with --ignored --nocapture"]
fn long_document_timings() {
    let mut out = Vec::new();
    // `LONG_DOC_SIZES=3000` runs one size (for a profiler).
    let sizes: Vec<usize> = std::env::var("LONG_DOC_SIZES").map_or(vec![500, 3000], |v| {
        v.split(',')
            .map(|n| n.trim().parse().expect("a number"))
            .collect()
    });
    for paragraphs in sizes {
        let mut pane = pane();
        let t = std::time::Instant::now();
        let opened = open(&mut pane, paragraphs);
        let wall = t.elapsed().as_secs_f64() * 1e3;
        out.push(format!("{paragraphs} paragraphs, open ({wall:.0} ms wall)"));
        out.push(line("  open frame", &opened));
        settle(&mut pane.app);
        focus(&mut pane);
        out.push(line("  keystroke at start", &best(9, || type_x(&mut pane))));
        // Scrolling: 60 wheel steps of 120 px, every frame counted.
        let mut total = FrameStats::default();
        let mut worst = FrameStats::default();
        for _ in 0..60 {
            let s = wheel(&mut pane, -120.0);
            if s.get(TimeLayoutNs) + s.get(TimePaintNs)
                > worst.get(TimeLayoutNs) + worst.get(TimePaintNs)
            {
                worst = s;
            }
            total.accumulate(&s);
        }
        let mean = |c| total.get(c) as f64 / 60.0 / 1e6;
        out.push(format!(
            "  scroll, mean of 60 frames    layout {:>7.2} ms  paint {:>6.2} ms  ({} frames laid out)",
            mean(TimeLayoutNs),
            mean(TimePaintNs),
            total.get(TaffyRootComputes),
        ));
        out.push(line("  scroll, worst frame", &worst));
        let end = Pos(pane.handle.doc().content_size() - 2);
        pane.handle.set_selection(Selection::cursor(end));
        pane.handle.scroll_into_view(end, end);
        for _ in 0..8 {
            about_to_wait(&mut pane.app);
            paint(&mut pane.app);
        }
        settle(&mut pane.app);
        out.push(line("  keystroke at end", &best(9, || type_x(&mut pane))));
    }
    eprintln!("\n{}", out.join("\n"));
}

/// Paragraphs Parley shaped (measured or built) in a frame.
fn shapes(s: &FrameStats) -> u64 {
    s.get(ShapeMeasureIfc) + s.get(ShapeIfcBuild)
}

/// The editor's block elements that are laid out (not collapsed).
fn laid_out_blocks(pane: &Pane) -> usize {
    let doc = pane.app.doc.as_ref().unwrap().borrow();
    let editor = doc.tree.nodes[pane.scroller].children[0];
    doc.tree.nodes[editor]
        .children
        .iter()
        .filter(|&&id| {
            let n = &doc.tree.nodes[id];
            n.attributes.contains_key("data-pm-type") && n.estimated_height.is_none()
        })
        .count()
}

/// An editor that grows inside a scrolling pane is virtualized against the
/// pane (#2624): opening a long document shapes the blocks near the view and
/// collapses the rest, and the work is the same at 500 and 3,000 paragraphs.
/// Before, only an editor that scrolled itself was virtualized; this one
/// shaped every block twice on open (1,050 at 500 paragraphs, 6,300 at
/// 3,000).
#[test]
fn opening_a_long_document_shapes_only_what_is_near_the_view() {
    let mut seen = Vec::new();
    for paragraphs in [500, 3000] {
        let mut pane = pane();
        let s = open(&mut pane, paragraphs);
        let laid_out = laid_out_blocks(&pane);
        assert!(
            laid_out < 60,
            "{paragraphs} paragraphs: {laid_out} blocks laid out; only those near the view should be"
        );
        assert!(
            scroll_of(&pane) == 0.0
                && pane.app.doc.as_ref().unwrap().borrow().tree.nodes[pane.scroller]
                    .layout
                    .height
                    > 0.0,
            "control: the pane is at the top"
        );
        seen.push((paragraphs, shapes(&s), laid_out));
    }
    assert!(
        seen[0].1 > 0,
        "positive control: the open shaped the blocks it shows"
    );
    assert_eq!(
        (seen[0].1, seen[0].2),
        (seen[1].1, seen[1].2),
        "opening shapes and lays out the same blocks whatever the length: {seen:?}"
    );
}

/// A keystroke near the top and one at the end cost the same at 500 and
/// 3,000 paragraphs, in every counter that names work per node: what is
/// shaped, measured, cascaded and painted. (Taffy's own pass over the
/// editor's children is not counted, and still grows with the length.)
///
/// `paint_nodes_visited` is the one virtualizing in a pane would have
/// broken: paint's cull did not answer for a collapsed block, so each was
/// entered every frame (3,141 at 3,000 paragraphs, against 6).
#[test]
fn a_keystroke_costs_the_same_at_500_and_3000_paragraphs() {
    let counters = [
        ShapeMeasureIfc,
        ShapeIfcBuild,
        TaffyMeasureCalls,
        ElementsCascaded,
        PaintNodesVisited,
        TaffyRootComputes,
    ];
    let mut seen: Vec<Vec<u64>> = Vec::new();
    for paragraphs in [500, 3000] {
        let mut pane = pane();
        open(&mut pane, paragraphs);
        settle(&mut pane.app);
        focus(&mut pane);
        let before = pane.handle.doc().content_size();
        let mut row = Vec::new();
        for _ in 0..3 {
            let s = type_x(&mut pane);
            row.extend(counters.iter().map(|&c| s.get(c)));
        }
        assert_eq!(
            pane.handle.doc().content_size(),
            before + 3,
            "positive control"
        );
        let end = Pos(pane.handle.doc().content_size() - 2);
        pane.handle.set_selection(Selection::cursor(end));
        pane.handle.scroll_into_view(end, end);
        for _ in 0..8 {
            about_to_wait(&mut pane.app);
            paint(&mut pane.app);
        }
        settle(&mut pane.app);
        assert!(scroll_of(&pane) > 10_000.0, "control: scrolled to the end");
        let s = type_x(&mut pane);
        row.extend(counters.iter().map(|&c| s.get(c)));
        assert_eq!(
            pane.handle.doc().content_size(),
            before + 4,
            "positive control"
        );
        seen.push(row);
    }
    assert_eq!(
        seen[0], seen[1],
        "per-node work of a keystroke at 500 and 3,000 paragraphs ({counters:?}, four keystrokes)"
    );
    let painted = counters
        .iter()
        .position(|&c| c == PaintNodesVisited)
        .unwrap();
    assert!(
        seen[1][painted] < 20,
        "paint enters a handful of nodes, not every block"
    );
}

/// Every block on screen, and `margin` blocks beyond each edge of the view,
/// is laid out: the window moves while it still has that many to spare
/// (`KEEP`), so a block scrolled into view was laid out before it got there.
fn assert_margin_laid_out(pane: &Pane, margin: usize) {
    let doc = pane.app.doc.as_ref().unwrap().borrow();
    let scroller = &doc.tree.nodes[pane.scroller];
    let editor_id = scroller.children[0];
    let editor = &doc.tree.nodes[editor_id];
    let top = scroller.scroll_offset.1 as f32 - editor.layout.y;
    let bottom = top + scroller.layout.height;
    let blocks: Vec<&rinch_dom::Node> = editor
        .children
        .iter()
        .map(|&id| &doc.tree.nodes[id])
        .filter(|n| n.attributes.contains_key("data-pm-type"))
        .collect();
    let first = blocks
        .iter()
        .position(|n| n.layout.y + n.layout.height > top)
        .expect("a block in view");
    let last = blocks
        .iter()
        .rposition(|n| n.layout.y < bottom)
        .expect("a block in view");
    let from = first.saturating_sub(margin);
    let to = (last + margin).min(blocks.len() - 1);
    for (i, n) in blocks.iter().enumerate().take(to + 1).skip(from) {
        assert_eq!(
            n.estimated_height, None,
            "block {i} is laid out (view shows {first}..={last}, margin {margin})"
        );
    }
}

/// Scrolling lays the editor out only when the window of laid-out blocks
/// moves, and it moves once every several blocks scrolled past, not on every
/// one: 200 px steps through 16,000 px of paragraphs of about 130 px each.
/// The same at 500 and 3,000 paragraphs.
#[test]
fn scrolling_lays_out_only_when_the_window_moves() {
    let mut seen = Vec::new();
    for paragraphs in [500, 3000] {
        let mut pane = pane();
        open(&mut pane, paragraphs);
        settle(&mut pane.app);
        let (mut layouts, mut shaped) = (0, 0);
        for _ in 0..80 {
            let s = wheel(&mut pane, -200.0);
            assert_margin_laid_out(&pane, 3);
            if s.get(TaffyRootComputes) > 0 {
                layouts += 1;
            }
            shaped += shapes(&s);
        }
        assert!(
            (15990.0..=16010.0).contains(&scroll_of(&pane)),
            "positive control: scrolled 16,000 px, at {}",
            scroll_of(&pane)
        );
        assert!(
            laid_out_blocks(&pane) < 60,
            "still only the blocks near the view"
        );
        seen.push((layouts, shaped));
    }
    eprintln!("scroll: (layouts, shaped) at 500 and 3,000 paragraphs: {seen:?}");
    assert_eq!(
        seen[0], seen[1],
        "the same work at 500 and 3,000 paragraphs"
    );
    let (layouts, shaped) = seen[0];
    assert!(
        shaped > 0,
        "positive control: blocks scrolled into view were shaped"
    );
    assert!(
        layouts <= 20,
        "{layouts} of 80 scroll frames laid out; the window should move every few blocks"
    );
}

/// A document loaded over another with as many blocks is collapsed too. The
/// window used to re-sync only when the number of blocks changed, so every
/// block of the second document — new elements — was laid out.
#[test]
fn a_document_loaded_over_one_as_long_is_collapsed_too() {
    let mut pane = pane();
    open(&mut pane, 500);
    settle(&mut pane.app);
    let first = laid_out_blocks(&pane);
    let html = long_html(500).replace("lorem", "ipsum");
    assert!(pane.handle.load_html(&html));
    about_to_wait(&mut pane.app);
    let s = paint(&mut pane.app);
    assert!(
        pane.handle.doc().content_size() > 1000,
        "positive control: the second document is loaded"
    );
    assert_eq!(
        laid_out_blocks(&pane),
        first,
        "the same blocks laid out as before"
    );
    assert!(
        shapes(&s) < 200,
        "only the blocks near the view were shaped: {}",
        shapes(&s)
    );
}

/// The `i`th block of the pane's editor: its id, estimated height and
/// laid-out `y`.
fn block(pane: &Pane, i: usize) -> (u64, Option<f32>, f32) {
    let doc = pane.app.doc.as_ref().unwrap().borrow();
    let editor = doc.tree.nodes[pane.scroller].children[0];
    let id = doc.tree.nodes[editor]
        .children
        .iter()
        .copied()
        .filter(|&id| doc.tree.nodes[id].attributes.contains_key("data-pm-type"))
        .nth(i)
        .expect("the block exists");
    let n = &doc.tree.nodes[id];
    (id, n.estimated_height, n.layout.y)
}

/// The pane stops scrolling (its `overflow-y` goes back to `visible`) while
/// it is scrolled far down: its top blocks are back on screen, and the
/// window follows the next scrollport up (`<body>`, at its top) to them.
/// #2624's review: the window kept following the pane, which no frame after
/// the restyle re-read, and left the top blocks collapsed for good.
#[test]
fn a_pane_that_stops_scrolling_hands_the_window_to_the_next_scrollport() {
    let mut pane = pane();
    open(&mut pane, 500);
    settle(&mut pane.app);
    for _ in 0..80 {
        wheel(&mut pane, -200.0);
    }
    settle(&mut pane.app);
    assert!(scroll_of(&pane) > 15_000.0, "control: scrolled");
    assert!(
        block(&pane, 0).1.is_some(),
        "control: block 0 collapsed while scrolled away"
    );
    {
        let mut doc = pane.app.doc.as_ref().unwrap().borrow_mut();
        doc.set_attribute(
            rinch_core::dom::NodeId(pane.scroller),
            "style",
            "flex: 1; display: flex; flex-direction: column; position: relative; z-index: 0",
        );
    }
    for _ in 0..4 {
        about_to_wait(&mut pane.app);
        paint(&mut pane.app);
    }
    settle(&mut pane.app);
    assert_eq!(block(&pane, 0).1, None, "block 0 is on screen and laid out");
    // `<body>` (`overflow-y: auto` in the UA sheet) is the scrollport now,
    // at its top: the window follows it there.
    assert!(laid_out_blocks(&pane) < 60, "the window follows the body");
}

/// An editor in a `position: fixed` panel inside a scrolled ancestor: the
/// ancestor's scroll does not move the panel, so the editor's top stays on
/// screen whatever the scroll. #2624's review: the window read the
/// ancestor's scroll and collapsed the block at the top of the screen.
#[test]
fn an_editor_in_a_fixed_panel_ignores_the_page_scrolling_behind_it() {
    let slot: Rc<RefCell<Option<(crate::editor::EditorHandle, u64, u64)>>> =
        Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; overflow-y: auto");
        let spacer = scope.create_element("div");
        spacer.set_attribute("style", "height: 20000px");
        root.append_child(&spacer);
        let panel = scope.create_element("div");
        panel.set_attribute(
            "style",
            "position: fixed; left: 0; top: 0; width: 800px; height: 600px; overflow: hidden",
        );
        let (container, handle) = crate::editor::mount_editor(scope);
        container.set_attribute(
            "style",
            "font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        panel.append_child(&container);
        root.append_child(&panel);
        *slot_in.borrow_mut() = Some((handle, root.node_id().0, container.node_id().0));
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    let (handle, root, editor) = slot.borrow_mut().take().expect("captured at mount");
    let html: String = (0..200)
        .map(|i| format!("<p>para {i} lorem ipsum dolor</p>"))
        .collect();
    assert!(handle.load_html(&html));
    about_to_wait(&mut app);
    paint(&mut app);
    settle(&mut app);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_top(rinch_core::dom::NodeId(root), 5000.0);
    for _ in 0..4 {
        about_to_wait(&mut app);
        paint(&mut app);
    }
    settle(&mut app);
    let doc = app.doc.as_ref().unwrap().borrow();
    assert_eq!(
        doc.tree.nodes[root].scroll_offset.1, 5000.0,
        "control: the page scrolled"
    );
    let first = doc.tree.nodes[editor]
        .children
        .iter()
        .copied()
        .find(|&id| doc.tree.nodes[id].attributes.contains_key("data-pm-type"))
        .expect("a first block");
    assert_eq!(
        doc.tree.nodes[first].estimated_height, None,
        "the panel's first block is on screen and laid out"
    );
}
