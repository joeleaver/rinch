//! Web event-surface demo/test for `rinch-web`.
//!
//! Exercises every event path the web backend dispatches — click, mousedown/
//! mouseup/mousemove, hover (enter/leave), contextmenu, element drag-and-drop,
//! and the `viewport_height` ClickContext field — with text readouts (stable
//! `id`s) so the behavior can be asserted in a real browser (e.g. Playwright).

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    let clicks = Signal::new(0_i32);
    let mouse = Signal::new(String::from("none"));
    let hover = Signal::new(String::from("out"));
    let ctx = Signal::new(0_i32);
    let dnd = Signal::new(String::from("idle"));
    let dropped = Signal::new(String::from("none"));
    let vp = Signal::new(0.0_f32);

    let drag = DragContext::<String>::new();

    rsx! {
        div { style: "font-family: sans-serif; padding: 24px; display: flex; flex-direction: column; gap: 12px; max-width: 480px;",
            h2 { "rinch-web event surface" }

            // ── Click ──────────────────────────────────────────────
            button { id: "btn-click", onclick: move || clicks.update(|n| *n += 1), "Click me" }
            p { id: "out-clicks", "clicks: " {|| clicks.get().to_string()} }

            // ── Mouse down / up / move ─────────────────────────────
            div {
                id: "mouse-box",
                style: "width: 220px; height: 70px; background: #e7f5ff; border: 1px solid #228be6; display: flex; align-items: center; justify-content: center;",
                onmousedown: move || mouse.set("down".into()),
                onmouseup: move || mouse.set("up".into()),
                onmousemove: move || mouse.set("move".into()),
                "mouse target"
            }
            p { id: "out-mouse", "mouse: " {|| mouse.get()} }

            // ── Hover (enter / leave) ──────────────────────────────
            div {
                id: "hover-box",
                style: "width: 220px; height: 50px; background: #fff3bf; display: flex; align-items: center; justify-content: center;",
                onmouseenter: move || hover.set("in".into()),
                onmouseleave: move || hover.set("out".into()),
                "hover target"
            }
            p { id: "out-hover", "hover: " {|| hover.get()} }

            // ── Context menu (right-click) ─────────────────────────
            div {
                id: "ctx-box",
                style: "width: 220px; height: 50px; background: #ffe3e3; display: flex; align-items: center; justify-content: center;",
                oncontextmenu: move || ctx.update(|n| *n += 1),
                "right-click me"
            }
            p { id: "out-ctx", "ctxmenu: " {|| ctx.get().to_string()} }

            // ── Element drag-and-drop ──────────────────────────────
            div {
                id: "drag-item",
                draggable: true,
                style: "width: 140px; height: 56px; background: #d3f9d8; border: 1px solid #2f9e44; cursor: grab; display: flex; align-items: center; justify-content: center;",
                ondragstart: move || { drag.set("payload".into()); dnd.set("dragstart".into()); },
                ondragend: move || dnd.update(|s| { if !s.starts_with("dropped") { *s = "dragend".into(); } }),
                "drag me"
            }
            div {
                id: "drop-zone",
                style: "width: 220px; height: 70px; background: #f1f3f5; border: 2px dashed #adb5bd; display: flex; align-items: center; justify-content: center;",
                ondragenter: move || dnd.set("enter".into()),
                ondragover: move || {},
                ondragleave: move || dnd.set("leave".into()),
                ondrop: move || { if let Some(p) = drag.take() { dropped.set(p); } },
                "drop here"
            }
            p { id: "out-dnd", "dnd: " {|| dnd.get()} }
            p { id: "out-dropped", "dropped: " {|| dropped.get()} }

            // ── viewport_height ClickContext field ─────────────────
            button {
                id: "btn-vp",
                onclick: move || vp.set(get_click_context().viewport_height),
                "Read viewport_height"
            }
            p { id: "out-vp", "viewport_h: " {|| format!("{:.0}", vp.get())} }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();
    let theme = ThemeProviderProps {
        dark_mode: false,
        ..Default::default()
    };
    rinch_web::mount(theme, app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
