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

/// Same for an `em` root font-size: `html { font-size: 1.25em }` resolves
/// against the 16px initial, computing a 20px `rem` basis.
/// (Chrome oracle: root computed font-size 20px, 2rem box 40px wide.)
#[test]
fn em_root_font_size_computes_the_rem_basis() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 1.25em } .x { width: 2rem }");

    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(width(&doc, div), 40.0, "2rem against 1.25em of 16px = 20px");
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

// ---------------------------------------------------------------------------
// Adopted from the adversarial review campaign for PR #507. These are shaped
// as probes for specific mutant classes that survived the original suite,
// rather than as feature tests — each was the sole kill for a live mutation
// of this PR's own code.
// ---------------------------------------------------------------------------

/// The scenario that justified dropping stylo's `used_root_font_size()`
/// gate: a bare `set_viewport()` — which `RinchApp` calls directly
/// (`app/mod.rs`), with no cache clear — rebuilds the Device, and a fresh
/// Device resets that flag to `false`. A root font-size change right after
/// must still recascade descendants; an implementation that trusts the flag
/// skips the recascade and leaves stale `rem` layout.
///
/// Kills: reinstating stylo's flag-gate around the descendant recascade.
#[test]
fn root_font_size_change_after_bare_set_viewport_recascades() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 40.0);

    doc.set_viewport(800.0, 600.0); // bare Device rebuild, no cache clear
    let html = NodeId(doc.tree.html_id);
    doc.set_style(html, "font-size", "24px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 48.0, "2rem against the 24px root");
}

/// The one witness that `device_params.root_font_size` actually tracks the
/// Device — the invariant this PR's architecture introduces. If
/// `sync_root_font_size` writes the Device but not the stored param, the
/// Device *looks* right until the next rebuild restores the stale 16px
/// default; every full-recascade fixture then self-heals (the root
/// recascades first and re-syncs before descendants resolve), so the bug
/// is only visible on a descendant-ONLY recascade after a bare
/// `set_viewport()` to a new size — no full invalidation, root cache-hit,
/// sync never runs, the descendant resolves `rem` against the restored
/// default.
///
/// Kills: dropping `self.device_params.root_font_size = size;` in
/// `sync_root_font_size`.
#[test]
fn descendant_recascade_after_bare_set_viewport_keeps_the_basis() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 40.0);

    // Bare rebuild to a NEW size, so resolve_layout's own viewport-change
    // branch (full invalidation) never fires — tree.viewport is already
    // synced when it runs.
    doc.set_viewport(1024.0, 768.0);
    doc.set_style(div, "height", "5px"); // dirty ONLY the descendant
    doc.resolve_layout(1024.0, 768.0);
    assert_eq!(
        width(&doc, div),
        40.0,
        "descendant recascade against the rebuilt Device keeps the 20px basis"
    );
}

/// Moving the root BACK to a smaller size must also recascade — the change
/// detection is symmetric, not grow-only. (A `<=` comparison in the basis
/// gate passes every other fixture in this file.)
#[test]
fn root_font_size_shrink_recascades() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 40.0);

    let html = NodeId(doc.tree.html_id);
    doc.set_style(html, "font-size", "10px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 20.0, "2rem against the 10px root");
}

/// The "not too much" direction: a root recascade that does NOT change the
/// root font-size (here, a `color` change on `<html>`) must not clear
/// descendant caches — without the equality gate, any root restyle defeats
/// targeted invalidation tree-wide. Cache retention is observed directly
/// via ServoArc pointer identity of the descendant's cached primary style.
///
/// Kills: removing the `size == device_params.root_font_size` early return
/// from `sync_root_font_size`.
#[test]
fn unchanged_basis_keeps_descendant_caches() {
    let mut doc = RinchDocument::new();
    doc.load_css("html { font-size: 20px } .x { width: 2rem }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "x");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, div), 40.0);

    let arc_before = doc.tree.nodes[div.0]
        .stylo_element_data
        .borrow()
        .as_ref()
        .and_then(|d| d.styles.primary.clone())
        .expect("descendant has a cached style");

    // Recascade the root without touching its font-size.
    let html = NodeId(doc.tree.html_id);
    doc.set_style(html, "color", "red");
    doc.resolve_layout(800.0, 600.0);

    let arc_after = doc.tree.nodes[div.0]
        .stylo_element_data
        .borrow()
        .as_ref()
        .and_then(|d| d.styles.primary.clone())
        .expect("descendant still has a cached style");

    assert!(
        servo_arc::Arc::ptr_eq(&arc_before, &arc_after),
        "descendant cache must be retained when the rem basis did not change"
    );
    assert_eq!(width(&doc, div), 40.0);
}
