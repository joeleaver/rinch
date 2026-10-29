//! #884 — a stylesheet added, replaced or removed **after the first layout**
//! restyles exactly what its rules can reach.
//!
//! `load_css` after the first `resolve_layout` used to add its sheet to the
//! Stylist and invalidate nothing: every already-styled node kept its old
//! style, and the next layout was the pre-rule one. A `<style>` element's sheet
//! was loaded the moment text was appended to it (connected or not), was never
//! unloaded when the element left the document, and every edit of its text
//! *appended another* sheet, so the old rules kept applying.
//!
//! What a browser does, and what these pin: a sheet's rules are run through
//! Stylo's stylesheet invalidation (`StylesheetInvalidationSet`, the set
//! Gecko and Servo use) — each selector's rightmost id/class/tag, or its
//! ancestor-side one, names the elements to restyle; a selector it cannot
//! bucket (`*`, `:root`, a bare pseudo-class), a referenced `@keyframes` and
//! the like restyle the whole document. A rule that matches nothing restyles
//! nothing and does not relayout. A `<style>` element's sheet applies while
//! the element is connected, in tree order, after the `load_css` sheets (which
//! are `<head>` sheets on the web) — removing the element reverts its rules.
//!
//! Geometry here is box widths and heights set by the rules themselves; the
//! one text-derived value (a span's font size) is read from the computed
//! style, not from shaped text.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DimensionValue;
use rinch_dom::perf::Counter::*;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

const BASE_CSS: &str = "body { margin: 0; } .box { height: 10px; line-height: 20px; }";

fn width_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

fn font_size_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_size
}

/// `<div class="box"><span>text</span></div>` in `body`, laid out once.
fn laid_out_box() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    let body = doc.body();
    let bx = doc.create_element("div");
    doc.set_attribute(bx, "class", "box");
    doc.append_child(body, bx);
    let span = doc.create_element("span");
    doc.append_child(bx, span);
    let t = doc.create_text("text");
    doc.append_child(span, t);
    doc.resolve_layout(VW, VH);
    (doc, bx, span)
}

fn style_element(doc: &mut RinchDocument, css: &str) -> NodeId {
    let style = doc.create_element("style");
    let text = doc.create_text(css);
    doc.append_child(style, text);
    style
}

/// The issue's own probe (`p09_load_css_after_layout`), with the box's width
/// as the layout half: a rule loaded after the first layout reaches the
/// already-styled nodes and the next layout.
#[test]
fn load_css_after_first_layout_restyles_and_relays_out() {
    let (mut doc, bx, span) = laid_out_box();
    assert_eq!(font_size_of(&doc, span), 16.0, "precondition");
    assert_eq!(width_of(&doc, bx), VW, "precondition: a block fills body");

    doc.load_css(".box span { font-size: 27px; } .box { width: 137px; }");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        font_size_of(&doc, span),
        27.0,
        "#884: a descendant rule loaded after layout must restyle the span"
    );
    assert_eq!(
        width_of(&doc, bx),
        137.0,
        "#884: a rule loaded after layout must reach the next layout"
    );
}

/// A twin with the rule loaded up front agrees with the late load: the late
/// sheet cascades after the earlier ones, exactly as if loaded first in order.
#[test]
fn late_load_matches_a_twin_that_loaded_the_rule_up_front() {
    let rule = ".box { width: 211px; height: 33px; }";
    let (mut late, late_box, _) = laid_out_box();
    late.load_css(rule);
    late.resolve_layout(VW, VH);

    let mut twin = RinchDocument::new();
    twin.load_css(BASE_CSS);
    twin.load_css(rule);
    let body = twin.body();
    let bx = twin.create_element("div");
    twin.set_attribute(bx, "class", "box");
    twin.append_child(body, bx);
    twin.resolve_layout(VW, VH);

    assert_eq!(width_of(&twin, bx), 211.0, "positive control");
    assert_eq!(height_of(&twin, bx), 33.0, "positive control");
    assert_eq!(width_of(&late, late_box), 211.0);
    assert_eq!(height_of(&late, late_box), 33.0);
}

/// A rule that matches no element restyles nothing and lays nothing out;
/// a rule that matches one element restyles that element, not the document.
#[test]
fn a_late_rule_restyles_only_what_it_matches() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    // Fifty unrelated siblings: a whole-document restyle would cascade them.
    for _ in 0..50 {
        let d = doc.create_element("div");
        doc.set_attribute(d, "class", "other");
        doc.append_child(body, d);
    }
    doc.resolve_layout(VW, VH);

    doc.tree.perf.reset();
    doc.load_css(".nothing-has-this-class { width: 3px; }");
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(ElementsCascaded), 0, "a rule matching nothing");
    assert_eq!(s.get(FullRestyles), 0, "a rule matching nothing");
    assert_eq!(s.get(TaffyRootComputes), 0, "no relayout");

    doc.tree.perf.reset();
    doc.load_css(".box { width: 137px; }");
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(
        width_of(&doc, bx),
        137.0,
        "positive control: the rule applied"
    );
    assert_eq!(s.get(FullRestyles), 0, "a class rule is targeted");
    // `.box` and its span (its descendants are re-cascaded only when an
    // inherited value moved; the width is not inherited).
    assert!(
        s.get(ElementsCascaded) <= 2,
        "a class rule restyles the elements with that class, not the 52 others: {}",
        s.get(ElementsCascaded)
    );
    assert_eq!(s.get(TaffyRootComputes), 1, "and it does relayout");
}

/// A selector Stylo cannot bucket (`*`) restyles the whole document.
#[test]
fn a_late_universal_rule_restyles_every_element() {
    let (mut doc, bx, span) = laid_out_box();
    doc.load_css("* { min-height: 7px; }");
    doc.resolve_layout(VW, VH);
    let min_h = |doc: &RinchDocument, id: NodeId| match doc
        .tree
        .get(id.0)
        .unwrap()
        .computed_style
        .min_height
    {
        DimensionValue::Length(v) => v,
        other => panic!("min-height not a length: {other:?}"),
    };
    assert_eq!(min_h(&doc, bx), 7.0);
    assert_eq!(min_h(&doc, span), 7.0);
}

/// Later sheets win over earlier ones, whenever each was loaded; the theme
/// sheet stays ahead of every app sheet when it is replaced after layout.
#[test]
fn late_sheets_keep_cascade_order_and_the_theme_slot() {
    let (mut doc, bx, _) = laid_out_box();
    doc.load_css(".box { width: 100px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 100.0);

    doc.load_css(".box { width: 200px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "the later sheet wins");

    // The theme slot is ahead of every app sheet: its rule loses.
    doc.set_theme_css(".box { width: 300px; height: 44px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        200.0,
        "app CSS still overrides the theme"
    );
    assert_eq!(
        height_of(&doc, bx),
        10.0,
        "BASE_CSS's height is an app sheet too"
    );

    // A theme custom property read by an app rule: replacing the theme sheet
    // after layout restyles its readers, with no `recompute_all_styles_full`.
    doc.load_css(".box { min-width: var(--w884, 0px); }");
    doc.set_theme_css(":root { --w884: 250px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 250.0, "theme variable applied");
    doc.set_theme_css(":root { --w884: 220px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 220.0, "replaced theme sheet applied");
    doc.set_theme_css("");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        200.0,
        "the old theme sheet's rules are gone"
    );
}

/// With no `load_css` sheet at all, the theme slot is still ahead of the
/// `<style>` sheets: set after layout, the theme's rule loses to the
/// `<style>`'s (review of PR #2041, probe p23).
#[test]
fn theme_set_with_only_style_element_sheets_stays_before_them() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let bx = doc.create_element("div");
    doc.append_child(body, bx);
    let style = style_element(&mut doc, "div { width: 45px; height: 10px; }");
    doc.append_child(body, style);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 45.0, "the <style> applies");

    doc.set_theme_css("div { width: 46px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 45.0, "<style> cascades over the theme");
}

/// A `<style>` element inserted after layout applies; removed, its rules are
/// gone; re-inserted, they are back.
#[test]
fn style_element_inserted_removed_and_reinserted_after_layout() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    let style = style_element(&mut doc, ".box { width: 123px; }");
    doc.append_child(body, style);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 123.0, "inserted");

    doc.remove_child(body, style);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        VW,
        "#884: a removed <style> must revert"
    );

    doc.append_child(body, style);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 123.0, "re-inserted");

    doc.remove_node(style);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), VW, "remove_node reverts too");
}

/// Editing a `<style>`'s text replaces its sheet: the old rules are gone.
#[test]
fn style_element_text_edit_replaces_its_rules() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    let style = style_element(&mut doc, ".box { width: 123px; height: 21px; }");
    doc.append_child(body, style);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 123.0);
    assert_eq!(height_of(&doc, bx), 21.0);

    doc.set_text_content(style, ".box { width: 145px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 145.0, "the new text applies");
    assert_eq!(
        height_of(&doc, bx),
        10.0,
        "#884: the old text's `height: 21px` must be gone"
    );
}

/// `<style>` sheets cascade in tree order, after the `load_css` sheets, and
/// an edit keeps a sheet where its element is.
#[test]
fn style_elements_cascade_in_tree_order() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    let s1 = style_element(&mut doc, ".box { width: 100px; }");
    doc.append_child(body, s1);
    let s2 = style_element(&mut doc, ".box { width: 200px; }");
    doc.append_child(body, s2);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "the later element wins");

    doc.set_text_content(s1, ".box { width: 120px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        200.0,
        "#884: an edited earlier <style> stays before the later one"
    );

    // Inserted before both in tree order: it loses to both, though it came last.
    let s0 = style_element(&mut doc, ".box { width: 50px; }");
    doc.insert_before(body, s0, s1);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "tree order, not insertion order");

    doc.remove_child(body, s2);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 120.0, "s1 is now the last");

    // A `load_css` sheet is a `<head>` sheet: the body `<style>` still wins.
    doc.load_css(".box { width: 999px; }");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        120.0,
        "a <style> element follows load_css"
    );
}

/// A `<style>` in a subtree that is not in the document applies nothing until
/// the subtree is attached.
#[test]
fn style_element_in_a_detached_subtree_applies_once_attached() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    let wrap = doc.create_element("div");
    let style = style_element(&mut doc, ".box { width: 77px; }");
    doc.append_child(wrap, style);
    doc.resolve_layout(VW, VH + 1.0);
    assert_eq!(width_of(&doc, bx), VW, "detached: not applied");

    doc.append_child(body, wrap);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 77.0, "attached: applied");

    doc.remove_child(body, wrap);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), VW, "its ancestor removed: reverted");
}

/// `set_inner_html` with a `<style>`: applied, and reverted when the markup is
/// replaced.
#[test]
fn set_inner_html_style_applies_and_reverts() {
    let (mut doc, bx, _) = laid_out_box();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.resolve_layout(VW, VH);

    doc.set_inner_html(host, "<style>.box { width: 66px; }</style><p>x</p>");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 66.0, "markup <style> applied");

    doc.set_inner_html(host, "<p>y</p>");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        VW,
        "#884: replaced markup's <style> reverted"
    );
}
