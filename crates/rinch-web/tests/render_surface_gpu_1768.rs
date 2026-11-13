//! A `RenderSurface`'s GPU render callback in the browser (#1768, the
//! `web-gpu` feature): with a WebGPU adapter it is handed the canvas's current
//! texture at the canvas's device-pixel size, on demand unless the surface is
//! continuous, and follows a resize; without one the surface reports no GPU
//! frames and its ordinary render callback draws instead.
//!
//! Which half runs depends on the browser: headless Chrome and Firefox (CI)
//! have no adapter, a Chrome with a GPU has one. Each test asks the browser
//! first (`navigator.gpu.requestAdapter()`) and states what it saw.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver cargo test -p rinch-web --features web-gpu \
//!   --target wasm32-unknown-unknown --test render_surface_gpu_1768
//! ```
#![cfg(all(target_arch = "wasm32", feature = "web-gpu"))]

use std::cell::{Cell, RefCell};
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

/// Whether this browser has a WebGPU adapter: the question rinch asks before
/// it requests its device.
async fn has_webgpu_adapter() -> bool {
    let nav = web_sys::window().unwrap().navigator();
    let Ok(gpu) = js_sys::Reflect::get(&nav, &"gpu".into()) else {
        return false;
    };
    if gpu.is_undefined() || gpu.is_null() {
        return false;
    }
    let request: js_sys::Function = js_sys::Reflect::get(&gpu, &"requestAdapter".into())
        .unwrap()
        .unchecked_into();
    let Ok(promise) = request.call0(&gpu) else {
        return false;
    };
    match wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise)).await {
        Ok(adapter) => !adapter.is_null() && !adapter.is_undefined(),
        Err(_) => false,
    }
}

/// Frames until `done`, at most `n` (the device arrives asynchronously).
async fn frames_until(n: u32, done: impl Fn() -> bool) {
    for _ in 0..n {
        if done() {
            return;
        }
        next_frame().await;
    }
}

/// The GPU frames a surface drew: each one's size and device generation.
type GpuFrames = Rc<RefCell<Vec<((u32, u32), u64)>>>;

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    gpu_frames: GpuFrames,
    /// The CPU render callback's runs.
    cpu_calls: Rc<Cell<u32>>,
}

/// A 120×80 surface with a GPU render callback that clears its target, and a
/// CPU render callback (the fallback) that submits a frame.
async fn mount(continuous: Option<bool>) -> Mounted {
    let surface = create_render_surface();
    if let Some(on) = continuous {
        surface.set_continuous(on);
    }
    let gpu_frames: GpuFrames = Rc::default();
    {
        let seen = gpu_frames.clone();
        surface.set_gpu_render_callback(move |f| {
            assert_eq!(
                (f.texture.width(), f.texture.height()),
                f.size,
                "the texture is the frame's size"
            );
            let mut encoder = f.device.create_command_encoder(&Default::default());
            encoder.begin_render_pass(&rinch::wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(rinch::wgpu::RenderPassColorAttachment {
                    view: f.target,
                    resolve_target: None,
                    ops: rinch::wgpu::Operations {
                        load: rinch::wgpu::LoadOp::Clear(rinch::wgpu::Color::GREEN),
                        store: rinch::wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            f.queue.submit(Some(encoder.finish()));
            seen.borrow_mut().push((f.size, f.generation));
        });
    }
    let cpu_calls: Rc<Cell<u32>> = Rc::default();
    {
        let c = cpu_calls.clone();
        surface.set_render_callback(move |writer, w, h| {
            c.set(c.get() + 1);
            writer.submit_frame(&vec![255; (w * h * 4) as usize], w, h);
        });
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 10px; top: 10px; width: 120px; height: 80px;",
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
    Mounted {
        root,
        host,
        surface,
        gpu_frames,
        cpu_calls,
    }
}

fn unmount(m: Mounted) {
    m.root.unmount();
    m.host.remove();
}

fn physical(css: f64) -> u32 {
    (css * web_sys::window().unwrap().device_pixel_ratio()).round() as u32
}

#[wasm_bindgen_test]
async fn gpu_frames_are_the_canvas_or_the_fallback_draws() {
    let adapter = has_webgpu_adapter().await;
    let m = mount(None).await;
    let surface = m.surface.clone();
    let cpu = m.cpu_calls.clone();
    let gpu = m.gpu_frames.clone();
    frames_until(120, || {
        surface.gpu_frames_available() && !gpu.borrow().is_empty() || cpu.get() > 0
    })
    .await;
    if adapter {
        assert!(
            m.surface.gpu_frames_available(),
            "an adapter: the device came"
        );
        let seen = m.gpu_frames.borrow().clone();
        assert_eq!(
            seen.first().copied(),
            Some(((physical(120.0), physical(80.0)), 1)),
            "the first frame is the canvas's device-pixel size, on the first device"
        );
        let canvas: web_sys::HtmlCanvasElement = document()
            .get_element_by_id(&format!("rinch-surface-{}", m.surface.id()))
            .unwrap()
            .unchecked_into();
        assert_eq!(
            (canvas.width(), canvas.height()),
            (physical(120.0), physical(80.0)),
            "the canvas's backing store"
        );
        assert_eq!(m.cpu_calls.get(), 0, "the fallback did not run");
    } else {
        assert!(
            !m.surface.gpu_frames_available(),
            "no adapter: no GPU frames"
        );
        assert!(m.cpu_calls.get() > 0, "the render callback drew instead");
        assert!(
            m.gpu_frames.borrow().is_empty(),
            "the GPU callback never ran"
        );
        let canvas: web_sys::HtmlCanvasElement = document()
            .get_element_by_id(&format!("rinch-surface-{}", m.surface.id()))
            .unwrap()
            .unchecked_into();
        assert!(
            canvas.get_context("2d").ok().flatten().is_some(),
            "the canvas was left for the 2D fallback (no WebGPU context claimed it)"
        );
    }
    unmount(m);
}

#[wasm_bindgen_test]
async fn unset_pacing_is_on_demand_and_a_resize_draws() {
    if !has_webgpu_adapter().await {
        // No adapter here (headless): the fallback test above covers this
        // browser; the pacing of the GPU frames needs a device.
        return;
    }
    let m = mount(None).await;
    let gpu = m.gpu_frames.clone();
    frames_until(120, || !gpu.borrow().is_empty()).await;
    frames(5).await;
    assert_eq!(m.gpu_frames.borrow().len(), 1, "one frame, then quiet");

    m.surface.request_frame();
    frames(3).await;
    assert_eq!(m.gpu_frames.borrow().len(), 2, "a request draws one");

    m.host
        .set_attribute(
            "style",
            "position: fixed; left: 10px; top: 10px; width: 150px; height: 60px;",
        )
        .unwrap();
    frames(4).await;
    let seen = m.gpu_frames.borrow().clone();
    assert_eq!(seen.len(), 3, "a resize draws one");
    assert_eq!(
        seen[2].0,
        (physical(150.0), physical(60.0)),
        "at the new size"
    );

    m.surface.set_continuous(true);
    let before = m.gpu_frames.borrow().len();
    frames(5).await;
    assert!(
        m.gpu_frames.borrow().len() >= before + 4,
        "continuous: every frame"
    );
    unmount(m);
}
