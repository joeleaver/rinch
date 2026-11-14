//! #2648: hover, the cursor and the mouse events go to the inline element
//! under the pointer, as Chrome 157 sends them.
//!
//! An IFC's text is hit as its root, so `hit_test` answers the paragraph
//! for a point over an inline element's glyphs or (#2645) its padding. The
//! click and drag walks refined that to the inline element
//! (`hit_testing::event_target_at`); `:hover`, `:active`, the cursor,
//! `onmouseenter`/`onmouseleave`, the `data-onmouse*` handlers,
//! `oncontextmenu`, `onwheel` and the focus a press takes stayed on the
//! paragraph, so a padded `<a>` with `a:hover` or `cursor: pointer` showed
//! neither. They all start at `RinchApp::shared_target` now
//! (`hit_testing::element_target_at`).
//!
//! The scene, set in Inter, and Chrome 157's answers (headless, 1x, driven
//! by real mouse input through the DevTools protocol, the same body under
//! `@font-face { font-family: ProbeFace; src: url(Inter-Regular.ttf) }`):
//! a padded link; three nested spans, the middle one `cursor: crosshair`;
//! a link holding a `pointer-events: none` span, beside another one; a
//! `tabindex` span. The points sweep each line at its middle and in the
//! padding above and below the glyphs, keeping only points whose neighbours
//! 1.5px either side hit the same element in Chrome, then jump between the
//! nested spans and the links so several elements are entered and left in
//! one move.

use super::text_selection_across_tests::node;
use super::*;
use rinch_platform::CursorStyle;

const VP: (u32, u32) = (300, 260);

const BODY: &str = r##"
<div id=b1 style="left:10px;top:10px;width:270px">xx <a id=a1 href="#x" style="padding:7px 11px">MMM</a> yy</div>
<div id=b2 style="left:10px;top:70px;width:270px">ab <span id=n1 style="padding:3px 5px">cd <span id=n2 style="padding:2px 4px;cursor:crosshair">MM <span id=n3>WW</span> MM</span> ef</span> gh</div>
<div id=b3 style="left:10px;top:130px;width:270px">xx <a id=a3 href="#y" style="padding:6px 9px">pq <span id=pn style="pointer-events:none;padding:4px 6px">MMM</span> rs</a> <span id=pm style="pointer-events:none;padding:5px">MM</span> zz</div>
<div id=b4 style="left:10px;top:190px;width:270px">xx <span id=f1 tabindex=0 style="padding:5px 8px;outline:none">MMM</span> yy</div>
"##;

const CSS: &str = "html, body { margin: 0; background: #fff; }
    #b1, #b2, #b3, #b4 { position: absolute; font: 16px/40px sans-serif; user-select: text; }
    a[href] { cursor: pointer; }
    a:hover { background: #f00; }";

/// The scene's elements, each given every handler below.
const IDS: &[&str] = &[
    "b1", "a1", "b2", "n1", "n2", "n3", "b3", "a3", "pn", "pm", "b4", "f1",
];

/// `(x, y, elementFromPoint, :hover, events, cursor)`.
type Move = (
    f32,
    f32,
    Option<&'static str>,
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
);

/// `(x, y, :active, events)`.
type Press = (f32, f32, &'static [&'static str], &'static [&'static str]);

/// The pointer's path, one move at a time, and what Chrome 157 answers
/// after each: `elementFromPoint` (`None`: no element of the scene),
/// the scene's elements matching `:hover`, the `mouseleave` and
/// `mouseenter` events the move fired, in order, and the computed
/// `cursor` of the element under the pointer.
const MOVES: &[Move] = &[
    (15.0, 19.0, Some("b1"), &["b1"], &["enter:b1"], "auto"),
    (21.0, 19.0, Some("b1"), &["b1"], &[], "auto"),
    (27.0, 19.0, Some("b1"), &["b1"], &[], "auto"),
    (
        34.5,
        19.0,
        Some("a1"),
        &["b1", "a1"],
        &["enter:a1"],
        "pointer",
    ),
    (64.5, 19.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (93.0, 19.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (100.5, 19.0, Some("b1"), &["b1"], &["leave:a1"], "auto"),
    (148.5, 19.0, Some("b1"), &["b1"], &[], "auto"),
    (196.5, 19.0, Some("b1"), &["b1"], &[], "auto"),
    (15.0, 30.0, Some("b1"), &["b1"], &[], "auto"),
    (21.0, 30.0, Some("b1"), &["b1"], &[], "auto"),
    (27.0, 30.0, Some("b1"), &["b1"], &[], "auto"),
    (
        34.5,
        30.0,
        Some("a1"),
        &["b1", "a1"],
        &["enter:a1"],
        "pointer",
    ),
    (64.5, 30.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (93.0, 30.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (100.5, 30.0, Some("b1"), &["b1"], &["leave:a1"], "auto"),
    (148.5, 30.0, Some("b1"), &["b1"], &[], "auto"),
    (196.5, 30.0, Some("b1"), &["b1"], &[], "auto"),
    (15.0, 41.0, Some("b1"), &["b1"], &[], "auto"),
    (21.0, 41.0, Some("b1"), &["b1"], &[], "auto"),
    (27.0, 41.0, Some("b1"), &["b1"], &[], "auto"),
    (
        34.5,
        41.0,
        Some("a1"),
        &["b1", "a1"],
        &["enter:a1"],
        "pointer",
    ),
    (64.5, 41.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (93.0, 41.0, Some("a1"), &["b1", "a1"], &[], "pointer"),
    (100.5, 41.0, Some("b1"), &["b1"], &["leave:a1"], "auto"),
    (148.5, 41.0, Some("b1"), &["b1"], &[], "auto"),
    (196.5, 41.0, Some("b1"), &["b1"], &[], "auto"),
    (
        15.0,
        79.0,
        Some("b2"),
        &["b2"],
        &["leave:b1", "enter:b2"],
        "auto",
    ),
    (22.5, 79.0, Some("b2"), &["b2"], &[], "auto"),
    (28.5, 79.0, Some("b2"), &["b2"], &[], "auto"),
    (36.0, 79.0, Some("n1"), &["b2", "n1"], &["enter:n1"], "auto"),
    (46.5, 79.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (57.0, 79.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (
        64.5,
        79.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &["enter:n2"],
        "crosshair",
    ),
    (
        114.0,
        79.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        163.5,
        79.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        171.0,
        79.0,
        Some("n1"),
        &["b2", "n1"],
        &["leave:n2"],
        "auto",
    ),
    (180.0, 79.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (189.0, 79.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (196.5, 79.0, Some("b2"), &["b2"], &["leave:n1"], "auto"),
    (15.0, 90.0, Some("b2"), &["b2"], &[], "auto"),
    (22.5, 90.0, Some("b2"), &["b2"], &[], "auto"),
    (28.5, 90.0, Some("b2"), &["b2"], &[], "auto"),
    (36.0, 90.0, Some("n1"), &["b2", "n1"], &["enter:n1"], "auto"),
    (46.5, 90.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (57.0, 90.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (
        64.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &["enter:n2"],
        "crosshair",
    ),
    (
        79.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        94.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        102.0,
        90.0,
        Some("n3"),
        &["b2", "n1", "n2", "n3"],
        &["enter:n3"],
        "crosshair",
    ),
    (
        114.0,
        90.0,
        Some("n3"),
        &["b2", "n1", "n2", "n3"],
        &[],
        "crosshair",
    ),
    (
        126.0,
        90.0,
        Some("n3"),
        &["b2", "n1", "n2", "n3"],
        &[],
        "crosshair",
    ),
    (
        133.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &["leave:n3"],
        "crosshair",
    ),
    (
        148.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        163.5,
        90.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        171.0,
        90.0,
        Some("n1"),
        &["b2", "n1"],
        &["leave:n2"],
        "auto",
    ),
    (180.0, 90.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (189.0, 90.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (196.5, 90.0, Some("b2"), &["b2"], &["leave:n1"], "auto"),
    (15.0, 101.0, Some("b2"), &["b2"], &[], "auto"),
    (22.5, 101.0, Some("b2"), &["b2"], &[], "auto"),
    (28.5, 101.0, Some("b2"), &["b2"], &[], "auto"),
    (
        36.0,
        101.0,
        Some("n1"),
        &["b2", "n1"],
        &["enter:n1"],
        "auto",
    ),
    (46.5, 101.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (57.0, 101.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (
        64.5,
        101.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &["enter:n2"],
        "crosshair",
    ),
    (
        114.0,
        101.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        163.5,
        101.0,
        Some("n2"),
        &["b2", "n1", "n2"],
        &[],
        "crosshair",
    ),
    (
        171.0,
        101.0,
        Some("n1"),
        &["b2", "n1"],
        &["leave:n2"],
        "auto",
    ),
    (180.0, 101.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (189.0, 101.0, Some("n1"), &["b2", "n1"], &[], "auto"),
    (196.5, 101.0, Some("b2"), &["b2"], &["leave:n1"], "auto"),
    (
        15.0,
        139.0,
        Some("b3"),
        &["b3"],
        &["leave:b2", "enter:b3"],
        "auto",
    ),
    (21.0, 139.0, Some("b3"), &["b3"], &[], "auto"),
    (27.0, 139.0, Some("b3"), &["b3"], &[], "auto"),
    (
        34.5,
        139.0,
        Some("a3"),
        &["b3", "a3"],
        &["enter:a3"],
        "pointer",
    ),
    (90.0, 139.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (144.0, 139.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (151.5, 139.0, Some("b3"), &["b3"], &["leave:a3"], "auto"),
    (174.0, 139.0, Some("b3"), &["b3"], &[], "auto"),
    (196.5, 139.0, Some("b3"), &["b3"], &[], "auto"),
    (15.0, 150.0, Some("b3"), &["b3"], &[], "auto"),
    (21.0, 150.0, Some("b3"), &["b3"], &[], "auto"),
    (27.0, 150.0, Some("b3"), &["b3"], &[], "auto"),
    (
        34.5,
        150.0,
        Some("a3"),
        &["b3", "a3"],
        &["enter:a3"],
        "pointer",
    ),
    (90.0, 150.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (144.0, 150.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (151.5, 150.0, Some("b3"), &["b3"], &["leave:a3"], "auto"),
    (174.0, 150.0, Some("b3"), &["b3"], &[], "auto"),
    (196.5, 150.0, Some("b3"), &["b3"], &[], "auto"),
    (15.0, 161.0, Some("b3"), &["b3"], &[], "auto"),
    (21.0, 161.0, Some("b3"), &["b3"], &[], "auto"),
    (27.0, 161.0, Some("b3"), &["b3"], &[], "auto"),
    (
        34.5,
        161.0,
        Some("a3"),
        &["b3", "a3"],
        &["enter:a3"],
        "pointer",
    ),
    (90.0, 161.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (144.0, 161.0, Some("a3"), &["b3", "a3"], &[], "pointer"),
    (151.5, 161.0, Some("b3"), &["b3"], &["leave:a3"], "auto"),
    (174.0, 161.0, Some("b3"), &["b3"], &[], "auto"),
    (196.5, 161.0, Some("b3"), &["b3"], &[], "auto"),
    (
        15.0,
        30.0,
        Some("b1"),
        &["b1"],
        &["leave:b3", "enter:b1"],
        "auto",
    ),
    (
        114.0,
        90.0,
        Some("n3"),
        &["b2", "n1", "n2", "n3"],
        &["leave:b1", "enter:b2", "enter:n1", "enter:n2", "enter:n3"],
        "crosshair",
    ),
    (
        90.0,
        150.0,
        Some("a3"),
        &["b3", "a3"],
        &[
            "leave:n3", "leave:n2", "leave:n1", "leave:b2", "enter:b3", "enter:a3",
        ],
        "pointer",
    ),
    (
        114.0,
        90.0,
        Some("n3"),
        &["b2", "n1", "n2", "n3"],
        &[
            "leave:a3", "leave:b3", "enter:b2", "enter:n1", "enter:n2", "enter:n3",
        ],
        "crosshair",
    ),
    (
        46.5,
        79.0,
        Some("n1"),
        &["b2", "n1"],
        &["leave:n3", "leave:n2"],
        "auto",
    ),
    (295.0, 255.0, None, &[], &["leave:n1", "leave:b2"], "auto"),
];

/// A left click, a right click and a wheel at each point, and what Chrome
/// 157 answers: the elements matching `:active` while the left button is
/// down, and the `mousedown` (both presses), `contextmenu` and `wheel`
/// events with their targets, in order.
const PRESSES: &[Press] = &[
    (
        34.5,
        30.0,
        &["b1", "a1"],
        &["mousedown:a1", "mousedown:a1", "contextmenu:a1", "wheel:a1"],
    ),
    (
        64.5,
        19.0,
        &["b1", "a1"],
        &["mousedown:a1", "mousedown:a1", "contextmenu:a1", "wheel:a1"],
    ),
    (
        114.0,
        90.0,
        &["b2", "n1", "n2", "n3"],
        &["mousedown:n3", "mousedown:n3", "contextmenu:n3", "wheel:n3"],
    ),
    (
        46.5,
        79.0,
        &["b2", "n1"],
        &["mousedown:n1", "mousedown:n1", "contextmenu:n1", "wheel:n1"],
    ),
    (
        90.0,
        150.0,
        &["b3", "a3"],
        &["mousedown:a3", "mousedown:a3", "contextmenu:a3", "wheel:a3"],
    ),
    (
        174.0,
        150.0,
        &["b3"],
        &["mousedown:b3", "mousedown:b3", "contextmenu:b3", "wheel:b3"],
    ),
    (
        15.0,
        30.0,
        &["b1"],
        &["mousedown:b1", "mousedown:b1", "contextmenu:b1", "wheel:b1"],
    ),
];

/// Clicks, and the element Chrome 157 focuses (`None`: the body).
const FOCUS: &[(f32, f32, Option<&str>)] = &[
    (40.0, 210.0, Some("f1")),
    (60.0, 220.0, Some("f1")),
    (15.0, 220.0, None),
    (40.0, 198.0, Some("f1")),
];

type Log = Rc<RefCell<Vec<String>>>;

/// The scene, laid out in Inter, every element of it carrying an
/// `onmouseenter`, `onmouseleave`, `onmousedown`, `oncontextmenu` and a
/// consuming `onwheel` that log `"<event>:<id>"`.
fn mount() -> (RinchApp, Log) {
    let mut app = super::perf_expect::new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        root.append_child(&style);
        let content = scope.create_element("div");
        content.set_inner_html(BODY);
        root.append_child(&content);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let log: Log = Rc::default();
    for &id in IDS {
        let n = rinch_core::dom::NodeId(node(&app, id));
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        for (attr, what) in [
            ("data-onenter", "enter"),
            ("data-onleave", "leave"),
            ("data-onmousedown", "mousedown"),
            ("data-oncontextmenu", "contextmenu"),
        ] {
            let log = log.clone();
            let h = events::register_handler(Rc::new(move || {
                log.borrow_mut().push(format!("{what}:{id}"));
            }));
            d.set_attribute(n, attr, &h.0.to_string());
        }
        let log = log.clone();
        let h = events::register_wheel_handler(events::WheelCallback::new(
            move |_w: &events::WheelEvent| {
                log.borrow_mut().push(format!("wheel:{id}"));
                true
            },
        ));
        d.set_attribute(n, "data-onwheel", &h.0.to_string());
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, log)
}

/// The scene's `id` of `n`, if it has one.
fn id_of(app: &RinchApp, n: u64) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(n)
        .and_then(|n| n.attributes.get("id"))
        .filter(|id| IDS.contains(&id.as_str()))
        .cloned()
}

/// The scene's elements that are `:hover`, in document order.
fn hovered(app: &RinchApp) -> Vec<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    IDS.iter()
        .filter(|&&id| {
            d.tree
                .nodes
                .iter()
                .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
                .is_some_and(|(_, n)| n.is_hovered)
        })
        .map(|id| id.to_string())
        .collect()
}

/// The scene's elements that are `:active`, in document order.
fn active(app: &RinchApp) -> Vec<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    IDS.iter()
        .filter(|&&id| {
            d.tree
                .nodes
                .iter()
                .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
                .is_some_and(|(_, n)| n.is_active)
        })
        .map(|id| id.to_string())
        .collect()
}

fn move_to(app: &mut RinchApp, x: f32, y: f32) -> Option<CursorStyle> {
    let actions = app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    actions.iter().rev().find_map(|a| match a {
        AppAction::SetCursor(c) => Some(*c),
        _ => None,
    })
}

/// Every move lands where Chrome's does: the element `:hover` and its
/// ancestors, the elements left and entered in Chrome's order (innermost
/// first out, outermost first in), and the cursor the element names. A
/// `pointer-events: none` span passes the pointer to the link around it, or
/// to the block.
#[test]
fn hover_enter_leave_and_the_cursor_follow_the_inline_element_as_in_chrome() {
    let (mut app, log) = mount();
    move_to(&mut app, 295.0, 255.0);
    log.borrow_mut().clear();
    let mut bad = Vec::new();
    for &(x, y, at, hover, events, cursor) in MOVES {
        let cs = move_to(&mut app, x, y);
        app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        let got_hover = hovered(&app);
        let got_events = std::mem::take(&mut *log.borrow_mut());
        // The innermost `:hover` element is the one under the pointer.
        let target = got_hover.last().cloned();
        if target.as_deref() != at {
            bad.push(format!("({x}, {y}) target {target:?}, Chrome {at:?}"));
        }
        if got_hover != hover {
            bad.push(format!("({x}, {y}) :hover {got_hover:?}, Chrome {hover:?}"));
        }
        if got_events != events {
            bad.push(format!(
                "({x}, {y}) events {got_events:?}, Chrome {events:?}"
            ));
        }
        // An element's own cursor is Chrome's; `auto` is the I-beam over
        // text (Chrome's is the arrow in an inline element's padding,
        // where no text is: not this issue's).
        let want = match cursor {
            "pointer" => Some(CursorStyle::Pointer),
            "crosshair" => Some(CursorStyle::Crosshair),
            _ => None,
        };
        let ok = match want {
            Some(w) => cs == Some(w),
            None => !matches!(cs, Some(CursorStyle::Pointer | CursorStyle::Crosshair)),
        };
        if !ok {
            bad.push(format!("({x}, {y}) cursor {cs:?}, Chrome {cursor}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches over {} moves:\n{}",
        bad.len(),
        MOVES.len(),
        bad.join("\n")
    );
}

/// `a:hover` restyles the link while the pointer is over its text or its
/// padding, and not past it.
#[test]
fn a_links_hover_rule_applies_over_its_text_and_padding() {
    let (mut app, _log) = mount();
    let a1 = node(&app, "a1");
    let bg = |app: &RinchApp| {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(a1).unwrap().computed_style.background_color()
    };
    let red = Some(peniko::Color::from_rgba8(255, 0, 0, 255));
    for &(x, y, want) in &[
        (64.5, 30.0, true),   // its text
        (34.5, 30.0, true),   // its left padding
        (64.5, 19.0, true),   // its top padding
        (64.5, 41.0, true),   // its bottom padding
        (100.5, 30.0, false), // past it
    ] {
        move_to(&mut app, x, y);
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        assert_eq!(bg(&app) == red, want, "({x}, {y}): {:?}", bg(&app));
    }
}

/// A press sets `:active` on the inline element and its ancestors, and its
/// `mousedown`, `contextmenu` and `wheel` go to the element, as in Chrome
/// (each press, the right one too, is a `mousedown`).
#[test]
fn a_press_activates_and_targets_the_inline_element_as_in_chrome() {
    let (mut app, log) = mount();
    let mut bad = Vec::new();
    for &(x, y, want_active, want) in PRESSES {
        move_to(&mut app, x, y);
        log.borrow_mut().clear();
        let left = MouseButton::Left;
        app.handle_event(PlatformEvent::MouseDown { x, y, button: left }, VP, 1.0);
        let got_active = active(&app);
        app.handle_event(PlatformEvent::MouseUp { x, y, button: left }, VP, 1.0);
        let right = MouseButton::Right;
        app.handle_event(
            PlatformEvent::MouseDown {
                x,
                y,
                button: right,
            },
            VP,
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseUp {
                x,
                y,
                button: right,
            },
            VP,
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x: 0.0,
                delta_y: -10.0,
            },
            VP,
            1.0,
        );
        app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        let got = std::mem::take(&mut *log.borrow_mut());
        if got_active != want_active {
            bad.push(format!(
                "({x}, {y}) :active {got_active:?}, Chrome {want_active:?}"
            ));
        }
        if got != want {
            bad.push(format!("({x}, {y}) {got:?}, Chrome {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A press on a `tabindex` span's text or padding focuses the span; one
/// past it focuses nothing.
#[test]
fn a_press_on_a_focusable_inline_element_focuses_it_as_in_chrome() {
    let (mut app, _log) = mount();
    for &(x, y, want) in FOCUS {
        let left = MouseButton::Left;
        app.handle_event(PlatformEvent::MouseDown { x, y, button: left }, VP, 1.0);
        app.handle_event(PlatformEvent::MouseUp { x, y, button: left }, VP, 1.0);
        let focused = {
            let d = app.doc.as_ref().unwrap().borrow();
            d.tree.focused_node
        };
        let got = focused.and_then(|n| id_of(&app, n));
        let claim = match app.focus_target {
            FocusTarget::Node(n) => id_of(&app, n),
            _ => None,
        };
        assert_eq!(claim.as_deref(), want, "({x}, {y}): the keyboard claim");
        if want.is_some() {
            assert_eq!(got.as_deref(), want, "({x}, {y}): :focus");
        }
    }
}

/// #2672: the mouse leaving the window leaves everything it was over, as in
/// Chrome, whose `:hover` clears and whose elements hear `mouseleave` when
/// the mouse leaves the page. Before, only the `title` tooltip went: `a1`
/// stayed `:hover`, red, and neither it nor its block heard a leave.
#[test]
fn the_mouse_leaving_the_window_clears_hover_and_leaves_innermost_first() {
    let (mut app, log) = mount();
    let a1 = node(&app, "a1");
    let bg = |app: &RinchApp| {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(a1).unwrap().computed_style.background_color()
    };
    let red = Some(peniko::Color::from_rgba8(255, 0, 0, 255));
    move_to(&mut app, 295.0, 255.0);
    move_to(&mut app, 64.5, 30.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    // Control: over the link, as Chrome.
    assert_eq!(hovered(&app), ["b1", "a1"]);
    assert_eq!(bg(&app), red);
    assert_eq!(*log.borrow(), ["enter:b1", "enter:a1"]);
    log.borrow_mut().clear();

    let actions = app.handle_event(PlatformEvent::MouseLeft, VP, 1.0);
    assert!(actions.contains(&AppAction::RequestRedraw), "{actions:?}");
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(hovered(&app), Vec::<String>::new(), ":hover after leaving");
    assert_ne!(bg(&app), red, "a:hover after leaving");
    assert_eq!(*log.borrow(), ["leave:a1", "leave:b1"]);
    log.borrow_mut().clear();

    // A second leave tells nobody again.
    app.handle_event(PlatformEvent::MouseLeft, VP, 1.0);
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());

    // Coming back to the same point enters again, outermost first.
    move_to(&mut app, 64.5, 30.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(hovered(&app), ["b1", "a1"]);
    assert_eq!(bg(&app), red);
    assert_eq!(*log.borrow(), ["enter:b1", "enter:a1"]);
}
