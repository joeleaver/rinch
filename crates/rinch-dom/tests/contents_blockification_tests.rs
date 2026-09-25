//! A child of a `display: contents` element inside a flex or grid container is
//! a flex/grid item, and is **blockified** (#998).
//!
//! css-display-3 §2.7: a flex or grid container's in-flow children are
//! blockified. A `display: contents` element generates no box, so *its*
//! children are the container's items (§2.5), and are blockified too. Stylo's
//! style adjuster does that from the **layout parent** style, which skips
//! `display: contents` ancestors; rinch's hand-rolled cascade used to pass the
//! DOM parent's style in both slots, so the adjuster saw a `display: contents`
//! parent and left the child `inline`.
//!
//! Chrome 153, measured with the markup of each fixture below:
//! `getComputedStyle(span).display` is `"block"` for `flex > contents > span`,
//! `grid > contents > span`, `flex > contents > contents > span`, and for a
//! `::before` generated on a `display: contents` child of a flex container;
//! it is `"inline"` for `block > contents > span`.
//!
//! `rsx!` puts a `display: contents` wrapper around every reactive site, `for`,
//! `if` and component site, so this is the ordinary shape of a `for` row inside
//! a `Group` or `Stack`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DisplayValue;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn display(doc: &RinchDocument, id: NodeId) -> DisplayValue {
    doc.tree.get(id.0).unwrap().computed_style.display
}

/// `container > contents > span`, laid out once.
fn wrapped_span(container_style: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = el(&mut doc, body, "div", container_style);
    let wrapper = el(&mut doc, container, "div", "display: contents");
    let span = el(&mut doc, wrapper, "span", "");
    let t = doc.create_text("item");
    doc.append_child(span, t);
    doc.resolve_layout(VW, VH);
    (doc, container, wrapper, span)
}

#[test]
fn a_flex_items_contents_child_is_blockified() {
    let (doc, _, wrapper, span) = wrapped_span("display: flex");
    assert_eq!(display(&doc, wrapper), DisplayValue::Contents);
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

#[test]
fn a_grid_items_contents_child_is_blockified() {
    let (doc, _, _, span) = wrapped_span("display: grid");
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

/// `inline-flex` is a flex container too: its items are blockified whatever
/// the container's outer display.
#[test]
fn an_inline_flex_items_contents_child_is_blockified() {
    let (doc, _, _, span) = wrapped_span("display: inline-flex");
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

/// The control: a block container's children are not blockified, through a
/// wrapper or not. Without it "always blockify behind a wrapper" would pass.
#[test]
fn a_block_containers_contents_child_stays_inline() {
    let (doc, _, _, span) = wrapped_span("display: block");
    assert_eq!(display(&doc, span), DisplayValue::Inline);
}

/// The walk skips every `display: contents` ancestor, not only the first.
#[test]
fn two_contents_wrappers_deep_is_still_blockified() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let flex = el(&mut doc, body, "div", "display: flex");
    let outer = el(&mut doc, flex, "div", "display: contents");
    let inner = el(&mut doc, outer, "div", "display: contents");
    let span = el(&mut doc, inner, "span", "");
    let t = doc.create_text("item");
    doc.append_child(span, t);
    doc.resolve_layout(VW, VH);
    assert_eq!(display(&doc, inner), DisplayValue::Contents);
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

/// The container stops being a flex container: the item behind the wrapper
/// goes back to `inline`. The wrapper's own `display` does not change, so what
/// re-cascades the item is the one inherited-flag bit Stylo sets on a contents
/// element in an item container (`DIPLAY_CONTENTS_IN_ITEM_CONTAINER`), which
/// `child_cascade` compares.
///
/// The flip is a **class** change against a stylesheet rule. An inline
/// `set_style` would not test that: a `style` attribute write restyles the
/// element's whole subtree by itself, wrapper or no wrapper.
#[test]
fn the_container_leaving_flex_unblockifies_the_wrapped_child() {
    let mut doc = RinchDocument::new();
    doc.load_css(".f { display: flex; }");
    let body = doc.body();
    let container = el(&mut doc, body, "div", "");
    doc.set_attribute(container, "class", "f");
    let wrapper = el(&mut doc, container, "div", "display: contents");
    let span = el(&mut doc, wrapper, "span", "");
    let t = doc.create_text("item");
    doc.append_child(span, t);
    doc.resolve_layout(VW, VH);
    assert_eq!(display(&doc, span), DisplayValue::Block);
    doc.set_attribute(container, "class", "");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(display(&doc, container), DisplayValue::Block);
    assert_eq!(display(&doc, span), DisplayValue::Inline);
    doc.set_attribute(container, "class", "f");
    doc.resolve_layout(VW, VH);
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

/// A wrapper that *becomes* `display: contents` inside a flex container makes
/// its children flex items. Its own display changed, which already asks for its
/// children to be re-cascaded; the pin is that they are now blockified.
#[test]
fn a_wrapper_becoming_contents_blockifies_its_child() {
    let (mut doc, _, wrapper, span) = wrapped_span("display: flex");
    doc.set_style(wrapper, "display", "block");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(display(&doc, wrapper), DisplayValue::Block);
    assert_eq!(display(&doc, span), DisplayValue::Inline);
    doc.set_style(wrapper, "display", "contents");
    doc.resolve_layout(VW, VH);
    assert_eq!(display(&doc, span), DisplayValue::Block);
}

/// A child inserted behind an already-styled wrapper goes through the
/// targeted-insert cascade (`resolve_inserted_subtree`), which starts from the
/// new node with its parent's style handed in — so the layout parent has to be
/// found from the tree there too, not only on the recursive walk.
#[test]
fn a_child_inserted_behind_a_styled_wrapper_is_blockified() {
    let (mut doc, _, wrapper, _) = wrapped_span("display: flex");
    let late = el(&mut doc, wrapper, "span", "");
    let t = doc.create_text("late");
    doc.append_child(late, t);
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(display(&doc, late), DisplayValue::Block);
}

/// A `::before` generated on a `display: contents` child of a flex container
/// is a child of that flex container's box, so it is a flex item and is
/// blockified. Its layout parent is the wrapper's layout parent, not the
/// wrapper.
///
/// **What this pins today is the element-cascade route, not
/// `resolve_pseudo_element`'s.** The generated `<span>` is walked and
/// re-cascaded as a plain child of the wrapper right after it is created, which
/// replaces the style the pseudo cascade gave it (a pre-existing defect,
/// #1004 — it also drops a `::before`'s own `color`). So the pseudo
/// cascade's layout parent is not observable here until that is fixed.
#[test]
fn a_contents_elements_before_in_a_flex_container_is_blockified() {
    let mut doc = RinchDocument::new();
    doc.load_css(".w::before { content: \"x\"; }");
    let body = doc.body();
    let flex = el(&mut doc, body, "div", "display: flex");
    let wrapper = el(&mut doc, flex, "div", "display: contents");
    doc.set_attribute(wrapper, "class", "w");
    doc.resolve_layout(VW, VH);
    let before = doc
        .tree
        .get(wrapper.0)
        .unwrap()
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .expect("the wrapper generates a ::before");
    assert_eq!(
        doc.tree.get(before).unwrap().computed_style.display,
        DisplayValue::Block
    );
}

fn abs_box(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.x, n.layout.y, n.layout.width, n.layout.height)
}

/// What blockification does to a box, against Chrome 153 (same markup, same
/// declared line box). In a `Stack`-like column (`flex-direction: column`, the
/// default `align-items: stretch`) a wrapped `span` and a wrapped
/// `inline-flex` both stretch to the column's 300px, one line box apart:
/// Chrome gives `[0, 0, 300, 20]` and `[0, 20, 300, 20]`. In a `Group`-like
/// row an `inline-block` with a declared width keeps it, one `gap` after its
/// sibling: Chrome gives `g2.x == g1.width + 10`, 40 wide, both 20 tall. The
/// sibling's own width is text-measured and so is not pinned.
///
/// **A guard, not a fail-first fixture**: these boxes came out the same before
/// #998 (measured at `eb820ab3`), because rinch already laid an unblockified
/// inline item of a flex container out as a flex item. What the fix moves is
/// the text inside such an item — an IFC root now, where it was a Taffy text
/// leaf — and the restyle of one to `display: inline`, which no longer orphans
/// it (`scoped_ifc_oracle_tests`' former known divergence). This pins that
/// the change of path did not move the boxes of the everyday `Stack`/`Group`
/// shapes.
#[test]
fn wrapped_items_lay_out_as_chromes_blockified_items() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let stack = el(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column; width: 300px; font-size: 16px; line-height: 20px",
    );
    let sw = el(&mut doc, stack, "div", "display: contents");
    let s1 = el(&mut doc, sw, "span", "");
    let t = doc.create_text("ab");
    doc.append_child(s1, t);
    let s2 = el(&mut doc, sw, "span", "display: inline-flex");
    let t = doc.create_text("cd");
    doc.append_child(s2, t);
    let group = el(
        &mut doc,
        body,
        "div",
        "display: flex; gap: 10px; width: 300px; font-size: 16px; line-height: 20px",
    );
    let gw = el(&mut doc, group, "div", "display: contents");
    let g1 = el(&mut doc, gw, "span", "padding: 0 5px");
    let t = doc.create_text("ab");
    doc.append_child(g1, t);
    let g2 = el(&mut doc, gw, "span", "display: inline-block; width: 40px");
    let t = doc.create_text("cd");
    doc.append_child(g2, t);
    doc.resolve_layout(VW, VH);

    assert_eq!(abs_box(&doc, s1), (0.0, 0.0, 300.0, 20.0));
    assert_eq!(abs_box(&doc, s2), (0.0, 20.0, 300.0, 20.0));
    let (g1x, g1y, g1w, g1h) = abs_box(&doc, g1);
    let (g2x, g2y, g2w, g2h) = abs_box(&doc, g2);
    assert_eq!((g1x, g1y, g1h), (0.0, 0.0, 20.0));
    assert!(g1w > 10.0, "the padded span has its text's width: {g1w}");
    assert_eq!((g2x, g2y, g2w, g2h), (g1w + 10.0, 0.0, 40.0, 20.0));
}
