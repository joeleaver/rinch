//! `rem` resolution against the root element's font-size (issue #279), and
//! the Stylo device parameters that must survive viewport-driven `Device`
//! rebuilds (issues #279/#211).
//!
//! The trap these tests exist to avoid: a fixture whose root font-size is
//! 16px cannot distinguish a working `set_root_font_size` from the
//! hard-coded 16px initial. Every fixture here uses a root font-size where
//! correct and broken code disagree.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

fn width(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.width
}

fn height(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

/// `html { font-size: 20px }` must make `1rem` = 20px, not the 16px initial.
///
/// Kills: never calling `Device::set_root_font_size` (the root font-size
/// stays pinned at stylo's `FONT_MEDIUM_PX` initial).
#[test]
fn rem_resolves_against_root_font_size() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem; height: 1rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(width(&doc, div), 40.0, "2rem against a 20px root");
    assert_eq!(height(&doc, div), 20.0, "1rem against a 20px root");
}

/// An element's own (or an ancestor's) `font-size` must affect `em` but
/// never `rem`.
///
/// Kills: feeding the cascading element's own font-size to the device, or
/// resolving `rem` like `em`.
#[test]
fn rem_ignores_element_and_ancestor_font_size() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html { font-size: 20px }
         .parent { font-size: 40px }
         .x { width: 2rem; height: 2em }",
    );

    let body = doc.body();
    let parent = doc.create_element("div");
    doc.set_attribute(parent, "class", "parent");
    doc.append_child(body, parent);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(parent, div);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        width(&doc, div),
        40.0,
        "rem resolves against the root (20px)"
    );
    assert_eq!(
        height(&doc, div),
        80.0,
        "em resolves against the element's inherited font-size (40px)"
    );
}

/// A relative root font-size (`html { font-size: 125% }`) resolves against
/// the 16px initial, and that *computed* value (20px) is the `rem` basis.
///
/// Kills: feeding the specified (unresolved) value to the device instead of
/// the computed one.
#[test]
fn percentage_root_font_size_computes_the_rem_basis() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 125% } .x { width: 2rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(width(&doc, div), 40.0, "2rem against 125% of 16px = 20px");
}

/// `rem` on the root's own `font-size` refers to the property's *initial*
/// value (16px), and descendants then resolve `rem` against the root's
/// computed result.
#[test]
fn rem_on_root_font_size_resolves_against_initial() {
    let mut doc = RinchDocument::new();
    // Root: 1.5rem = 1.5 x 16px initial = 24px computed.
    doc.load_css("html { font-size: 1.5rem } .x { width: 1em; height: 1rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(width(&doc, div), 24.0, "1em inherits the root's 24px");
    assert_eq!(
        height(&doc, div),
        24.0,
        "1rem against the root's computed 24px"
    );
}

/// The whole point of storing device parameters on the document: a window
/// resize rebuilds the Stylo `Device` from scratch, and a naive
/// `set_root_font_size` fix is silently erased by that rebuild.
///
/// Kills: rebuilding the `Device` from defaults instead of from
/// `DeviceParams` (i.e. dropping the `set_root_font_size` re-apply in
/// `build_device`).
#[test]
fn rem_basis_survives_viewport_resize() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 40.0, "before the resize");

    // The resize rebuilds the Device (set_stylist_viewport) and forces a
    // full recascade. The rem basis must come through it intact.
    doc.resolve_layout(1024.0, 768.0);
    assert_eq!(width(&doc, div), 40.0, "after the resize");
}

/// Changing the root's font-size after the fact must recascade descendants
/// whose cached styles hold `rem` lengths resolved against the old basis.
///
/// `set_style` on `<html>` invalidates only the root itself (targeted
/// resolution), so without the descendant cache clear in
/// `sync_root_font_size` the `.x` box would stay at its stale 32px.
#[test]
fn root_font_size_change_recascades_cached_descendants() {
    let mut doc = RinchDocument::new();
    doc.load_css(".x { width: 2rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 32.0, "2rem against the 16px default");

    // Inline style on <html>: invalidates the root only — descendants keep
    // their cached styles unless the basis change clears them.
    let html = NodeId(doc.tree.html_id);
    doc.set_style(html, "font-size", "20px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        width(&doc, div),
        40.0,
        "2rem re-resolved against the new 20px root"
    );
}

/// The dpr half of the shared refactor (issue #211): `set_device_pixel_ratio`
/// must reach the `resolution` media features and survive the same
/// viewport-driven `Device` rebuild.
///
/// Kills: hard-coding `Scale::new(1.0)` at either Device construction site,
/// and rebuilding the Device without the stored dpr.
#[test]
fn device_pixel_ratio_reaches_resolution_media_queries_and_survives_resize() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".x { width: 100px; height: 10px }
         @media (min-resolution: 2dppx) { .x { width: 200px } }",
    );

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width(&doc, div),
        100.0,
        "dpr defaults to 1: media query must not match"
    );

    doc.set_device_pixel_ratio(2.0);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 200.0, "at dpr 2 the media query matches");

    // A resize rebuilds the Device; the dpr must come through it intact.
    doc.resolve_layout(1024.0, 768.0);
    assert_eq!(
        width(&doc, div),
        200.0,
        "dpr survives the viewport-driven rebuild"
    );
}
