//! A `RenderSurface`'s render callback in the browser is paced by
//! `set_continuous` / `request_frame` as on desktop, and `layout_rect()`
//! reports where the canvas is (issue #1768).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test render_surface_pacing_1768
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;

use rinch::render_surface::{RenderSurface, RenderSurfaceHandle, create_render_surface};
use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Resolves at the next animation frame.
async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let cb = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let _ = web_sys::window()
            .unwrap()
            .request_animation_frame(cb.unchecked_ref());
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

async fn frames(n: u32) {
    for _ in 0..n {
        next_frame().await;
    }
}

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    calls: Rc<Cell<u32>>,
}

/// A 120×80 surface at (63, 41) whose callback counts its runs; `pacing` is
/// what `set_continuous` is called with (`None`: not called). The callback
/// requests another frame on its first `ask_again` runs.
async fn mount(pacing: Option<bool>, ask_again: u32) -> Mounted {
    let surface = create_render_surface();
    if let Some(on) = pacing {
        surface.set_continuous(on);
    }
    let calls: Rc<Cell<u32>> = Rc::default();
    {
        let (c, me) = (calls.clone(), surface.clone());
        surface.set_render_callback(move |_writer, _w, _h| {
            c.set(c.get() + 1);
            if c.get() <= ask_again {
                me.request_frame();
            }
        });
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 63px; top: 41px; width: 120px; height: 80px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let mounted = surface.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            RenderSurface {
                surface: Some(mounted.clone()),
            }
            .render(scope, &[])
        },
    );
    // Canvas init (a microtask), the ResizeObserver's first size, first frames.
    frames(4).await;
    Mounted {
        root,
        host,
        surface,
        calls,
    }
}

fn unmount(m: Mounted) {
    m.root.unmount();
    m.host.remove();
}

#[wasm_bindgen_test]
async fn layout_rect_is_where_the_canvas_is() {
    let m = mount(None, 0).await;
    let canvas = document()
        .get_element_by_id(&format!("rinch-surface-{}", m.surface.id()))
        .expect("the surface's canvas");
    let r = canvas.get_bounding_client_rect();
    assert_eq!(
        (r.left(), r.top()),
        (63.0, 41.0),
        "control: the canvas's own rect"
    );
    let (x, y, w, h) = m.surface.layout_rect();
    assert_eq!((x, y), (63.0, 41.0), "layout_rect's position (was (0, 0))");
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    assert_eq!(
        (w, h),
        ((120.0 * dpr).round() as u32, (80.0 * dpr).round() as u32),
        "physical size"
    );
    unmount(m);
}

#[wasm_bindgen_test]
async fn the_original_pacing_runs_every_frame() {
    // Positive control for the on-demand test: the same harness, counting.
    let m = mount(None, 0).await;
    let before = m.calls.get();
    assert!(before >= 1, "it ran at all");
    frames(5).await;
    assert!(
        m.calls.get() >= before + 4,
        "ran {} times in 5 frames",
        m.calls.get() - before
    );
    unmount(m);
}

#[wasm_bindgen_test]
async fn a_continuous_surface_runs_every_frame() {
    let m = mount(Some(true), 0).await;
    let before = m.calls.get();
    frames(5).await;
    assert!(
        m.calls.get() >= before + 4,
        "ran {} times in 5 frames",
        m.calls.get() - before
    );
    unmount(m);
}

#[wasm_bindgen_test]
async fn an_on_demand_surface_runs_once_per_request() {
    let m = mount(Some(false), 0).await;
    assert_eq!(m.calls.get(), 1, "the first size drew once");
    frames(5).await;
    assert_eq!(m.calls.get(), 1, "quiet frames draw nothing");

    m.surface.request_frame();
    m.surface.request_frame();
    frames(3).await;
    assert_eq!(m.calls.get(), 2, "two requests are one frame");

    m.surface.set_continuous(true);
    let before = m.calls.get();
    frames(5).await;
    assert!(
        m.calls.get() >= before + 4,
        "continuous again runs every frame"
    );

    // Back to on demand while the loop is running: its next frame draws
    // nothing, and it stops.
    m.surface.set_continuous(false);
    let before = m.calls.get();
    frames(4).await;
    assert_eq!(m.calls.get(), before, "on demand again: nothing requested");
    unmount(m);
}

#[wasm_bindgen_test]
async fn a_callback_that_requests_a_frame_gets_it() {
    let m = mount(Some(false), 2).await;
    frames(5).await;
    assert_eq!(m.calls.get(), 3, "the first draw and the two it asked for");
    unmount(m);
}

/// A surface with no size (its host `display: none`, so the ResizeObserver
/// reports nothing) does not keep an empty animation-frame loop running for a
/// request; the request draws once the surface gets a size.
#[wasm_bindgen_test]
async fn a_request_without_a_size_runs_no_loop_until_a_size_arrives() {
    let surface = create_render_surface();
    surface.set_continuous(false);
    let calls: Rc<Cell<u32>> = Rc::default();
    {
        let c = calls.clone();
        surface.set_render_callback(move |_writer, _w, _h| c.set(c.get() + 1));
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute("style", "display: none; width: 90px; height: 30px;")
        .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let mounted = surface.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            RenderSurface {
                surface: Some(mounted.clone()),
            }
            .render(scope, &[])
        },
    );
    frames(4).await;
    surface.request_frame();
    assert!(
        surface.__render_loop_running(),
        "control: the request started the loop"
    );
    frames(4).await;
    assert!(
        !surface.__render_loop_running(),
        "no size: the loop stopped"
    );
    assert_eq!(calls.get(), 0, "and nothing drew");

    host.set_attribute("style", "width: 90px; height: 30px;")
        .unwrap();
    frames(4).await;
    assert_eq!(calls.get(), 1, "a size: the request drew once");
    assert!(
        !surface.__render_loop_running(),
        "and the loop stopped again"
    );
    root.unmount();
    host.remove();
}
