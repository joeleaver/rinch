//! A `background-image: url(data:…)` is decoded in place, as an `<img
//! src="data:…">` is, on every target (#2100).
//!
//! It used to go to the document's loader like any other background URL: the
//! default `FileImageLoader` read `data:…` as a file name and failed, and on
//! `wasm32` (no load threads) nothing was asked at all. Only `<img>` had the
//! in-place `data:` decode. A `data:` URL is never offered to any loader now.

use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A 3x2 opaque-red PNG, base64.
const RED_3X2: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";

/// The document's loader: counts, and has nothing.
struct Counting(Arc<AtomicUsize>);

impl ImageLoader for Counting {
    fn load(&self, _src: &str) -> ImageLoadResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ImageLoadResult::Failed("the document's loader has nothing".into())
    }
}

#[test]
fn a_data_url_background_is_decoded_by_the_layout_that_cascades_it() {
    let mut doc = RinchDocument::new();
    let asked = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(Counting(asked.clone())));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("width: 7px; height: 5px; background-image: url({RED_3X2})"),
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let img = doc
        .tree
        .image_cache
        .get(RED_3X2)
        .expect("decoded by the layout that cascaded it, before its paint");
    assert_eq!((img.width, img.height), (3, 2));
    assert_eq!(&img.data[..4], &[255, 0, 0, 255]);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        0,
        "a `data:` URL is never offered to the document's loader"
    );
}

/// The same with no loader installed: a `data:` URL needs none.
#[test]
fn a_data_url_background_needs_no_loader() {
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = None;
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("width: 7px; height: 5px; background-image: url({RED_3X2})"),
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.image_cache.get(RED_3X2).is_some());
}
