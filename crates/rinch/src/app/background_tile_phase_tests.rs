//! #468 (review round 2 of PR #1143): a partial repaint of a box drawn over
//! a tiled background keeps the tile phase — the frame is asserted partial,
//! then compared with a from-scratch frame over the whole surface — and does
//! not re-rasterise the tile (`background_tile_rasters` 0, a cache hit
//! instead): the tile's pixels did not change.
use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::RefCell;

const W: u32 = 600;
const H: u32 = 400;

fn mount(bg: &str) -> (RinchApp, Rc<RefCell<Option<NodeHandle>>>) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out = slot.clone();
    let bg = bg.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 600px; height: 400px;");
        let big = scope.create_element("div");
        big.set_attribute(
            "style",
            &format!("position: absolute; left: 7px; top: 3px; width: 560px; height: 360px; {bg}"),
        );
        root.append_child(&big);
        let c = scope.create_element("div");
        c.set_attribute(
            "style",
            "position: absolute; left: 203px; top: 101px; width: 57px; height: 33px; background: rgb(0, 200, 0);",
        );
        root.append_child(&c);
        *out.borrow_mut() = Some(c);
        root
    });
    app.mount_component(W as f32, H as f32);
    (app, slot)
}

fn frame(app: &mut RinchApp, scale: f64) -> (Vec<u8>, FrameStats) {
    app.resolve_and_repaint(W as f32, H as f32);
    let size = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let px = app.build_pixels(scale, size, false).0.to_vec();
    (px, app.end_perf_frame().expect("mounted"))
}
fn full_frame(app: &mut RinchApp, scale: f64) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let size = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let px = app.build_pixels(scale, size, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}
fn ndiff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .any(|(p, q)| (*p as i32 - *q as i32).abs() > 2)
        })
        .count()
}

const RB: &str =
    "background-image: linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

fn check(bg: &str, scale: f64) {
    let (mut app, slot) = mount(bg);
    frame(&mut app, scale);
    frame(&mut app, scale);
    let (before, _) = frame(&mut app, scale);
    slot.borrow()
        .as_ref()
        .unwrap()
        .set_style("background", "rgb(0, 0, 0)");
    let (after, stats) = frame(&mut app, scale);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert!(ndiff(&before, &after) > 0, "positive control");
    let full = full_frame(&mut app, scale);
    assert_eq!(ndiff(&after, &full), 0, "{bg} @ {scale}: partial != full");
    assert_eq!(
        stats.get(Counter::BackgroundTileRasters),
        0,
        "{bg} @ {scale}: the partial repaint re-rasterised an unchanged tile: {stats:?}"
    );
    assert!(
        stats.get(Counter::BackgroundTileCacheHits) >= 1,
        "{bg} @ {scale}: positive control, the layer took the pattern path: {stats:?}"
    );
}

#[test]
fn partial_repaint_keeps_pattern_phase() {
    check(
        &format!("{RB}; background-size: 10px 10px; background-position: 3.4px 5.6px"),
        1.0,
    );
}
#[test]
fn partial_repaint_keeps_pattern_phase_hidpi() {
    check(
        &format!("{RB}; background-size: 10px 10px; background-position: 3.4px 5.6px"),
        1.5,
    );
}
#[test]
fn partial_repaint_keeps_tile_phase_repeat_x() {
    check(
        &format!(
            "{RB}; background-size: 10px 10px; background-position: 3.4px 5.6px; background-repeat: repeat-x"
        ),
        1.25,
    );
}
#[test]
fn partial_repaint_keeps_phase_rounded() {
    check(
        &format!(
            "{RB}; border-radius: 40px; background-size: 10px 10px; background-position: 3.4px 5.6px"
        ),
        1.25,
    );
}

/// Cost of a frame with a large repeating image tile (pattern path): full and a partial.
#[test]
#[ignore = "timing probe"]
fn timing_large_tile() {
    for (label, tile, size) in [
        ("256px natural", 256u32, "auto"),
        ("2000px img at 300px", 2000, "300px 300px"),
        ("2000px img at 500px", 2000, "500px 500px"),
    ] {
        for sc in [1.0f64, 2.0] {
            let (mut app, slot) = mount(&format!(
                "background-image: url(r1143big.png); background-size: {size}"
            ));
            frame(&mut app, sc);
            std::thread::sleep(std::time::Duration::from_millis(150));
            frame(&mut app, sc);
            {
                let d = app.doc.as_ref().unwrap();
                let mut d = d.borrow_mut();
                let mut data = Vec::with_capacity((tile * tile * 4) as usize);
                for y in 0..tile {
                    for x in 0..tile {
                        data.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 90, 255]);
                    }
                }
                d.tree.image_cache.insert_decoded(
                    "r1143big.png".into(),
                    rinch_dom::image_cache::DecodedImage::new(data, tile, tile),
                );
            }
            frame(&mut app, sc);
            frame(&mut app, sc);
            let f = full_frame(&mut app, sc);
            let i = ((300 * W + 300) * 4) as usize;
            eprintln!("sanity px(300,300) = {:?}", &f[i..i + 4]);
            let mut best_full = f64::MAX;
            let mut best_part = f64::MAX;
            for i in 0..7 {
                let t = std::time::Instant::now();
                let _ = full_frame(&mut app, sc);
                best_full = best_full.min(t.elapsed().as_secs_f64() * 1e3);
                slot.borrow().as_ref().unwrap().set_style(
                    "background",
                    if i % 2 == 0 {
                        "rgb(0,0,0)"
                    } else {
                        "rgb(0,9,0)"
                    },
                );
                app.resolve_and_repaint(W as f32, H as f32);
                let t = std::time::Instant::now();
                let _ =
                    app.build_pixels(sc, ((W as f64 * sc) as u32, (H as f64 * sc) as u32), false);
                best_part = best_part.min(t.elapsed().as_secs_f64() * 1e3);
                let _ = app.end_perf_frame();
            }
            eprintln!(
                "tile {label} @{sc}: full {best_full:.2} ms, partial (57x33 child) {best_part:.2} ms"
            );
        }
    }
}
