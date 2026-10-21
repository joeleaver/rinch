//! The `title` tooltip (#1646): shown after the pointer rests, hidden by what
//! the user does, placed inside the window, painted after the document.
//!
//! Time is the test's: a move sets a deadline from `Instant::now()`, and the
//! show is driven with `tick_title_tooltip` at an explicit instant on either
//! side of it, so no fixture sleeps 500 ms or races the clock.
//!
//! Every pointer is off the boxes' corners and centres, and the boxes are not
//! evenly spaced, so a placement or a hit that went wrong reads a different
//! number.

use super::perf_expect::{SIZE, expect_frame, mount_settled, paint};
use super::*;
use rinch_core::dom::NodeId;
use rinch_dom::perf::Counter;
use std::cell::Cell;
use std::time::Duration;

const VP: (f32, f32) = (SIZE.0 as f32, SIZE.1 as f32);

/// Over `#save` (title "Save file") through its untitled child span.
const ON_SAVE_CHILD: (f32, f32) = (61.0, 57.0);
/// Over `#save` itself, off its child.
const ON_SAVE: (f32, f32) = (127.0, 73.0);
/// Over `#outer` (title "Outer"), outside both of its children.
const ON_OUTER: (f32, f32) = (333.0, 211.0);
/// Over `#inner` (title "Inner"), inside `#outer`.
const ON_INNER: (f32, f32) = (317.0, 63.0);
/// Over `#blank` (`title=""`), inside `#outer`.
const ON_BLANK: (f32, f32) = (431.0, 67.0);
/// Over untitled content.
const ON_PLAIN: (f32, f32) = (607.0, 331.0);
/// Over `#corner` (a long title) near the bottom-right corner.
const ON_CORNER: (f32, f32) = (781.0, 589.0);

#[derive(Default)]
struct Ids {
    save: Cell<usize>,
    outer: Cell<usize>,
}

fn mount() -> (RinchApp, Rc<Ids>) {
    let ids = Rc::new(Ids::default());
    let i = ids.clone();
    let app = mount_settled(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 800px; height: 600px; \
             font: 16px/20px sans-serif",
        );
        let boxed = |scope: &mut RenderScope, parent: &NodeHandle, style: &str| {
            let n = scope.create_element("div");
            n.set_attribute("style", &format!("position: absolute; {style}"));
            parent.append_child(&n);
            n
        };
        let save = boxed(
            scope,
            &root,
            "left: 40px; top: 40px; width: 110px; height: 44px",
        );
        save.set_attribute("title", "Save file");
        i.save.set(save.node_id().0);
        let child = scope.create_element("span");
        child.append_child(&scope.create_text("S"));
        save.append_child(&child);

        let outer = boxed(
            scope,
            &root,
            "left: 290px; top: 30px; width: 230px; height: 210px",
        );
        outer.set_attribute("title", "Outer");
        i.outer.set(outer.node_id().0);
        let inner = boxed(
            scope,
            &outer,
            "left: 10px; top: 20px; width: 60px; height: 70px",
        );
        inner.set_attribute("title", "Inner");
        let blank = boxed(
            scope,
            &outer,
            "left: 120px; top: 25px; width: 50px; height: 60px",
        );
        blank.set_attribute("title", "");

        let corner = boxed(
            scope,
            &root,
            "left: 760px; top: 570px; width: 40px; height: 30px",
        );
        corner.set_attribute("title", "A long advisory title near the corner");
        root
    });
    (app, ids)
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

/// Turn the loop at `at`, as `AboutToWait` would at that instant.
fn tick_at(app: &mut RinchApp, at: Instant) -> bool {
    app.tick_title_tooltip(at, VP.0, VP.1)
}

/// Rest the pointer at `p` long enough for its tooltip.
fn rest_at(app: &mut RinchApp, p: (f32, f32)) {
    move_to(app, p);
    let due = app
        .title_tooltip_deadline()
        .expect("a move over a title arms the timer");
    tick_at(app, due);
}

#[test]
fn the_tooltip_shows_only_once_the_pointer_has_rested() {
    let (mut app, _) = mount();
    move_to(&mut app, ON_SAVE_CHILD);
    let due = app.title_tooltip_deadline().expect("armed");
    assert!(!tick_at(&mut app, due - Duration::from_millis(1)));
    assert_eq!(app.title_tooltip_text(), None, "one millisecond early");
    assert!(tick_at(&mut app, due));
    // The child span carries no title: its parent's applies.
    assert_eq!(app.title_tooltip_text(), Some("Save file"));
    assert_eq!(
        app.title_tooltip_deadline(),
        None,
        "nothing left to wait for"
    );
}

#[test]
fn a_move_restarts_the_rest_timer() {
    let (mut app, _) = mount();
    move_to(&mut app, ON_SAVE_CHILD);
    let first = app.title_tooltip_deadline().expect("armed");
    std::thread::sleep(Duration::from_millis(3));
    move_to(&mut app, ON_SAVE);
    let second = app.title_tooltip_deadline().expect("armed");
    assert!(second > first, "the second move restarts the wait");
    assert!(
        !tick_at(&mut app, first),
        "the first deadline no longer shows it"
    );
    assert!(tick_at(&mut app, second));
    assert_eq!(app.title_tooltip_text(), Some("Save file"));
}

#[test]
fn it_is_placed_below_right_of_the_pointer_it_rested_at() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE_CHILD);
    let (x, y, w, h) = app.title_tooltip_rect().expect("shown");
    assert_eq!((x, y), (ON_SAVE_CHILD.0 + 10.0, ON_SAVE_CHILD.1 + 20.0));
    assert!(w > 20.0 && h > 14.0, "a box with text in it: {w}x{h}");
    // A move inside what it describes keeps it where it is.
    move_to(&mut app, ON_SAVE);
    assert_eq!(app.title_tooltip_rect().map(|r| (r.0, r.1)), Some((x, y)));
    assert_eq!(app.title_tooltip_deadline(), None);
}

#[test]
fn at_the_window_corner_it_flips_left_and_up_to_stay_inside() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_CORNER);
    let (x, y, w, h) = app.title_tooltip_rect().expect("shown");
    assert!(x + w <= VP.0, "inside the right edge: {x} + {w}");
    assert!(y + h <= VP.1, "inside the bottom edge: {y} + {h}");
    assert_eq!(x, ON_CORNER.0 - w, "flipped to end at the pointer");
    assert_eq!(y, ON_CORNER.1 - h - 4.0, "flipped above the pointer");
}

#[test]
fn the_nearest_title_wins_and_an_empty_one_suppresses_the_ancestors() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_INNER);
    assert_eq!(app.title_tooltip_text(), Some("Inner"));

    // Onto the outer box: the inner tooltip goes at once, the outer one waits.
    move_to(&mut app, ON_OUTER);
    assert_eq!(app.title_tooltip_text(), None);
    let due = app
        .title_tooltip_deadline()
        .expect("armed for the outer box");
    tick_at(&mut app, due);
    assert_eq!(app.title_tooltip_text(), Some("Outer"));

    // `title=""` is the relevant title and says nothing: not the outer's.
    move_to(&mut app, ON_BLANK);
    assert_eq!(app.title_tooltip_text(), None);
    assert_eq!(app.title_tooltip_deadline(), None);
}

#[test]
fn moving_onto_untitled_content_hides_it_and_arms_nothing() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    move_to(&mut app, ON_PLAIN);
    assert_eq!(app.title_tooltip_text(), None);
    assert_eq!(app.title_tooltip_deadline(), None);
}

#[test]
fn a_press_hides_it_and_keeps_it_away_until_another_title() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    app.handle_event(
        PlatformEvent::MouseDown {
            x: ON_SAVE.0,
            y: ON_SAVE.1,
            button: MouseButton::Left,
        },
        SIZE,
        1.0,
    );
    assert_eq!(app.title_tooltip_text(), None);
    app.handle_event(
        PlatformEvent::MouseUp {
            x: ON_SAVE.0,
            y: ON_SAVE.1,
            button: MouseButton::Left,
        },
        SIZE,
        1.0,
    );
    // Resting on (and moving within) the pressed element brings nothing back.
    move_to(&mut app, ON_SAVE_CHILD);
    assert_eq!(app.title_tooltip_deadline(), None);
    // Another title arms it again.
    move_to(&mut app, ON_INNER);
    assert!(app.title_tooltip_deadline().is_some());
}

#[test]
fn a_press_before_the_rest_ends_cancels_the_wait() {
    let (mut app, _) = mount();
    move_to(&mut app, ON_SAVE);
    let due = app.title_tooltip_deadline().expect("armed");
    app.handle_event(
        PlatformEvent::MouseDown {
            x: ON_SAVE.0,
            y: ON_SAVE.1,
            button: MouseButton::Right,
        },
        SIZE,
        1.0,
    );
    assert!(!tick_at(&mut app, due));
    assert_eq!(app.title_tooltip_text(), None);
}

#[test]
fn a_wheel_or_a_key_hides_it() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: ON_SAVE.0,
            y: ON_SAVE.1,
            delta_x: 0.0,
            delta_y: 40.0,
        },
        SIZE,
        1.0,
    );
    assert_eq!(app.title_tooltip_text(), None, "wheel");

    rest_at(&mut app, ON_INNER);
    assert_eq!(app.title_tooltip_text(), Some("Inner"));
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyA,
            logical_key: Some("a".into()),
            text: Some("a".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        SIZE,
        1.0,
    );
    assert_eq!(app.title_tooltip_text(), None, "key");
}

#[test]
fn a_window_blur_a_cancel_or_leaving_the_window_hides_it() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    app.handle_event(PlatformEvent::WindowFocus(false), SIZE, 1.0);
    assert_eq!(app.title_tooltip_text(), None, "blur");

    rest_at(&mut app, ON_INNER);
    app.handle_event(PlatformEvent::PointerCancel, SIZE, 1.0);
    assert_eq!(app.title_tooltip_text(), None, "cancel");

    rest_at(&mut app, ON_OUTER);
    // What the shell does on winit's `PointerLeft` for the mouse.
    app.hide_title_tooltip();
    assert_eq!(app.title_tooltip_text(), None, "left the window");
    assert_eq!(app.title_tooltip_deadline(), None);
}

#[test]
fn it_follows_its_title_and_goes_with_it() {
    let (mut app, ids) = mount();
    rest_at(&mut app, ON_SAVE);
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut()
        .set_attribute(NodeId(ids.save.get()), "title", "Save all files");
    // `AboutToWait` picks the change up.
    assert!(
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0)
            .contains(&AppAction::RequestRedraw)
    );
    assert_eq!(app.title_tooltip_text(), Some("Save all files"));

    doc.borrow_mut()
        .remove_attribute(NodeId(ids.save.get()), "title");
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    assert_eq!(app.title_tooltip_text(), None);
}

#[test]
fn a_title_removed_while_the_pointer_rests_shows_nothing() {
    let (mut app, ids) = mount();
    move_to(&mut app, ON_OUTER);
    let due = app.title_tooltip_deadline().expect("armed");
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut()
        .remove_attribute(NodeId(ids.outer.get()), "title");
    tick_at(&mut app, due);
    assert_eq!(app.title_tooltip_text(), None);
}

#[test]
fn turned_off_it_arms_nothing() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    app.set_title_tooltips(false);
    assert_eq!(app.title_tooltip_text(), None);
    move_to(&mut app, ON_INNER);
    assert_eq!(app.title_tooltip_deadline(), None);
}

// ── Pixels ─────────────────────────────────────────────────────────────────

/// How many pixels of the frame on screen are exactly `rgb`.
fn count_rgb(app: &mut RinchApp, rgb: [u8; 3]) -> usize {
    let (px, _, _) = app.build_pixels(1.0, SIZE, false);
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] == rgb[0] && p[1] == rgb[1] && p[2] == rgb[2])
        .count()
}

const THEME: &str = ":root { --rinch-color-surface: rgb(11, 22, 33); \
                     --rinch-color-border: rgb(201, 3, 7); \
                     --rinch-color-text: rgb(5, 250, 9); }";
const SURFACE: [u8; 3] = [11, 22, 33];

/// The theme's surface colour fills the box, and nothing else in the
/// document is that colour: a local pixel oracle for drawn, moved and gone.
#[test]
fn it_paints_in_the_theme_colours_and_its_pixels_go_with_it() {
    let (mut app, _) = mount();
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(THEME);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    paint(&mut app);
    assert_eq!(count_rgb(&mut app, SURFACE), 0, "nothing before it shows");

    rest_at(&mut app, ON_SAVE);
    let (_, _, w, h) = app.title_tooltip_rect().unwrap();
    let filled = count_rgb(&mut app, SURFACE);
    // The interior less the 1px border and the text: most of the box.
    assert!(
        filled as f32 > (w - 2.0) * (h - 2.0) * 0.5,
        "the surface fills the box: {filled} px of {w}x{h}"
    );

    // An incremental frame (the previous one is kept): the old rect is the
    // damage, so it is repainted without the box.
    move_to(&mut app, ON_PLAIN);
    assert_eq!(count_rgb(&mut app, SURFACE), 0, "hidden leaves no pixels");
}

#[test]
fn without_the_theme_it_falls_back_to_light_colours() {
    let (mut app, _) = mount();
    rest_at(&mut app, ON_SAVE);
    // The fallback border, #dee2e6, appears nowhere else in this document.
    assert!(count_rgb(&mut app, [0xde, 0xe2, 0xe6]) > 50);
}

// ── Cost ───────────────────────────────────────────────────────────────────

/// A pointer move over untitled content in a document that has no `title`
/// at all: the tooltip reads one counter and adds nothing to the frame.
#[test]
fn a_move_in_a_document_without_titles_costs_nothing_more() {
    let mut app = mount_settled(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        root
    });
    move_to(&mut app, ON_PLAIN);
    assert_eq!(app.title_tooltip_deadline(), None);
    let stats = paint(&mut app);
    expect_frame(
        "move_without_titles",
        &stats,
        &[
            (Counter::PaintCachedFrames, 1),
            (Counter::StackingOrderBuilds, 1),
            (Counter::HitTests, 1),
            (Counter::HitTestNodesVisited, 2),
            (Counter::HitExtentsComputed, 1),
        ],
    );
}

/// Showing the tooltip runs no style, layout or text pass: one partial
/// repaint of its own rect.
#[test]
fn showing_it_repaints_its_rect_and_lays_out_nothing() {
    let (mut app, _) = mount();
    move_to(&mut app, ON_SAVE);
    paint(&mut app);
    app.reset_perf();
    let due = app.title_tooltip_deadline().unwrap();
    assert!(tick_at(&mut app, due));
    let stats = paint(&mut app);
    // One damage rect, the tooltip's 68x29 box grown by a pixel of
    // anti-aliasing; the document's two nodes under it are repainted (the
    // root and `#save`), and the text's glyphs are rasterised for the first
    // time. No cascade, no layout, no shaping counter.
    expect_frame(
        "tooltip_show",
        &stats,
        &[
            (Counter::PaintFrames, 1),
            (Counter::RepaintPartial, 1),
            (Counter::DamageRects, 1),
            (Counter::RepaintedPx, 1972),
            (Counter::SurfacePx, 480000),
            (Counter::PaintNodesVisited, 2),
            (Counter::StackingOrderBuilds, 1),
            (Counter::GlyphCacheHits, 1),
            (Counter::GlyphCacheMisses, 7),
            (Counter::ClipMasks, 1),
            (Counter::ClipMaskPx, 2376),
            (Counter::PaintSurfaceAllocs, 1),
        ],
    );
}
