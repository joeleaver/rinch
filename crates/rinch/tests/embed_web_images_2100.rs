//! An embedded `RinchContext` in the browser loads every image source it
//! loads on the desktop (#2100): `data:` URLs, and the app's own schemes
//! (`register_image_scheme`), for `<img>` and `background-image` alike.
//!
//! There are no load threads on wasm32, and the load request was a no-op
//! there: only `<img src="data:…">`, which is decoded in place, ever showed.
//! A scheme loader was never asked, and a `background-image` never loaded.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch --no-default-features --features embed \
//!     --target wasm32-unknown-unknown --test embed_web_images_2100
//! ```
#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "embed"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::image::{ImageLoadResult, register_image_scheme, reload_image};
use rinch_platform::AppAction;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// A 3x2 opaque-red PNG.
const RED_3X2_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 3, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 157, 116, 102, 26, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31,
    134, 25, 144, 57, 0, 155, 126, 11, 245, 114, 176, 185, 60, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
    96, 130,
];

/// The same PNG as a `data:` URL.
const RED_3X2_DATA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 400,
        height: 300,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

fn decoded(ctx: &RinchContext, src: &str) -> Option<(u32, u32)> {
    let doc = ctx.app().doc()?.borrow();
    doc.tree.image_cache.get(src).map(|i| (i.width, i.height))
}

fn img_box(ctx: &RinchContext, id: rinch_core::dom::NodeId) -> (f32, f32) {
    let doc = ctx.app().doc().expect("mounted").borrow();
    let n = doc.tree.nodes.get(id.0).expect("node in tree");
    (n.layout.width, n.layout.height)
}

#[wasm_bindgen_test]
fn every_image_source_loads_in_an_embedded_context() {
    let asked = Arc::new(AtomicUsize::new(0));
    let late_ready = Arc::new(AtomicBool::new(false));
    let (a, ready) = (asked.clone(), late_ready.clone());
    register_image_scheme("embed2100", move |src: &str| {
        a.fetch_add(1, Ordering::SeqCst);
        if src == "embed2100:late" && !ready.load(Ordering::SeqCst) {
            return ImageLoadResult::Failed("not here yet".into());
        }
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });

    let ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ids_in = ids.clone();
    let mut ctx = RinchContext::new(cfg(), move |scope: &mut RenderScope| -> NodeHandle {
        let root = scope.create_element("div");
        let img = |scope: &mut RenderScope, src: &str| {
            let img = scope.create_element("img");
            img.set_attribute("src", src);
            img
        };
        let bg = |scope: &mut RenderScope, url: &str| {
            let div = scope.create_element("div");
            div.set_attribute(
                "style",
                &format!("width: 7px; height: 5px; background-image: url({url})"),
            );
            div
        };
        let control = img(scope, RED_3X2_DATA);
        let scheme_img = img(scope, "embed2100:img");
        let late_img = img(scope, "embed2100:late");
        let data_bg = bg(scope, RED_3X2_DATA);
        let scheme_bg = bg(scope, "embed2100:bg");
        for n in [&control, &scheme_img, &late_img, &data_bg, &scheme_bg] {
            root.append_child(n);
        }
        ids_in.lock().unwrap().extend([
            control.node_id(),
            scheme_img.node_id(),
            late_img.node_id(),
        ]);
        root
    });
    // In the frame that asked: the mount's own layout. A scheme loader that
    // answers at once is published by it, background included (the layout
    // drains again after it requests the frame's backgrounds).
    assert_eq!(
        decoded(&ctx, "embed2100:bg"),
        Some((3, 2)),
        "a background scheme url, by the layout that cascaded it"
    );
    assert_eq!(
        decoded(&ctx, "embed2100:img"),
        Some((3, 2)),
        "an <img src=scheme:>, by the first layout"
    );
    ctx.update(&[]);
    let ids = ids.lock().unwrap().clone();
    let (control, scheme_img, late_img) = (ids[0], ids[1], ids[2]);

    // Positive control: the one path that always worked.
    assert_eq!(
        decoded(&ctx, RED_3X2_DATA),
        Some((3, 2)),
        "control: <img src=data:>"
    );
    assert_eq!(img_box(&ctx, control), (3.0, 2.0), "control: its box");

    // The app's scheme, for an `<img>`: the picture, and its box.
    assert_eq!(
        decoded(&ctx, "embed2100:img"),
        Some((3, 2)),
        "<img src=scheme:>"
    );
    assert_eq!(
        img_box(&ctx, scheme_img),
        (3.0, 2.0),
        "the scheme <img>'s box"
    );
    // The app's scheme, for a background.
    assert_eq!(
        decoded(&ctx, "embed2100:bg"),
        Some((3, 2)),
        "background scheme url"
    );

    // A source the app's loader could not answer yet stays empty, and shows
    // once the app reloads it: the next update lays it out and asks for a
    // redraw.
    assert_eq!(decoded(&ctx, "embed2100:late"), None);
    assert_eq!(img_box(&ctx, late_img), (0.0, 0.0));
    late_ready.store(true, Ordering::SeqCst);
    reload_image("embed2100:late");
    let mut redraw = false;
    for _ in 0..3 {
        redraw |= ctx.update(&[]).contains(&AppAction::RequestRedraw);
        if decoded(&ctx, "embed2100:late").is_some() {
            break;
        }
    }
    assert_eq!(decoded(&ctx, "embed2100:late"), Some((3, 2)), "reloaded");
    assert_eq!(
        img_box(&ctx, late_img),
        (3.0, 2.0),
        "the reloaded <img>'s box"
    );
    assert!(redraw, "the reload's landing asks the host for a redraw");

    // Each scheme source was asked once, the late one once more on reload.
    assert_eq!(asked.load(Ordering::SeqCst), 4);
    drop(ctx);
    rinch_core::image::unregister_image_scheme("embed2100");
}

/// The `data:` background last, in its own test so its failure at the base
/// does not hide the scheme half's.
#[wasm_bindgen_test]
fn a_data_url_background_loads_in_an_embedded_context() {
    let ctx = RinchContext::new(cfg(), move |scope: &mut RenderScope| -> NodeHandle {
        let div = scope.create_element("div");
        div.set_attribute(
            "style",
            &format!("width: 7px; height: 5px; background-image: url({RED_3X2_DATA})"),
        );
        div
    });
    assert_eq!(
        decoded(&ctx, RED_3X2_DATA),
        Some((3, 2)),
        "background-image: url(data:)"
    );
}

/// A loader is called with no document borrowed (review of #2107). One that
/// tells the app a picture was asked for by writing a signal — `send`, the
/// portable way, which on wasm32 runs at once — flushes an effect that writes
/// the DOM. Called inside `set_attribute` or `resolve_layout`, that write
/// re-borrowed the document and panicked `RefCell already borrowed`.
fn mount_signalling(scheme: &'static str, src: &'static str, as_bg: bool) -> RinchContext {
    RinchContext::new(cfg(), move |scope: &mut RenderScope| -> NodeHandle {
        let asked: rinch_core::reactive::Signal<u32> = rinch_core::reactive::Signal::new(0);
        register_image_scheme(scheme, move |_src: &str| {
            asked.send(1);
            ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
        });
        let root = scope.create_element("div");
        root.set_attribute("data-asked", "0");
        let r = root.clone();
        std::mem::forget(rinch_core::reactive::Effect::new(move || {
            r.set_attribute("data-asked", &asked.get().to_string());
        }));
        let el = if as_bg {
            let div = scope.create_element("div");
            div.set_attribute(
                "style",
                &format!("width: 7px; height: 5px; background-image: url({src})"),
            );
            div
        } else {
            let img = scope.create_element("img");
            img.set_attribute("src", src);
            img
        };
        root.append_child(&el);
        root
    })
}

fn asked_attr(ctx: &RinchContext) -> Option<String> {
    let doc = ctx.app().doc()?.borrow();
    doc.tree
        .nodes
        .iter()
        .find_map(|(_, n)| n.attributes.get("data-asked").cloned())
}

#[wasm_bindgen_test]
fn a_loader_that_writes_a_signal_from_an_img_src() {
    let mut ctx = mount_signalling("embed2100sig", "embed2100sig:x", false);
    ctx.update(&[]);
    assert_eq!(decoded(&ctx, "embed2100sig:x"), Some((3, 2)));
    assert_eq!(asked_attr(&ctx).as_deref(), Some("1"), "the effect ran");
    drop(ctx);
    rinch_core::image::unregister_image_scheme("embed2100sig");
}

#[wasm_bindgen_test]
fn a_loader_that_writes_a_signal_from_a_background() {
    let mut ctx = mount_signalling("embed2100sigbg", "embed2100sigbg:x", true);
    // Same frame: the mount loads it.
    assert_eq!(decoded(&ctx, "embed2100sigbg:x"), Some((3, 2)));
    ctx.update(&[]);
    assert_eq!(asked_attr(&ctx).as_deref(), Some("1"), "the effect ran");
    drop(ctx);
    rinch_core::image::unregister_image_scheme("embed2100sigbg");
}
