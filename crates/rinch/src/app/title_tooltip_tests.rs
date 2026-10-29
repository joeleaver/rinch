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
    save: Cell<u64>,
    outer: Cell<u64>,
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
    // One damage rect, the tooltip's 67x27 box grown by a pixel of
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
            (Counter::RepaintedPx, 2001),
            (Counter::SurfacePx, 480000),
            (Counter::PaintNodesVisited, 2),
            (Counter::StackingOrderBuilds, 1),
            (Counter::GlyphCacheHits, 1),
            (Counter::GlyphCacheMisses, 8),
            (Counter::ClipMasks, 1),
            (Counter::ClipMaskPx, 2409),
            (Counter::PaintSurfaceAllocs, 1),
        ],
    );
}

// ── Review of #1652 (scratch fixtures) ─────────────────────────────────────
mod review_1652 {
    use super::*;
    use rinch_dom::perf::FullRepaintReason;

    fn key(app: &mut RinchApp, k: KeyCode, lk: &str) -> Vec<AppAction> {
        app.handle_event(
            PlatformEvent::KeyDown {
                key: k,
                logical_key: Some(lk.into()),
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Fresh,
            },
            SIZE,
            1.0,
        )
    }

    /// The event's actions plus the `AboutToWait` turn after it: does the
    /// shell get asked for a frame?
    fn frame_asked(app: &mut RinchApp, mut a: Vec<AppAction>) -> bool {
        a.extend(app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0));
        a.contains(&AppAction::RequestRedraw)
    }

    #[test]
    fn r1a_a_shift_press_that_hides_it_asks_for_a_frame() {
        let (mut app, _) = mount();
        rest_at(&mut app, ON_SAVE);
        paint(&mut app);
        let a = key(&mut app, KeyCode::ShiftLeft, "Shift");
        assert_eq!(app.title_tooltip_text(), None);
        assert!(frame_asked(&mut app, a), "hidden, but no frame asked for");
    }

    /// The whole sequence a real Shift tap delivers.
    #[test]
    fn r1e_a_real_shift_tap_sequence_asks_for_a_frame() {
        let (mut app, _) = mount();
        rest_at(&mut app, ON_SAVE);
        paint(&mut app);
        let mut a = Vec::new();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        a.extend(app.handle_event(PlatformEvent::ModifiersChanged(shift), SIZE, 1.0));
        a.extend(key(&mut app, KeyCode::ShiftLeft, "Shift"));
        a.extend(app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0));
        a.extend(app.handle_event(
            PlatformEvent::ModifiersChanged(Modifiers::default()),
            SIZE,
            1.0,
        ));
        a.extend(app.handle_event(
            PlatformEvent::KeyUp {
                key: KeyCode::ShiftLeft,
                logical_key: Some("Shift".into()),
                modifiers: Modifiers::default(),
            },
            SIZE,
            1.0,
        ));
        assert_eq!(app.title_tooltip_text(), None);
        assert!(frame_asked(&mut app, a), "hidden, but no frame asked for");
    }

    #[test]
    fn r1b_a_wheel_over_unscrollable_content_asks_for_a_frame() {
        let (mut app, _) = mount();
        rest_at(&mut app, ON_SAVE);
        paint(&mut app);
        let a = app.handle_event(
            PlatformEvent::MouseWheel {
                x: ON_SAVE.0,
                y: ON_SAVE.1,
                delta_x: 0.0,
                delta_y: 40.0,
            },
            SIZE,
            1.0,
        );
        assert_eq!(app.title_tooltip_text(), None);
        assert!(frame_asked(&mut app, a), "hidden, but no frame asked for");
    }

    #[test]
    fn r1c_a_window_blur_that_hides_it_asks_for_a_frame() {
        let (mut app, _) = mount();
        rest_at(&mut app, ON_SAVE);
        paint(&mut app);
        let a = app.handle_event(PlatformEvent::WindowFocus(false), SIZE, 1.0);
        assert_eq!(app.title_tooltip_text(), None);
        assert!(frame_asked(&mut app, a), "hidden, but no frame asked for");
    }

    #[test]
    fn r1d_a_press_on_plain_content_that_hides_it_asks_for_a_frame() {
        let (mut app, _) = mount();
        rest_at(&mut app, ON_OUTER);
        paint(&mut app);
        let mut a = app.handle_event(
            PlatformEvent::MouseDown {
                x: ON_OUTER.0,
                y: ON_OUTER.1,
                button: MouseButton::Left,
            },
            SIZE,
            1.0,
        );
        a.extend(app.handle_event(
            PlatformEvent::MouseUp {
                x: ON_OUTER.0,
                y: ON_OUTER.1,
                button: MouseButton::Left,
            },
            SIZE,
            1.0,
        ));
        assert_eq!(app.title_tooltip_text(), None);
        assert!(frame_asked(&mut app, a), "hidden, but no frame asked for");
    }

    /// A small document built by the caller.
    fn mount_with(build: impl Fn(&mut RenderScope, &NodeHandle) + 'static) -> RinchApp {
        mount_settled(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: relative; width: 800px; height: 600px; font: 16px/20px sans-serif",
            );
            build(scope, &root);
            root
        })
    }

    #[test]
    fn r2a_an_svg_title_attribute_is_no_tooltip_as_in_chrome() {
        let mut app = mount_with(|scope, root| {
            let svg = scope.create_element("svg");
            svg.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            svg.set_attribute("width", "160");
            svg.set_attribute("height", "60");
            let rect = scope.create_element("rect");
            rect.set_attribute("width", "160");
            rect.set_attribute("height", "60");
            rect.set_attribute("fill", "#9cf");
            rect.set_attribute("title", "SVG-ATTR");
            svg.append_child(&rect);
            root.append_child(&svg);
        });
        move_to(&mut app, (171.0, 133.0));
        if let Some(due) = app.title_tooltip_deadline() {
            tick_at(&mut app, due);
        }
        assert_eq!(
            app.title_tooltip_text(),
            None,
            "Chrome 155 shows nothing for an SVG `title` attribute"
        );
    }

    /// Not done: rinch's hit test answers the `<svg>` for a point over one of
    /// its shapes, so a shape's own `<title>` is never reached.
    #[test]
    #[ignore = "SVG shapes are not hit-tested individually (#1646 not done)"]
    fn r2b_an_svg_title_child_is_the_tooltip_as_in_chrome() {
        let mut app = mount_with(|scope, root| {
            let svg = scope.create_element("svg");
            svg.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            svg.set_attribute("width", "160");
            svg.set_attribute("height", "60");
            let rect = scope.create_element("rect");
            rect.set_attribute("width", "160");
            rect.set_attribute("height", "60");
            let t = scope.create_element("title");
            t.append_child(&scope.create_text("SVG-CHILD"));
            rect.append_child(&t);
            svg.append_child(&rect);
            root.append_child(&svg);
        });
        move_to(&mut app, (171.0, 133.0));
        if let Some(due) = app.title_tooltip_deadline() {
            tick_at(&mut app, due);
        }
        assert_eq!(
            app.title_tooltip_text(),
            Some("SVG-CHILD"),
            "Chrome 155 shows an SVG <title> child"
        );
    }

    #[test]
    fn r4_a_disabled_buttons_title_shows_as_in_chrome() {
        let mut app = mount_with(|scope, root| {
            let b = scope.create_element("button");
            b.set_attribute("disabled", "");
            b.set_attribute("title", "DISABLED");
            b.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            root.append_child(&b);
        });
        rest_at(&mut app, (171.0, 133.0));
        assert_eq!(app.title_tooltip_text(), Some("DISABLED"));
    }

    /// An `<svg>`'s own `<title>` child is its tooltip (Chrome 155 shows an
    /// SVG `<title>` child), and the `<svg>`'s `title` attribute is not.
    #[test]
    fn r2d_an_svg_roots_title_child_is_its_tooltip() {
        let mut app = mount_with(|scope, root| {
            let svg = scope.create_element("svg");
            svg.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            svg.set_attribute("width", "160");
            svg.set_attribute("height", "60");
            svg.set_attribute("title", "SVG-ROOT-ATTR");
            let t = scope.create_element("title");
            t.append_child(&scope.create_text("SVG-ROOT-CHILD"));
            svg.append_child(&t);
            let rect = scope.create_element("rect");
            rect.set_attribute("width", "160");
            rect.set_attribute("height", "60");
            svg.append_child(&rect);
            root.append_child(&svg);
        });
        rest_at(&mut app, (171.0, 133.0));
        assert_eq!(app.title_tooltip_text(), Some("SVG-ROOT-CHILD"));
    }

    /// A document whose only title is an SVG `<title>` element: the counter
    /// that lets an untitled document skip the walk must count it.
    #[test]
    fn r2e_a_title_element_alone_is_counted() {
        let mut app = mount_with(|scope, root| {
            let svg = scope.create_element("svg");
            svg.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            let t = scope.create_element("title");
            t.append_child(&scope.create_text("ONLY-CHILD"));
            svg.append_child(&t);
            root.append_child(&svg);
        });
        rest_at(&mut app, (171.0, 133.0));
        assert_eq!(app.title_tooltip_text(), Some("ONLY-CHILD"));
    }

    #[test]
    fn r2c_an_svg_roots_title_attribute_is_no_tooltip_as_in_chrome() {
        let mut app = mount_with(|scope, root| {
            let svg = scope.create_element("svg");
            svg.set_attribute(
                "style",
                "position: absolute; left: 100px; top: 100px; width: 160px; height: 60px",
            );
            svg.set_attribute("width", "160");
            svg.set_attribute("height", "60");
            svg.set_attribute("title", "SVG-ROOT-ATTR");
            let rect = scope.create_element("rect");
            rect.set_attribute("width", "160");
            rect.set_attribute("height", "60");
            svg.append_child(&rect);
            root.append_child(&svg);
        });
        move_to(&mut app, (171.0, 133.0));
        if let Some(due) = app.title_tooltip_deadline() {
            tick_at(&mut app, due);
        }
        assert_eq!(
            app.title_tooltip_text(),
            None,
            "Chrome 155 shows nothing for <svg title>"
        );
    }

    /// Kills M9 (the in-document check): the doc says a detached source hides it.
    #[test]
    fn r8_removing_the_titled_element_takes_it_down() {
        let (mut app, ids) = mount();
        rest_at(&mut app, ON_SAVE);
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().remove_node(NodeId(ids.save.get()));
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
        assert_eq!(app.title_tooltip_text(), None);
    }

    // ── pixel oracle: a partial frame equals a full repaint ──

    fn frame(app: &mut RinchApp, scale: f64) -> Vec<u8> {
        if app.has_pending_layout() {
            app.resolve_and_repaint(VP.0, VP.1);
        }
        app.build_pixels(scale, SIZE_PX(scale), false).0.to_vec()
    }
    #[allow(non_snake_case)]
    fn SIZE_PX(_scale: f64) -> (u32, u32) {
        SIZE
    }
    fn full(app: &mut RinchApp, scale: f64) -> Vec<u8> {
        app.invalidate_previous_frame(FullRepaintReason::Invalidated);
        app.request_repaint();
        frame(app, scale)
    }
    fn assert_partial_is_full(app: &mut RinchApp, scale: f64, what: &str) {
        let p = frame(app, scale);
        let f = full(app, scale);
        let diff = p.chunks(4).zip(f.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(
            diff, 0,
            "{what}: {diff} px differ between the partial frame and a full repaint"
        );
    }

    fn scroller_app() -> (RinchApp, Rc<Cell<u64>>) {
        let id = Rc::new(Cell::new(0));
        let i = id.clone();
        let app = mount_with(move |scope, root| {
            let sc = scope.create_element("div");
            sc.set_attribute(
                "style",
                "position: absolute; left: 30px; top: 40px; width: 260px; height: 180px; overflow: auto",
            );
            sc.set_attribute("title", "Scroller");
            for k in 0..40 {
                let row = scope.create_element("div");
                let c = (k * 37 % 200) + 30;
                row.set_attribute(
                    "style",
                    &format!(
                        "height: 13px; background: rgb({c}, {}, {})",
                        255 - c,
                        (c * 3) % 256
                    ),
                );
                row.append_child(&scope.create_text(&format!("row {k}")));
                sc.append_child(&row);
            }
            i.set(sc.node_id().0);
            root.append_child(&sc);
            let fixed = scope.create_element("div");
            fixed.set_attribute(
                "style",
                "position: fixed; left: 400px; top: 300px; width: 300px; height: 200px; \
                 z-index: 9999; background: rgb(1, 2, 3)",
            );
            fixed.set_attribute("title", "Over a fixed overlay");
            root.append_child(&fixed);
        });
        (app, id)
    }

    fn scroll_oracle(scale: f64) {
        let (mut app, sc) = scroller_app();
        frame(&mut app, scale);
        rest_at(&mut app, (243.0, 187.0)); // near the scroller's bottom-right: the tooltip overlaps it and the page
        assert_eq!(app.title_tooltip_text(), Some("Scroller"));
        assert_partial_is_full(&mut app, scale, "shown");
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().set_scroll_top(NodeId(sc.get()), 37.0);
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
        assert_partial_is_full(&mut app, scale, "scrolled under it");
        move_to(&mut app, (751.0, 41.0));
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
        assert_eq!(app.title_tooltip_text(), None);
        assert_partial_is_full(&mut app, scale, "hidden");
        // Over the fixed overlay: drawn on top of it.
        rest_at(&mut app, (690.0, 489.0));
        assert_eq!(app.title_tooltip_text(), Some("Over a fixed overlay"));
        assert_partial_is_full(&mut app, scale, "over the overlay");
        move_to(&mut app, (751.0, 41.0));
        assert_partial_is_full(&mut app, scale, "hidden off the overlay");
    }

    #[test]
    fn r9_partial_frames_equal_a_full_repaint_at_1x() {
        scroll_oracle(1.0);
    }

    #[test]
    fn r10_partial_frames_equal_a_full_repaint_at_1_25x() {
        scroll_oracle(1.25);
    }

    #[test]
    fn r10b_partial_frames_equal_a_full_repaint_at_2x() {
        scroll_oracle(2.0);
    }
}

/// `App::title_tooltips(false)` reaches the app `run` creates afterwards on
/// the thread (`set_title_tooltips_default`), and only that thread.
#[test]
fn the_startup_default_turns_off_apps_created_after_it() {
    set_title_tooltips_default(false);
    let (mut off, _) = mount();
    set_title_tooltips_default(true);
    assert!(!off.title_tooltips_enabled());
    move_to(&mut off, ON_SAVE);
    assert_eq!(off.title_tooltip_deadline(), None);
    let (on, _) = mount();
    assert!(on.title_tooltips_enabled());
}

/// A component's `title` reaches the desktop tooltip (#1653): `ActionIcon`'s
/// and `CloseButton`'s own prop, and the quoted `"title"` on any component,
/// both written on the component's root `<button>`.
mod component_titles_1653 {
    use super::*;
    use crate as rinch;
    use rinch_components::{ActionIcon, Button, CloseButton};
    use rinch_core::Signal;
    use rinch_macros::rsx;

    const ON_ICON: (f32, f32) = (63.0, 57.0);
    const ON_CLOSE: (f32, f32) = (231.0, 61.0);
    const ON_BUTTON: (f32, f32) = (419.0, 73.0);

    fn mount_buttons(tip: Signal<String>) -> RinchApp {
        mount_settled(move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    style: "position: relative; width: 800px; height: 600px; \
                            font: 16px/20px sans-serif",
                    ActionIcon {
                        title: "Save file",
                        style: "position: absolute; left: 40px; top: 40px; \
                                width: 50px; height: 40px",
                        "S"
                    }
                    CloseButton {
                        title: "Dismiss",
                        style: "position: absolute; left: 210px; top: 40px; \
                                width: 44px; height: 44px",
                    }
                    Button {
                        "title": {move || tip.get()},
                        style: "position: absolute; left: 380px; top: 50px; \
                                width: 90px; height: 50px",
                        "Delete"
                    }
                }
            }
        })
    }

    #[test]
    fn an_action_icons_and_a_close_buttons_title_prop_is_their_tooltip() {
        let mut app = mount_buttons(Signal::new("Delete the row".into()));
        rest_at(&mut app, ON_ICON);
        assert_eq!(app.title_tooltip_text(), Some("Save file"));
        rest_at(&mut app, ON_CLOSE);
        assert_eq!(app.title_tooltip_text(), Some("Dismiss"));
    }

    #[test]
    fn a_quoted_title_on_a_component_is_its_tooltip_and_follows_its_signal() {
        let tip = Signal::new("Delete the row".to_string());
        let mut app = mount_buttons(tip);
        rest_at(&mut app, ON_BUTTON);
        assert_eq!(app.title_tooltip_text(), Some("Delete the row"));
        tip.set("Delete 3 rows".to_string());
        move_to(&mut app, ON_PLAIN);
        rest_at(&mut app, ON_BUTTON);
        assert_eq!(app.title_tooltip_text(), Some("Delete 3 rows"));
    }
}
