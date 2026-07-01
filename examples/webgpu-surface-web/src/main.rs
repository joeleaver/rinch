//! WebGPU `RenderSurface` demo for `rinch-web` (issue #91).
//!
//! Rinch owns the chrome (the left panel) as **real browser DOM**; the app owns
//! the pixels in a `<canvas>` "viewport hole" that rinch sizes, HiDPI-scales,
//! and wires for input. The app creates its own **wgpu WebGPU** surface on that
//! canvas and draws a triangle each animation frame. The browser composites the
//! DOM and the canvas by z-order — there is no rinch compositor on web.
//!
//! This mirrors `examples/game-embed` (desktop) on the web: **desktop** = rinch
//! owns the window and the engine hands it frames; **web** = the engine owns the
//! canvas surface.
//!
//! Build & run:
//! ```bash
//! cd examples/webgpu-surface-web
//! trunk serve --release --port 8080
//! ```
//! Serve over `http://localhost` (or https) so `navigator.gpu` is exposed; the
//! `webgl` cargo feature provides a fallback where WebGPU is unavailable.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, SurfaceEvent, create_render_surface,
};
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// A single full-screen triangle — no vertex buffer, positions baked in the
/// shader (indexed by `vertex_index`).
const TRIANGLE_WGSL: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>( 0.0,  0.6),
        vec2<f32>(-0.6, -0.6),
        vec2<f32>( 0.6, -0.6),
    );
    return vec4<f32>(p[i], 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(0.29, 0.82, 0.60, 1.0);
}
"#;

/// The app-owned wgpu state, built asynchronously once the canvas exists.
struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
}

impl GpuState {
    /// Reconfigure the swapchain to a new **physical** backing size.
    fn reconfigure(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Draw one frame.
    fn render(&self) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(_) => {
                // Lost/outdated surface — reconfigure and try once more.
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return,
                }
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("triangle"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.06,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

/// Probe whether WebGPU is not just *exposed* but actually has a working adapter.
///
/// Some browsers expose `navigator.gpu` yet `requestAdapter()` resolves to `null`
/// (no functional adapter — headless Chrome, software GL, missing Vulkan, …). We
/// must know this *before* handing the canvas to wgpu, because wgpu's WebGPU
/// backend claims a `webgpu` context on the canvas — which then taints it so the
/// WebGL2 fallback can't bind. Probing on a throwaway (canvas-less) adapter
/// request avoids that.
async fn webgpu_has_adapter() -> bool {
    use wasm_bindgen::{JsCast, JsValue};
    // Reached via js_sys/Reflect so we don't need web-sys's WebGPU bindings
    // (which are behind the `web_sys_unstable_apis` cfg).
    let Some(win) = web_sys::window() else {
        return false;
    };
    let navigator = win.navigator();
    let nav: &JsValue = navigator.as_ref();
    let Ok(gpu) = js_sys::Reflect::get(nav, &JsValue::from_str("gpu")) else {
        return false;
    };
    if gpu.is_undefined() || gpu.is_null() {
        return false; // navigator.gpu absent — no WebGPU at all
    }
    // gpu.requestAdapter() -> Promise<GPUAdapter?>
    let Ok(req) = js_sys::Reflect::get(&gpu, &JsValue::from_str("requestAdapter")) else {
        return false;
    };
    let Ok(func) = req.dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(promise) = func.call0(&gpu).and_then(|p| p.dyn_into::<js_sys::Promise>()) else {
        return false;
    };
    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(adapter) => !adapter.is_null() && !adapter.is_undefined(),
        Err(_) => false,
    }
}

/// Build the wgpu stack on the app-owned canvas. Async because `request_adapter`
/// / `request_device` are promises on the web.
async fn init_gpu(canvas: web_sys::HtmlCanvasElement, width: u32, height: u32) -> Option<GpuState> {
    // Prefer WebGPU, but only if it has a real adapter; otherwise use WebGL2 so
    // the canvas isn't tainted by a non-functional `webgpu` context.
    let (backends, backend_name) = if webgpu_has_adapter().await {
        (wgpu::Backends::BROWSER_WEBGPU, "WebGPU")
    } else {
        (wgpu::Backends::GL, "WebGL2")
    };
    log::info!("webgpu-surface-web: rendering via {backend_name}");

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });

    // rinch handed us the raw <canvas>; wgpu takes ownership of it for the
    // surface (so the surface is `'static`).
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .ok()?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .ok()?;

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("webgpu-surface-web"),
            required_features: wgpu::Features::empty(),
            // downlevel_webgl2 limits keep the WebGL fallback valid too.
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        })
        .await
        .ok()?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats[0];

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("triangle"),
        source: wgpu::ShaderSource::Wgsl(TRIANGLE_WGSL.into()),
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("triangle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    });

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &config);

    Some(GpuState {
        surface,
        device,
        queue,
        config,
        pipeline,
    })
}

#[component]
fn app() -> NodeHandle {
    // rinch creates & manages the <canvas>; we render into it ourselves.
    let surface: RenderSurfaceHandle = create_render_surface();

    // Readouts proving events + resize notifications reach the app (real,
    // reactive rinch DOM around the canvas hole).
    let last_event = Signal::new(String::from("(click the canvas, then type)"));
    let backing = Signal::new(String::from("—"));

    // Input over the canvas is delivered here (pointer / wheel / key / focus) —
    // rinch does not swallow it.
    surface.set_event_handler(move |ev| {
        let s = match ev {
            SurfaceEvent::MouseDown { x, y, button } => {
                format!("MouseDown {x:.0},{y:.0} {button:?}")
            }
            SurfaceEvent::MouseUp { x, y, button } => format!("MouseUp {x:.0},{y:.0} {button:?}"),
            SurfaceEvent::MouseMove { x, y } => format!("MouseMove {x:.0},{y:.0}"),
            SurfaceEvent::MouseWheel {
                delta_x, delta_y, ..
            } => format!("Wheel {delta_x:.0},{delta_y:.0}"),
            SurfaceEvent::KeyDown(k) => format!("KeyDown {:?}", k.key),
            SurfaceEvent::KeyUp(k) => format!("KeyUp {:?}", k.key),
            SurfaceEvent::FocusGained => "FocusGained".into(),
            SurfaceEvent::FocusLost => "FocusLost".into(),
            other => format!("{other:?}"),
        };
        last_event.set(s);
    });

    // App-owned wgpu state, filled in asynchronously once the canvas is live.
    let gpu: Rc<RefCell<Option<GpuState>>> = Rc::new(RefCell::new(None));

    // Resize notification (ResizeObserver-driven). Physical px = CSS × dpr, so
    // reconfiguring the wgpu surface here is HiDPI-correct out of the box.
    {
        let gpu = gpu.clone();
        surface.set_resize_callback(move |w, h| {
            backing.set(format!("{w} × {h} px"));
            if let Some(state) = gpu.borrow_mut().as_mut() {
                state.reconfigure(w, h);
            }
        });
    }

    // Per-frame driver via rinch's requestAnimationFrame loop. On the first tick
    // the canvas is already available (stored in a microtask that runs before
    // rAF), so we kick off async GPU init once, then render every frame.
    {
        let gpu = gpu.clone();
        let surface_cb = surface.clone();
        let mut init_started = false;
        surface.set_render_callback(move |_writer, width, height| {
            if let Some(state) = gpu.borrow_mut().as_mut() {
                // Reconcile size in case a resize raced the async init.
                if (width, height) != (state.config.width, state.config.height) {
                    state.reconfigure(width, height);
                }
                state.render();
                return;
            }
            if init_started {
                return; // init in flight
            }
            if let Some(canvas) = surface_cb.canvas_element() {
                init_started = true;
                let gpu = gpu.clone();
                spawn_local(async move {
                    match init_gpu(canvas, width, height).await {
                        Some(state) => *gpu.borrow_mut() = Some(state),
                        None => log::error!(
                            "webgpu-surface-web: no WebGPU/WebGL adapter — is the page served \
                             over localhost/https?"
                        ),
                    }
                });
            }
        });
    }

    rsx! {
        div {
            style: "display:flex; height:100vh; width:100vw; \
                    font-family: system-ui, -apple-system, sans-serif; \
                    color:#e7e7ea; background:#111318;",

            // ── rinch DOM chrome (real DOM, composited AROUND the canvas) ──
            div {
                style: "width:300px; padding:22px; display:flex; flex-direction:column; \
                        gap:14px; background:#181b22; border-right:1px solid #262a33;",
                h1 { style:"margin:0; font-size:18px;", "WebGPU RenderSurface" }
                p {
                    style:"margin:0; font-size:13px; color:#9aa0ab; line-height:1.55;",
                    "This panel is real rinch DOM. The triangle is drawn by the app's own \
                     wgpu WebGPU context into a <canvas> viewport that rinch sizes, \
                     HiDPI-scales, and wires for input."
                }
                div { style:"height:1px; background:#262a33;" }
                div { style:"font-size:12px; color:#9aa0ab;", "Backing size (physical px)" }
                div {
                    style:"font-size:15px; color:#7fd7a6; font-variant-numeric:tabular-nums;",
                    {|| backing.get()}
                }
                div { style:"font-size:12px; color:#9aa0ab; margin-top:6px;", "Last canvas event" }
                div {
                    style:"font-size:15px; color:#7fb2ff; font-variant-numeric:tabular-nums;",
                    {|| last_event.get()}
                }
                div { style:"flex:1;" }
                p {
                    style:"margin:0; font-size:11px; color:#6b7180; line-height:1.5;",
                    "Click the canvas to focus it, then press keys — pointer, wheel, and \
                     keyboard events all flow to the app. Resize the window to see the \
                     backing size track devicePixelRatio."
                }
            }

            // ── the viewport hole ──────────────────────────────────────────
            div {
                style: "flex:1; position:relative; min-width:0;",
                RenderSurface { surface: Some(surface) }
            }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();
    let theme = ThemeProviderProps {
        dark_mode: true,
        ..Default::default()
    };
    rinch_web::mount(theme, app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
