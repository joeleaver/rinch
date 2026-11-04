//! #2111 — an SVG **file** as an image: `<img src="x.svg">`,
//! `background-image: url(x.svg)` and `mask-image: url(x.svg)`, on the
//! software painter. Before, the image cache decoded PNG, JPEG, GIF and WebP
//! only, so each of these drew nothing (and a mask hid its box).
//!
//! Every expected pixel and size is Chrome 156's (headless,
//! `--force-device-scale-factor=1` and `=2`, the same markup), unless a test
//! says otherwise. The GPU twin is `crates/rinch/tests/gpu_svg_images_2111.rs`.
#![cfg(feature = "software-renderer")]

use base64::Engine;
use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::sync::Arc;

/// 16x16 with a viewBox: a red bar whose left edge is on a half pixel
/// (`x="2.5"`), so a raster made at the natural size and scaled up shows a
/// blended column where one made at the drawn size is crisp, and a blue dot.
const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><rect x="2.5" y="2" width="5" height="12" fill="#f00"/><circle cx="12" cy="8" r="3" fill="#00f"/></svg>"##;
/// 16x8, all red, with a viewBox.
const WIDE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8" viewBox="0 0 16 8"><rect width="16" height="8" fill="#f00"/></svg>"##;
/// The same without a viewBox.
const NO_VIEWBOX: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="#f00"/></svg>"##;

fn b64(svg: &str) -> String {
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    )
}

/// `encodeURIComponent`'s form: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )`.
fn pct(svg: &str) -> String {
    let mut out = String::from("data:image/svg+xml,");
    for b in svg.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

const W: u32 = 200;
const H: u32 = 120;

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; } .a { position: absolute; display: block; }",
    );
    doc
}

fn add(doc: &mut RinchDocument, tag: &str, style: &str, src: Option<&str>) -> NodeId {
    let body = doc.body();
    let n = doc.create_element(tag);
    doc.set_attribute(n, "class", "a");
    doc.set_attribute(n, "style", style);
    if let Some(src) = src {
        doc.set_attribute(n, "src", src);
    }
    doc.append_child(body, n);
    n
}

/// The Chrome reference page: `ref.html` of the PR, laid out in a 200x120
/// window.
fn reference_page() -> RinchDocument {
    let mut doc = new_doc();
    add(&mut doc, "img", "left: 10px; top: 10px", Some(&b64(ICON)));
    add(
        &mut doc,
        "img",
        "left: 40px; top: 10px; width: 32px; height: 32px",
        Some(&b64(ICON)),
    );
    add(
        &mut doc,
        "div",
        &format!(
            "left: 80px; top: 10px; width: 60px; height: 40px; \
             background: url(\"{}\") 0 0 / 20px 20px",
            pct(ICON)
        ),
        None,
    );
    add(
        &mut doc,
        "div",
        &format!(
            "left: 150px; top: 10px; width: 32px; height: 32px; background-color: #0a0; \
             mask-image: url(\"{}\"); mask-size: 100% 100%; mask-repeat: no-repeat",
            b64(ICON)
        ),
        None,
    );
    add(
        &mut doc,
        "img",
        "left: 10px; top: 60px; width: 40px; height: 40px",
        Some(&b64(WIDE)),
    );
    add(
        &mut doc,
        "img",
        "left: 60px; top: 60px; width: 40px; height: 40px",
        Some(&b64(NO_VIEWBOX)),
    );
    // `background-size: auto`: the icon's natural 16x16, as the `<img>` at
    // (10, 10) draws it (not in Chrome's page; compared with that `<img>`).
    add(
        &mut doc,
        "div",
        &format!(
            "left: 110px; top: 60px; width: 40px; height: 40px; \
             background: url(\"{}\") no-repeat",
            b64(ICON)
        ),
        None,
    );
    doc
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = &doc.tree.nodes[id.0];
    (n.layout.width, n.layout.height)
}

fn paint_at(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    doc.resolve_layout(W as f32, H as f32);
    let (pw, ph) = (
        (W as f64 * scale).round() as u32,
        (H as f64 * scale).round() as u32,
    );
    let mut painter = TinySkiaPainter::new(pw, ph);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Each `(point, want)` within `tol` per channel.
fn expect(p: &TinySkiaPainter, want: &[((u32, u32), [u8; 3])], tol: u8) {
    let bad: Vec<_> = want
        .iter()
        .filter_map(|&((x, y), w)| {
            let got = px(p, x, y);
            (got.iter().zip(w).any(|(a, b)| a.abs_diff(b) > tol)).then_some(((x, y), got, w))
        })
        .collect();
    assert!(bad.is_empty(), "(point, rinch, chrome): {bad:?}");
}

fn row(x0: u32, y: u32, want: &[[u8; 3]]) -> Vec<((u32, u32), [u8; 3])> {
    want.iter()
        .enumerate()
        .map(|(i, &w)| ((x0 + i as u32, y), w))
        .collect()
}

const WH: [u8; 3] = [255, 255, 255];
const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];
const G: [u8; 3] = [0, 170, 0];

/// Anti-aliasing differs a little between Skia and tiny-skia at the dot's
/// curved edge; straight edges and interiors agree far closer than this.
const TOL: u8 = 24;

/// `<img>` at its natural 16x16 and drawn at 32x32 by CSS, at 1x. The 32px
/// one is rasterised at 32px: its bar's left edge (x 2.5 → 45) is crisp,
/// where a 16px raster scaled up would blend columns 44 and 45.
#[test]
fn an_img_svg_icon_matches_chrome_at_1x() {
    let mut doc = reference_page();
    let p = paint_at(&mut doc, 1.0);
    let half_r = [255, 127, 127];
    let edge_b = [15, 15, 255];
    // Natural size: the bar from 12.5 to 17.5, the dot from 19 to 25.
    expect(&p, &row(11, 12, &[WH, half_r, R, R, R, R, half_r, WH]), TOL);
    expect(&p, &row(18, 18, &[WH, edge_b, B, B, B, B, edge_b, WH]), TOL);
    // Drawn at 32px.
    let mut b = vec![WH, R, R, R, R, R, R, R, R, R, R, WH, WH, WH];
    b.extend([
        [71, 71, 255],
        B,
        B,
        B,
        B,
        B,
        B,
        B,
        B,
        B,
        B,
        [71, 71, 255],
        WH,
    ]);
    expect(&p, &row(44, 24, &b), TOL);
}

/// The same page at 2x: the natural-size icon is 32 device pixels and drawn
/// from a 32px raster, crisp (x 2.5 → device 25).
#[test]
fn an_img_svg_icon_matches_chrome_at_2x() {
    let mut doc = reference_page();
    let p = paint_at(&mut doc, 2.0);
    let edge_b = [23, 23, 255];
    let mut a = vec![WH];
    a.extend([R; 10]);
    a.extend([WH, WH, WH, edge_b]);
    a.extend([B; 10]);
    a.extend([edge_b, WH]);
    expect(&p, &row(24, 36, &a), TOL);
    // The 32px one is 64 device pixels: its bar from 90 to 110.
    let mut b = vec![WH];
    b.extend([R; 20]);
    b.push(WH);
    expect(&p, &row(89, 48, &b), TOL);
}

/// `background: url(icon.svg) 0 0 / 20px 20px` (the percent-encoded `data:`
/// form): every 20px tile is the icon rasterised at 20px (the bar from
/// 2.5 * 1.25 = 3.125 to 9.375), repeated across the 60px box.
///
/// The bar's edge pixels are compared loosely: tiny-skia measures coverage
/// on a quarter-pixel grid, so an edge at 3.125 covers as if at 3.25 (63
/// where Skia has 31) — the same in a raster of any size. Every pixel inside
/// the bar and the dot is exact, and a tile rasterised at the natural 16px
/// and scaled to 20 blurs the bar's inner columns.
#[test]
fn an_svg_background_with_background_size_matches_chrome() {
    let mut doc = reference_page();
    let p = paint_at(&mut doc, 1.0);
    for x0 in [80, 100, 120] {
        expect(&p, &row(x0 + 2, 18, &[WH]), 0);
        expect(&p, &row(x0 + 4, 18, &[R, R, R, R, R]), 0);
        expect(&p, &row(x0 + 10, 18, &[WH]), 0);
        expect(&p, &row(x0 + 12, 18, &[B, B, B, B, B, B]), 0);
        expect(&p, &row(x0 + 19, 18, &[WH]), 0);
        // The edges: Chrome's values, within the quarter-pixel step.
        expect(
            &p,
            &[
                ((x0 + 3, 18), [255, 31, 31]),
                ((x0 + 9, 18), [255, 159, 159]),
                ((x0 + 11, 18), [122, 122, 255]),
                ((x0 + 18, 18), [122, 122, 255]),
            ],
            40,
        );
    }
    // 2x: 40 device pixels a tile, the bar from 6.25 to 18.75.
    let p = paint_at(&mut doc, 2.0);
    for x0 in [160, 200, 240] {
        expect(&p, &row(x0 + 5, 36, &[WH]), 0);
        expect(&p, &row(x0 + 7, 36, &[R; 11]), 0);
        expect(&p, &row(x0 + 19, 36, &[WH]), 0);
        expect(&p, &row(x0 + 24, 36, &[B; 12]), 0);
        expect(
            &p,
            &[
                ((x0 + 6, 36), [255, 63, 63]),
                ((x0 + 18, 36), [255, 63, 63]),
                ((x0 + 23, 36), [127, 127, 255]),
                ((x0 + 36, 36), [135, 135, 255]),
            ],
            40,
        );
    }
}

/// `background-size: auto` sizes an SVG background by its natural size, in
/// CSS pixels: at 1x and 2x it is exactly the natural-size `<img>`'s picture.
#[test]
fn an_auto_sized_svg_background_is_its_natural_size() {
    let mut doc = reference_page();
    for scale in [1u32, 2] {
        let p = paint_at(&mut doc, scale as f64);
        let (img, bg) = ((10 * scale, 10 * scale), (110 * scale, 60 * scale));
        for y in 0..18 * scale {
            for x in 0..18 * scale {
                assert_eq!(
                    px(&p, bg.0 + x, bg.1 + y),
                    px(&p, img.0 + x, img.1 + y),
                    "{scale}x: ({x}, {y})"
                );
            }
        }
        // And it drew something.
        expect(&p, &[((bg.0 + 5 * scale, bg.1 + 5 * scale), R)], 0);
    }
}

/// The icon-tint case: `background-color` under `mask-image: url(icon.svg)`
/// at `mask-size: 100% 100%`. Green shows through the icon's shapes only.
/// Before #2111 the mask image never decoded, which hides the whole box.
#[test]
fn an_svg_mask_image_tints_an_icon() {
    let mut doc = reference_page();
    let p = paint_at(&mut doc, 1.0);
    let mut want = vec![WH];
    want.extend([G; 10]);
    want.extend([WH, WH, WH, [23, 178, 23]]);
    want.extend([G; 10]);
    want.extend([[23, 178, 23], WH]);
    expect(&p, &row(154, 26, &want), TOL);
    let p = paint_at(&mut doc, 2.0);
    let mut want = vec![WH];
    want.extend([G; 20]);
    want.push(WH);
    expect(&p, &row(309, 52, &want), TOL);
    expect(&p, &row(336, 52, &[[7, 172, 7], G, G]), TOL);
}

/// Drawn into a box of another shape, a file with a viewBox keeps its shape
/// (`preserveAspectRatio`'s default, `xMidYMid meet`: 40x20, centred), and
/// one without is stretched over the box. Chrome 156 does both.
#[test]
fn an_svg_in_another_shape_follows_its_view_box() {
    let mut doc = reference_page();
    let p = paint_at(&mut doc, 1.0);
    expect(
        &p,
        &[
            ((30, 69), WH),
            ((30, 70), R),
            ((30, 89), R),
            ((30, 90), WH),
            ((11, 80), R),
            ((48, 80), R),
        ],
        TOL,
    );
    expect(
        &p,
        &[((61, 61), R), ((98, 98), R), ((80, 62), R), ((80, 97), R)],
        TOL,
    );
}

/// The size an `<img>` of an SVG file takes, inline and as a block, in a
/// 400px body. Chrome 156: the natural size is the root's absolute
/// `width`/`height`, a missing one through the ratio (both attributes', else
/// the viewBox's) or 300x150, and with a ratio alone the image fills the
/// containing block's width.
///
/// Two answers are rinch's, not Chrome's, and pinned as such: rinch rounds
/// every layout to whole pixels (10.5x7.25 is 11x7, as for an `<svg>`
/// element), and an **inline** image with a ratio alone is 300 wide where
/// Chrome fills the containing block, the gap an inline `<svg>` element has
/// (`replaced::SVG_FILL_WIDTH`: an atomic inline is sized with no definite
/// parent width).
#[test]
fn svg_image_natural_sizes_match_chrome() {
    let svg = |attrs: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" {attrs}/>"#);
    // (attributes, Chrome 156, rinch inline where it differs, rinch block
    // where it differs).
    type Size = (f32, f32);
    let cases: &[(&str, Size, Option<Size>, Option<Size>)] = &[
        (r#"width="24" height="16""#, (24.0, 16.0), None, None),
        (
            r#"viewBox="0 0 16 8""#,
            (400.0, 200.0),
            Some((300.0, 150.0)),
            None,
        ),
        (
            r#"viewBox="0 0 10 10""#,
            (400.0, 400.0),
            Some((300.0, 300.0)),
            None,
        ),
        ("", (300.0, 150.0), None, None),
        (r#"width="30" viewBox="0 0 16 8""#, (30.0, 15.0), None, None),
        (r#"width="30""#, (30.0, 150.0), None, None),
        (r#"height="30""#, (300.0, 30.0), None, None),
        (
            r#"width="50%" height="20" viewBox="0 0 16 8""#,
            (40.0, 20.0),
            None,
            None,
        ),
        (r#"width="2em" height="1in""#, (32.0, 96.0), None, None),
        (
            r#"width="10.5" height="7.25""#,
            (10.5, 7.25),
            Some((11.0, 7.0)),
            Some((11.0, 7.0)),
        ),
    ];
    for display in ["inline", "block"] {
        let mut doc = RinchDocument::new();
        doc.load_css("html, body { margin: 0; } body { width: 400px; }");
        let body = doc.body();
        let mut nodes = Vec::new();
        for &(attrs, chrome, inline, block) in cases {
            // Each in its own block, so an inline one sits on its own line.
            let wrap = doc.create_element("div");
            let img = doc.create_element("img");
            doc.set_attribute(img, "style", &format!("display: {display}"));
            doc.set_attribute(img, "src", &pct(&svg(attrs)));
            doc.append_child(wrap, img);
            doc.append_child(body, wrap);
            let rinch = if display == "inline" { inline } else { block };
            nodes.push((attrs, img, rinch.unwrap_or(chrome)));
        }
        doc.resolve_layout(800.0, 600.0);
        let bad: Vec<_> = nodes
            .iter()
            .filter_map(|&(attrs, img, want)| {
                let got = size(&doc, img);
                (got != want).then_some((attrs, got, want))
            })
            .collect();
        assert!(bad.is_empty(), "{display}: (svg, got, want): {bad:?}");
    }
}

/// A malformed SVG (and a `.svg` source that is not SVG at all) fails as a
/// failed image does: cached as failed, nothing drawn, no panic.
#[test]
fn a_malformed_svg_fails_cleanly() {
    let bad = pct(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect"#);
    let not_svg = pct("<html/>");
    let mut doc = new_doc();
    let a = add(&mut doc, "img", "left: 0; top: 0", Some(&bad));
    let b = add(&mut doc, "img", "left: 20px; top: 0", Some(&not_svg));
    let p = paint_at(&mut doc, 1.0);
    assert!(doc.tree.image_cache.is_failed(&bad));
    assert!(doc.tree.image_cache.is_failed(&not_svg));
    for n in [a, b] {
        assert_eq!(
            size(&doc, n),
            (0.0, 0.0),
            "a failed image's box, as for a PNG"
        );
    }
    assert!(p.pixels().chunks(4).all(|c| c == [255, 255, 255, 255]));
}

/// Both `data:image/svg+xml` forms decode to the same picture: base64, and
/// percent-encoded, which decoded nothing before #2111 (only base64 did).
#[test]
fn both_data_url_forms_decode() {
    let mut doc = new_doc();
    let (b, p) = (b64(ICON), pct(ICON));
    add(&mut doc, "img", "left: 0; top: 0", Some(&b));
    add(&mut doc, "img", "left: 20px; top: 0", Some(&p));
    let painter = paint_at(&mut doc, 1.0);
    for src in [&b, &p] {
        let img = doc.tree.image_cache.get(src).expect("decoded");
        assert!(img.vector().is_some());
        assert_eq!(img.natural().width, Some(16.0));
    }
    // The same pixels, 20px apart.
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(px(&painter, x, y), px(&painter, x + 20, y), "({x}, {y})");
        }
    }
    // A percent-encoded raster image decodes too (any `data:` URL may be).
    let bytes = rinch_dom::image_cache::decode_data_uri("data:text/plain,a%20b%zz%4").unwrap();
    assert_eq!(bytes, b"a b%zz%4");
}

/// A loader's bytes for a `.svg` source (an `<img>` of a file, or an app
/// scheme) decode as SVG on the load thread.
#[test]
fn an_svg_file_from_the_loader_decodes() {
    struct Files;
    impl ImageLoader for Files {
        fn load(&self, src: &str) -> ImageLoadResult {
            match src {
                "icons/a.svg" => ImageLoadResult::Loaded(ICON.as_bytes().to_vec()),
                _ => ImageLoadResult::Failed("no such file".into()),
            }
        }
    }
    let mut doc = new_doc();
    doc.tree.image_loader = Some(Arc::new(Files));
    let img = add(&mut doc, "img", "left: 0; top: 0", Some("icons/a.svg"));
    let start = std::time::Instant::now();
    while doc.tree.image_cache.get("icons/a.svg").is_none() {
        assert!(start.elapsed().as_secs() < 10, "the load never landed");
        std::thread::sleep(std::time::Duration::from_millis(5));
        doc.drain_pending_images();
    }
    doc.resolve_layout(W as f32, H as f32);
    assert_eq!(size(&doc, img), (16.0, 16.0));
}

/// At a fractional device size the raster is a whole pixel larger, not
/// smaller: a 16px square at 1.3x covers 20.8 device pixels from 13, so
/// column 33 is 80% covered and must be drawn (a raster cut to 20 pixels
/// would drop it). The expected values are the geometry's, off the
/// whole-pixel fixed point the reference page sits on.
#[test]
fn a_fractional_device_size_keeps_its_last_column() {
    let square = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><rect width="16" height="16" fill="#f00"/></svg>"##;
    let mut doc = new_doc();
    add(&mut doc, "img", "left: 10px; top: 10px", Some(&b64(square)));
    let p = paint_at(&mut doc, 1.3);
    expect(
        &p,
        &[
            ((12, 20), WH),
            ((13, 20), R),
            ((32, 20), R),
            ((33, 20), [255, 51, 51]),
            ((34, 20), WH),
            ((20, 33), [255, 51, 51]),
            ((20, 34), WH),
        ],
        40,
    );
}

/// Rasters are made per drawn size and kept: a repaint at a size already
/// drawn makes none, a new device scale makes one, and the sizes drawn stay
/// kept (within the thread's byte budget, `svg_image_limits_2111_tests`).
#[test]
fn rasters_are_made_per_size_and_bounded() {
    let src = b64(ICON);
    let mut doc = new_doc();
    let img = add(&mut doc, "img", "left: 0; top: 0", Some(&src));
    let made = |doc: &RinchDocument| {
        doc.tree
            .image_cache
            .get(&src)
            .unwrap()
            .vector()
            .unwrap()
            .rasters_made()
    };
    paint_at(&mut doc, 1.0);
    assert_eq!(made(&doc), 1);
    paint_at(&mut doc, 1.0);
    assert_eq!(made(&doc), 1, "the same size again: the kept raster");
    paint_at(&mut doc, 2.0);
    assert_eq!(made(&doc), 2, "a DPI change: a new raster");
    paint_at(&mut doc, 1.0);
    assert_eq!(made(&doc), 2, "back to 1x: still kept");
    for (i, w) in [17, 18, 19, 20, 21].into_iter().enumerate() {
        doc.set_attribute(img, "style", &format!("left: 0; top: 0; width: {w}px"));
        paint_at(&mut doc, 1.0);
        assert_eq!(made(&doc), 3 + i as u64);
    }
    // Well under the byte budget: all seven sizes are kept, and going back
    // to the first makes nothing.
    let vector = doc.tree.image_cache.get(&src).unwrap();
    assert_eq!(vector.vector().unwrap().rasters_held(), 7);
    doc.set_attribute(img, "style", "left: 0; top: 0");
    paint_at(&mut doc, 1.0);
    assert_eq!(made(&doc), 7);
}

/// Nothing outside the file is read and no text is drawn: an `<image>` of
/// an SVG file on disk (which usvg's default resolver loads and draws, so
/// this fails with rinch's resolvers taken out), of a `data:` SVG, of a
/// `data:` PNG, and a `<text>` draw nothing; the rest of the file draws.
#[test]
fn an_svg_reads_nothing_outside_itself() {
    const RED_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";
    let red = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    let dir = std::env::temp_dir().join(format!("rinch-2111-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let on_disk = dir.join("red.svg");
    std::fs::write(&on_disk, red).unwrap();
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="50" height="10" viewBox="0 0 50 10">
        <image x="0" y="0" width="10" height="10" href="{RED_PNG}"/>
        <image x="10" y="0" width="10" height="10" xlink:href="{}"/>
        <image x="20" y="0" width="10" height="10" href="{}"/>
        <text x="30" y="10" font-size="10" fill="#f00">MM</text>
        <rect x="40" y="0" width="10" height="10" fill="#00f"/></svg>"##,
        on_disk.display(),
        b64(red)
    );
    let mut doc = new_doc();
    add(&mut doc, "img", "left: 0; top: 0", Some(&pct(&svg)));
    let p = paint_at(&mut doc, 1.0);
    let _ = std::fs::remove_dir_all(&dir);
    for y in 0..10 {
        for x in 0..40 {
            assert_eq!(px(&p, x, y), WH, "({x}, {y})");
        }
    }
    expect(&p, &[((45, 5), B)], 0);
}

/// What rasterising costs on the UI thread: an icon, and two 1200x800
/// illustrations of 2,000 filled cubic paths at 1x and 2x — "small" shapes
/// (each within 60px, opaque) and a worst case ("large": each spans the
/// canvas, half transparent). `#[ignore]`d; run with
/// `--release -- --ignored --nocapture svg_raster_cost_probe`.
#[test]
#[ignore = "a timing probe"]
fn svg_raster_cost_probe() {
    let mut seed = 2111u32;
    let mut rnd = move |n: u32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 8) % n
    };
    let mut illustration = |span: u32, opacity: &str| {
        let mut paths = String::new();
        for _ in 0..2000 {
            let (x, y) = (rnd(1200 - span.min(1199)), rnd(800 - span.min(799)));
            let mut p = || (x + rnd(span), y + rnd(span.min(800)));
            let (a, b, c, d) = (p(), p(), p(), p());
            paths.push_str(&format!(
                r##"<path d="M{} {} C{} {} {} {} {} {} Z" fill="#{:06x}" fill-opacity="{opacity}"/>"##,
                a.0,
                a.1,
                b.0,
                b.1,
                c.0,
                c.1,
                d.0,
                d.1,
                rnd(0xffffff)
            ));
        }
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="800" viewBox="0 0 1200 800">{paths}</svg>"#
        )
    };
    let small = illustration(60, "1");
    let large = illustration(1200, "0.5");
    for (name, svg, sizes) in [
        ("icon", ICON.to_string(), [(16.0, 16.0), (32.0, 32.0)]),
        ("small shapes", small, [(1200.0, 800.0), (2400.0, 1600.0)]),
        ("large shapes", large, [(1200.0, 800.0), (2400.0, 1600.0)]),
    ] {
        let t = std::time::Instant::now();
        let v = rinch_dom::svg_image::decode_svg(svg.as_bytes()).unwrap();
        let parse = t.elapsed();
        for (w, h) in sizes {
            let t = std::time::Instant::now();
            let r = v.raster(w, h);
            eprintln!(
                "{name} ({} bytes): parse {parse:?}, raster {w}x{h} {:?}, estimated work {:.2e}{}",
                svg.len(),
                t.elapsed(),
                v.estimated_work(w as u32, h as u32),
                if r.is_none() {
                    " (refused: over the work budget)"
                } else {
                    ""
                }
            );
        }
    }
}
