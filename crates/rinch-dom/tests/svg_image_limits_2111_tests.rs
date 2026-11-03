//! #2111, review of PR #2125: an SVG file is untrusted input, and parsing or
//! rasterising one must neither crash the app nor hold its UI thread.
//!
//! Each test here was watched failing (a stack-overflow abort, an unbounded
//! entity expansion, a multi-second raster, a re-raster every paint) before
//! the limits in `rinch_dom::svg_image` existed. A stack overflow aborts the
//! whole process, so these live in their own test binary.

use base64::Engine;
use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::{decode_data_uri, decode_image_bytes};
use rinch_dom::svg_image::{self, MAX_DEPTH, MAX_NODES, MAX_RASTER_PX, decode_svg, sniff_svg};
use std::time::{Duration, Instant};

const NS: &str = r#"xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink""#;

/// Run `f` on a thread with a 2 MB stack (the size of a test thread, and of
/// the smallest UI-thread stack a desktop gives), so a debug build's frames
/// are measured against a real budget.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn nested_groups(depth: usize) -> String {
    format!(
        r#"<svg {NS} width="10" height="10">{}<rect width="10" height="10"/>{}</svg>"#,
        "<g>".repeat(depth),
        "</g>".repeat(depth)
    )
}

/// Deeply nested `<g>` overflowed the stack — roxmltree 0.21.1 and usvg
/// recurse once per level — and aborted the process: 4,000 levels (28 KB)
/// in a release build, 1,000 in a debug one (roxmltree 0.21.2 refuses past
/// its own limit, which is deeper than usvg survives). Both are refused now,
/// and a file at the depth limit still parses and rasterises on a 2 MB stack
/// in a debug build.
#[test]
fn deep_nesting_is_refused_not_a_stack_overflow() {
    for depth in [1000, 4000] {
        // Refused by reading the bytes, before any parser (roxmltree
        // recursed per level up to 0.21.1).
        let err = svg_image::prescan(nested_groups(depth).as_bytes()).expect_err("refused");
        assert!(err.contains("elements nested"), "{err}");
        let deep = nested_groups(depth);
        let err =
            on_small_stack(move || decode_svg(deep.as_bytes()).map(|_| ())).expect_err("refused");
        assert!(err.contains("nested"), "{depth}: {err}");
    }
    let at_limit = nested_groups(MAX_DEPTH - 2);
    let drew = on_small_stack(move || {
        let v = decode_svg(at_limit.as_bytes()).expect("the limit itself parses");
        v.raster(10.0, 10.0).is_some()
    });
    assert!(drew);
}

/// A chain of `<use>`, each naming the one before, nests as deeply as
/// `<g>` does once usvg expands it, while the file stays flat.
#[test]
fn a_deep_use_chain_is_refused() {
    let mut body = String::from(r#"<rect id="u0" width="1" height="1"/>"#);
    for i in 1..4000 {
        body.push_str(&format!(r##"<use id="u{i}" href="#u{}"/>"##, i - 1));
    }
    let svg = format!(r#"<svg {NS} width="10" height="10"><defs>{body}</defs></svg>"#);
    let err = on_small_stack(move || decode_svg(svg.as_bytes()).map(|_| ())).expect_err("refused");
    assert!(err.contains("nested"), "{err}");
}

/// Nested entities (a "billion laughs"): a 1 KB file that expands to tens of
/// megabytes. Refused at once, before anything is expanded.
#[test]
fn nested_entities_are_refused() {
    let mut dtd = String::from(
        r#"<!ENTITY a0 "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx">"#,
    );
    for i in 1..6 {
        dtd.push_str(&format!(
            r#"<!ENTITY a{i} "{}">"#,
            format!("&a{};", i - 1).repeat(16)
        ));
    }
    let svg = format!(
        r#"<?xml version="1.0"?><!DOCTYPE svg [{dtd}]><svg {NS} width="10" height="10"><desc>&a5;</desc></svg>"#
    );
    let t = Instant::now();
    let err = decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused");
    // Refused by rinch's own check, before roxmltree expands anything.
    assert!(err.contains("refers to another entity"), "{err}");
    assert!(t.elapsed() < Duration::from_millis(500));
}

/// Flat entities, many times over: also bounded, by the total they expand to.
#[test]
fn many_references_to_a_large_entity_are_refused() {
    let big = "x".repeat(4096);
    let svg = format!(
        r#"<!DOCTYPE svg [<!ENTITY e "{big}">]><svg {NS} width="10" height="10"><desc>{}</desc></svg>"#,
        "&e;".repeat(1024)
    );
    let err = decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused");
    assert!(err.contains("entit"), "{err}");
}

/// What Adobe Illustrator writes: flat namespace entities used in
/// attributes. They still work.
#[test]
fn illustrators_flat_entities_still_parse() {
    let svg = r##"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
	<!ENTITY ns_xlink "http://www.w3.org/1999/xlink">
]>
<svg version="1.1" xmlns="&ns_svg;" xmlns:xlink="&ns_xlink;" width="10" height="10">
<rect width="10" height="10" fill="#f00"/></svg>"##;
    assert!(sniff_svg(svg.as_bytes()), "an internal subset is skipped");
    let v = decode_svg(svg.as_bytes()).expect("parses");
    let r = v.raster(10.0, 10.0).unwrap();
    assert_eq!(&r.image.data[..4], &[255, 0, 0, 255]);
}

/// `<use>` fan-out: each level uses the one below ten times, so four levels
/// draw a full-size rect 10,000 times (a 1.2 KB file; 12 s at 1200x800 before
/// the limit). The raster is refused, quickly, and draws nothing.
#[test]
fn use_fan_out_is_refused_at_raster_time() {
    let mut defs = String::from(r##"<rect id="l0" width="400" height="400" fill="#f00"/>"##);
    for level in 1..=4 {
        defs.push_str(&format!(r#"<g id="l{level}">"#));
        for _ in 0..10 {
            defs.push_str(&format!(r##"<use href="#l{}"/>"##, level - 1));
        }
        defs.push_str("</g>");
    }
    let svg = format!(
        r##"<svg {NS} width="400" height="400"><defs>{defs}</defs><use href="#l4"/></svg>"##
    );
    let v = decode_svg(svg.as_bytes()).expect("10,000 nodes is under the node limit");
    let t = Instant::now();
    assert!(v.raster(400.0, 400.0).is_none(), "over the raster budget");
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
    // A small draw of the same file is within it.
    assert!(v.raster(40.0, 40.0).is_some());
}

/// Five levels of fan-out is past the node limit: refused at decode, before
/// usvg expands it.
#[test]
fn use_fan_out_past_the_node_limit_is_refused_at_decode() {
    let mut defs = String::from(r#"<rect id="l0" width="1" height="1"/>"#);
    for level in 1..=5 {
        defs.push_str(&format!(r#"<g id="l{level}">"#));
        for _ in 0..10 {
            defs.push_str(&format!(r##"<use href="#l{}"/>"##, level - 1));
        }
        defs.push_str("</g>");
    }
    let svg =
        format!(r##"<svg {NS} width="10" height="10"><defs>{defs}</defs><use href="#l5"/></svg>"##);
    assert!(10u64.pow(5) < 2 * MAX_NODES);
    let err = decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused");
    assert!(err.contains("elements"), "{err}");
}

/// The pixel cap is applied to the pixels allocated, after rounding up: a
/// 64,000,000 x 0.04 draw allocated 64,000,000 x 1.
#[test]
fn the_pixel_cap_holds_after_rounding_up() {
    let v = decode_svg(nested_groups(1).as_bytes()).unwrap();
    for (w, h) in [(64e6, 0.04), (0.04, 64e6), (1e5, 1e5)] {
        if let Some(r) = v.raster(w, h) {
            let px = r.image.width as f64 * r.image.height as f64;
            assert!(
                px <= MAX_RASTER_PX,
                "{w}x{h}: {}x{}",
                r.image.width,
                r.image.height
            );
        }
    }
}

/// A doctype with an internal subset, and one with a quoted `>`, are
/// skipped by the sniffer.
#[test]
fn the_sniffer_skips_a_doctypes_internal_subset() {
    assert!(sniff_svg(
        br#"<!DOCTYPE svg [ <!ENTITY a "x>y"> <!-- ] > --> ]><svg>"#
    ));
    assert!(sniff_svg(br#"<!DOCTYPE svg PUBLIC "a>b" "c"><svg/>"#));
    assert!(!sniff_svg(br#"<!DOCTYPE html [ ]><html>"#));
}

/// Forgiving base64, as Chrome decodes a `data:` URL: missing padding,
/// white space and percent-escapes in the base64 are all accepted.
#[test]
fn base64_data_urls_are_decoded_forgivingly() {
    let want = b"<svg/>!";
    let b64 = base64::engine::general_purpose::STANDARD.encode(want);
    assert!(b64.ends_with('='));
    let unpadded = b64.trim_end_matches('=');
    for url in [
        format!("data:image/svg+xml;base64,{b64}"),
        format!("data:image/svg+xml;base64,{unpadded}"),
        format!("data:image/svg+xml;base64,{}", b64.replace('=', "%3D")),
        format!("data:image/svg+xml;base64,{} {}", &b64[..4], &b64[4..]),
    ] {
        assert_eq!(decode_data_uri(&url).as_deref(), Some(&want[..]), "{url}");
    }
    assert_eq!(
        decode_data_uri("data:;base64,a"),
        None,
        "one left over is invalid"
    );
}

fn paint(doc: &mut RinchDocument) {
    doc.resolve_layout(400.0, 200.0);
    let mut painter = rinch_dom::paint::vello_painter::VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (400.0, 200.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

const ICON: &str = r##"data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20viewBox%3D%220%200%2016%2016%22%3E%3Crect%20width%3D%228%22%20height%3D%228%22%2F%3E%3C%2Fsvg%3E"##;

/// Five `<img>` of one SVG at five sizes are five rasters, made once: a
/// raster used by the frame being painted is never evicted (before, an image
/// kept 4 and the fifth evicted one each paint: 5, 10, 15 rasters).
#[test]
fn five_sizes_on_screen_do_not_thrash() {
    five_sizes(ICON);
    // The same when the five pass the byte budget from the start: what the
    // frame draws is kept over it, not evicted by the next raster it makes.
    let _budget = svg_image::test_budget(1);
    five_sizes(&format!("{ICON}%3C!--over-budget--%3E"));
}

fn five_sizes(src: &str) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    for w in [12, 16, 20, 24, 28] {
        let img = doc.create_element("img");
        doc.set_attribute(img, "src", src);
        doc.set_attribute(img, "style", &format!("width: {w}px; height: {w}px"));
        doc.append_child(body, img);
    }
    let made = |doc: &RinchDocument| {
        let v = doc.tree.image_cache.get(src).unwrap();
        v.vector().unwrap().rasters_made()
    };
    paint(&mut doc);
    assert_eq!(made(&doc), 5);
    paint(&mut doc);
    paint(&mut doc);
    assert_eq!(made(&doc), 5);
}

/// All SVG rasters on a thread share one byte budget: a gallery of many
/// files keeps what fits, the least recently drawn going first, and a raster
/// no frame has drawn for `RASTER_IDLE_FRAMES` frames is let go.
#[test]
fn svg_rasters_share_a_byte_budget_and_are_released_when_idle() {
    let _budget = svg_image::test_budget(64 * 1024);
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let mut imgs = Vec::new();
    for i in 0..20 {
        // Twenty distinct files, 32x32 (8 KB a raster as charged).
        let src = format!("{ICON}%3C!--{i}--%3E");
        let img = doc.create_element("img");
        doc.set_attribute(img, "src", &src);
        doc.set_attribute(img, "style", "width: 32px; height: 32px; display: none");
        doc.append_child(body, img);
        imgs.push(img);
    }
    // Show each one for one frame, then hide it.
    for &img in &imgs {
        doc.set_attribute(img, "style", "width: 32px; height: 32px");
        paint(&mut doc);
        doc.set_attribute(img, "style", "width: 32px; height: 32px; display: none");
        assert!(
            svg_image::raster_bytes() <= 64 * 1024,
            "{}",
            svg_image::raster_bytes()
        );
    }
    assert!(svg_image::raster_bytes() > 0);
    for _ in 0..=svg_image::RASTER_IDLE_FRAMES {
        paint(&mut doc);
    }
    assert_eq!(svg_image::raster_bytes(), 0, "idle rasters are released");
}

/// A `data:` SVG is parsed on the UI thread, inside layout: it too is
/// refused rather than overflowing that thread's stack.
#[test]
fn a_deep_data_url_svg_fails_on_the_ui_thread_without_crashing() {
    let deep = nested_groups(1000);
    let failed = on_small_stack(move || {
        let src = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&deep)
        );
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let img = doc.create_element("img");
        doc.set_attribute(img, "src", &src);
        doc.append_child(body, img);
        doc.resolve_layout(400.0, 200.0);
        doc.tree.image_cache.is_failed(&src)
    });
    assert!(failed);
}

/// Bytes the decoders know nothing of still fail as they did.
#[test]
fn garbage_fails() {
    assert!(decode_image_bytes(b"<svg").is_err());
}
