//! The `Drawer`'s slide-in in a real browser, issue #751.
//!
//! This is the web twin of `rinch/src/app/drawer_open_animation_tests.rs`, and
//! it is the half that says the bug was never desktop-only. `rinch-components`
//! ships **one** stylesheet to both backends. While the drawer's root was
//! `display: none`, Chrome refused the panel's `transform` transition for
//! exactly the reason rinch's own engine does — css-transitions-1 §3 gives an
//! element that was not being rendered a before-change style equal to its
//! after-change style — so the Drawer had never animated here at all, and #703
//! only brought desktop into line with it.
//!
//! Measured in Chrome 150, opening a `--md` (380px) left drawer, reading the
//! panel's computed `transform` and its own `getAnimations()`:
//!
//! | `.rinch-drawer__root--hidden` | closed root `display` | closed panel `transform` | `getAnimations()` | frame 1 | +120ms (wall clock, once started) |
//! |---|---|---|---|---|---|
//! | `display: none !important` | `none` | `none` | **0** | `matrix(1, 0, 0, 1, 0, 0)` | `matrix(1, 0, 0, 1, 0, 0)` |
//! | `visibility: hidden !important` | `block` | `matrix(1, 0, 0, 1, -380, 0)` | **1** | `matrix(1, 0, 0, 1, -380, 0)` | `matrix(1, 0, 0, 1, -126.775, 0)` |
//!
//! The top row is the panel teleporting to its open position on the first frame
//! with nothing running; the bottom row is the slide. Both fixtures below are
//! red on the top row — at the `visibility` precondition, which is the
//! mechanism.
//!
//! **Mount through `rinch_web::mount_into`, not a hand-rolled `RenderScope`.**
//! `mount_tree` is what pushes the reactive owner and installs the render scope;
//! without it the drawer's `opened_fn` effect is created and then disposed with
//! the local scope, and `opened.set(true)` changes no class at all — a silent
//! failure that looks exactly like a CSS bug.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::Drawer;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-751";

thread_local! {
    /// The root the previous fixture mounted, so each one starts from an empty
    /// page. A `Drawer`'s root is `position: fixed; inset: 0`, so a leftover
    /// would take every `elementFromPoint` below.
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A closed `Drawer` in a fresh host, mounted through `rinch_web::mount_into` —
/// the real entry point, which is what arms the reactive owner the drawer's
/// `opened_fn` effect belongs to. Hand-rolling a `RenderScope` here does not:
/// the effect is registered but nothing pushes an owner, the scope drops at the
/// end of the helper, and the drawer's classes then simply stop changing, with
/// nothing to see but a signal write that appears to do nothing.
///
/// `setup_theme_css` carries `generate_component_css()` with it (the facade's
/// `components` feature), so the real drawer stylesheet is in the page.
///
/// Returns the signal that opens it and the panel element — the one carrying
/// `transition: transform 300ms ease`.
fn mount_closed_drawer() -> (Signal<bool>, web_sys::Element) {
    let bdoc = browser_document();

    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = bdoc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }

    let host = bdoc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();

    let opened = Signal::new(false);
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        Drawer {
            opened_fn: Some(Rc::new(move || opened.get())),
            position: "left".to_string(),
            size: "md".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));

    let panel = bdoc.query_selector(".rinch-drawer").unwrap().unwrap();
    (opened, panel)
}

/// The browser's own list of running animations on an element — its
/// `active_transitions`. Called through `Reflect` because `Element::get_animations`
/// sits behind web-sys' unstable APIs.
fn running_animations(el: &web_sys::Element) -> u32 {
    let f = js_sys::Reflect::get(el, &JsValue::from_str("getAnimations")).unwrap();
    let f: js_sys::Function = f.dyn_into().unwrap();
    let arr: js_sys::Array = f.call0(el).unwrap().dyn_into().unwrap();
    arr.length()
}

/// The `e` entry of the element's computed `matrix(a, b, c, d, e, f)` — the
/// horizontal translate, in px. `none` reads as 0.
fn translate_x(el: &web_sys::Element) -> f64 {
    let s = web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value("transform")
        .unwrap();
    if !s.starts_with("matrix(") {
        return 0.0;
    }
    let inner = &s["matrix(".len()..s.len() - 1];
    inner
        .split(',')
        .nth(4)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn computed(el: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

/// Resolve on the next animation frame, after style has been recalculated.
async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The element's first running animation — the panel's slide.
fn first_animation(el: &web_sys::Element) -> JsValue {
    let f = js_sys::Reflect::get(el, &JsValue::from_str("getAnimations")).unwrap();
    let f: js_sys::Function = f.dyn_into().unwrap();
    let arr: js_sys::Array = f.call0(el).unwrap().dyn_into().unwrap();
    arr.get(0)
}

/// `v[key]`, or `undefined` when `v` is not an object — so a failure message
/// about a missing animation reports it rather than throwing.
fn prop(v: &JsValue, key: &str) -> JsValue {
    if !v.is_object() {
        return JsValue::UNDEFINED;
    }
    js_sys::Reflect::get(v, &JsValue::from_str(key)).unwrap()
}

/// Call the zero-argument method `name` on `v`.
fn call(v: &JsValue, name: &str) -> JsValue {
    let f: js_sys::Function = prop(v, name).dyn_into().unwrap();
    f.call0(v).unwrap()
}

/// How long to wait for the slide to start moving on the browser's own clock.
///
/// Generous on purpose (issue #945): a loaded CI runner can take longer than
/// the whole 300ms slide to commit the frame that starts it, and nothing below
/// depends on how long that takes. A healthy run moves within a frame or two.
const START_DEADLINE_MS: f64 = 2_000.0;

/// Opening the drawer runs the 300ms slide in Chrome.
#[wasm_bindgen_test]
async fn the_drawer_slides_in() {
    let (opened, panel) = mount_closed_drawer();

    // Preconditions, each keeping the assertions below off a fixed point.
    assert_eq!(
        computed(&panel, "visibility"),
        "hidden",
        "precondition: the closed panel inherits `visibility: hidden` from the \
         root — it is hidden, but it IS being rendered, which is what gives it a \
         before-change style"
    );
    assert_ne!(
        computed(&panel, "display"),
        "none",
        "precondition (#751): nothing in the closed drawer is `display: none`. \
         That spelling is what refused the transition"
    );
    let closed_tx = translate_x(&panel);
    assert!(
        closed_tx < -100.0,
        "precondition: closed, the panel is translated off-screen to the left \
         (a `--md` drawer is 380px wide): got {closed_tx}"
    );

    opened.set(true);
    next_frame().await;

    assert_eq!(
        running_animations(&panel),
        1,
        "the browser is running the panel's 300ms slide. On the `display: none` \
         spelling this is 0: the panel's ancestor stopped being rendered and its \
         own `transform` retargeted in one style pass, and css-transitions-1 §3 \
         starts nothing there"
    );

    let first = translate_x(&panel);
    assert!(
        first < -1.0,
        "one frame in, the panel has not arrived: got {first}, from {closed_tx}. \
         A refused transition reads 0 here"
    );

    // The slide advances on the browser's own clock — waited for, not slept for
    // (issue #945). A `transform` transition is composited, so Chrome holds it
    // *pending* — `currentTime` 0, the panel at its start value — until the
    // compositor commits the frame that starts it. Measured in Chrome 153: after
    // 150ms of busy wall clock plus one task with no frame committed between
    // them, the animation was still `pending` at `currentTime` 0 and the panel
    // still read -380, which is CI's `-380 -> -380` exactly; it moved one frame
    // later. So poll frame by frame up to a deadline no healthy run comes near.
    // On the `display: none` spelling there is no animation and the panel sits
    // at its open position from frame 1, so this never moves and fails at the
    // deadline, as well as at the two assertions above.
    let anim = first_animation(&panel);
    let start = js_sys::Date::now();
    let mut later = translate_x(&panel);
    while later <= first && js_sys::Date::now() - start < START_DEADLINE_MS {
        next_frame().await;
        later = translate_x(&panel);
    }
    assert!(
        later > first,
        "the slide advances: {first} -> {later} (moving right, towards 0) within \
         {START_DEADLINE_MS}ms; the animation is pending={:?} at currentTime={:?}",
        prop(&anim, "pending"),
        prop(&anim, "currentTime"),
    );

    // And it is the 300ms slide, sampled mid-way on the animation's own
    // timeline rather than the wall clock, which a stalled runner can overshoot
    // in either direction: paused and sought to 120ms, the panel is strictly
    // between where it started and where it lands. An instant transition reads
    // 0 here and an unstarted one -380.
    let timing = call(&prop(&anim, "effect"), "getComputedTiming");
    assert_eq!(
        prop(&timing, "duration").as_f64(),
        Some(300.0),
        "the slide is the sheet's 300ms transition"
    );
    call(&anim, "pause");
    js_sys::Reflect::set(
        &anim,
        &JsValue::from_str("currentTime"),
        &JsValue::from_f64(120.0),
    )
    .unwrap();
    let mid = translate_x(&panel);
    assert!(
        closed_tx < mid && mid < 0.0,
        "120ms into the 300ms slide the panel is on its way: got {mid}, strictly \
         between {closed_tx} and 0"
    );
}

/// The closed drawer is out of the way, by the browser's own rules.
///
/// `visibility: hidden` is what replaced `display: none`, so these are the three
/// things that had to survive the change: no hit, no focus, and the panel's
/// ancestor genuinely hidden rather than merely transparent.
#[wasm_bindgen_test]
async fn the_closed_drawer_is_out_of_the_way() {
    let (opened, panel) = mount_closed_drawer();
    let bdoc = browser_document();
    let root = bdoc.query_selector(".rinch-drawer__root").unwrap().unwrap();

    // Off the fixed point: the closed root is a real box covering the point
    // probed below. Under `display: none` it had no box and the probe would
    // have missed it for the wrong reason.
    let rect = root.get_bounding_client_rect();
    assert!(
        rect.width() > 0.0 && rect.height() > 0.0,
        "precondition: the closed root still has a box ({} x {})",
        rect.width(),
        rect.height()
    );

    // 1. It takes no clicks.
    let hit = bdoc.element_from_point(20.0, 200.0);
    let hit_in_drawer = hit
        .as_ref()
        .is_some_and(|el| root.contains(Some(el.unchecked_ref())));
    assert!(
        !hit_in_drawer,
        "a point over the closed drawer hits through it, not into it"
    );

    // 2. Its close button takes no focus.
    let close = bdoc
        .query_selector(".rinch-drawer__close")
        .unwrap()
        .unwrap();
    let close_el: web_sys::HtmlElement = close.clone().dyn_into().unwrap();
    close_el.focus().unwrap();
    assert_ne!(
        bdoc.active_element().map(|e| e.is_same_node(Some(&close))),
        Some(true),
        "the closed drawer's close button refuses focus"
    );

    // 3. Positive controls: all three come back when it opens.
    opened.set(true);
    next_frame().await;

    assert_eq!(
        computed(&panel, "visibility"),
        "visible",
        "positive control: the open panel is visible"
    );
    let hit = bdoc.element_from_point(20.0, 200.0);
    assert!(
        hit.as_ref()
            .is_some_and(|el| root.contains(Some(el.unchecked_ref()))),
        "positive control: the open drawer does take the click"
    );
    close_el.focus().unwrap();
    assert_eq!(
        bdoc.active_element().map(|e| e.is_same_node(Some(&close))),
        Some(true),
        "positive control: the open drawer's close button does take focus"
    );
}
