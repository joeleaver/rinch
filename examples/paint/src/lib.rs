//! Painting app demo — demonstrates RenderSurface capabilities.
//!
//! Features:
//! - Main canvas with multiple brush types (round, square, spray, eraser)
//! - Navigator panel showing the full canvas with viewport rectangle
//! - Zoom (mouse wheel) and pan (middle-mouse drag, navigator click)
//! - Color palette, brush size slider, undo/redo
//!
//! This is a shared library — use `paint-desktop` or `paint-web` to run it.

use std::sync::{Arc, Mutex};

use rinch::prelude::*;
use rinch::render_surface::{
    RenderSurface, SurfaceEvent, SurfaceMouseButton, create_render_surface,
};
use rinch_tabler_icons::{TablerIcon, render_tabler_icon};

// ── Constants ────────────────────────────────────────────────────────────────

const CANVAS_W: u32 = 2048;
const CANVAS_H: u32 = 2048;
const NAV_SIZE: u32 = 200;
const MAX_UNDO: usize = 20;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BrushType {
    Round,
    Square,
    Spray,
    Eraser,
}

struct PaintState {
    canvas: Vec<u8>,
    canvas_w: u32,
    canvas_h: u32,

    zoom: f32,
    pan_x: f32,
    pan_y: f32,

    brush: BrushType,
    brush_size: u32,
    color: [u8; 4],
    drawing: bool,
    last_draw_pos: Option<(f32, f32)>,

    panning: bool,
    pan_anchor_screen: Option<(f32, f32)>,
    pan_anchor_canvas: Option<(f32, f32)>,

    undo_stack: Vec<Vec<u8>>,
    redo_stack: Vec<Vec<u8>>,

    canvas_dirty: bool,
    nav_dirty: bool,
}

impl PaintState {
    fn new() -> Self {
        let size = (CANVAS_W * CANVAS_H * 4) as usize;
        let mut canvas = vec![255u8; size];
        for chunk in canvas.as_chunks_mut::<4>().0 {
            chunk[3] = 255;
        }
        Self {
            canvas,
            canvas_w: CANVAS_W,
            canvas_h: CANVAS_H,
            zoom: 1.0,
            pan_x: CANVAS_W as f32 / 2.0,
            pan_y: CANVAS_H as f32 / 2.0,
            brush: BrushType::Round,
            brush_size: 8,
            color: [0, 0, 0, 255],
            drawing: false,
            last_draw_pos: None,
            panning: false,
            pan_anchor_screen: None,
            pan_anchor_canvas: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            canvas_dirty: true,
            nav_dirty: true,
        }
    }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.canvas.clone());
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.canvas.clone());
            self.canvas = prev;
            self.canvas_dirty = true;
            self.nav_dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.canvas.clone());
            self.canvas = next;
            self.canvas_dirty = true;
            self.nav_dirty = true;
        }
    }

    fn clear(&mut self) {
        self.push_undo();
        for chunk in self.canvas.as_chunks_mut::<4>().0 {
            chunk[0] = 255;
            chunk[1] = 255;
            chunk[2] = 255;
            chunk[3] = 255;
        }
        self.canvas_dirty = true;
        self.nav_dirty = true;
    }

    fn screen_to_canvas(&self, sx: f32, sy: f32, view_w: f32, view_h: f32) -> (f32, f32) {
        let cx = (sx - view_w / 2.0) / self.zoom + self.pan_x;
        let cy = (sy - view_h / 2.0) / self.zoom + self.pan_y;
        (cx, cy)
    }

    fn set_pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x >= 0 && y >= 0 && (x as u32) < self.canvas_w && (y as u32) < self.canvas_h {
            let idx = ((y as u32 * self.canvas_w + x as u32) * 4) as usize;
            self.canvas[idx] = color[0];
            self.canvas[idx + 1] = color[1];
            self.canvas[idx + 2] = color[2];
            self.canvas[idx + 3] = color[3];
        }
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn draw_at(state: &mut PaintState, cx: f32, cy: f32) {
    let r = state.brush_size as i32;
    let color = if state.brush == BrushType::Eraser {
        [255, 255, 255, 255]
    } else {
        state.color
    };
    let ix = cx as i32;
    let iy = cy as i32;

    match state.brush {
        BrushType::Round | BrushType::Eraser => {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy <= r * r {
                        state.set_pixel(ix + dx, iy + dy, color);
                    }
                }
            }
        }
        BrushType::Square => {
            for dy in -r..=r {
                for dx in -r..=r {
                    state.set_pixel(ix + dx, iy + dy, color);
                }
            }
        }
        BrushType::Spray => {
            let count = (r * r) as usize;
            for _ in 0..count {
                let angle = fastrand::f32() * std::f32::consts::TAU;
                let dist = fastrand::f32().sqrt() * r as f32;
                let dx = (angle.cos() * dist) as i32;
                let dy = (angle.sin() * dist) as i32;
                state.set_pixel(ix + dx, iy + dy, color);
            }
        }
    }
}

fn draw_stroke(state: &mut PaintState, from: (f32, f32), to: (f32, f32)) {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let dist = (dx * dx + dy * dy).sqrt();
    let step = (state.brush_size as f32 / 3.0).max(1.0);
    let steps = (dist / step).ceil() as usize;

    for i in 0..=steps {
        let t = if steps == 0 {
            1.0
        } else {
            i as f32 / steps as f32
        };
        let x = from.0 + dx * t;
        let y = from.1 + dy * t;
        draw_at(state, x, y);
    }
}

// ── Canvas rendering ─────────────────────────────────────────────────────────

fn render_canvas_view(s: &PaintState, output: &mut [u8], out_w: u32, out_h: u32) {
    let zoom = s.zoom;
    let pan_x = s.pan_x;
    let pan_y = s.pan_y;
    let cw = s.canvas_w;
    let ch = s.canvas_h;

    let left = pan_x - out_w as f32 / (2.0 * zoom);
    let top = pan_y - out_h as f32 / (2.0 * zoom);

    for sy in 0..out_h {
        for sx in 0..out_w {
            let cx = left + sx as f32 / zoom;
            let cy = top + sy as f32 / zoom;
            let out_idx = ((sy * out_w + sx) * 4) as usize;

            if cx >= 0.0 && cy >= 0.0 && (cx as u32) < cw && (cy as u32) < ch {
                let ci = ((cy as u32 * cw + cx as u32) * 4) as usize;
                output[out_idx] = s.canvas[ci];
                output[out_idx + 1] = s.canvas[ci + 1];
                output[out_idx + 2] = s.canvas[ci + 2];
                output[out_idx + 3] = 255;
            } else {
                let check = ((cx as i32 / 16) + (cy as i32 / 16)) & 1;
                let v = if check == 0 { 200u8 } else { 230u8 };
                output[out_idx] = v;
                output[out_idx + 1] = v;
                output[out_idx + 2] = v;
                output[out_idx + 3] = 255;
            }
        }
    }
}

fn render_navigator_view(
    s: &PaintState,
    output: &mut [u8],
    out_w: u32,
    out_h: u32,
    view_w: u32,
    view_h: u32,
) {
    let cw = s.canvas_w;
    let ch = s.canvas_h;
    let scale_x = cw as f32 / out_w as f32;
    let scale_y = ch as f32 / out_h as f32;

    for ny in 0..out_h {
        for nx in 0..out_w {
            let cx = (nx as f32 * scale_x) as u32;
            let cy = (ny as f32 * scale_y) as u32;
            let ci = ((cy.min(ch - 1) * cw + cx.min(cw - 1)) * 4) as usize;
            let ni = ((ny * out_w + nx) * 4) as usize;
            output[ni] = s.canvas[ci];
            output[ni + 1] = s.canvas[ci + 1];
            output[ni + 2] = s.canvas[ci + 2];
            output[ni + 3] = 255;
        }
    }

    // Viewport rectangle
    let zoom = s.zoom;
    let pan_x = s.pan_x;
    let pan_y = s.pan_y;

    let vp_left = ((pan_x - view_w as f32 / (2.0 * zoom)) / cw as f32 * out_w as f32) as i32;
    let vp_top = ((pan_y - view_h as f32 / (2.0 * zoom)) / ch as f32 * out_h as f32) as i32;
    let vp_right = ((pan_x + view_w as f32 / (2.0 * zoom)) / cw as f32 * out_w as f32) as i32;
    let vp_bottom = ((pan_y + view_h as f32 / (2.0 * zoom)) / ch as f32 * out_h as f32) as i32;

    let red = [255u8, 60, 60, 255];
    for t in 0..2i32 {
        for x in vp_left..=vp_right {
            set_pixel_safe(output, x, vp_top + t, out_w, out_h, red);
            set_pixel_safe(output, x, vp_bottom - t, out_w, out_h, red);
        }
        for y in vp_top..=vp_bottom {
            set_pixel_safe(output, vp_left + t, y, out_w, out_h, red);
            set_pixel_safe(output, vp_right - t, y, out_w, out_h, red);
        }
    }
}

fn set_pixel_safe(output: &mut [u8], x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
        let idx = ((y as u32 * w + x as u32) * 4) as usize;
        output[idx] = color[0];
        output[idx + 1] = color[1];
        output[idx + 2] = color[2];
        output[idx + 3] = color[3];
    }
}

// ── Color palette ────────────────────────────────────────────────────────────

const COLORS: &[[u8; 3]] = &[
    [0, 0, 0],
    [127, 127, 127],
    [136, 0, 21],
    [237, 28, 36],
    [255, 127, 39],
    [255, 242, 0],
    [34, 177, 76],
    [0, 162, 232],
    [63, 72, 204],
    [163, 73, 164],
    [255, 255, 255],
    [195, 195, 195],
    [185, 122, 87],
    [255, 174, 201],
    [255, 201, 14],
    [239, 228, 176],
    [181, 230, 29],
    [153, 217, 234],
    [112, 146, 190],
    [200, 191, 231],
];

// ── Helpers ──────────────────────────────────────────────────────────────────

fn rgba_to_hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn hex_to_rgba(hex: &str) -> Option<[u8; 4]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b, 255])
}

// ── App ──────────────────────────────────────────────────────────────────────

#[component]
pub fn app() -> NodeHandle {
    let paint_state = Arc::new(Mutex::new(PaintState::new()));

    let tool_signal = Signal::new(BrushType::Round);
    let color_signal = Signal::new([0u8, 0, 0, 255]);
    let brush_size_signal = Signal::new(8.0_f64);
    let zoom_signal = Signal::new(1.0_f32);
    let status_signal = Signal::new(String::from("Ready"));
    let can_undo = Signal::new(false);
    let can_redo = Signal::new(false);
    let show_color_picker = Signal::new(false);

    let canvas_surface = create_render_surface();
    let nav_surface = create_render_surface();

    // ── Canvas render callback ──────────────────────────────────────────
    {
        let ps = paint_state.clone();
        let mut canvas_output: Vec<u8> = Vec::new();
        canvas_surface.set_render_callback(move |writer, width, height| {
            let mut s = ps.lock().unwrap();
            if !s.canvas_dirty {
                return;
            }
            s.canvas_dirty = false;
            let size = (width * height * 4) as usize;
            if canvas_output.len() != size {
                canvas_output.resize(size, 0);
            }
            render_canvas_view(&s, &mut canvas_output, width, height);
            writer.submit_frame(&canvas_output, width, height);
        });
    }

    // ── Navigator render callback ───────────────────────────────────────
    {
        let ps = paint_state.clone();
        let canvas_for_nav = canvas_surface.clone();
        let mut nav_output: Vec<u8> = Vec::new();
        nav_surface.set_render_callback(move |writer, width, height| {
            let mut s = ps.lock().unwrap();
            if !s.nav_dirty {
                return;
            }
            s.nav_dirty = false;
            let size = (width * height * 4) as usize;
            if nav_output.len() != size {
                nav_output.resize(size, 0);
            }
            // Use the canvas surface's layout size for viewport rect calculation
            let canvas_view_size = canvas_for_nav.layout_size();
            render_navigator_view(
                &s,
                &mut nav_output,
                width,
                height,
                canvas_view_size.0,
                canvas_view_size.1,
            );
            writer.submit_frame(&nav_output, width, height);
        });
    }

    // ── Canvas events ────────────────────────────────────────────────────
    {
        let ps = paint_state.clone();
        let canvas_handle = canvas_surface.clone();
        canvas_surface.set_event_handler(move |event| {
            let (lw, lh) = canvas_handle.layout_size();
            let view_w = lw as f32;
            let view_h = lh as f32;

            match event {
                SurfaceEvent::MouseDown {
                    x,
                    y,
                    button: SurfaceMouseButton::Left,
                } => {
                    let mut s = ps.lock().unwrap();
                    s.brush = tool_signal.get();
                    s.brush_size = brush_size_signal.get() as u32;
                    s.color = color_signal.get();
                    s.push_undo();
                    s.drawing = true;
                    let (cx, cy) = s.screen_to_canvas(x, y, view_w, view_h);
                    draw_at(&mut s, cx, cy);
                    s.last_draw_pos = Some((cx, cy));
                    s.canvas_dirty = true;
                    s.nav_dirty = true;
                    can_undo.set(!s.undo_stack.is_empty());
                    can_redo.set(!s.redo_stack.is_empty());
                }
                SurfaceEvent::MouseMove { x, y } => {
                    let mut s = ps.lock().unwrap();
                    if s.drawing {
                        let (cx, cy) = s.screen_to_canvas(x, y, view_w, view_h);
                        if let Some(last) = s.last_draw_pos {
                            draw_stroke(&mut s, last, (cx, cy));
                        }
                        s.last_draw_pos = Some((cx, cy));
                        s.canvas_dirty = true;
                        s.nav_dirty = true;
                    } else if s.panning
                        && let (Some(anchor_screen), Some(anchor_canvas)) =
                            (s.pan_anchor_screen, s.pan_anchor_canvas)
                    {
                        let dx = (x - anchor_screen.0) / s.zoom;
                        let dy = (y - anchor_screen.1) / s.zoom;
                        s.pan_x = anchor_canvas.0 - dx;
                        s.pan_y = anchor_canvas.1 - dy;
                        s.canvas_dirty = true;
                        s.nav_dirty = true;
                        zoom_signal.set(s.zoom);
                    }
                    let (cx, cy) = s.screen_to_canvas(x, y, view_w, view_h);
                    status_signal.set(format!(
                        "({:.0}, {:.0})",
                        cx.clamp(0.0, CANVAS_W as f32),
                        cy.clamp(0.0, CANVAS_H as f32)
                    ));
                }
                SurfaceEvent::MouseUp {
                    button: SurfaceMouseButton::Left,
                    ..
                } => {
                    let mut s = ps.lock().unwrap();
                    s.drawing = false;
                    s.last_draw_pos = None;
                }
                SurfaceEvent::MouseDown {
                    x,
                    y,
                    button: SurfaceMouseButton::Middle,
                } => {
                    let mut s = ps.lock().unwrap();
                    s.panning = true;
                    s.pan_anchor_screen = Some((x, y));
                    s.pan_anchor_canvas = Some((s.pan_x, s.pan_y));
                }
                SurfaceEvent::MouseUp {
                    button: SurfaceMouseButton::Middle,
                    ..
                } => {
                    let mut s = ps.lock().unwrap();
                    s.panning = false;
                    s.pan_anchor_screen = None;
                    s.pan_anchor_canvas = None;
                }
                SurfaceEvent::MouseWheel { x, y, delta_y, .. } => {
                    let mut s = ps.lock().unwrap();
                    let (cx, cy) = s.screen_to_canvas(x, y, view_w, view_h);
                    let factor = if delta_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let new_zoom = (s.zoom * factor).clamp(0.1, 10.0);
                    s.pan_x = cx - (x - view_w / 2.0) / new_zoom;
                    s.pan_y = cy - (y - view_h / 2.0) / new_zoom;
                    s.zoom = new_zoom;
                    s.canvas_dirty = true;
                    s.nav_dirty = true;
                    zoom_signal.set(new_zoom);
                }
                SurfaceEvent::KeyDown(key) => {
                    let mut s = ps.lock().unwrap();
                    s.brush = tool_signal.get();
                    s.brush_size = brush_size_signal.get() as u32;
                    s.color = color_signal.get();

                    match key.key.as_str() {
                        "z" if key.ctrl => {
                            s.undo();
                            can_undo.set(!s.undo_stack.is_empty());
                            can_redo.set(!s.redo_stack.is_empty());
                        }
                        "y" if key.ctrl => {
                            s.redo();
                            can_undo.set(!s.undo_stack.is_empty());
                            can_redo.set(!s.redo_stack.is_empty());
                        }
                        "1" => {
                            s.brush = BrushType::Round;
                            tool_signal.set(BrushType::Round);
                        }
                        "2" => {
                            s.brush = BrushType::Square;
                            tool_signal.set(BrushType::Square);
                        }
                        "3" => {
                            s.brush = BrushType::Spray;
                            tool_signal.set(BrushType::Spray);
                        }
                        "4" => {
                            s.brush = BrushType::Eraser;
                            tool_signal.set(BrushType::Eraser);
                        }
                        "[" => {
                            s.brush_size = s.brush_size.saturating_sub(2).max(1);
                            brush_size_signal.set(s.brush_size as f64);
                        }
                        "]" => {
                            s.brush_size = (s.brush_size + 2).min(50);
                            brush_size_signal.set(s.brush_size as f64);
                        }
                        "+" | "=" => {
                            let new_zoom = (s.zoom + 0.25).min(10.0);
                            s.zoom = new_zoom;
                            s.canvas_dirty = true;
                            s.nav_dirty = true;
                            zoom_signal.set(new_zoom);
                        }
                        "-" => {
                            let new_zoom = (s.zoom - 0.25).max(0.1);
                            s.zoom = new_zoom;
                            s.canvas_dirty = true;
                            s.nav_dirty = true;
                            zoom_signal.set(new_zoom);
                        }
                        "0" => {
                            s.zoom = 1.0;
                            s.pan_x = CANVAS_W as f32 / 2.0;
                            s.pan_y = CANVAS_H as f32 / 2.0;
                            s.canvas_dirty = true;
                            s.nav_dirty = true;
                            zoom_signal.set(1.0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        });
    }

    // ── Navigator events ─────────────────────────────────────────────────
    {
        let ps = paint_state.clone();
        let nav_dragging = std::cell::Cell::new(false);
        nav_surface.set_event_handler(move |event| match event {
            SurfaceEvent::MouseDown {
                x,
                y,
                button: SurfaceMouseButton::Left,
            } => {
                nav_dragging.set(true);
                let mut s = ps.lock().unwrap();
                let scale_x = s.canvas_w as f32 / NAV_SIZE as f32;
                let scale_y = s.canvas_h as f32 / NAV_SIZE as f32;
                s.pan_x = x * scale_x;
                s.pan_y = y * scale_y;
                s.canvas_dirty = true;
                s.nav_dirty = true;
                zoom_signal.set(s.zoom);
            }
            SurfaceEvent::MouseMove { x, y } if nav_dragging.get() => {
                let mut s = ps.lock().unwrap();
                let scale_x = s.canvas_w as f32 / NAV_SIZE as f32;
                let scale_y = s.canvas_h as f32 / NAV_SIZE as f32;
                s.pan_x = x * scale_x;
                s.pan_y = y * scale_y;
                s.canvas_dirty = true;
                s.nav_dirty = true;
                zoom_signal.set(s.zoom);
            }
            SurfaceEvent::MouseUp {
                button: SurfaceMouseButton::Left,
                ..
            } => {
                nav_dragging.set(false);
            }
            _ => {}
        });
    }

    // ── Toolbar callbacks ────────────────────────────────────────────────
    let ps_undo = paint_state.clone();
    let ps_redo = paint_state.clone();
    let ps_clear = paint_state.clone();

    // Build color palette
    let mut color_swatches: Vec<NodeHandle> = Vec::new();
    for rgb in COLORS.iter() {
        let cr = rgb[0];
        let cg = rgb[1];
        let cb = rgb[2];
        let color_rgba = [cr, cg, cb, 255u8];
        let swatch = rsx! {
            div {
                style: {move || {
                    let active = color_signal.get();
                    let is_active = active[0] == cr && active[1] == cg && active[2] == cb;
                    let border = if is_active {
                        "2px solid var(--rinch-primary-color)"
                    } else if cr > 240 && cg > 240 && cb > 240 {
                        "1px solid #ccc"
                    } else {
                        "1px solid transparent"
                    };
                    format!(
                        "width: 24px; height: 24px; background: rgb({cr},{cg},{cb}); \
                         border-radius: 3px; border: {border}; cursor: pointer;"
                    )
                }},
                onclick: move || color_signal.set(color_rgba),
            }
        };
        color_swatches.push(swatch);
    }

    rsx! {
        div {
            style: "display: flex; flex-direction: column; width: 100vw; height: 100vh; \
                    background: var(--rinch-color-body); overflow: hidden;",

            // ── Toolbar ──────────────────────────────────────────────
            div {
                style: "display: flex; flex-direction: row; align-items: center; gap: 8px; \
                        padding: 6px 12px; border-bottom: 1px solid var(--rinch-color-default-border); \
                        background: var(--rinch-color-body); min-height: 44px;",

                // Tool buttons
                ActionIcon {
                    icon: TablerIcon::Brush,
                    variant: {|| if tool_signal.get() == BrushType::Round { "filled" } else { "subtle" }},
                    size: "lg",
                    onclick: move || tool_signal.set(BrushType::Round),
                }
                ActionIcon {
                    icon: TablerIcon::Square,
                    variant: {|| if tool_signal.get() == BrushType::Square { "filled" } else { "subtle" }},
                    size: "lg",
                    onclick: move || tool_signal.set(BrushType::Square),
                }
                ActionIcon {
                    icon: TablerIcon::Spray,
                    variant: {|| if tool_signal.get() == BrushType::Spray { "filled" } else { "subtle" }},
                    size: "lg",
                    onclick: move || tool_signal.set(BrushType::Spray),
                }
                ActionIcon {
                    icon: TablerIcon::Eraser,
                    variant: {|| if tool_signal.get() == BrushType::Eraser { "filled" } else { "subtle" }},
                    size: "lg",
                    onclick: move || tool_signal.set(BrushType::Eraser),
                }

                div { style: "width: 1px; height: 24px; background: var(--rinch-color-default-border); margin: 0 4px;" }

                // Brush size
                div { style: "display: flex; align-items: center; gap: 4px; min-width: 160px;",
                    div { style: "font-size: 13px; color: var(--rinch-color-dimmed); white-space: nowrap;",
                        "Size:"
                    }
                    div { style: "flex: 1; min-width: 100px;",
                        Slider {
                            min: 1.0,
                            max: 50.0,
                            value_signal: Some(brush_size_signal),
                            step: 1.0,
                            size: "sm",
                            onchange: move |v: f64| brush_size_signal.set(v),
                        }
                    }
                    div { style: "font-size: 13px; color: var(--rinch-color-dimmed); min-width: 32px; text-align: right;",
                        {|| format!("{}px", brush_size_signal.get() as u32)}
                    }
                }

                div { style: "width: 1px; height: 24px; background: var(--rinch-color-default-border); margin: 0 4px;" }

                // Undo / Redo / Clear
                ActionIcon {
                    icon: TablerIcon::ArrowBackUp,
                    variant: {|| if can_undo.get() { "subtle" } else { "transparent" }},
                    size: "lg",
                    color: {|| if can_undo.get() { "" } else { "gray" }},
                    onclick: {
                        let ps = ps_undo.clone();
                        move || {
                            let mut s = ps.lock().unwrap();
                            s.undo();
                            can_undo.set(!s.undo_stack.is_empty());
                            can_redo.set(!s.redo_stack.is_empty());
                        }
                    },
                }
                ActionIcon {
                    icon: TablerIcon::ArrowForwardUp,
                    variant: {|| if can_redo.get() { "subtle" } else { "transparent" }},
                    size: "lg",
                    color: {|| if can_redo.get() { "" } else { "gray" }},
                    onclick: {
                        let ps = ps_redo.clone();
                        move || {
                            let mut s = ps.lock().unwrap();
                            s.redo();
                            can_undo.set(!s.undo_stack.is_empty());
                            can_redo.set(!s.redo_stack.is_empty());
                        }
                    },
                }
                ActionIcon {
                    icon: TablerIcon::Trash,
                    variant: "subtle",
                    size: "lg",
                    onclick: {
                        let ps = ps_clear.clone();
                        move || {
                            let mut s = ps.lock().unwrap();
                            s.clear();
                            can_undo.set(!s.undo_stack.is_empty());
                            can_redo.set(!s.redo_stack.is_empty());
                        }
                    },
                }

                div { style: "width: 1px; height: 24px; background: var(--rinch-color-default-border); margin: 0 4px;" }

                // Zoom display
                div { style: "font-size: 13px; color: var(--rinch-color-dimmed); white-space: nowrap;",
                    {|| format!("Zoom: {:.0}%", zoom_signal.get() * 100.0)}
                }
            }

            // ── Body ─────────────────────────────────────────────────
            div {
                style: "display: flex; flex-direction: row; flex: 1; overflow: hidden;",

                // Main canvas — overlays rendered on top of the RenderSurface
                // via CSS stacking contexts (position + z-index).
                div { style: "flex: 1; overflow: hidden; display: flex; position: relative;",
                    RenderSurface { surface: Some(canvas_surface) }

                    // ── Overlay: bottom status bar ──
                    div {
                        style: "position: absolute; bottom: 0; left: 0; right: 0; z-index: 1; \
                                display: flex; flex-wrap: nowrap; align-items: center; justify-content: space-between; \
                                padding: 6px 12px; \
                                background: rgba(0, 0, 0, 0.55); color: white; font-size: 12px; \
                                pointer-events: none;",

                        // Left: cursor position + tool + brush size
                        div { style: "display: flex; align-items: center; gap: 6px;",
                            {render_tabler_icon(__scope, TablerIcon::CursorText, rinch_tabler_icons::TablerIconStyle::Outline)}
                            span { {|| status_signal.get()} }
                            span { style: "opacity: 0.6;", "\u{2022}" }
                            span {
                                {|| match tool_signal.get() {
                                    BrushType::Round => "Round",
                                    BrushType::Square => "Square",
                                    BrushType::Spray => "Spray",
                                    BrushType::Eraser => "Eraser",
                                }}
                            }
                            span { style: "opacity: 0.6;", "\u{2022}" }
                            span { {|| format!("{}px", brush_size_signal.get() as u32)} }
                        }

                        // Right: zoom
                        div { style: "display: flex; align-items: center; gap: 4px;",
                            {render_tabler_icon(__scope, TablerIcon::ZoomIn, rinch_tabler_icons::TablerIconStyle::Outline)}
                            span { {|| format!("{:.0}%", zoom_signal.get() * 100.0)} }
                        }
                    }

                    // ── Overlay: clickable color picker (top-left) ──
                    div {
                        style: "position: absolute; top: 12px; left: 12px; z-index: 2;",

                        // Swatch button — always visible
                        div {
                            style: "display: flex; align-items: center; gap: 8px; \
                                    background: rgba(0, 0, 0, 0.55); border-radius: 6px; \
                                    padding: 6px 10px; cursor: pointer;",
                            onclick: move || show_color_picker.update(|v| *v = !*v),
                            div {
                                style: {|| {
                                    let c = color_signal.get();
                                    format!(
                                        "width: 20px; height: 20px; border-radius: 4px; \
                                         background: rgb({},{},{}); border: 1px solid rgba(255,255,255,0.4);",
                                        c[0], c[1], c[2]
                                    )
                                }},
                            }
                            span { style: "color: white; font-size: 12px;",
                                {|| {
                                    let c = color_signal.get();
                                    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
                                }}
                            }
                        }

                        // Color picker dropdown — toggled via CSS display
                        div {
                            style: {|| {
                                let vis = if show_color_picker.get() {
                                    "display: flex"
                                } else {
                                    "display: none"
                                };
                                format!("position: absolute; top: 40px; left: 0; \
                                        background: var(--rinch-color-body); \
                                        border: 1px solid var(--rinch-color-default-border); \
                                        border-radius: var(--rinch-radius-default); \
                                        box-shadow: var(--rinch-shadow-md); \
                                        padding: 8px; z-index: 10; {vis}")
                            }},
                            ColorPicker {
                                value: {rgba_to_hex(color_signal.get())},
                                swatches: {COLORS.iter().map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])).collect::<Vec<_>>()},
                                swatches_per_row: 10,
                                onchange: move |hex: String| {
                                    if let Some(rgba) = hex_to_rgba(&hex) {
                                        color_signal.set(rgba);
                                    }
                                },
                            }
                        }
                    }
                }

                // Right sidebar
                div {
                    style: "width: 220px; display: flex; flex-direction: column; gap: 8px; padding: 8px; \
                            border-left: 1px solid var(--rinch-color-default-border); \
                            background: var(--rinch-color-body);",

                    div { style: "font-size: 12px; font-weight: 600; color: var(--rinch-color-dimmed);",
                        "NAVIGATOR"
                    }

                    div {
                        style: "width: 200px; height: 200px; border: 1px solid var(--rinch-color-default-border); \
                                border-radius: 4px; overflow: hidden;",
                        RenderSurface { surface: Some(nav_surface) }
                    }

                    div { style: "font-size: 12px; font-weight: 600; color: var(--rinch-color-dimmed); margin-top: 4px;",
                        "COLORS"
                    }

                    // Color palette
                    div { style: "display: flex; flex-wrap: wrap; gap: 3px;",
                        {color_swatches}
                    }

                    // Current color
                    div { style: "display: flex; align-items: center; gap: 8px; margin-top: 4px;",
                        div { style: "font-size: 12px; color: var(--rinch-color-dimmed);", "Current:" }
                        div {
                            style: {|| {
                                let c = color_signal.get();
                                format!(
                                    "width: 28px; height: 28px; border-radius: 4px; \
                                     background: rgb({},{},{}); \
                                     border: 1px solid var(--rinch-color-default-border);",
                                    c[0], c[1], c[2]
                                )
                            }},
                        }
                    }

                    // Status
                    div { style: "margin-top: auto; font-size: 12px; color: var(--rinch-color-dimmed);",
                        {|| status_signal.get()}
                    }
                }
            }
        }
    }
}
