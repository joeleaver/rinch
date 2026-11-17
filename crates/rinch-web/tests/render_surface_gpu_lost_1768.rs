//! A `RenderSurface` whose WebGPU device is gone for good (#1768, the
//! `web-gpu` feature): after its canvas drew GPU frames and the device was
//! `destroy()`ed, the render callback's frames reach the canvas.
//!
//! A canvas keeps the first kind of context made on it, so the one that held
//! the WebGPU context refuses the fallback's 2D context; rinch gives the
//! surface a fresh canvas (`renew_gpu_canvas`). In a file of its own: the
//! page's device is page-global, and this test ends it for the page.
//!
//! It needs a WebGPU adapter, so headless CI (none) passes it trivially. Run it
//! against SwiftShader's WebGPU with a `webdriver.json` beside this crate's
//! `Cargo.toml` (not committed):
//!
//! ```json
//! { "goog:chromeOptions": { "args": ["--enable-unsafe-webgpu",
//!     "--enable-features=Vulkan", "--use-webgpu-adapter=swiftshader"] } }
//! ```
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver cargo test -p rinch-web --features web-gpu \
//!   --target wasm32-unknown-unknown --test render_surface_gpu_lost_1768
//! ```
//!
//! Headless Chromium 153 with SwiftShader loses every device a few frames
//! after it is made, with reason `destroyed`, a page that never calls
//! `destroy()` included (a plain-JavaScript device too). So there the loss can
//! come before this test's own `destroy()`; either way the device is gone
//! after the canvas drew a GPU frame, which is the case this pins.
#![cfg(all(target_arch = "wasm32", feature = "web-gpu"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::render_surface::{RenderSurface, create_render_surface};
use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
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

/// Frames until `done`, at most `n`.
async fn frames_until(n: u32, done: impl Fn() -> bool) {
    for _ in 0..n {
        if done() {
            return;
        }
        next_frame().await;
    }
}

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

/// The surface's canvas, looked up again: it may be a new element.
fn canvas_of(id: usize) -> web_sys::HtmlCanvasElement {
    document()
        .get_element_by_id(&format!("rinch-surface-{id}"))
        .expect("the surface's canvas is in the page")
        .unchecked_into()
}

/// The canvas's centre pixel through a 2D context, or `None` when the canvas
/// will not take one.
fn centre_pixel(canvas: &web_sys::HtmlCanvasElement) -> Option<[u8; 4]> {
    let ctx: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok().flatten()?.unchecked_into();
    let (x, y) = (canvas.width() / 2, canvas.height() / 2);
    let data = ctx
        .get_image_data(x as f64, y as f64, 1.0, 1.0)
        .ok()?
        .data();
    Some([data[0], data[1], data[2], data[3]])
}

const FALLBACK: [u8; 4] = [255, 0, 0, 255];

#[wasm_bindgen_test]
async fn a_destroyed_device_hands_the_canvas_to_the_fallback() {
    let adapter = has_webgpu_adapter().await;
    console_log!("render_surface_gpu_lost_1768: WebGPU adapter: {adapter}");
    if !adapter {
        // No adapter (headless CI): no device to destroy. The fallback of a
        // page without WebGPU is pinned in `render_surface_gpu_1768.rs`.
        return;
    }
    let surface = create_render_surface();
    let device: Rc<RefCell<Option<rinch::wgpu::Device>>> = Rc::default();
    let gpu_frames: Rc<Cell<u32>> = Rc::default();
    {
        let (device, gpu_frames) = (device.clone(), gpu_frames.clone());
        surface.set_gpu_render_callback(move |f| {
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
            *device.borrow_mut() = Some(f.device.clone());
            gpu_frames.set(gpu_frames.get() + 1);
        });
    }
    let cpu_frames: Rc<Cell<u32>> = Rc::default();
    {
        let cpu_frames = cpu_frames.clone();
        surface.set_render_callback(move |writer, w, h| {
            cpu_frames.set(cpu_frames.get() + 1);
            let pixels: Vec<u8> = FALLBACK
                .iter()
                .copied()
                .cycle()
                .take((w * h * 4) as usize)
                .collect();
            writer.submit_frame(&pixels, w, h);
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

    // The positive control: the device came and the canvas drew a GPU frame.
    let frames = gpu_frames.clone();
    frames_until(600, || frames.get() > 0).await;
    assert!(gpu_frames.get() > 0, "an adapter: a GPU frame was drawn");
    assert_eq!(cpu_frames.get(), 0, "the fallback did not run yet");
    let before = canvas_of(surface.id());
    let parent = before.parent_element().expect("the canvas is in the page");
    assert!(
        before.get_context("2d").ok().flatten().is_none(),
        "the canvas that drew GPU frames refuses a 2D context"
    );

    device.borrow().as_ref().unwrap().destroy();
    let s = surface.clone();
    frames_until(600, || {
        !s.gpu_frames_available() && centre_pixel(&canvas_of(s.id())) == Some(FALLBACK)
    })
    .await;

    assert!(
        !surface.gpu_frames_available(),
        "a destroyed device: no GPU frames"
    );
    assert!(cpu_frames.get() > 0, "the render callback draws instead");
    let after = canvas_of(surface.id());
    assert_eq!(
        centre_pixel(&after),
        Some(FALLBACK),
        "the render callback's pixels reach the canvas"
    );
    assert!(
        after.get_attribute("data-render-surface").as_deref()
            == Some(surface.id().to_string().as_str()),
        "the new canvas keeps the surface's attributes"
    );
    assert!(
        !after.is_same_node(Some(&before)),
        "a new canvas, since the old one cannot take a 2D context"
    );
    assert!(
        after
            .parent_element()
            .is_some_and(|p| p.is_same_node(Some(&parent))),
        "in the same place"
    );

    root.unmount();
    assert!(
        document()
            .get_element_by_id(&format!("rinch-surface-{}", surface.id()))
            .is_none(),
        "unmounting removes the new canvas"
    );
    host.remove();
}
