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
    assert!(err.contains("refers to another"), "{err}");
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
    assert!(err.contains("expands to"), "{err}");
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

/// What each filter costs resvg per pixel of its region, for calibrating
/// the work budget. `#[ignore]`d; `--release -- --ignored --nocapture
/// filter_cost_probe`.
#[test]
#[ignore = "a timing probe"]
fn filter_cost_probe() {
    let cases: &[(&str, &str)] = &[
        ("fill", ""),
        ("blur 3", r#"<feGaussianBlur stdDeviation="3"/>"#),
        ("blur 1e4", r#"<feGaussianBlur stdDeviation="10000"/>"#),
        ("blur 1.5 (iir)", r#"<feGaussianBlur stdDeviation="1.5"/>"#),
        ("shadow 1e4", r#"<feDropShadow stdDeviation="10000"/>"#),
        ("morph 2", r#"<feMorphology radius="2"/>"#),
        ("morph 8", r#"<feMorphology radius="8"/>"#),
        ("turb 1", r#"<feTurbulence numOctaves="1"/>"#),
        ("turb 8", r#"<feTurbulence numOctaves="8"/>"#),
        (
            "conv 3",
            r#"<feConvolveMatrix order="3" kernelMatrix="1 1 1 1 1 1 1 1 1"/>"#,
        ),
        (
            "conv 9",
            format!(
                r#"<feConvolveMatrix order="9" kernelMatrix="{}"/>"#,
                "1 ".repeat(81)
            )
            .leak(),
        ),
        (
            "diffuse",
            r#"<feDiffuseLighting><feDistantLight/></feDiffuseLighting>"#,
        ),
        (
            "specular",
            r#"<feSpecularLighting specularExponent="20"><fePointLight x="5" y="5" z="5"/></feSpecularLighting>"#,
        ),
        (
            "displace",
            r#"<feTurbulence result="t"/><feDisplacementMap in="SourceGraphic" in2="t" scale="20"/>"#,
        ),
        ("tile", r#"<feOffset dx="5" result="o"/><feTile in="o"/>"#),
        (
            "colormatrix",
            r#"<feColorMatrix type="hueRotate" values="30"/>"#,
        ),
    ];
    for (name, fe) in cases {
        let filter = if fe.is_empty() {
            String::new()
        } else {
            format!(r#"<filter id="f" x="0" y="0" width="1" height="1">{fe}</filter>"#)
        };
        let attr = if fe.is_empty() {
            ""
        } else {
            r#" filter="url(#f)""#
        };
        let svg = format!(
            r##"<svg {NS} width="400" height="400">{filter}<rect width="400" height="400" fill="#f80"{attr}/></svg>"##
        );
        let v = decode_svg(svg.as_bytes()).unwrap();
        let t = Instant::now();
        let r = v.raster(400.0, 400.0);
        eprintln!(
            "{name}: {:?} ({:.1} ns/px), estimated {:.2e}, drawn {}",
            t.elapsed(),
            t.elapsed().as_nanos() as f64 / 160_000.0,
            v.estimated_work(400, 400),
            r.is_some()
        );
    }
    // Dashes: 100 paths of length 1,000 dashed every 0.1.
    let paths: String = (0..100)
        .map(|i| {
            format!(r##"<path d="M0 {i} H1000" stroke="#000" stroke-dasharray="0.05 0.05"/>"##)
        })
        .collect();
    let svg = format!(r#"<svg {NS} width="1000" height="100">{paths}</svg>"#);
    let v = decode_svg(svg.as_bytes()).unwrap();
    let t = Instant::now();
    v.raster(1000.0, 100.0);
    eprintln!("dashes 1e6: {:?}", t.elapsed());
}

// ---- Second review of #2125: seven ways past the first limits, each under
// 80 KB, each proved in release. Each test asserts a refusal (or a fast
// draw) well inside the time the attack took.

fn quick<T>(limit_ms: u64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    assert!(
        t.elapsed() < Duration::from_millis(limit_ms),
        "took {:?}",
        t.elapsed()
    );
    out
}

/// A name declared twice: roxmltree keeps the first declaration, so a
/// reader that kept the last counted 1 byte where 64 KB was expanded (78 KB
/// file: 1.59 s, 529 MB). A duplicate refuses the file.
#[test]
fn a_duplicate_entity_is_refused() {
    let svg = format!(
        r#"<!DOCTYPE svg [<!ENTITY a "{}"><!ENTITY a "x">]><svg {NS} width="10" height="10"><desc>{}</desc></svg>"#,
        "A".repeat(65536),
        "&a;".repeat(4096)
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    assert!(err.contains("twice"), "{err}");
}

/// A processing instruction in the internal subset whose text holds `]>`:
/// a reader that did not know PIs ended the subset there and never saw the
/// nested entities after it (5.6 KB: 415 MB). Anything but a flat entity in
/// the subset refuses the file.
#[test]
fn a_processing_instruction_in_the_subset_is_refused() {
    let mut dtd = format!(r#"<?x ]>?><!ENTITY a0 "{}">"#, "x".repeat(64));
    for i in 1..3 {
        dtd.push_str(&format!(
            r#"<!ENTITY a{i} "{}">"#,
            format!("&a{};", i - 1).repeat(64)
        ));
    }
    let svg = format!(
        r#"<!DOCTYPE svg [{dtd}]><svg {NS} width="10" height="10"><desc>{}</desc></svg>"#,
        "&a2;".repeat(4)
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    assert!(
        err.contains("holds something other than flat entities"),
        "{err}"
    );
}

fn filtered(fe: &str, size: u32) -> String {
    format!(
        r##"<svg {NS} width="{size}" height="{size}"><filter id="f">{fe}</filter><rect width="{size}" height="{size}" fill="#f80" filter="url(#f)"/></svg>"##
    )
}

/// `feMorphology radius="50"` on 100x100: its cost per pixel grows with the
/// radius squared, so 256 px took 1.32 s. Refused there, drawn small.
#[test]
fn a_large_morphology_radius_is_refused() {
    let v = decode_svg(filtered(r#"<feMorphology radius="50"/>"#, 100).as_bytes()).unwrap();
    assert!(quick(300, || v.raster(256.0, 256.0)).is_none());
    assert!(v.raster(16.0, 16.0).is_some());
}

/// The morphology charge and cap, apart: radius 60 over 512x512 is under
/// the radius cap but its cost per pixel (121² samples) passes the budget;
/// radius 100 over 64x64 is cheap but past the cap of 64 device pixels.
#[test]
fn morphology_is_charged_by_its_radius_and_capped() {
    let v = decode_svg(filtered(r#"<feMorphology radius="60"/>"#, 512).as_bytes()).unwrap();
    assert!(quick(300, || v.raster(512.0, 512.0)).is_none(), "charged");
    let v = decode_svg(filtered(r#"<feMorphology radius="100"/>"#, 64).as_bytes()).unwrap();
    assert!(v.raster(64.0, 64.0).is_none(), "capped");
    assert!(v.raster(32.0, 32.0).is_some(), "50 px is under the cap");
}

/// `feTurbulence numOctaves="1e5"` took over 2 s; its octaves are clamped
/// to a browser's 10, so it draws, quickly.
#[test]
fn turbulence_octaves_are_clamped() {
    let v = decode_svg(
        filtered(
            r#"<feTurbulence baseFrequency="0.05" numOctaves="1e5"/>"#,
            100,
        )
        .as_bytes(),
    )
    .unwrap();
    assert!(quick(300, || v.raster(100.0, 100.0)).is_some());
}

/// `feConvolveMatrix order="64"` (8.4 KB; 1.72 s at 400x400) is refused.
#[test]
fn a_large_convolve_matrix_is_refused() {
    let fe = format!(
        r#"<feConvolveMatrix order="64" kernelMatrix="{}"/>"#,
        "1 ".repeat(64 * 64)
    );
    let err = decode_svg(filtered(&fe, 400).as_bytes())
        .map(|_| ())
        .expect_err("refused");
    assert!(err.contains("ConvolveMatrix"), "{err}");
}

/// Dense dashes: a dash every 0.005 over 100 lines of 1,000 is 20 million
/// dashes. Charged per dash, the raster is refused.
#[test]
fn dense_dashes_are_charged() {
    let paths: String = (0..100)
        .map(|i| {
            format!(r##"<path d="M0 {i} H1000" stroke="#000" stroke-dasharray="0.005 0.005"/>"##)
        })
        .collect();
    let svg = format!(r#"<svg {NS} width="1000" height="100">{paths}</svg>"#);
    let v = decode_svg(svg.as_bytes()).unwrap();
    assert!(quick(300, || v.raster(1000.0, 100.0)).is_none());
}

/// A huge blur deviation costs what a small one does (resvg's box blur):
/// drawn, quickly.
#[test]
fn a_huge_blur_draws_quickly() {
    let v = decode_svg(filtered(r#"<feGaussianBlur stdDeviation="1000000"/>"#, 400).as_bytes())
        .unwrap();
    assert!(quick(500, || v.raster(400.0, 400.0)).is_some());
}

/// A filter region far past the canvas makes resvg allocate up to five
/// times the canvas each way per pixmap: past the layer cap, refused.
#[test]
fn a_huge_filter_region_is_refused() {
    let fe = r##"<feFlood flood-color="#f00"/>"##;
    let svg = format!(
        r##"<svg {NS} width="100" height="100"><filter id="f" filterUnits="userSpaceOnUse" x="-100000" y="-100000" width="200000" height="200000">{fe}</filter><rect width="100" height="100" filter="url(#f)"/></svg>"##
    );
    let v = decode_svg(svg.as_bytes()).unwrap();
    assert!(quick(300, || v.raster(1024.0, 1024.0)).is_none());
    assert!(v.raster(100.0, 100.0).is_some());
}

/// A stylesheet's `marker-mid: url(#m)` reaches every vertex of every path,
/// which usvg expands while it builds the tree: counted from the sheet
/// before usvg runs, and refused.
#[test]
fn stylesheet_references_are_counted_before_usvg_expands_them() {
    let rects: String = (0..100)
        .map(|i| format!(r#"<rect x="{i}" width="1" height="1"/>"#))
        .collect();
    let d: String = (0..5000)
        .map(|i| format!(" L{} {}", i % 100, i / 100))
        .collect();
    let svg = format!(
        r#"<svg {NS} width="100" height="100"><style>path {{ marker-mid: url(#m) }}</style><marker id="m">{rects}</marker><path d="M0 0{d}" stroke="black"/></svg>"#
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    // Refused before usvg built anything (usvg's own count says "built").
    assert!(err.contains("expands to"), "{err}");
}
