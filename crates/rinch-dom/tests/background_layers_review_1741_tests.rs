//! Review of #1741: rinch-dom pixels against Chrome 155 screenshots.
//! PHASE=gen writes the HTML + PNG inputs to .scratch/r1741; PHASE=cmp
//! compares against Chrome's screenshots (`<name>.chrome.png`).
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.scratch/r1741");

fn img1() -> (Vec<u8>, u32, u32) {
    // 4x4, left half opaque magenta, right half transparent.
    let mut d = Vec::new();
    for _ in 0..4 {
        for x in 0..4 {
            d.extend_from_slice(if x < 2 {
                &[255, 0, 255, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    (d, 4, 4)
}
fn img2() -> (Vec<u8>, u32, u32) {
    // 20x10, left half red, right half blue, opaque.
    let mut d = Vec::new();
    for _ in 0..10 {
        for x in 0..20 {
            d.extend_from_slice(if x < 10 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    (d, 20, 10)
}

const CHECKER: &str = "background-image: linear-gradient(45deg, #ccc 25%, transparent 25%), \
    linear-gradient(-45deg, #ccc 25%, transparent 25%), \
    linear-gradient(45deg, transparent 75%, #ccc 75%), \
    linear-gradient(-45deg, transparent 75%, #ccc 75%); \
    background-size: 8px 8px; background-position: 0 0, 0 4px, 4px -4px, -4px 0;";

fn probes() -> Vec<(&'static str, String, f64)> {
    let mut many = String::from(
        "width: 80px; height: 80px; background-color: rgb(250,250,200); background-image: ",
    );
    let mut imgs = Vec::new();
    let mut pos = Vec::new();
    for i in 0..12 {
        let r = (i * 37 % 256) as u8;
        let g = (i * 91 % 256) as u8;
        let b = (i * 53 % 256) as u8;
        imgs.push(format!(
            "linear-gradient(rgba({r},{g},{b},0.6), rgba({r},{g},{b},0.6))"
        ));
        pos.push(format!("{}px {}px", i * 5, (i * 7) % 50));
    }
    many += &imgs.join(", ");
    many += "; background-size: 20px 20px, 25px 15px, 15px 25px; background-repeat: no-repeat; background-position: ";
    many += &pos.join(", ");
    many += ";";
    vec![
        ("many", many, 1.0),
        (
            "mixed",
            "width: 80px; height: 80px; background-image: \
             linear-gradient(rgba(255,0,0,.7), rgba(255,0,0,.7)), \
             linear-gradient(rgba(0,160,0,.7), rgba(0,160,0,.7)), \
             linear-gradient(rgba(0,0,255,.7), rgba(0,0,255,.7)), \
             linear-gradient(to right, rgb(255,255,0) 50%, rgb(0,255,255) 50%); \
             background-size: 10px 6px, 6px 10px, 20px 20px, 16px 16px; \
             background-repeat: repeat-x, repeat-y, no-repeat, repeat; \
             background-position: 3px 7px, 11px 2px, 30px 30px, 5px 5px;"
                .into(),
            1.0,
        ),
        (
            "shorthand",
            "width: 60px; height: 60px; background: \
             linear-gradient(red, red) 0 0 / 10px 10px no-repeat, \
             linear-gradient(rgba(0,0,255,.5), rgba(0,0,255,.5)) 5px 5px / 20px 20px repeat-x, \
             url(img2.png) 30px 30px / 16px 8px no-repeat, rgb(0, 200, 0);"
                .into(),
            1.0,
        ),
        (
            "covercontain",
            "width: 80px; height: 50px; background: \
             url(img1.png) center / contain no-repeat, \
             url(img2.png) 0 0 / cover no-repeat, gray;"
                .into(),
            1.0,
        ),
        (
            "covercontain_grad",
            "width: 80px; height: 50px; background: \
             linear-gradient(rgba(255,0,0,.5), rgba(0,0,255,.5)) center / contain no-repeat, \
             linear-gradient(to right, yellow, cyan) 0 0 / cover no-repeat, gray;"
                .into(),
            1.0,
        ),
        (
            "fail",
            "width: 40px; height: 40px; background: \
             linear-gradient(rgba(255,0,0,.5), rgba(255,0,0,.5)) 0 0 / 10px 10px no-repeat, \
             url(missing-1741.png) 0 0 / 30px 30px no-repeat, \
             linear-gradient(blue, blue) 10px 10px / 20px 20px no-repeat, green;"
                .into(),
            1.0,
        ),
        (
            "radius",
            "width: 60px; height: 60px; box-sizing: border-box; padding: 5px; border: 6px solid rgba(0,0,0,0.2); \
             border-radius: 20px; background-image: \
             linear-gradient(red, red), linear-gradient(lime, lime), linear-gradient(blue, blue), \
             linear-gradient(to bottom right, rgb(255,200,0), rgb(120,0,200)); \
             background-size: 12px 12px, 12px 12px, 12px 12px, 100% 100%; \
             background-repeat: no-repeat; \
             background-origin: border-box, padding-box, content-box, border-box;"
                .into(),
            1.0,
        ),
        (
            "swatch_radius",
            format!("width: 28px; height: 28px; border-radius: 6px; {CHECKER}"),
            1.0,
        ),
        ("ctl_cover_single", "width: 80px; height: 50px; background: url(img2.png) 0 0 / cover no-repeat, gray;".into(), 1.0),
        ("ctl_contain_single", "width: 80px; height: 50px; background: url(img1.png) center / contain no-repeat, gray;".into(), 1.0),
        ("ctl_radius_grad", "width: 60px; height: 60px; box-sizing: border-box; padding: 5px; border: 6px solid rgba(0,0,0,0.2); border-radius: 20px; background-image: linear-gradient(to bottom right, rgb(255,200,0), rgb(120,0,200));".into(), 1.0),
        ("ctl_radius_grad_noborder", "width: 72px; height: 72px; border-radius: 20px; background-image: linear-gradient(to bottom right, rgb(255,200,0), rgb(120,0,200));".into(), 1.0),
        ("ctl_radius_squares", "width: 60px; height: 60px; box-sizing: border-box; padding: 5px; border: 6px solid rgba(0,0,0,0.2); border-radius: 20px; background-color: white; background-image: linear-gradient(red, red), linear-gradient(lime, lime), linear-gradient(blue, blue); background-size: 12px 12px; background-repeat: no-repeat; background-origin: border-box, padding-box, content-box;".into(), 1.0),
        ("ctl_strip_dpr15", "width: 60px; height: 60px; background-image: linear-gradient(rgba(255,0,0,.7), rgba(255,0,0,.7)); background-size: 7px 5px; background-repeat: repeat-x; background-position: 3px 7px;".into(), 1.5),
        ("pin_repeat", "width: 40px; height: 20px; background-image: linear-gradient(red, red), linear-gradient(blue, blue); background-size: 10px 10px; background-repeat: no-repeat, repeat-x; background-position: 0 0, 0 10px;".into(), 1.0),
        ("pin_origin", "box-sizing: border-box; width: 40px; height: 40px; border: 5px solid transparent; padding: 5px; background-image: linear-gradient(red, red), linear-gradient(blue, blue); background-size: 4px 4px; background-repeat: no-repeat; background-origin: border-box, content-box;".into(), 1.0),
        ("checker_dpr15", format!("width: 32px; height: 32px; {CHECKER}"), 1.5),
        (
            "mixed_dpr15",
            "width: 60px; height: 60px; background-image: \
             linear-gradient(rgba(255,0,0,.7), rgba(255,0,0,.7)), \
             linear-gradient(to right, rgb(255,255,0) 50%, rgb(0,255,255) 50%); \
             background-size: 7px 5px, 9px 9px; background-repeat: repeat-x, repeat; \
             background-position: 3px 7px, 2px 2px;"
                .into(),
            1.5,
        ),
    ]
}

fn mount(style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; {style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(100.0, 100.0);
    let (d, w, h) = img1();
    doc.tree
        .image_cache
        .insert_decoded("img1.png".into(), DecodedImage::new(d, w, h));
    let (d, w, h) = img2();
    doc.tree
        .image_cache
        .insert_decoded("img2.png".into(), DecodedImage::new(d, w, h));
    doc
}

#[test]
#[ignore = "review probe: prints rinch's pixels for comparison with Chrome; asserts nothing"]
fn review_1741_probe() {
    let phase = std::env::var("PHASE").unwrap_or_default();
    std::fs::create_dir_all(DIR).unwrap();
    if phase == "gen" {
        for (name, (d, w, h)) in [("img1", img1()), ("img2", img2())] {
            image::save_buffer(
                format!("{DIR}/{name}.png"),
                &d,
                w,
                h,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        for (name, style, _) in probes() {
            std::fs::write(
                format!("{DIR}/{name}.html"),
                format!(
                    "<!doctype html><html><head><style>html,body{{margin:0;background:#fff}}</style></head>\
                     <body><div style=\"position:absolute;left:0;top:0;{style}\"></div></body></html>"
                ),
            )
            .unwrap();
        }
        return;
    }
    for (name, style, scale) in probes() {
        let mut doc = mount(&style);
        let side = (100.0 * scale) as u32;
        let mut p = TinySkiaPainter::new(side, side);
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut p,
            scale,
            (100.0, 100.0),
            &mut doc.font_cx,
            &mut lcx,
        );
        let mine = p.pixels().to_vec();
        image::save_buffer(
            format!("{DIR}/{name}.rinch.png"),
            &mine,
            side,
            side,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let Ok(chrome) = image::open(format!("{DIR}/{name}.chrome.png")) else {
            println!("{name}: no chrome png");
            continue;
        };
        let chrome = chrome.to_rgba8();
        if chrome.width() != side || chrome.height() != side {
            println!(
                "{name}: size {}x{} vs {side}",
                chrome.width(),
                chrome.height()
            );
            continue;
        }
        let (mut d2, mut d16, mut max) = (0, 0, 0u8);
        let mut first = Vec::new();
        for i in 0..(side * side) as usize {
            let a = &mine[i * 4..i * 4 + 3];
            let b = &chrome.as_raw()[i * 4..i * 4 + 3];
            let m = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            max = max.max(m);
            if m > 2 {
                d2 += 1;
            }
            if m > 16 {
                d16 += 1;
                if first.len() < 6 {
                    first.push((i as u32 % side, i as u32 / side, a.to_vec(), b.to_vec()));
                }
            }
        }
        println!("{name}: >2: {d2}  >16: {d16}  max {max}  first {first:?}");
    }
}

/// Two 10x10 no-repeat squares, red on top of blue, both at `0 0`, moving to
/// `40px 0` over 1s. Chrome interpolates every entry (both at 20px at 50%).
#[test]
#[ignore = "review probe: prints rinch's pixels for comparison with Chrome; asserts nothing"]
fn review_1741_transition_two_layers() {
    for (name, end) in [("both", "40px 0, 40px 0"), ("lower_only", "0 0, 40px 0")] {
        let mut doc = RinchDocument::new();
        doc.load_css(&format!(
            "html, body {{ margin: 0; background: white; }} \
             .t {{ background-position: 0 0, 0 0; transition: background-position 1000ms linear; }} .t.end {{ background-position: {end}; }}"
        ));
        let body = doc.body();
        let node = doc.create_element("div");
        doc.set_attribute(
            node,
            "style",
            "position: absolute; left: 0; top: 0; width: 60px; height: 20px; \
             background-image: linear-gradient(red, red), linear-gradient(blue, blue); \
             background-size: 10px 10px; background-repeat: no-repeat;",
        );
        doc.append_child(body, node);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(100.0, 100.0);
        doc.set_attribute(node, "class", "t");
        doc.resolve_layout(101.0, 100.0);
        doc.set_attribute(node, "class", "t end");
        doc.resolve_layout(102.0, 100.0);
        let started = doc
            .tree
            .active_transitions
            .get(&node.0)
            .and_then(|m| m.values().map(|t| t.start_time_ms).next());
        if let Some(s) = started {
            rinch_dom::transition::tick_transitions(&mut doc.tree, s + 500.0);
        }
        let mut p = TinySkiaPainter::new(100, 100);
        let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut p,
            1.0,
            (100.0, 100.0),
            &mut doc.font_cx,
            &mut lcx,
        );
        let row: String = (0..6)
            .map(|i| {
                let x = i * 10 + 5;
                let d = &p.pixels()[((5 * 100 + x) * 4) as usize..][..3];
                match d {
                    [255, 0, 0] => 'R',
                    [0, 0, 255] => 'B',
                    [255, 255, 255] => 'W',
                    _ => '?',
                }
            })
            .collect();
        println!(
            "transition {name}: started={} row at 50% = {row}",
            started.is_some()
        );
    }
}

struct PngLoader;
impl rinch_core::image::ImageLoader for PngLoader {
    fn load(&self, _src: &str) -> rinch_core::image::ImageLoadResult {
        let mut bytes = Vec::new();
        let (d, w, h) = img2();
        image::write_buffer_with_format(
            &mut std::io::Cursor::new(&mut bytes),
            &d,
            w,
            h,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        rinch_core::image::ImageLoadResult::Loaded(bytes)
    }
}

/// A url named only by a LOWER layer: when it decodes, the node is pushed
/// paint-dirty (`drain_pending_images` walks `background_image_urls`), so the
/// incremental paint redraws it. Kills mutant M2 (only the first layer's url
/// repaints on decode), which the PR's own `f` (request only) does not.
#[test]
fn review_1741_a_lower_layers_decode_repaints_its_node() {
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(std::sync::Arc::new(PngLoader));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "position: absolute; left: 0; top: 0; width: 20px; height: 10px; \
         background: linear-gradient(lime, lime) 0 0 / 2px 2px no-repeat, url(lower-1741.png) no-repeat;",
    );
    doc.append_child(body, div);
    doc.resolve_layout(100.0, 100.0);
    doc.tree.paint_dirty_nodes.clear();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while doc.tree.image_cache.get("lower-1741.png").is_none() {
        assert!(std::time::Instant::now() < deadline, "never decoded");
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.resolve_layout(100.0, 100.0);
    }
    assert!(
        doc.tree.paint_dirty_nodes.contains(&div.0),
        "the decode of a lower layer's url must repaint its node"
    );
}

fn px_of(style: &str, pts: &[(u32, u32)]) -> Vec<[u8; 3]> {
    let mut doc = mount(style);
    let mut p = TinySkiaPainter::new(100, 100);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        1.0,
        (100.0, 100.0),
        &mut doc.font_cx,
        &mut lcx,
    );
    pts.iter()
        .map(|&(x, y)| {
            let i = ((y * 100 + x) * 4) as usize;
            [p.pixels()[i], p.pixels()[i + 1], p.pixels()[i + 2]]
        })
        .collect()
}

/// Chrome 155 (`pin_repeat` probe): each layer takes its own repeat entry.
/// Kills M11 (every layer reads repeat entry 0).
#[test]
fn review_1741_each_layer_takes_its_own_repeat() {
    let got = px_of(
        "width: 40px; height: 20px; background-image: linear-gradient(red, red), linear-gradient(blue, blue); background-size: 10px 10px; background-repeat: no-repeat, repeat-x; background-position: 0 0, 0 10px;",
        &[(5, 5), (25, 5), (5, 15), (35, 15)],
    );
    assert_eq!(
        got,
        vec![[255, 0, 0], [255, 255, 255], [0, 0, 255], [0, 0, 255]]
    );
}

/// Chrome 155 (`pin_origin` probe): each layer takes its own origin. Kills M8
/// (every layer positioned in the first layer's origin box).
#[test]
fn review_1741_each_layer_takes_its_own_origin() {
    let got = px_of(
        "box-sizing: border-box; width: 40px; height: 40px; border: 5px solid transparent; padding: 5px; background-image: linear-gradient(red, red), linear-gradient(blue, blue); background-size: 4px 4px; background-repeat: no-repeat; background-origin: border-box, content-box;",
        &[(2, 2), (12, 12), (7, 7)],
    );
    assert_eq!(got, vec![[255, 0, 0], [0, 0, 255], [255, 255, 255]]);
}
