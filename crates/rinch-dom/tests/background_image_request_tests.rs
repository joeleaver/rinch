//! A `background-image` URL is requested from the URLs the cascade produced,
//! not by walking every node in the document on every layout.
//!
//! `request_background_image_loads` used to scan the whole node slab each
//! `resolve_layout` — O(document) on a one-row hover, and the largest single
//! cost of `dom::hover.list_500` in `rinch-bench`. It now drains
//! `NodeTree::pending_background_urls`, which the cascade fills. These pin that
//! nothing the scan used to start is lost: a URL reached by a later restyle,
//! a URL cascaded while no loader was installed, and a URL shared by two nodes
//! (started once).

use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingLoader(Arc<AtomicUsize>);

impl ImageLoader for CountingLoader {
    fn load(&self, _src: &str) -> ImageLoadResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ImageLoadResult::Failed("nope".into())
    }
}

/// Wait (bounded) for the background loader threads to reach `n` loads, then
/// a little longer so an extra, unwanted load would also have landed.
fn loads_settle_at(count: &AtomicUsize, n: usize) -> usize {
    for _ in 0..200 {
        if count.load(Ordering::SeqCst) >= n {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    count.load(Ordering::SeqCst)
}

#[test]
fn a_background_image_reached_by_a_class_change_is_requested() {
    let mut doc = RinchDocument::new();
    let count = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(CountingLoader(count.clone())));
    let body = doc.body();
    let style = doc.create_element("style");
    let css = doc.create_text(".pic { background-image: url(bg-request-class.png); }");
    doc.append_child(style, css);
    doc.append_child(body, style);
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 10px; height: 10px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert!(!doc.tree.image_cache.contains("bg-request-class.png"));

    doc.set_attribute(div, "class", "pic");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.image_cache.contains("bg-request-class.png"),
        "the restyle that produced the background image must start its load"
    );
    assert_eq!(loads_settle_at(&count, 1), 1);

    // Restyling it again (the URL now cached) starts nothing new.
    doc.set_attribute(div, "style", "width: 11px; height: 10px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(loads_settle_at(&count, 1), 1);
    assert!(doc.tree.pending_background_urls.is_empty());
}

#[test]
fn a_url_cascaded_without_a_loader_starts_once_one_is_installed() {
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = None;
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background-image: url(bg-request-late.png)",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert!(!doc.tree.image_cache.contains("bg-request-late.png"));

    let count = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(CountingLoader(count.clone())));
    // Nothing is restyled: the URL must still be remembered.
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.image_cache.contains("bg-request-late.png"),
        "a URL noted while no loader was installed is started by the next layout with one"
    );
    assert_eq!(loads_settle_at(&count, 1), 1);
}

#[test]
fn a_url_two_nodes_share_is_started_once() {
    let mut doc = RinchDocument::new();
    let count = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(CountingLoader(count.clone())));
    let body = doc.body();
    for _ in 0..2 {
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "width: 10px; height: 10px; background-image: url(bg-request-shared.png)",
        );
        doc.append_child(body, div);
    }
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.image_cache.contains("bg-request-shared.png"));
    assert_eq!(loads_settle_at(&count, 1), 1);
}
