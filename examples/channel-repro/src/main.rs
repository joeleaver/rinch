//! Minimal red↔blue channel-order repro.
//!
//! Three swatches, all authored as PURE RED where it matters:
//!   1. a plain `div` with `background: rgb(255,0,0)` (vello solid fill path)
//!   2. a `RenderSurface` fed a known RGBA quadrant pattern (inline surface
//!      pixels → `paint::image::paint_image` → `Painter::draw_image`)
//!   3. an `<img>` pointing at a 64×64 pure-red PNG (normal image cache path)
//!
//! Whichever swatch comes out blue names the path that swaps channels.
//! The surface's quadrants are R / G / B / orange clockwise from top-left so a
//! single screenshot pins down the exact permutation.

use rinch::prelude::*;
use rinch::render_surface::create_render_surface;

const SW: u32 = 160;
const SH: u32 = 160;

/// R / G / B / orange quadrants, tightly packed RGBA8.
fn quadrant_frame() -> Vec<u8> {
    let mut px = Vec::with_capacity((SW * SH * 4) as usize);
    for y in 0..SH {
        for x in 0..SW {
            let c = match (x < SW / 2, y < SH / 2) {
                (true, true) => [255u8, 0, 0, 255],   // top-left: pure red
                (false, true) => [0, 255, 0, 255],    // top-right: pure green
                (true, false) => [0, 0, 255, 255],    // bottom-left: pure blue
                (false, false) => [255, 128, 0, 255], // bottom-right: orange
            };
            px.extend_from_slice(&c);
        }
    }
    px
}

#[component]
fn app() -> NodeHandle {
    let surface = create_render_surface();
    let writer = surface.writer();

    // Feed the surface from a background thread, the same way runt's engine
    // thread does. Keep submitting so the first frame can't be lost to a race
    // with window creation.
    std::thread::spawn(move || {
        let frame = quadrant_frame();
        loop {
            writer.submit_frame(&frame, SW, SH);
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });

    let img_src = concat!(env!("CARGO_MANIFEST_DIR"), "/red.png");

    rsx! {
        div { style: "display: flex; flex-direction: row; gap: 20px; padding: 20px; \
                      background: #ffffff;",

            // 1. solid fill — vello fill path
            div { id: "swatch-fill",
                style: "width: 160px; height: 160px; background: rgb(255, 0, 0);" }

            // 2. render surface — inline surface-pixels path
            div { style: "width: 160px; height: 160px; background: #000000;",
                RenderSurface { surface: Some(surface) }
            }

            // 3. <img> — image cache path
            img { id: "swatch-img", src: img_src,
                style: "width: 160px; height: 160px;" }
        }
    }
}

fn main() {
    App::new(app).title("channel-repro").size(620, 220).run();
}
