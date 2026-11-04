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
    // Three levels (1,000 rects) is within it at a small size.
    let svg = svg.replace(
        r##"<use href="#l4"/></svg>"##,
        r##"<use href="#l3"/></svg>"##,
    );
    let v = decode_svg(svg.as_bytes()).unwrap();
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
        ("flood", r##"<feFlood flood-color="#08f"/>"##),
        ("offset", r#"<feOffset dx="5"/>"#),
        (
            "composite",
            r#"<feOffset dx="5" result="o"/><feComposite in="SourceGraphic" in2="o"/>"#,
        ),
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

/// Filters outside the profile — the ones whose cost per pixel a file sets
/// (`feMorphology`'s radius: 1.32 s at 256 px; `feTurbulence numOctaves="1e5"`:
/// over 2 s; `feConvolveMatrix order="64"`: 1.72 s) and the rest — refuse
/// the file at decode, quickly.
#[test]
fn filters_outside_the_profile_are_refused() {
    let kernel = format!(
        r#"<feConvolveMatrix order="64" kernelMatrix="{}"/>"#,
        "1 ".repeat(64 * 64)
    );
    for fe in [
        r#"<feMorphology radius="50"/>"#,
        r#"<feTurbulence baseFrequency="0.05" numOctaves="1e5"/>"#,
        kernel.as_str(),
        r#"<feDiffuseLighting><feDistantLight/></feDiffuseLighting>"#,
        r#"<feSpecularLighting><fePointLight/></feSpecularLighting>"#,
        r#"<feTurbulence result="t"/><feDisplacementMap in="SourceGraphic" in2="t"/>"#,
        r#"<feOffset result="o"/><feTile in="o"/>"#,
        r#"<feComponentTransfer><feFuncR type="identity"/></feComponentTransfer>"#,
        r##"<feImage href="#r"/>"##,
    ] {
        let svg = filtered(fe, 100);
        let err = quick(300, || {
            decode_svg(svg.as_bytes()).map(|_| ()).expect_err(fe)
        });
        assert!(err.contains("profile"), "{fe}: {err}");
    }
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
    // In a stylesheet.
    let svg = format!(
        r#"<svg {NS} width="100" height="100"><style>path {{ marker-mid: url(#m) }}</style><marker id="m">{rects}</marker><path d="M0 0{d}" stroke="black"/></svg>"#
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    // Inlined onto the path, and counted per vertex like an attribute.
    assert!(err.contains("expands to"), "{err}");
    // As an attribute: within the profile, but counted per vertex before
    // usvg built anything (usvg's own count says "built").
    let svg = format!(
        r#"<svg {NS} width="100" height="100"><marker id="m">{rects}</marker><path d="M0 0{d}" stroke="black" marker-mid="url(#m)"/></svg>"#
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    assert!(err.contains("expands to"), "{err}");
}

/// How much of a real-world corpus the profile draws. `#[ignore]`d; run
/// with `RINCH_SVG_CORPUS=<list.tsv of label\tpath> … -- --ignored
/// --nocapture svg_corpus_probe`.
#[test]
#[ignore = "a corpus probe"]
fn svg_corpus_probe() {
    use std::collections::BTreeMap;
    let Ok(list) = std::env::var("RINCH_SVG_CORPUS") else {
        return;
    };
    let list = std::fs::read_to_string(list).unwrap();
    // label -> (drawn, refused by the profile, failed otherwise, not drawn at 48px)
    let mut by: BTreeMap<String, (u32, u32, u32, u32)> = BTreeMap::new();
    let mut reasons: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut slowest = (Duration::ZERO, String::new());
    let mut slowest_raster = (Duration::ZERO, String::new());
    // Sanitized vs original, both through usvg and resvg at 48x48: files
    // identical, within 2 levels, and the worst differences.
    let (mut same, mut close, mut worst) = (0u32, 0u32, Vec::<(u8, usize, String)>::new());
    let render = |text: &str| -> Option<Vec<u8>> {
        use resvg::{tiny_skia, usvg};
        let options = usvg::Options {
            default_size: usvg::Size::from_wh(300.0, 150.0)?,
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(text, &options).ok()?;
        let size = tree.size();
        let mut pixmap = tiny_skia::Pixmap::new(48, 48)?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(48.0 / size.width(), 48.0 / size.height()),
            &mut pixmap.as_mut(),
        );
        Some(pixmap.take())
    };
    for line in list.lines() {
        let Some((label, path)) = line.split_once('\t') else {
            continue;
        };
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let t = Instant::now();
        let entry = by.entry(label.to_string()).or_default();
        let size: f64 = std::env::var("RINCH_SVG_CORPUS_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(48.0);
        match decode_svg(&bytes) {
            Ok(v) => {
                let r = Instant::now();
                let drawn = v.raster(size, size).is_some();
                if r.elapsed() > slowest_raster.0 {
                    slowest_raster = (r.elapsed(), path.to_string());
                }
                if !drawn && std::env::var("RINCH_SVG_LIST_UNDRAWN").is_ok() {
                    eprintln!(
                        "undrawn: {path} {:.2e}",
                        v.estimated_work(size as u32, size as u32)
                    );
                }
                if drawn {
                    entry.0 += 1;
                } else {
                    entry.3 += 1;
                }
                let original = std::str::from_utf8(&bytes).ok().and_then(render);
                let rebuilt = svg_image::sanitized_text(&bytes)
                    .ok()
                    .and_then(|t| render(&t));
                if let (Some(a), Some(b)) = (original, rebuilt) {
                    let diffs: Vec<u8> = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).collect();
                    let max = diffs.iter().copied().max().unwrap_or(0);
                    let count = diffs.iter().filter(|d| **d > 2).count();
                    if max == 0 {
                        same += 1;
                    } else if max <= 2 {
                        close += 1;
                    } else {
                        worst.push((max, count, path.to_string()));
                    }
                }
            }
            Err(e) => {
                let key: String = e.chars().filter(|c| !c.is_ascii_digit()).collect();
                if e.contains("profile") {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
                *reasons.entry((label.to_string(), key)).or_default() += 1;
            }
        }
        if t.elapsed() > slowest.0 {
            slowest = (t.elapsed(), path.to_string());
        }
    }
    for (label, (ok, refused, failed, undrawn)) in &by {
        let n = ok + refused + failed + undrawn;
        eprintln!(
            "{label}: {ok}/{n} drawn ({:.1}%), {refused} refused by the profile, {failed} failed to parse, {undrawn} not drawn (over a budget)",
            100.0 * f64::from(*ok) / f64::from(n)
        );
    }
    let mut top: Vec<_> = reasons.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    for ((label, why), n) in top.iter().take(40) {
        eprintln!("  {n:5} {label}: {why}");
    }
    eprintln!("slowest: {:?} {}", slowest.0, slowest.1);
    eprintln!(
        "slowest raster: {:?} {}",
        slowest_raster.0, slowest_raster.1
    );
    worst.sort_by_key(|w| std::cmp::Reverse((w.1, w.0)));
    eprintln!(
        "rebuilt vs original at 48x48: {same} identical, {close} within 2 levels, {} differ",
        worst.len()
    );
    for (max, count, path) in worst.iter().take(25) {
        eprintln!("  max {max}, {count} channels over 2: {path}");
    }
}

// ---- Third review of #2125, and the profile that answers it (Joe,
// 2026-11-08): a file outside the profile is refused whole, quickly.

/// Refused at decode or at raster time; either way nothing is drawn.
fn refused(svg: &str, size: f64) -> bool {
    match decode_svg(svg.as_bytes()) {
        Err(_) => true,
        Ok(v) => v.raster(size, size).is_none(),
    }
}

fn r3_css(levels: usize) -> String {
    format!(
        r#"<svg {NS} viewBox="0 0 10 10"><style>x{}{{fill:red}}</style>{}<rect width="10" height="10"/>{}</svg>"#,
        " *".repeat(levels),
        "<g>".repeat(100),
        "</g>".repeat(100)
    )
}

fn r3_pattern(n: u32) -> String {
    format!(
        r#"<svg {NS} viewBox="0 0 100 100" width="100" height="100"><defs><pattern id="p" patternUnits="userSpaceOnUse" width="{n}" height="{n}"><rect width="{n}" height="{n}" fill="red"/></pattern></defs><rect width="100" height="100" fill="url(#p)"/></svg>"#
    )
}

fn r3_chain(kind: &str, attr: &str, n: usize) -> String {
    let defs: String = (0..n)
        .map(|i| {
            format!(
                r#"<{kind} id="c{i}" {attr}="url(#c{})"><rect width="100" height="100" fill="white"/></{kind}>"#,
                i + 1
            )
        })
        .collect();
    format!(
        r#"<svg {NS} viewBox="0 0 100 100"><defs>{defs}<{kind} id="c{n}"><rect width="100" height="100"/></{kind}></defs><rect width="100" height="100" {attr}="url(#c0)"/></svg>"#
    )
}

fn r3_stops(n: usize, uses: usize) -> String {
    let stops: String = (0..n)
        .map(|i| {
            format!(
                r##"<stop offset="{:.4}" stop-color="#{:02x}0000"/>"##,
                i as f64 / n as f64,
                i % 256
            )
        })
        .collect();
    format!(
        r##"<svg {NS} viewBox="0 0 100 100"><defs><linearGradient id="g">{stops}</linearGradient><rect id="r" width="100" height="100" fill="url(#g)"/></defs>{}</svg>"##,
        r##"<use href="#r"/>"##.repeat(uses)
    )
}

fn r3_merge(n: usize) -> String {
    format!(
        r#"<svg {NS} viewBox="0 0 100 100"><defs><filter id="f"><feMerge>{}</feMerge></filter></defs><rect width="100" height="100" filter="url(#f)"/></svg>"#,
        r#"<feMergeNode in="SourceGraphic"/>"#.repeat(n)
    )
}

/// The third review's five inputs: a CSS descendant chain that usvg's
/// matching backtracks on (832 B, over 10 s, inside usvg's parse); a
/// 16384-px pattern tile (1.05 GB at a 100x100 draw); 120-deep chains of
/// masks and of clip paths (2.3 s, 1.2 GB); 1,900 gradient stops drawn 8
/// times (2.6 s); an `feMerge` of 250 inputs (7 s). Each is refused, in
/// well under the time it took.
#[test]
fn the_third_reviews_inputs_are_refused_quickly() {
    let cases = [
        ("css", r3_css(5)),
        ("pattern", r3_pattern(16384)),
        ("mask chain", r3_chain("mask", "mask", 120)),
        ("clip chain", r3_chain("clipPath", "clip-path", 120)),
        ("stops", r3_stops(1900, 8)),
        ("merge", r3_merge(250)),
    ];
    for (name, svg) in cases {
        assert!(quick(300, || refused(&svg, 100.0)), "{name}");
    }
    // And the profile's edges: two compounds, 64 stops, 8 inputs, a mask
    // that uses no mask, a tile within the raster cap all draw.
    let svg = format!(
        r#"<svg {NS} viewBox="0 0 10 10"><style>g rect{{fill:red}} .a, #b, g > .c{{fill:blue}}</style><g><rect width="10" height="10"/></g></svg>"#
    );
    assert!(!refused(&svg, 10.0), "simple selectors");
    assert!(!refused(&r3_stops(64, 2), 100.0), "64 stops");
    assert!(!refused(&r3_merge(8), 100.0), "8 inputs");
    assert!(!refused(&r3_chain("mask", "mask", 0), 100.0), "one mask");
    assert!(!refused(&r3_pattern(512), 100.0), "a 512 tile");
}

/// Each of the profile's other refusals.
#[test]
fn the_profile_refuses_what_it_does_not_list() {
    let wrap = |body: &str| {
        format!(r#"<svg {NS} viewBox="0 0 10 10">{body}<rect width="10" height="10"/></svg>"#)
    };
    for (why, body) in [
        (
            "@ rule",
            "<style>@media print { rect { fill: red } }</style>",
        ),
        (
            "@import",
            "<style>@import url(x.css); rect { fill: red }</style>",
        ),
        ("pseudo-class", "<style>rect:hover { fill: red }</style>"),
        (
            "pseudo-class on a class",
            "<style>.a:hover { fill: red }</style>",
        ),
        (
            "attribute selector on an id",
            "<style>#b[x] { fill: red }</style>",
        ),
        ("attribute selector", "<style>rect[x] { fill: red }</style>"),
        ("two combinators", "<style>svg g rect { fill: red }</style>"),
        (
            "style hidden in an ignored element",
            "<foreignObject><style>x * * * * * { fill: red }</style></foreignObject>",
        ),
        (
            "too many primitives",
            r#"<filter id="f"><feOffset/><feOffset/><feOffset/><feOffset/><feOffset/><feOffset/><feOffset/></filter>"#,
        ),
        (
            "a pattern using a clip",
            r#"<clipPath id="c"/><pattern id="p"><rect clip-path="url(#c)"/></pattern>"#,
        ),
        (
            "a marker using a pattern through <use>",
            r##"<pattern id="p"/><rect id="r" fill="url(#p)"/><marker id="m"><use href="#r"/></marker>"##,
        ),
        ("tref", "<tref/>"),
    ] {
        let svg = wrap(body);
        let err = quick(300, || {
            decode_svg(svg.as_bytes()).map(|_| ()).expect_err(why)
        });
        assert!(err.contains("profile"), "{why}: {err}");
        // An @ rule is named as one, whatever else is wrong with it.
        if why.starts_with('@') {
            assert!(err.contains("@ rule"), "{why}: {err}");
        }
    }
}

/// What the profile ignores draws as nothing, and the rest of the file
/// draws: text, images, foreignObject, and what usvg does not know
/// (Inkscape's `sodipodi:namedview` and `flowRoot`, metadata's RDF).
#[test]
fn ignored_elements_do_not_refuse_the_file() {
    let svg = format!(
        r##"<svg {NS} xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" viewBox="0 0 10 10">
        <sodipodi:namedview pagecolor="#fff"/><metadata><rdf:RDF/></metadata>
        <flowRoot><flowRegion><rect width="5" height="5"/></flowRegion></flowRoot>
        <text x="0" y="5">hi</text><image href="x.png"/><foreignObject><div/></foreignObject>
        <rect width="10" height="10" fill="#f00"/></svg>"##
    );
    let v = decode_svg(svg.as_bytes()).expect("ignored, not refused");
    let r = v.raster(10.0, 10.0).unwrap();
    assert_eq!(&r.image.data[..4], &[255, 0, 0, 255]);
}

/// Real-world shapes draw: a Tabler icon, a KDE Breeze icon with its
/// colour-scheme stylesheet, Inkscape's drop shadow (five primitives) with
/// its editor metadata, and an Adobe Illustrator export (namespace
/// entities, class rules, a `switch` around `foreignObject` and a group).
/// The corpus these were picked from is in the PR (`svg_corpus_probe`).
#[test]
fn real_world_shapes_draw() {
    let tabler = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0"/><path d="M9 12l2 2l4 -4"/></svg>"#;
    let breeze = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs id="defs3051"><style type="text/css" id="current-color-scheme">
      .ColorScheme-Text { color:#232629; }
      </style></defs><path style="fill:currentColor;fill-opacity:1;stroke:none" d="M2 2h12v12H2z" class="ColorScheme-Text"/></svg>"#;
    let inkscape = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" width="64" height="64" viewBox="0 0 64 64" inkscape:version="1.3">
      <sodipodi:namedview id="nv" pagecolor="#ffffff"/>
      <defs><filter id="shadow" style="color-interpolation-filters:sRGB" x="-0.2" y="-0.2" width="1.4" height="1.4">
        <feFlood flood-opacity="0.5" flood-color="rgb(0,0,0)" result="flood"/>
        <feComposite in="flood" in2="SourceGraphic" operator="in" result="composite1"/>
        <feGaussianBlur in="composite1" stdDeviation="2" result="blur"/>
        <feOffset dx="2" dy="2" result="offset"/>
        <feComposite in="SourceGraphic" in2="offset" operator="over" result="composite2"/>
      </filter><linearGradient id="lg" x1="0" x2="1"><stop offset="0" stop-color="#36f"/><stop offset="1" stop-color="#0c9"/></linearGradient></defs>
      <g inkscape:label="Layer 1" inkscape:groupmode="layer"><rect x="8" y="8" width="44" height="44" rx="6" fill="url(#lg)" style="filter:url(#shadow)"/></g></svg>"##;
    let illustrator = r##"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 27.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_extend "http://ns.adobe.com/Extensibility/1.0/">
	<!ENTITY ns_ai "http://ns.adobe.com/AdobeIllustrator/10.0/">
	<!ENTITY ns_graphs "http://ns.adobe.com/Graphs/1.0/">
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
	<!ENTITY ns_xlink "http://www.w3.org/1999/xlink">
]>
<svg version="1.1" id="Layer_1" xmlns:x="&ns_extend;" xmlns:i="&ns_ai;" xmlns:graph="&ns_graphs;" xmlns="&ns_svg;" xmlns:xlink="&ns_xlink;" x="0px" y="0px" viewBox="0 0 32 32" style="enable-background:new 0 0 32 32;" xml:space="preserve">
<style type="text/css">
	.st0{fill:#E30613;}
	.st1{fill:url(#SVGID_1_);}
</style>
<switch>
	<foreignObject requiredExtensions="&ns_ai;" x="0" y="0" width="1" height="1"><i:aipgfRef xlink:href="#adobe_illustrator_pgf"></i:aipgfRef></foreignObject>
	<g i:extraneous="self">
		<linearGradient id="SVGID_1_" gradientUnits="userSpaceOnUse" x1="0" y1="16" x2="32" y2="16"><stop offset="0" style="stop-color:#FFFFFF"/><stop offset="1" style="stop-color:#000000"/></linearGradient>
		<rect class="st0" width="32" height="16"/><rect y="16" class="st1" width="32" height="16"/>
	</g>
</switch>
<i:pgf id="adobe_illustrator_pgf"><![CDATA[eJzLSM3JyVcozy/KSQEAGgQEXQ==]]></i:pgf>
</svg>"##;
    for (name, svg) in [
        ("tabler", tabler),
        ("breeze", breeze),
        ("inkscape", inkscape),
        ("illustrator", illustrator),
    ] {
        let v = decode_svg(svg.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let r = v
            .raster(48.0, 48.0)
            .unwrap_or_else(|| panic!("{name} not drawn"));
        assert!(
            r.image.data.chunks(4).any(|p| p[3] > 0),
            "{name} drew nothing"
        );
    }
}

/// Every attack the three reviews found, refused within 300 ms in one
/// place (release and debug alike): what each did before is in its own
/// test above.
#[test]
fn every_known_attack_is_refused_quickly() {
    let deep = nested_groups(1000);
    let mut chain = String::from(r#"<rect id="u0" width="1" height="1"/>"#);
    for i in 1..4000 {
        chain.push_str(&format!(r##"<use id="u{i}" href="#u{}"/>"##, i - 1));
    }
    let chain = format!(r#"<svg {NS} width="10" height="10"><defs>{chain}</defs></svg>"#);
    let dup = format!(
        r#"<!DOCTYPE svg [<!ENTITY a "{}"><!ENTITY a "x">]><svg {NS} width="10" height="10"><desc>{}</desc></svg>"#,
        "A".repeat(65536),
        "&a;".repeat(4096)
    );
    let pi =
        format!(r#"<!DOCTYPE svg [<?x ]>?><!ENTITY a "x">]><svg {NS} width="10" height="10"/>"#);
    let dashes: String = (0..100)
        .map(|i| {
            format!(r##"<path d="M0 {i} H1000" stroke="#000" stroke-dasharray="0.005 0.005"/>"##)
        })
        .collect();
    let dashes = format!(r#"<svg {NS} width="1000" height="100">{dashes}</svg>"#);
    let cases: Vec<(&str, String, f64)> = vec![
        ("deep nesting", deep, 10.0),
        ("use chain", chain, 10.0),
        ("duplicate entity", dup, 10.0),
        ("PI in the subset", pi, 10.0),
        ("dense dashes", dashes, 1000.0),
        ("css", r3_css(5), 10.0),
        ("pattern tile", r3_pattern(16384), 100.0),
        ("mask chain", r3_chain("mask", "mask", 120), 100.0),
        ("clip chain", r3_chain("clipPath", "clip-path", 120), 100.0),
        ("stops", r3_stops(1900, 8), 100.0),
        ("merge", r3_merge(250), 100.0),
        (
            "a marker on unseparated path data",
            format!(
                r##"<svg {NS} viewBox="0 0 100 100"><marker id="mk"><path id="q" d="M0 0h1v1z"/>{}</marker><path d="M0 0{}" marker-mid="url(#mk)"/></svg>"##,
                r##"<use href="#q"/>"##.repeat(64),
                "l.1.1".repeat(4096)
            ),
            100.0,
        ),
        (
            "a filter inside <image>",
            format!(
                r#"<svg {NS} viewBox="0 0 100 100"><image><filter id="f"><feTurbulence numOctaves="64"/></filter></image><rect width="100" height="100" filter="url(#f)"/></svg>"#
            ),
            -1.0,
        ),
        (
            "morphology",
            filtered(r#"<feMorphology radius="50"/>"#, 100),
            256.0,
        ),
        (
            "64-stop gradient pile",
            gradient_rects(false, 64, 48),
            2400.0,
        ),
    ];
    for (name, svg, size) in cases {
        // A negative size: drawn, harmlessly, at 1200x800 (the attack is
        // defused rather than refused).
        let r = on_small_stack(move || {
            let t = Instant::now();
            let refused = if size < 0.0 {
                let _ = decode_svg(svg.as_bytes()).map(|v| v.raster(1200.0, 800.0));
                true
            } else {
                refused(&svg, size)
            };
            (refused, t.elapsed())
        });
        assert!(r.0, "{name} was drawn");
        assert!(r.1 < Duration::from_millis(300), "{name}: {:?}", r.1);
    }
}

// ---- Fourth review of #2125: the checker and usvg read the original text
// two ways, and each difference was a bypass. usvg now reads only the
// document `svg_sanitize` rebuilds. These pin the three inputs and what the
// rebuild does with references.

fn sanitized(svg: &str) -> String {
    svg_image::sanitized_text(svg.as_bytes()).expect("within the profile")
}

/// r4-1: a filter inside `<image>` (233 B, 3.72 s at 1200x800): the
/// checker skipped the `<image>`, usvg still read its children's ids. The
/// `<image>` is left out of the rebuilt document, so `url(#f)` names nothing
/// and the rect draws unfiltered.
#[test]
fn a_filter_inside_an_ignored_element_is_not_reachable() {
    let svg = format!(
        r#"<svg {NS} viewBox="0 0 100 100" width="100" height="100"><image><filter id="f"><feTurbulence baseFrequency="0.05" numOctaves="64"/></filter></image><rect width="100" height="100" filter="url(#f)"/></svg>"#
    );
    let out = sanitized(&svg);
    assert!(
        !out.contains("filter") && !out.contains("Turbulence"),
        "{out}"
    );
    let v = decode_svg(svg.as_bytes()).unwrap();
    assert!(quick(300, || v.raster(1200.0, 800.0)).is_some());
}

/// r4-2: duplicate ids with decoys. usvg resolved `url()` to the last
/// element with an id and `<use>` to the first; the checker took the first,
/// so a 512-deep mask chain passed it (51 KB; 2.5 s, 1.3 GB). The rebuilt
/// document gives every element its own id and every reference rewritten
/// to the element this module resolved, so the chain is what the check sees.
#[test]
fn duplicate_ids_cannot_hide_a_chain() {
    let n = 512;
    let decoys: String = (1..=n).map(|i| format!(r#"<g id="m{i}"/>"#)).collect();
    let masks: String = (0..n)
        .map(|i| {
            format!(
                r#"<mask id="m{i}" mask="url(#m{})"><rect width="100" height="100" fill="white"/></mask>"#,
                i + 1
            )
        })
        .collect();
    // Decoys first or last, each reference resolves to the first element
    // of the kind it wants, in a document where every id is unique: the
    // chain is what the check sees, and it is refused either way.
    for svg in [
        format!(
            r#"<svg {NS} viewBox="0 0 100 100" width="100" height="100"><defs>{decoys}{masks}</defs><rect width="100" height="100" mask="url(#m0)"/></svg>"#
        ),
        format!(
            r#"<svg {NS} viewBox="0 0 100 100" width="100" height="100"><defs>{masks}{decoys}</defs><rect width="100" height="100" mask="url(#m0)"/></svg>"#
        ),
    ] {
        let err = quick(300, || {
            decode_svg(svg.as_bytes()).map(|_| ()).expect_err("a chain")
        });
        assert!(err.contains("uses a <mask>"), "{err}");
    }
}

/// r4-3: a marker on path data written without separators (`.1.1.1…`)
/// that the checker read as 2 vertices (4,096 real; usvg spent 2.39 s and
/// 873 MB before the tally refused it). Vertices are counted from the path
/// data as rewritten, so it is refused before usvg runs.
#[test]
fn markers_are_counted_on_the_real_vertices() {
    let uses: String = r##"<use href="#q"/>"##.repeat(64);
    let d = format!("M0 0{}", "l.1.1".repeat(4096));
    let svg = format!(
        r##"<svg {NS} viewBox="0 0 100 100" width="100" height="100"><marker id="mk" markerWidth="3" markerHeight="3" overflow="visible"><path id="q" d="M0 0h1v1z"/>{uses}</marker><path d="{d}" marker-mid="url(#mk)" stroke="black"/></svg>"##
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    assert!(err.contains("expands to"), "{err}");
    // `h`/`v` count one vertex each, not half.
    // 1,200 real vertices, 600 by the old reckoning: under the limit then.
    let d = format!("M0 0{}", "h1v1".repeat(600));
    let svg = format!(
        r##"<svg {NS} viewBox="0 0 100 100"><marker id="mk"><path id="q" d="M0 0h1v1z"/>{uses}</marker><path d="{d}" marker-mid="url(#mk)"/></svg>"##
    );
    let err = quick(300, || {
        decode_svg(svg.as_bytes()).map(|_| ()).expect_err("refused")
    });
    assert!(err.contains("expands to"), "{err}");
}

/// References the rebuilt document cannot resolve to an emitted element of
/// the right kind are dropped (a paint falls back, or is `none`), and an
/// `href` on anything but `<use>` and a gradient is left out.
#[test]
fn unresolved_references_are_dropped() {
    let out = sanitized(&format!(
        r##"<svg {NS} viewBox="0 0 10 10"><mask id="m"/><linearGradient id="m"><stop/></linearGradient><filter id="f"/><filter id="g" href="#f"/><pattern id="p"/><pattern id="q" href="#p"/><style>rect {{ mask: url(#m) }}</style><rect width="10" height="10" fill="url(#m) red" stroke="url(#nowhere)"/></svg>"##
    ));
    // `m` is a mask and a gradient: the fill (a paint server) reaches the
    // gradient, the CSS mask the mask, as Chrome 156 resolves each (its
    // first element of the kind). `nowhere` is nothing: the stroke is none.
    assert!(out.contains("fill=\"url(#i2) #ff0000\""), "{out}");
    assert!(out.contains("mask=\"url(#i1)\""), "{out}");
    assert!(out.contains("stroke=\"none\""), "{out}");
    assert!(!out.contains("href"), "{out}");
    assert!(!out.contains("<style"), "{out}");
}

/// Print the rebuilt document for `RINCH_SVG_DUMP=<file>` (a debugging aid).
#[test]
#[ignore = "a debugging aid"]
fn svg_sanitize_dump() {
    if let Ok(path) = std::env::var("RINCH_SVG_DUMP") {
        let bytes = std::fs::read(path).unwrap();
        eprintln!(
            "{}",
            svg_image::sanitized_text(&bytes).unwrap_or_else(|e| e)
        );
    }
}

/// One file's estimated work and raster time, for calibrating the budget:
/// `RINCH_SVG_WORK=<file> RINCH_SVG_SIZE=<w>x<h> … -- --ignored --nocapture
/// svg_work_probe`, timed from outside with `/usr/bin/time -v` (user + sys).
#[test]
#[ignore = "a calibration probe"]
fn svg_work_probe() {
    let (Ok(path), Ok(size)) = (
        std::env::var("RINCH_SVG_WORK"),
        std::env::var("RINCH_SVG_SIZE"),
    ) else {
        return;
    };
    let (w, h) = size.split_once('x').unwrap();
    let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
    let v = decode_svg(&std::fs::read(&path).unwrap()).unwrap();
    let t = Instant::now();
    let drawn = v.raster(f64::from(w), f64::from(h)).is_some();
    eprintln!(
        "work {:.3e} drawn {drawn} wall {:?}",
        v.estimated_work(w, h),
        t.elapsed()
    );
    // Refused: what drawing it would have cost (the rebuilt text, through
    // resvg directly).
    if !drawn && std::env::var("RINCH_SVG_UNBUDGETED").is_ok() {
        use resvg::{tiny_skia, usvg};
        let text = svg_image::sanitized_text(&std::fs::read(&path).unwrap()).unwrap();
        let tree = usvg::Tree::from_str(&text, &usvg::Options::default()).unwrap();
        let size = tree.size();
        let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
        let t = Instant::now();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height()),
            &mut pixmap.as_mut(),
        );
        eprintln!("unbudgeted render {:?}", t.elapsed());
    }
}

// ---- Fifth review of #2125: the work budget's calibration. Gradients cost
// resvg 2-8x a solid pixel, more with more stops, and the cap was about
// 2.2 s of CPU; gradients are now charged by kind and stop count and the cap
// is about 0.4 s.

fn gradient_rects(radial: bool, stops: usize, rects: usize) -> String {
    let stops: String = (0..stops)
        .map(|i| {
            format!(
                r##"<stop offset="{}" stop-color="#{:02x}{:02x}{:02x}"/>"##,
                i as f64 / (stops - 1) as f64,
                (i * 4) % 256,
                (i * 7) % 256,
                (i * 11) % 256
            )
        })
        .collect();
    let gradient = if radial {
        format!(r#"<radialGradient id="g">{stops}</radialGradient>"#)
    } else {
        format!(r#"<linearGradient id="g" x1="0" y1="0" x2="1" y2="1">{stops}</linearGradient>"#)
    };
    format!(
        r#"<svg {NS} viewBox="0 0 1000 1000" width="1000" height="1000"><defs>{gradient}</defs>{}</svg>"#,
        r#"<rect width="1000" height="1000" fill="url(#g)"/>"#.repeat(rects)
    )
}

/// The fifth review's inputs: 48, 200 and 256 overlapping rects filled with
/// a 64-stop gradient (2.0 s, 8.7 s and 8.0 s at 2400x1600, within the old
/// cap) and 256 with a 2-stop one (3.7 s). Each is refused at 2400x1600,
/// quickly; each draws small.
#[test]
fn gradient_fills_are_charged_by_their_stops() {
    for (radial, stops, rects) in [
        (false, 64, 48),
        (false, 64, 256),
        (true, 64, 200),
        (false, 2, 256),
    ] {
        let svg = gradient_rects(radial, stops, rects);
        let v = decode_svg(svg.as_bytes()).unwrap();
        assert!(
            quick(300, || v.raster(2400.0, 1600.0)).is_none(),
            "{rects} rects of {stops} stops"
        );
        assert!(v.raster(48.0, 48.0).is_some(), "{rects} rects drawn small");
    }
    // An ordinary gradient background, a full-window rect at 2x, draws.
    let v = decode_svg(gradient_rects(false, 8, 1).as_bytes()).unwrap();
    assert!(v.raster(2400.0, 1600.0).is_some());
    // The charge grows with the stop count: at one size, 64 stops are
    // estimated dearer than 2, and a radial dearer than a linear.
    let work = |radial, stops| {
        decode_svg(gradient_rects(radial, stops, 4).as_bytes())
            .unwrap()
            .estimated_work(1000, 1000)
    };
    assert!(
        work(false, 64) > 2.5 * work(false, 2),
        "{} {}",
        work(false, 64),
        work(false, 2)
    );
    assert!(work(true, 2) > work(false, 2));
    // A solid fill costs one unit a pixel: 4 rects of 1000x1000, and 4
    // elements.
    let solid = decode_svg(
        format!(
            r##"<svg {NS} viewBox="0 0 1000 1000" width="1000" height="1000">{}</svg>"##,
            r##"<rect width="1000" height="1000" fill="#f00"/>"##.repeat(4)
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        solid.estimated_work(1000, 1000),
        4.0e6 + 4.0 * svg_image::WORK_PER_ELEMENT
    );
    assert!(work(false, 2) > 2.0 * solid.estimated_work(1000, 1000));
}

/// The cap is about half a second of CPU (release, user + sys): 64
/// half-transparent full-size rects at 2400x1600 (2.46e8, 0.50 s measured)
/// are refused; 32 (1.23e8, 0.21 s) draw. At the old cap, 1e9 (about 2.2 s),
/// both drew.
#[test]
fn the_raster_budget_is_about_half_a_second() {
    let rects = |n: usize| {
        format!(
            r##"<svg {NS} viewBox="0 0 1000 1000" width="1000" height="1000">{}</svg>"##,
            r##"<rect width="1000" height="1000" fill="#f00" fill-opacity="0.5"/>"##.repeat(n)
        )
    };
    let v = decode_svg(rects(64).as_bytes()).unwrap();
    assert!(quick(300, || v.raster(2400.0, 1600.0)).is_none());
    let v = decode_svg(rects(32).as_bytes()).unwrap();
    assert!(v.estimated_work(2400, 1600) < svg_image::MAX_RASTER_WORK);
}
