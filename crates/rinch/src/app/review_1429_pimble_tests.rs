//! Review fixture for PR #1429: the Pimble flow end to end on the desktop
//! runtime (software frames).
//!
//! An editor document holds an `image` atom whose `src` is
//! `pimble-blob:<store>/<blob>`. The blob has not synced yet when the document
//! is shown. The user keeps typing, the same picture is inserted a second
//! time, and then the blob arrives and the app calls
//! `rinch::image::reload_image` from its sync thread.

use super::*;
use rinch_core::image::ImageLoadResult;
use rinch_editor_core::{Pos, Selection};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VP: (u32, u32) = (800, 600);

const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const SRC: &str = "pimble-blob:store-7/blob-42";

fn red_px(px: &[u8]) -> usize {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| **p == [255, 0, 0, 255])
        .count()
}

fn img_boxes(app: &RinchApp) -> Vec<(f32, f32)> {
    let doc = app.doc.as_ref().expect("mounted");
    let d = doc.borrow();
    let mut ids: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.tag() == Some("img") && n.parent.is_some())
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids.iter()
        .map(|&id| {
            let n = &d.tree.nodes[id];
            (n.layout.width, n.layout.height)
        })
        .collect()
}

fn frame(app: &mut RinchApp, full: bool) -> usize {
    app.refresh_editor_overlays();
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    if full {
        app.scene_dirty = true;
        app.has_previous_frame = false;
    }
    let (px, _, _) = app.build_pixels(1.0, VP, false);
    red_px(px)
}

fn idle(app: &mut RinchApp) {
    for _ in 0..16 {
        frame(app, false);
        if !app.has_pending_layout() && !app.has_dirty_nodes() {
            return;
        }
    }
}

#[test]
fn the_pimble_flow_a_blob_that_arrives_after_its_image_was_shown() {
    let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let asked = Arc::new(AtomicUsize::new(0));
    let (store_in, asked_in) = (store.clone(), asked.clone());
    crate::image::register_image_scheme("pimble-blob", move |src: &str| {
        asked_in.fetch_add(1, Ordering::SeqCst);
        match store_in.lock().unwrap().get(src) {
            Some(bytes) => ImageLoadResult::Loaded(bytes.clone()),
            None => ImageLoadResult::Failed(format!("{src} has not synced")),
        }
    });

    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!(
        r#"<p>before <img src="{SRC}" alt="a picture"> after</p><p>second paragraph</p>"#
    )));
    let h = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "padding: 20px; font-size: 16px; line-height: 20px; color: #000; background: #fff",
        );
        let e = h.mount(scope);
        root.append_child(&e);
        root
    });
    app.mount_component(800.0, 600.0);

    // The miss lands; the app goes idle on an empty box.
    let failed = |app: &RinchApp| {
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .image_cache
            .is_failed(SRC)
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !failed(&app) && Instant::now() < deadline {
        frame(&mut app, false);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(failed(&app), "control: the loader's miss is cached");
    idle(&mut app);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(img_boxes(&app), vec![(0.0, 0.0)]);
    assert_eq!(frame(&mut app, false), 0, "control: nothing red yet");

    // The user types in the image's paragraph: the miss is not asked again.
    handle.set_selection(Selection::cursor(Pos(1)));
    for _ in 0..5 {
        assert!(handle.insert_text("x"));
        frame(&mut app, false);
    }
    // The same picture is inserted a second time (a paste of the same blob).
    handle.set_selection(Selection::cursor(Pos(3)));
    assert!(handle.insert_image(SRC, "again"));
    idle(&mut app);
    std::thread::sleep(Duration::from_millis(50));
    idle(&mut app);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        1,
        "typing and a second element naming the source ask the loader nothing"
    );
    assert_eq!(img_boxes(&app), vec![(0.0, 0.0), (0.0, 0.0)]);

    // The blob syncs; the sync thread says so.
    store
        .lock()
        .unwrap()
        .insert(SRC.to_string(), RED_3X2_PNG.to_vec());
    std::thread::spawn(|| crate::image::reload_image(SRC))
        .join()
        .unwrap();
    assert!(
        app.has_dirty_nodes() && app.has_pending_layout(),
        "the idle gates trip"
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut red = 0;
    while Instant::now() < deadline {
        red = frame(&mut app, false); // incremental frames only: the damage must name the images
        if img_boxes(&app) == vec![(3.0, 2.0), (3.0, 2.0)] && red > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(img_boxes(&app), vec![(3.0, 2.0), (3.0, 2.0)]);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        2,
        "one load serves both elements"
    );
    idle(&mut app);
    let incremental = frame(&mut app, false);
    let full = frame(&mut app, true);
    assert_eq!(
        full, 12,
        "control: a full repaint draws two 3x2 red pictures"
    );
    assert_eq!(
        incremental, full,
        "the incremental frames after the reload drew what a full repaint draws (last seen {red})"
    );

    // The document still names the app's source.
    let html = rinch_editor_core::serialize::node_to_html(&handle.doc());
    assert_eq!(html.matches(SRC).count(), 2, "{html}");

    // Undo removes the second image; the first keeps its picture and nothing reloads.
    assert!(handle.command("undo"));
    idle(&mut app);
    assert_eq!(img_boxes(&app), vec![(3.0, 2.0)]);
    assert_eq!(frame(&mut app, true), 6);
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}
