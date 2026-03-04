//! RenderSurface section — demonstrates embedding custom renderers.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch::prelude::*;
use rinch::render_surface::{SurfaceEvent, SurfaceMouseButton, create_render_surface};

// ── 3D Math Helpers ─────────────────────────────────────────────────────────

/// Unit cube vertices.
const CUBE_VERTS: [[f32; 3]; 8] = [
    [-1.0, -1.0, -1.0], // 0: left  bottom back
    [1.0, -1.0, -1.0],  // 1: right bottom back
    [1.0, 1.0, -1.0],   // 2: right top    back
    [-1.0, 1.0, -1.0],  // 3: left  top    back
    [-1.0, -1.0, 1.0],  // 4: left  bottom front
    [1.0, -1.0, 1.0],   // 5: right bottom front
    [1.0, 1.0, 1.0],    // 6: right top    front
    [-1.0, 1.0, 1.0],   // 7: left  top    front
];

/// 6 faces as quads (4 vertex indices each, CCW from outside).
const CUBE_FACES: [([usize; 4], [u8; 4]); 6] = [
    ([4, 5, 6, 7], [66, 133, 244, 255]),  // front  — blue
    ([1, 0, 3, 2], [234, 67, 53, 255]),   // back   — red
    ([0, 4, 7, 3], [52, 168, 83, 255]),   // left   — green
    ([5, 1, 2, 6], [251, 188, 4, 255]),   // right  — yellow
    ([7, 6, 2, 3], [255, 255, 255, 255]), // top    — white
    ([0, 1, 5, 4], [255, 109, 0, 255]),   // bottom — orange
];

fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

/// A projected vertex: screen x/y and eye-space Z for depth testing.
#[derive(Copy, Clone)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32, // eye-space z before projection (used for depth)
}

/// Perspective project with correct aspect ratio.
fn project(v: [f32; 3], w: f32, h: f32, zoom: f32) -> Vertex {
    let fov = 4.0;
    let d = v[2] + fov; // distance along view axis (always positive for visible geometry)
    let scale = (fov * zoom) / d;
    let base = w.min(h) * 0.5;
    Vertex {
        x: w * 0.5 + v[0] * scale * base,
        y: h * 0.5 - v[1] * scale * base,
        z: v[2], // raw eye-space z: front face = +1, back face = -1
    }
}

/// Rasterize a triangle with per-pixel Z-buffer depth testing.
/// No backface culling — the Z-buffer handles all visibility.
#[allow(clippy::too_many_arguments)]
fn fill_triangle_zbuf(
    buf: &mut [u8],
    zbuf: &mut [f32],
    w: i32,
    h: i32,
    v0: Vertex,
    v1: Vertex,
    v2: Vertex,
    color: [u8; 4],
) {
    let min_x = v0.x.min(v1.x).min(v2.x).max(0.0) as i32;
    let max_x = v0.x.max(v1.x).max(v2.x).min((w - 1) as f32) as i32;
    let min_y = v0.y.min(v1.y).min(v2.y).max(0.0) as i32;
    let max_y = v0.y.max(v1.y).max(v2.y).min((h - 1) as f32) as i32;

    let edge = |ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32| -> f32 {
        (bx - ax) * (py - ay) - (by - ay) * (px - ax)
    };

    let area = edge(v0.x, v0.y, v1.x, v1.y, v2.x, v2.y);
    if area.abs() < 0.001 {
        return; // degenerate
    }
    let inv_area = 1.0 / area;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = edge(v1.x, v1.y, v2.x, v2.y, px, py);
            let w1 = edge(v2.x, v2.y, v0.x, v0.y, px, py);
            let w2 = edge(v0.x, v0.y, v1.x, v1.y, px, py);

            // Accept pixel if all edge values have the same sign as area
            let inside = if area > 0.0 {
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            } else {
                w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
            };

            if inside {
                let b0 = (w0 * inv_area).abs();
                let b1 = (w1 * inv_area).abs();
                let b2 = (w2 * inv_area).abs();
                // Interpolate eye-space z. For a cube face (planar), this is exact.
                // Front face z ≈ +1, back face z ≈ -1.
                let z = b0 * v0.z + b1 * v1.z + b2 * v2.z;

                let pi = (y * w + x) as usize;
                // Smaller z = closer to camera (camera at z=-fov, looking toward +Z)
                if z < zbuf[pi] {
                    zbuf[pi] = z;
                    let idx = pi * 4;
                    buf[idx] = color[0];
                    buf[idx + 1] = color[1];
                    buf[idx + 2] = color[2];
                    buf[idx + 3] = color[3];
                }
            }
        }
    }
}

/// Rasterize a quad as two Z-buffered triangles.
fn fill_quad_zbuf(
    buf: &mut [u8],
    zbuf: &mut [f32],
    w: i32,
    h: i32,
    v: [Vertex; 4],
    color: [u8; 4],
) {
    fill_triangle_zbuf(buf, zbuf, w, h, v[0], v[1], v[2], color);
    fill_triangle_zbuf(buf, zbuf, w, h, v[0], v[2], v[3], color);
}

// ── Cube State ──────────────────────────────────────────────────────────────

struct CubeState {
    angle_x: f32,
    angle_y: f32,
    zoom: f32,
    dragging: bool,
    last_mouse: (f32, f32),
}

impl Default for CubeState {
    fn default() -> Self {
        Self {
            angle_x: 0.4,
            angle_y: 0.0,
            zoom: 1.0,
            dragging: false,
            last_mouse: (0.0, 0.0),
        }
    }
}

// ── Section ─────────────────────────────────────────────────────────────────

#[component]
pub fn render_surface_section() -> NodeHandle {
    let spinning = Signal::new(true);
    let speed = Signal::new(1.0_f64);

    // ── Solid Cube Demo ─────────────────────────────────────────────────

    let cube_surface = create_render_surface();
    let cube_state = Arc::new(Mutex::new(CubeState::default()));

    // Event handler for mouse interaction
    {
        let state = cube_state.clone();
        cube_surface.set_event_handler(move |event| {
            let mut s = state.lock().unwrap();
            match event {
                SurfaceEvent::MouseDown {
                    x,
                    y,
                    button: SurfaceMouseButton::Left,
                } => {
                    s.dragging = true;
                    s.last_mouse = (x, y);
                }
                SurfaceEvent::MouseUp {
                    button: SurfaceMouseButton::Left,
                    ..
                } => {
                    s.dragging = false;
                }
                SurfaceEvent::MouseMove { x, y } if s.dragging => {
                    let dx = x - s.last_mouse.0;
                    let dy = y - s.last_mouse.1;
                    s.angle_y += dx * 0.01;
                    s.angle_x += dy * 0.01;
                    s.last_mouse = (x, y);
                }
                SurfaceEvent::MouseWheel { delta_y, .. } => {
                    s.zoom = (s.zoom - delta_y * 0.001).clamp(0.3, 3.0);
                }
                _ => {}
            }
        });
    }

    // Render callback — runs every frame
    {
        let state = cube_state.clone();
        cube_surface.set_render_callback(move |writer, w, h| {
            let is_spinning = spinning.get();
            let spd = speed.get() as f32;

            let mut s = state.lock().unwrap();
            if is_spinning {
                s.angle_y += 0.02 * spd;
            }
            let angle_x = s.angle_x;
            let angle_y = s.angle_y;
            let zoom = s.zoom;
            drop(s);

            let wf = w as f32;
            let hf = h as f32;
            let w_i = w as i32;
            let h_i = h as i32;
            let npixels = (w * h) as usize;
            let mut buf = vec![0u8; npixels * 4];
            let mut zbuf = vec![f32::INFINITY; npixels];

            // Dark background
            for pixel in buf.chunks_exact_mut(4) {
                pixel[0] = 24;
                pixel[1] = 24;
                pixel[2] = 32;
                pixel[3] = 255;
            }

            // Transform and project all vertices
            let verts: Vec<Vertex> = CUBE_VERTS
                .iter()
                .map(|&v| {
                    let v = rotate_x(v, angle_x);
                    let v = rotate_y(v, angle_y);
                    project(v, wf, hf, zoom)
                })
                .collect();

            // Draw all 6 faces — Z-buffer resolves visibility per pixel
            for &(ref indices, color) in &CUBE_FACES {
                let quad = [
                    verts[indices[0]],
                    verts[indices[1]],
                    verts[indices[2]],
                    verts[indices[3]],
                ];
                fill_quad_zbuf(&mut buf, &mut zbuf, w_i, h_i, quad, color);
            }

            writer.submit_frame(&buf, w, h);
        });
    }

    // ── Animated Gradient Demo ───────────────────────────────────────────

    let gradient_surface = create_render_surface();
    {
        let frame = Rc::new(Cell::new(0u64));
        gradient_surface.set_render_callback(move |writer, w, h| {
            let f = frame.get();
            frame.set(f + 1);
            let t = f as f32 * 0.02;

            let size = (w * h * 4) as usize;
            let mut buf = vec![0u8; size];

            for y in 0..h {
                for x in 0..w {
                    let u = x as f32 / w as f32;
                    let v = y as f32 / h as f32;
                    let r = ((u + t).sin() * 0.5 + 0.5) * 255.0;
                    let g = ((v + t * 0.7).cos() * 0.5 + 0.5) * 255.0;
                    let b = (((u + v) * 2.0 + t * 1.3).sin() * 0.5 + 0.5) * 255.0;
                    let idx = ((y * w + x) * 4) as usize;
                    buf[idx] = r as u8;
                    buf[idx + 1] = g as u8;
                    buf[idx + 2] = b as u8;
                    buf[idx + 3] = 255;
                }
            }

            writer.submit_frame(&buf, w, h);
        });
    }

    // ── Layout ───────────────────────────────────────────────────────────

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "RenderSurface" }
                Text { size: "lg", color: "dimmed",
                    "Embed custom renderers into the rinch layout. RenderSurface accepts raw RGBA pixels from any source — game engines, terminal emulators, video decoders, or software rasterizers. Mouse and keyboard events are forwarded to the surface."
                }
            }
            Space { h: "xl" }

            // ── Solid Cube ────────────────────────────────────────────
            Title { order: 2, "Spinning Cube" }
            Text { size: "sm", color: "dimmed",
                "A solid 3D cube with a different color per face, rendered via software rasterization. Drag to rotate, scroll to zoom. The overlay panel uses rinch components positioned over the surface."
            }
            Space { h: "sm" }

            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",

                div { style: "position: relative; width: 100%; height: 400px;",
                    RenderSurface { surface: Some(cube_surface), style: "width: 100%; height: 100%;" }

                    // Overlay controls
                    div { style: "position: absolute; top: 12px; right: 12px; width: 200px;",
                        Paper { p: "sm", radius: "md",
                            style: "background: rgba(0, 0, 0, 0.7); color: white;",

                            Stack { gap: "xs",
                                Text { size: "xs", weight: "700", color: "white", "Controls" }

                                Switch {
                                    label: "Spin",
                                    checked_fn: move || spinning.get(),
                                    onchange: move || spinning.update(|v| *v = !*v)
                                }

                                Slider {
                                    label: "Speed",
                                    min: 0.1,
                                    max: 5.0,
                                    step: 0.1,
                                    value: 1.0,
                                    onchange: move |v: f64| speed.set(v)
                                }

                                Button {
                                    variant: "subtle",
                                    size: "xs",
                                    full_width: true,
                                    onclick: move || {
                                        let mut s = cube_state.lock().unwrap();
                                        s.zoom = 1.0;
                                        s.angle_x = 0.4;
                                        s.angle_y = 0.0;
                                    },
                                    "Reset View"
                                }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ── Animated Gradient ────────────────────────────────────
            Title { order: 2, "Animated Gradient" }
            Text { size: "sm", color: "dimmed",
                "A minimal example showing the render callback API. Each frame computes an animated color gradient and submits it as RGBA pixels."
            }
            Space { h: "sm" }

            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",
                div { style: "width: 100%; height: 200px;",
                    RenderSurface { surface: Some(gradient_surface), style: "width: 100%; height: 100%;" }
                }
            }

            Space { h: "lg" }

            // ── Code example ─────────────────────────────────────────
            Title { order: 2, "Usage" }
            Code {
                "let surface = create_render_surface();\n\nsurface.set_render_callback(|writer, w, h| {\n    let mut pixels = vec![0u8; (w * h * 4) as usize];\n    // ... fill pixels ...\n    writer.submit_frame(&pixels, w, h);\n});\n\nsurface.set_event_handler(|event| match event {\n    SurfaceEvent::MouseDown { x, y, .. } => { /* handle */ }\n    _ => {}\n});\n\nrsx! {\n    RenderSurface { surface: Some(surface) }\n}"
            }
        }
    }
}
