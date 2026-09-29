//! A block with no in-flow content is 0 tall (#296).
//!
//! CSS 2.1 §10.6.3 computes a block container's auto height from its line
//! boxes and in-flow children, and an empty one has neither: `<div></div>` is
//! 0px in every browser. rinch used to floor every childless block at one line
//! box (`ifc::apply_empty_block_line_floor`), a deliberate divergence that
//! existed only because `<input>`/`<textarea>` — childless however much text
//! they show — had no content height of their own. They are measured now
//! (#297, `form_control_intrinsic_height_tests.rs`), and the floor is gone, and
//! with it the four places it over-applied: a padded block (a border-box floor
//! the padding ate), a `display: grid` container, a block `<img>`, and an
//! author `min-height: 0`.
//!
//! Every number is measured in Chrome 153 on the same markup (`* {
//! box-sizing: border-box; margin: 0 }`, `line-height: 20px` on the container),
//! and each fixture sits off the floor's own value: every one was 20 before.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A 10x1 PNG. Its natural height (1) is under a line, which is the shape the
/// floor inflated.
const TEN_BY_ONE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAABCAYAAADn9T9+AAAAD0lEQVR4nGP4z8DwnxgMAJieE+3w0W2+AAAAAElFTkSuQmCC";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// A 300px container with a 20px line, holding `style`d children.
fn container(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    el(
        doc,
        body,
        "div",
        &format!("width: 300px; font-size: 10px; line-height: 20px; {style}"),
    )
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// Chrome 153: an empty `<div>` is 0, and an empty `<div style="padding:
/// 5px">` is its padding alone, 10.
#[test]
fn an_empty_block_is_as_tall_as_its_padding_and_nothing_more() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let empty = el(&mut doc, c, "div", "");
    let padded = el(&mut doc, c, "div", "padding: 5px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, empty), 0.0, "Chrome 153: 0");
    assert_eq!(height(&doc, padded), 10.0, "Chrome 153: 10");
}

/// #296 item 2. Chrome 153: an empty `display: grid` is 0 — a grid container is
/// not a block container and generates no line box, floor or no floor.
#[test]
fn an_empty_grid_container_is_zero_tall() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let grid = el(&mut doc, c, "div", "display: grid");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, grid), 0.0, "Chrome 153: 0");
}

/// #296 item 3. `img { display: block }` is the commonest image reset. A block
/// `<img>` whose natural height is under a line keeps it: Chrome 153 gives the
/// 10x1 image 1px, in a block container and as a flex item. Only the height is
/// asserted in block flow — a block `<img>`'s *width* fills its container in
/// rinch, which is #788.
#[test]
fn a_block_image_keeps_a_natural_height_under_one_line() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let block = el(&mut doc, c, "img", "display: block");
    doc.set_attribute(block, "src", TEN_BY_ONE);
    let row = container(&mut doc, "display: flex; align-items: flex-start");
    let item = el(&mut doc, row, "img", "");
    doc.set_attribute(item, "src", TEN_BY_ONE);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, block), 1.0, "Chrome 153: 1");
    assert_eq!(height(&doc, item), 1.0, "Chrome 153: 1");
    assert_eq!(doc.tree.get(item.0).unwrap().layout.width, 10.0);
}

/// #296 item 4. A collapsible flex item: `flex: 1; min-height: 0` in a 100px
/// column whose other item already takes 100px. Chrome 153: 0. The floor turned
/// the explicit `0` into `max(line, 0)` = 20, with no way to opt out short of a
/// `height`.
#[test]
fn a_min_height_of_zero_lets_an_empty_flex_item_collapse() {
    let mut doc = RinchDocument::new();
    let col = container(
        &mut doc,
        "display: flex; flex-direction: column; height: 100px",
    );
    let collapsible = el(&mut doc, col, "div", "flex: 1; min-height: 0");
    el(&mut doc, col, "div", "height: 100px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, collapsible), 0.0, "Chrome 153: 0");
}

/// What an empty block that *should* be one line tall says instead: the rich
/// text editor's empty paragraph declares `min-height: 1lh`
/// (`rinch-editor-view/src/styles.rs`), and that is honoured — Chrome 153: 20.
#[test]
fn min_height_one_lh_gives_an_empty_block_its_line() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let p = el(&mut doc, c, "p", "margin: 0; min-height: 1lh");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, p), 20.0, "Chrome 153: 20");
}

/// An empty `<h1>` is 0 tall too, whatever its UA margins (Chrome 153: 0).
/// The floor made it 20, which is what `ua_heading_typography_tests`' margin
/// fixture unknowingly leaned on (it now puts text in its headings).
#[test]
fn an_empty_heading_is_zero_tall() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "font-size: 16px");
    let h = el(&mut doc, c, "h1", "line-height: 20px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, h), 0.0, "Chrome 153: 0");
}
