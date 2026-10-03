//! #725 — `<li>` computes `display: list-item` and its marker glyph follows
//! `list-style-type` (`disc`/`circle`/`square`/`decimal`), rather than only
//! the parent tag (`ul` → bullet, `ol` → number) the way `resolve_list_marker`
//! picked before this fix.
//!
//! Re-verified at HEAD (2026-10-03): `resolve_list_marker` already existed
//! and already generated *some* marker for every `<li>`, so the issue's own
//! "desktop shows no list marker anywhere" is stale — the corrective comment
//! on the issue predates that machinery landing. What's still true, and what
//! this file pins:
//! - `DisplayValue` has no `ListItem` variant (now added).
//! - the marker glyph ignores `list-style-type`'s actual keyword — a `<ul>`
//!   styled `list-style-type: decimal` still shows a bullet, and a nested
//!   `<ul>` shows the same bullet as its parent instead of Chrome's
//!   `circle`/`square` (now fixed for the four keywords in scope: `disc`,
//!   `circle`, `square`, `decimal`; everything else keeps the old
//!   tag-based fallback rather than drawing nothing).
//! - `display: list-item` is also now the thing `resolve_list_marker` gates
//!   on, so an author override off it (e.g. the `rinch-components` `List`'s
//!   `--with-icon` item, `display: flex`) suppresses the marker, instead of
//!   showing a stray bullet beside the custom icon — a pre-existing bug this
//!   fix also closes as a side effect.
//!
//! `list-style-position` (`inside` vs `outside`) is NOT implemented — both
//! look like today's "inside"-shaped span (filed as a follow-up) — so no
//! fixture here claims to pin outside-vs-inside geometry.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DisplayValue;
use rinch_dom::testing::get_text_content;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.append_child(parent, e);
    e
}

fn li_with_style(doc: &mut RinchDocument, parent: NodeId, style: &str, text: &str) -> NodeId {
    let li = el(doc, parent, "li");
    if !style.is_empty() {
        doc.set_attribute(li, "style", style);
    }
    let t = doc.create_text(text);
    doc.append_child(li, t);
    li
}

/// The marker span's text, or `None` if the `<li>` generated no marker.
fn marker_text(doc: &RinchDocument, li: NodeId) -> Option<String> {
    let node = doc.tree.get(li.0)?;
    let m = node
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)?;
    Some(get_text_content(&doc.tree, m))
}

/// `<li>`'s computed `display` is `list-item`, not plain `block` — the
/// `DisplayValue::ListItem` variant this issue asked for.
///
/// Mutant this kills: reverting `display_from_stylo` to not check
/// `display.is_list_item()` (so a UA `li { display: list-item }` falls
/// through to the ordinary `(Block, Flow)` arm) makes this read `"block"`
/// instead of `"list-item"`.
#[test]
fn li_computes_display_list_item() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = el(&mut doc, ul, "li");
    doc.resolve_layout(800.0, 600.0);

    let display = doc.tree.get(li.0).unwrap().computed_style.display;
    assert_eq!(display, DisplayValue::ListItem);
}

/// A bare `<ul><li>` gets a disc bullet; a `<ul>` nested one level gets a
/// circle; nested two levels, a square — the UA defaults this issue asked
/// for, measured against Chrome 150's table in the issue body.
///
/// Mutant: deleting the `ul ul { list-style-type: circle }` /
/// `ul ul ul { list-style-type: square }` UA rules (or reverting
/// `resolve_list_marker`'s keyword match to the old tag-only heuristic)
/// makes every nested marker the same disc bullet as the top level.
#[test]
fn nested_ul_markers_follow_the_ua_disc_circle_square_scale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul1 = el(&mut doc, body, "ul");
    let li1 = el(&mut doc, ul1, "li");
    let ul2 = el(&mut doc, li1, "ul");
    let li2 = el(&mut doc, ul2, "li");
    let ul3 = el(&mut doc, li2, "ul");
    let li3 = el(&mut doc, ul3, "li");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(marker_text(&doc, li1).unwrap(), "\u{2022}\u{2002}", "disc");
    assert_eq!(
        marker_text(&doc, li2).unwrap(),
        "\u{25E6}\u{2002}",
        "circle"
    );
    assert_eq!(
        marker_text(&doc, li3).unwrap(),
        "\u{25AA}\u{2002}",
        "square"
    );
}

/// An explicit `list-style-type` wins over the UA default and is read for
/// its own keyword, not just the parent tag — the issue's core complaint
/// ("`list-style-type`... parsed and then ignored").
///
/// Mutant: reverting to the pre-fix tag-based heuristic makes a `<ul>`
/// styled `list-style-type: decimal` still show a disc bullet instead of
/// "1.".
#[test]
fn an_explicit_list_style_type_overrides_the_parent_tags_default() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    doc.set_attribute(ul, "style", "list-style-type: decimal;");
    let li = el(&mut doc, ul, "li");
    doc.resolve_layout(800.0, 600.0);

    let text = marker_text(&doc, li).expect("marker generated");
    assert!(
        text.starts_with('1'),
        "a <ul> styled list-style-type: decimal should number, got {text:?}"
    );
}

/// `list-style-type: none` still suppresses the marker entirely (unchanged
/// from before this fix — a regression guard).
#[test]
fn list_style_none_still_draws_nothing() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    doc.set_attribute(ul, "style", "list-style-type: none;");
    let li = el(&mut doc, ul, "li");
    doc.resolve_layout(800.0, 600.0);

    assert!(marker_text(&doc, li).is_none());
}

/// An author override off `display: list-item` suppresses the marker too —
/// matching Chrome, and closing a pre-existing double-render: before this
/// fix, `resolve_list_marker` never looked at `display` at all, so the
/// `rinch-components` `List`'s icon items (`.rinch-list__item--with-icon`
/// sets `display: flex`) got a stray bullet **beside** the custom icon.
///
/// Mutant: dropping the `!primary.get_box().display.is_list_item()` gate in
/// `resolve_list_marker` makes this draw a bullet anyway.
#[test]
fn an_li_with_display_overridden_off_list_item_draws_no_marker() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = li_with_style(&mut doc, ul, "display: flex;", "icon item");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        marker_text(&doc, li).is_none(),
        "display: flex on the <li> should suppress the UA list-item marker"
    );
}

/// `<ol>` keeps numbering by its own ordinal (`compute_list_item_counters`),
/// not a literal glyph — a regression guard against the new keyword match
/// swallowing the existing `decimal`/ordinal behaviour under `<ol>`.
#[test]
fn ol_items_still_number() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    let a = el(&mut doc, ol, "li");
    let b = el(&mut doc, ol, "li");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(marker_text(&doc, a).unwrap(), "1.\u{2002}");
    assert_eq!(marker_text(&doc, b).unwrap(), "2.\u{2002}");
}
