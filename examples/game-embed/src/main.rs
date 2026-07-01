//! Spinning cube with rinch UI overlay — demonstrates the embed API.
//!
//! The game owns the winit window and wgpu device. Rinch provides a HUD-style
//! UI overlay rendered via Vello onto a texture, then composited on top of the
//! cube with a fullscreen blit pass using premultiplied alpha blending.
//!
//! Run: `cargo run -p game-embed`

use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowAttributes, WindowId};

use rinch::prelude::*;
use rinch_platform::{
    KeyCode as PlatformKeyCode, Modifiers as PlatformModifiers, MouseButton as PlatformMouseButton,
    PlatformEvent,
};

// ── GPU types ───────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    tint: [f32; 4],
}

// ── Cube geometry ───────────────────────────────────────────────────────────

fn cube_vertices() -> Vec<Vertex> {
    let p: [[f32; 3]; 8] = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let colors: [[f32; 3]; 6] = [
        [1.0, 0.4, 0.4], // front  — red
        [0.4, 0.8, 1.0], // back   — cyan
        [0.4, 1.0, 0.5], // top    — green
        [1.0, 0.8, 0.3], // bottom — yellow
        [0.8, 0.5, 1.0], // right  — purple
        [1.0, 0.6, 0.3], // left   — orange
    ];
    // face index → 6 vertex indices (two triangles)
    let faces: [[usize; 6]; 6] = [
        [4, 5, 6, 4, 6, 7], // front
        [1, 0, 3, 1, 3, 2], // back
        [3, 7, 6, 3, 6, 2], // top
        [0, 1, 5, 0, 5, 4], // bottom
        [5, 1, 2, 5, 2, 6], // right
        [0, 4, 7, 0, 7, 3], // left
    ];
    let mut verts = Vec::with_capacity(36);
    for (fi, indices) in faces.iter().enumerate() {
        for &i in indices {
            verts.push(Vertex {
                pos: p[i],
                color: colors[fi],
            });
        }
    }
    verts
}

// ── Matrix math ─────────────────────────────────────────────────────────────

type Mat4 = [[f32; 4]; 4];

fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [[0.0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                o[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    o
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y / 2.0).tan();
    let nf = 1.0 / (near - far);
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) * nf, -1.0],
        [0.0, 0.0, 2.0 * far * near * nf, 0.0],
    ]
}

fn translate(x: f32, y: f32, z: f32) -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [x, y, z, 1.0],
    ]
}

fn rotate_y(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rotate_x(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

// ── Shaders ─────────────────────────────────────────────────────────────────

const CUBE_WGSL: &str = "
struct Uniforms { mvp: mat4x4<f32>, tint: vec4<f32> }
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec3<f32> }

@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) color: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.pos = u.mvp * vec4(pos, 1.0);
    o.color = color * u.tint.rgb;
    return o;
}
@fragment fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
";

const BLIT_WGSL: &str = "
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> }

@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // Fullscreen triangle from vertex index
    let x = f32(i32(i) / 2) * 4.0 - 1.0;
    let y = f32(i32(i) % 2) * 4.0 - 1.0;
    var o: VsOut;
    o.pos = vec4(x, y, 0.0, 1.0);
    o.uv = vec2((x + 1.0) / 2.0, 1.0 - (y + 1.0) / 2.0);
    return o;
}
@fragment fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(t, s, in.uv);
}
";

// ── Shared game state (signals created in UI, read by game loop) ────────────

#[derive(Clone, Copy)]
struct GameState {
    bg_color: Signal<[f32; 3]>,
    cube_tint: Signal<[f32; 3]>,
    rotation_speed: Signal<f32>,
    auto_rotate: Signal<bool>,
}

thread_local! {
    static GAME_STATE: Cell<Option<GameState>> = const { Cell::new(None) };
}

// ── UI component ────────────────────────────────────────────────────────────

const SWATCH: &str = "width:32px;height:32px;border-radius:6px;\
    border:2px solid rgba(255,255,255,0.2);cursor:pointer;";

const SMALL_BTN: &str = "width:28px;height:28px;border-radius:6px;\
    border:1px solid rgba(255,255,255,0.2);background:rgba(255,255,255,0.1);\
    color:white;cursor:pointer;font-size:16px;";

#[component]
fn game_ui() -> NodeHandle {
    let bg_color = Signal::new([0.1f32, 0.1, 0.15]);
    let cube_tint = Signal::new([1.0f32, 1.0, 1.0]);
    let rotation_speed = Signal::new(1.0f32);
    let auto_rotate = Signal::new(true);
    let label = Signal::new(String::new());

    // Export signals so the game loop can read them
    GAME_STATE.set(Some(GameState {
        bg_color,
        cube_tint,
        rotation_speed,
        auto_rotate,
    }));

    // Build the control panel with rsx
    let panel = rsx! {
        div {
            style: "position:absolute;top:16px;right:16px;width:260px;\
                    background:rgba(20,20,30,0.88);border-radius:12px;padding:20px;\
                    color:white;border:1px solid rgba(255,255,255,0.08);",

            // ── Title ──
            div {
                style: "font-size:16px;font-weight:bold;margin-bottom:16px;",
                "Game Embed Demo"
            }

            // ── Background color ──
            div {
                style: "margin-bottom:12px;",
                div { style: "font-size:11px;opacity:0.6;margin-bottom:6px;", "Background" }
                div {
                    style: "display:flex;gap:8px;",
                    button {
                        style: {format!("{SWATCH}background:#191926;")},
                        onclick: move || bg_color.set([0.1, 0.1, 0.15]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#0d1a33;")},
                        onclick: move || bg_color.set([0.05, 0.1, 0.2]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#0d261a;")},
                        onclick: move || bg_color.set([0.05, 0.15, 0.1]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#261a14;")},
                        onclick: move || bg_color.set([0.15, 0.1, 0.08]),
                    }
                }
            }

            // ── Cube tint ──
            div {
                style: "margin-bottom:12px;",
                div { style: "font-size:11px;opacity:0.6;margin-bottom:6px;", "Cube Color" }
                div {
                    style: "display:flex;gap:8px;",
                    button {
                        style: {format!("{SWATCH}background:#ffffff;")},
                        onclick: move || cube_tint.set([1.0, 1.0, 1.0]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#ff8080;")},
                        onclick: move || cube_tint.set([1.0, 0.5, 0.5]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#80b3ff;")},
                        onclick: move || cube_tint.set([0.5, 0.7, 1.0]),
                    }
                    button {
                        style: {format!("{SWATCH}background:#80ff99;")},
                        onclick: move || cube_tint.set([0.5, 1.0, 0.6]),
                    }
                }
            }

            // ── Rotation speed ──
            div {
                style: "margin-bottom:12px;",
                div { style: "font-size:11px;opacity:0.6;margin-bottom:6px;", "Rotation Speed" }
                div {
                    style: "display:flex;align-items:center;gap:8px;",
                    button {
                        style: SMALL_BTN,
                        onclick: move || rotation_speed.update(|s| *s = (*s - 0.25).max(0.0)),
                        "\u{2212}" // minus sign
                    }
                    div {
                        style: "flex:1;text-align:center;font-size:14px;",
                        {|| format!("{:.2}x", rotation_speed.get())}
                    }
                    button {
                        style: SMALL_BTN,
                        onclick: move || rotation_speed.update(|s| *s = (*s + 0.25).min(5.0)),
                        "+"
                    }
                }
            }

            // ── Auto-rotate toggle ──
            div {
                style: "margin-bottom:12px;",
                button {
                    style: {|| if auto_rotate.get() {
                        "width:100%;padding:6px 0;border-radius:6px;cursor:pointer;\
                         border:1px solid rgba(76,154,255,0.4);background:rgba(76,154,255,0.25);\
                         color:white;font-size:13px;"
                    } else {
                        "width:100%;padding:6px 0;border-radius:6px;cursor:pointer;\
                         border:1px solid rgba(255,255,255,0.2);background:rgba(255,255,255,0.08);\
                         color:rgba(255,255,255,0.6);font-size:13px;"
                    }},
                    onclick: move || auto_rotate.update(|v| *v = !*v),
                    {|| if auto_rotate.get() {
                        String::from("\u{2713} Auto-rotate")
                    } else {
                        String::from("Auto-rotate")
                    }}
                }
            }

            // ── Text input (proves wants_keyboard routing) ──
            div {
                style: "margin-bottom:16px;",
                div { style: "font-size:11px;opacity:0.6;margin-bottom:6px;", "Label (keyboard test)" }
                TextInput {
                    placeholder: "Type here\u{2026}",
                    value_fn: move || label.get(),
                    oninput: move |val: String| label.set(val),
                }
            }

            // ── Help text ──
            div {
                style: "font-size:11px;opacity:0.45;line-height:1.5;",
                "Drag to rotate \u{00b7} Space: toggle spin"
                br {}
                "R: reset rotation \u{00b7} Scroll: zoom"
            }
        }
    };

    rsx! {
        div {
            style: "width:100%;height:100%;position:relative;",
            div {
                data-viewport: "main",
                style: "position:absolute;top:0;left:0;width:100%;height:100%;",
            }
            {panel}
        }
    }
}

// ── Key translation (winit → platform) ──────────────────────────────────────

fn translate_key(key: winit::keyboard::KeyCode) -> PlatformKeyCode {
    use winit::keyboard::KeyCode as WK;
    match key {
        WK::ArrowLeft => PlatformKeyCode::ArrowLeft,
        WK::ArrowRight => PlatformKeyCode::ArrowRight,
        WK::ArrowUp => PlatformKeyCode::ArrowUp,
        WK::ArrowDown => PlatformKeyCode::ArrowDown,
        WK::Home => PlatformKeyCode::Home,
        WK::End => PlatformKeyCode::End,
        WK::PageUp => PlatformKeyCode::PageUp,
        WK::PageDown => PlatformKeyCode::PageDown,
        WK::Enter | WK::NumpadEnter => PlatformKeyCode::Enter,
        WK::Backspace => PlatformKeyCode::Backspace,
        WK::Delete => PlatformKeyCode::Delete,
        WK::Tab => PlatformKeyCode::Tab,
        WK::Escape => PlatformKeyCode::Escape,
        WK::Space => PlatformKeyCode::Space,
        WK::KeyA => PlatformKeyCode::KeyA,
        WK::KeyB => PlatformKeyCode::KeyB,
        WK::KeyC => PlatformKeyCode::KeyC,
        WK::KeyD => PlatformKeyCode::KeyD,
        WK::KeyE => PlatformKeyCode::KeyE,
        WK::KeyF => PlatformKeyCode::KeyF,
        WK::KeyG => PlatformKeyCode::KeyG,
        WK::KeyH => PlatformKeyCode::KeyH,
        WK::KeyI => PlatformKeyCode::KeyI,
        WK::KeyJ => PlatformKeyCode::KeyJ,
        WK::KeyK => PlatformKeyCode::KeyK,
        WK::KeyL => PlatformKeyCode::KeyL,
        WK::KeyM => PlatformKeyCode::KeyM,
        WK::KeyN => PlatformKeyCode::KeyN,
        WK::KeyO => PlatformKeyCode::KeyO,
        WK::KeyP => PlatformKeyCode::KeyP,
        WK::KeyQ => PlatformKeyCode::KeyQ,
        WK::KeyR => PlatformKeyCode::KeyR,
        WK::KeyS => PlatformKeyCode::KeyS,
        WK::KeyT => PlatformKeyCode::KeyT,
        WK::KeyU => PlatformKeyCode::KeyU,
        WK::KeyV => PlatformKeyCode::KeyV,
        WK::KeyW => PlatformKeyCode::KeyW,
        WK::KeyX => PlatformKeyCode::KeyX,
        WK::KeyY => PlatformKeyCode::KeyY,
        WK::KeyZ => PlatformKeyCode::KeyZ,
        WK::Digit0 => PlatformKeyCode::Digit0,
        WK::Digit1 => PlatformKeyCode::Digit1,
        WK::Digit2 => PlatformKeyCode::Digit2,
        WK::Digit3 => PlatformKeyCode::Digit3,
        WK::Digit4 => PlatformKeyCode::Digit4,
        WK::Digit5 => PlatformKeyCode::Digit5,
        WK::Digit6 => PlatformKeyCode::Digit6,
        WK::Digit7 => PlatformKeyCode::Digit7,
        WK::Digit8 => PlatformKeyCode::Digit8,
        WK::Digit9 => PlatformKeyCode::Digit9,
        WK::F1 => PlatformKeyCode::F1,
        WK::F2 => PlatformKeyCode::F2,
        WK::F3 => PlatformKeyCode::F3,
        WK::F4 => PlatformKeyCode::F4,
        WK::F5 => PlatformKeyCode::F5,
        WK::F6 => PlatformKeyCode::F6,
        WK::F7 => PlatformKeyCode::F7,
        WK::F8 => PlatformKeyCode::F8,
        WK::F9 => PlatformKeyCode::F9,
        WK::F10 => PlatformKeyCode::F10,
        WK::F11 => PlatformKeyCode::F11,
        WK::F12 => PlatformKeyCode::F12,
        WK::Equal => PlatformKeyCode::Equal,
        WK::Minus => PlatformKeyCode::Minus,
        _ => PlatformKeyCode::Other,
    }
}

fn translate_modifiers(m: winit::keyboard::ModifiersState) -> PlatformModifiers {
    PlatformModifiers {
        shift: m.shift_key(),
        ctrl: m.control_key(),
        alt: m.alt_key(),
        meta: m.meta_key(),
    }
}

// ── Application ─────────────────────────────────────────────────────────────

struct App {
    // winit
    window: Option<Arc<dyn Window>>,

    // wgpu core
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: wgpu::TextureFormat,
    surface_config: Option<wgpu::SurfaceConfiguration>,

    // cube pipeline
    cube_pipeline: Option<wgpu::RenderPipeline>,
    cube_vbuf: Option<wgpu::Buffer>,
    cube_ubuf: Option<wgpu::Buffer>,
    cube_bind_group: Option<wgpu::BindGroup>,
    depth_view: Option<wgpu::TextureView>,

    // blit pipeline (composites UI over cube)
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_layout: Option<wgpu::BindGroupLayout>,
    blit_sampler: Option<wgpu::Sampler>,

    // rinch embed
    rinch_ctx: Option<RinchContext>,
    overlay: Option<RinchOverlayRenderer>,

    // game state
    rot_y: f32,
    rot_x: f32,
    zoom: f32,
    dragging: bool,
    mouse_phys: (f32, f32),
    prev_mouse: (f32, f32),
    modifiers: winit::keyboard::ModifiersState,
    last_frame: Instant,

    // collected per-frame events for rinch
    pending_events: Vec<PlatformEvent>,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            device: None,
            queue: None,
            surface: None,
            surface_format: wgpu::TextureFormat::Bgra8Unorm,
            surface_config: None,
            cube_pipeline: None,
            cube_vbuf: None,
            cube_ubuf: None,
            cube_bind_group: None,
            depth_view: None,
            blit_pipeline: None,
            blit_layout: None,
            blit_sampler: None,
            rinch_ctx: None,
            overlay: None,
            rot_y: 0.4,
            rot_x: 0.3,
            zoom: 3.0,
            dragging: false,
            mouse_phys: (0.0, 0.0),
            prev_mouse: (0.0, 0.0),
            modifiers: winit::keyboard::ModifiersState::empty(),
            last_frame: Instant::now(),
            pending_events: Vec::new(),
        }
    }

    fn scale_factor(&self) -> f64 {
        self.window
            .as_ref()
            .map(|w: &Arc<dyn Window>| w.scale_factor())
            .unwrap_or(1.0)
    }

    fn size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w: &Arc<dyn Window>| {
                let s = w.surface_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((800, 600))
    }

    fn logical_mouse(&self) -> (f32, f32) {
        let sf = self.scale_factor() as f32;
        (self.mouse_phys.0 / sf, self.mouse_phys.1 / sf)
    }

    // ── Create depth texture ────────────────────────────────────────────────

    fn create_depth_texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    // ── Full GPU init ───────────────────────────────────────────────────────

    fn init_gpu(&mut self) {
        let window = self.window.as_ref().unwrap();
        let (w, h) = self.size();

        // Instance → adapter → device
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("No suitable GPU adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
            trace: Default::default(),
            experimental_features: Default::default(),
        }))
        .expect("Failed to create device");

        // Configure surface
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);
        self.surface_format = format;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // ── Cube pipeline ───────────────────────────────────────────────────

        let cube_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cube"),
            source: wgpu::ShaderSource::Wgsl(CUBE_WGSL.into()),
        });

        let cube_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let cube_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&cube_bind_layout],
            push_constant_ranges: &[],
        });

        let cube_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cube"),
            layout: Some(&cube_pipe_layout),
            vertex: wgpu::VertexState {
                module: &cube_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &cube_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let verts = cube_vertices();
        let cube_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube verts"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniforms = Uniforms {
            mvp: [[0.0; 4]; 4],
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        let cube_ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let cube_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &cube_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cube_ubuf.as_entire_binding(),
            }],
        });

        let depth_view = Self::create_depth_texture(&device, w, h);

        // ── Blit pipeline ───────────────────────────────────────────────────

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blit_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&blit_layout],
            push_constant_ranges: &[],
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&blit_pipe_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ── Rinch embed ─────────────────────────────────────────────────────

        let mut rinch_ctx = RinchContext::new(
            RinchContextConfig {
                width: w,
                height: h,
                scale_factor: self.scale_factor(),
                theme: Some(ThemeProviderProps {
                    dark_mode: true,
                    primary_color: Some("blue".into()),
                    ..Default::default()
                }),
            },
            game_ui,
        );

        // Attach debug server (game loop is polling, so no-op notify is fine)
        let _ = rinch_ctx.attach_debug("game-embed", || {});

        let overlay = RinchOverlayRenderer::new(&device, w, h, wgpu::TextureFormat::Rgba8Unorm);

        // Store everything
        self.surface = Some(surface);
        self.surface_config = Some(config);
        self.device = Some(device);
        self.queue = Some(queue);
        self.cube_pipeline = Some(cube_pipeline);
        self.cube_vbuf = Some(cube_vbuf);
        self.cube_ubuf = Some(cube_ubuf);
        self.cube_bind_group = Some(cube_bind_group);
        self.depth_view = Some(depth_view);
        self.blit_pipeline = Some(blit_pipeline);
        self.blit_layout = Some(blit_layout);
        self.blit_sampler = Some(blit_sampler);
        self.rinch_ctx = Some(rinch_ctx);
        self.overlay = Some(overlay);
    }

    // ── Render one frame ────────────────────────────────────────────────────

    fn render(&mut self) {
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let surface = self.surface.as_ref().unwrap();

        // Acquire surface texture
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                // Reconfigure and retry next frame
                if let Some(cfg) = &self.surface_config {
                    surface.configure(device, cfg);
                }
                return;
            }
            Err(e) => {
                eprintln!("Surface error: {e:?}");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Read game state from signals
        let gs = GAME_STATE.get().unwrap();
        let bg = gs.bg_color.get();
        let tint = gs.cube_tint.get();
        let speed = gs.rotation_speed.get();
        let auto_rot = gs.auto_rotate.get();

        // Animate rotation
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        if auto_rot {
            self.rot_y += speed * dt;
        }

        // Build MVP matrix
        let (w, h) = self.size();
        let aspect = w as f32 / h as f32;
        let proj = perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let view_mat = translate(0.0, 0.0, -self.zoom);
        // mat4_mul(a, b) computes B*A in standard math (column-major storage)
        let model = mat4_mul(&rotate_x(self.rot_x), &rotate_y(self.rot_y));
        let mv = mat4_mul(&model, &view_mat);
        let mvp = mat4_mul(&mv, &proj);

        let uniforms = Uniforms {
            mvp,
            tint: [tint[0], tint[1], tint[2], 1.0],
        };
        queue.write_buffer(
            self.cube_ubuf.as_ref().unwrap(),
            0,
            bytemuck::bytes_of(&uniforms),
        );

        // ── Pass 1: Cube ────────────────────────────────────────────────────

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0] as f64,
                            g: bg[1] as f64,
                            b: bg[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth_view.as_ref().unwrap(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(self.cube_pipeline.as_ref().unwrap());
            pass.set_bind_group(0, self.cube_bind_group.as_ref().unwrap(), &[]);
            pass.set_vertex_buffer(0, self.cube_vbuf.as_ref().unwrap().slice(..));
            pass.draw(0..36, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));

        // ── Rinch UI update + render to texture ─────────────────────────────

        let ctx = self.rinch_ctx.as_mut().unwrap();
        let events: Vec<_> = self.pending_events.drain(..).collect();
        let _actions = ctx.update(&events);
        let scene = ctx.scene();

        let overlay = self.overlay.as_mut().unwrap();
        let overlay_view = overlay.render(device, queue, scene);

        // ── Pass 2: Blit (composite UI over cube) ───────────────────────────

        let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: self.blit_layout.as_ref().unwrap(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&overlay_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(self.blit_sampler.as_ref().unwrap()),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(self.blit_pipeline.as_ref().unwrap());
            pass.set_bind_group(0, &blit_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));

        // Process debug commands (screenshots need the surface texture before present)
        let screenshot_reqs = self.rinch_ctx.as_mut().unwrap().process_debug_commands();
        if !screenshot_reqs.is_empty() {
            let fmt = self.surface_format;
            for req in screenshot_reqs {
                match rinch::embed::capture_texture_rgba(device, queue, &frame.texture, w, h, fmt) {
                    Ok(rgba) => req.respond(w, h, rgba),
                    Err(e) => req.fail(e),
                }
            }
        }

        frame.present();
    }

    // ── Handle resize ───────────────────────────────────────────────────────

    fn handle_resize(&mut self, w: u32, h: u32) {
        let w = w.max(1);
        let h = h.max(1);
        if let Some(cfg) = &mut self.surface_config {
            cfg.width = w;
            cfg.height = h;
            if let (Some(device), Some(surface)) = (&self.device, &self.surface) {
                surface.configure(device, cfg);
                self.depth_view = Some(Self::create_depth_texture(device, w, h));
                if let Some(overlay) = &mut self.overlay {
                    overlay.resize(device, w, h);
                }
            }
        }
        if let Some(ctx) = &mut self.rinch_ctx {
            ctx.resize(w, h);
        }
    }
}

// ── ApplicationHandler ──────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return; // already initialized
        }
        let attrs = WindowAttributes::default()
            .with_title("Game Embed Demo")
            .with_surface_size(winit::dpi::LogicalSize::new(1024u32, 720));
        let window: Arc<dyn Window> = Arc::from(event_loop.create_window(attrs).unwrap());
        self.window = Some(window);

        self.init_gpu();
        self.last_frame = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                self.handle_resize(size.width, size.height);
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(ctx) = &mut self.rinch_ctx {
                    ctx.set_scale_factor(scale_factor);
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
                self.pending_events
                    .push(PlatformEvent::ModifiersChanged(translate_modifiers(
                        self.modifiers,
                    )));
            }

            WindowEvent::PointerMoved { position, .. } => {
                let px = position.x as f32;
                let py = position.y as f32;

                // If dragging the cube, update rotation from mouse delta
                if self.dragging {
                    let dx = px - self.prev_mouse.0;
                    let dy = py - self.prev_mouse.1;
                    self.rot_y += dx * 0.005;
                    self.rot_x += dy * 0.005;
                }

                self.prev_mouse = self.mouse_phys;
                self.mouse_phys = (px, py);

                // Always send mouse move to rinch (for hover effects)
                self.pending_events
                    .push(PlatformEvent::MouseMove { x: px, y: py });
            }

            WindowEvent::PointerButton { state, button, .. } => {
                let platform_btn = match button {
                    winit::event::ButtonSource::Mouse(winit::event::MouseButton::Left) => {
                        PlatformMouseButton::Left
                    }
                    winit::event::ButtonSource::Mouse(winit::event::MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(winit::event::MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let (px, py) = self.mouse_phys;

                match state {
                    ElementState::Pressed => {
                        let (lx, ly) = self.logical_mouse();
                        let wants_ui = self
                            .rinch_ctx
                            .as_ref()
                            .is_some_and(|ctx| ctx.wants_mouse(lx, ly));

                        if wants_ui {
                            self.pending_events.push(PlatformEvent::MouseDown {
                                x: px,
                                y: py,
                                button: platform_btn,
                            });
                        } else if platform_btn == PlatformMouseButton::Left {
                            self.dragging = true;
                        }
                    }
                    ElementState::Released => {
                        if self.dragging && platform_btn == PlatformMouseButton::Left {
                            self.dragging = false;
                        } else {
                            self.pending_events.push(PlatformEvent::MouseUp {
                                x: px,
                                y: py,
                                button: platform_btn,
                            });
                        }
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_y = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64 * 40.0,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y,
                };
                let (lx, ly) = self.logical_mouse();
                let wants_ui = self
                    .rinch_ctx
                    .as_ref()
                    .is_some_and(|ctx| ctx.wants_mouse(lx, ly));

                if wants_ui {
                    let (px, py) = self.mouse_phys;
                    self.pending_events.push(PlatformEvent::MouseWheel {
                        x: px,
                        y: py,
                        delta_x: 0.0,
                        delta_y: scroll_y,
                    });
                } else {
                    // Zoom the camera
                    self.zoom = (self.zoom - scroll_y as f32 * 0.01).clamp(1.5, 10.0);
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let key_code = match event.physical_key {
                    PhysicalKey::Code(k) => translate_key(k),
                    _ => PlatformKeyCode::Other,
                };
                let text = event.text.as_ref().map(|s| s.to_string());
                let mods = translate_modifiers(self.modifiers);

                let wants_kb = self
                    .rinch_ctx
                    .as_ref()
                    .is_some_and(|ctx| ctx.wants_keyboard());

                if wants_kb {
                    self.pending_events.push(PlatformEvent::KeyDown {
                        key: key_code,
                        logical_key: None,
                        text,
                        modifiers: mods,
                    });
                } else {
                    // Game keyboard shortcuts
                    match key_code {
                        PlatformKeyCode::Space => {
                            if let Some(gs) = GAME_STATE.get() {
                                gs.auto_rotate.update(|v| *v = !*v);
                            }
                        }
                        PlatformKeyCode::KeyR => {
                            self.rot_x = 0.3;
                            self.rot_y = 0.4;
                            self.zoom = 3.0;
                        }
                        _ => {}
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.render();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let app = App::new();
    event_loop.run_app(app).unwrap();
}
