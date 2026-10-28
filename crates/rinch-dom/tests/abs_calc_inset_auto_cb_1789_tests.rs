//! A percentage inside `calc()` on an absolutely positioned box resolves
//! against its containing block's padding box even when that block's size
//! comes from its content (#1789, #1974).
//!
//! `top: calc(100% + 4px)` under a `position: relative` block whose height is
//! `auto` came out at `top: 4px`: `calc_layout` judged the percentage basis
//! by `calc_axis_definite` — the in-flow rule, under which an auto-height
//! block's height is indefinite and a percentage resolves as zero — while a
//! plain `top: 100%` (Taffy's own) resolves against the laid-out size, as CSS
//! says for an absolute box: its containing block's size never depends on
//! it, so the basis is always definite.
//!
//! Every expected value is Chrome 155's (`getBoundingClientRect` of the box
//! minus its containing block's, the same declarations). The fixture:
//! a containing block with `padding: 5px 7px; border: 3px` holding a
//! `50x28` child, inside a `406px` wrapper — so its padding box is 400x38
//! (auto width, content height) and every offset includes the 3px border.
//! Each percentage term is non-zero and differs from the px term, so the
//! broken reading (px only) and a content-box or border-box basis all miss.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const EPS: f32 = 0.51;

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < EPS,
        "{what}: expected {expected}, got {actual}"
    );
}

const CB: &str = "position: relative; box-sizing: content-box; padding: 5px 7px; border: 3px solid black; \
                  font-size: 10px; line-height: 10px; ";
const ABS: &str = "position: absolute; width: 20px; height: 10px; ";

/// Build `wrapper(406px + wrap) > cb(CB + cb) > [50x28 child, abs(ABS + abs)]`,
/// lay out at 800x600 and return `(cb, abs)`.
fn build(doc: &mut RinchDocument, wrap: &str, cb_style: &str, abs_style: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0;");
    let w = doc.create_element("div");
    doc.set_attribute(w, "style", &format!("width: 406px; {wrap}"));
    let cb = doc.create_element("div");
    doc.set_attribute(cb, "style", &format!("{CB}{cb_style}"));
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50px; height: 28px; flex: none;");
    let abs = doc.create_element("div");
    doc.set_attribute(abs, "style", &format!("{ABS}{abs_style}"));
    doc.append_child(cb, child);
    doc.append_child(cb, abs);
    doc.append_child(w, cb);
    doc.append_child(body, w);
    doc.resolve_layout(800.0, 600.0);
    (cb, abs)
}

/// The box's position relative to its containing block's border box, and
/// its size.
fn geom(doc: &RinchDocument, cb: NodeId, abs: NodeId) -> (f32, f32, f32, f32) {
    let c = doc.tree.get(cb.0).unwrap().layout;
    assert_near(c.width, 406.0, "containing block width");
    assert_near(c.height, 44.0, "containing block height");
    let l = doc.tree.get(abs.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

fn check(wrap: &str, cb_style: &str, abs_style: &str, x: f32, y: f32, what: &str) {
    let mut doc = RinchDocument::new();
    let (cb, abs) = build(&mut doc, wrap, cb_style, abs_style);
    let (ax, ay, _, _) = geom(&doc, cb, abs);
    assert_near(ax, x, &format!("{what}: x"));
    assert_near(ay, y, &format!("{what}: y"));
}

const TOP_LEFT: &str = "top: calc(100% + 4px); left: calc(50% - 10px);";
const BOTTOM_RIGHT: &str = "bottom: calc(100% - 2em); right: calc(25% + 3px);";

/// Control: the plain percentages already resolved (Chrome: 203, 41).
#[test]
fn plain_percent_insets_control() {
    check("", "", "top: 100%; left: 50%;", 203.0, 41.0, "plain %");
}

/// #1789's shape: an auto-height block. Chrome: (193, 45). Broken: y = 7.
#[test]
fn block_auto_height_top_left() {
    check("", "", TOP_LEFT, 193.0, 45.0, "block top/left");
}

/// `bottom: calc(100% - 2em)` (18px from the bottom) and `right: calc(25% +
/// 3px)` (103px). Chrome: (280, 13). Broken (px parts only): (377, 51).
#[test]
fn block_auto_height_bottom_right() {
    check("", "", BOTTOM_RIGHT, 280.0, 13.0, "block bottom/right");
}

/// #1974's shape, content-height row flex. Chrome: (193, 45).
#[test]
fn flex_row_auto_height() {
    check("", "display: flex;", TOP_LEFT, 193.0, 45.0, "flex row");
}

/// A declared height was already definite (control). Chrome: (193, 45).
#[test]
fn flex_row_fixed_height_control() {
    check(
        "",
        "display: flex; height: 44px; box-sizing: border-box;",
        TOP_LEFT,
        193.0,
        45.0,
        "flex row 44px border-box",
    );
}

/// Column flex, auto height. Chrome: (193, 45).
#[test]
fn flex_column_auto_height() {
    check(
        "",
        "display: flex; flex-direction: column;",
        TOP_LEFT,
        193.0,
        45.0,
        "flex column",
    );
}

/// A grid containing block, auto height. Chrome: (280, 13).
#[test]
fn grid_auto_height_bottom_right() {
    check("", "display: grid;", BOTTOM_RIGHT, 280.0, 13.0, "grid");
}

/// A containing block that is a grid *item*: neither of its axes is one
/// `calc_axis_definite` can prove, so `left` lost its percentage too.
/// Chrome: (193, 45).
#[test]
fn containing_block_is_a_grid_item() {
    check(
        "display: grid; grid-template-columns: 406px;",
        "",
        TOP_LEFT,
        193.0,
        45.0,
        "grid item",
    );
}

/// `max()` inside the calc (Stylo hands it over as a calc node; rinch
/// linearizes it to its large-basis arm, which is the `100%` one here).
/// Chrome: (3, 45).
#[test]
fn max_inside_calc() {
    check(
        "",
        "",
        "top: calc(max(100%, 10px) + 4px); left: 0;",
        3.0,
        45.0,
        "max()",
    );
}

/// The box's own `calc()` size resolves against the same padding box:
/// `height: calc(50% + 2px)` = 21, `width: calc(25% + 1px)` = 101.
/// Chrome: 101x21. Broken: height auto (0).
#[test]
fn own_size_calc_against_auto_height_block() {
    let mut doc = RinchDocument::new();
    let (cb, abs) = build(
        &mut doc,
        "",
        "",
        "top: 0; left: 0; height: calc(50% + 2px); width: calc(25% + 1px);",
    );
    let (x, y, w, h) = geom(&doc, cb, abs);
    assert_near(x, 3.0, "x");
    assert_near(y, 3.0, "y");
    assert_near(w, 101.0, "width");
    assert_near(h, 21.0, "height");
}

/// A percentage margin resolves against the containing block's width (400):
/// `margin-top: calc(10% + 2px)` = 42. Chrome: y = 45.
#[test]
fn margin_calc_in_grid_item_containing_block() {
    check(
        "display: grid; grid-template-columns: 406px;",
        "",
        "top: 0; left: 0; margin-top: calc(10% + 2px);",
        3.0,
        45.0,
        "margin",
    );
}

/// `position: fixed` resolves its insets against the viewport (800x600) in
/// the read-back: `top: calc(50% + 4px)` = 304, `left: calc(25% - 3px)` = 197.
#[test]
fn fixed_against_viewport() {
    let mut doc = RinchDocument::new();
    let (_, abs) = build(
        &mut doc,
        "",
        "",
        "position: fixed; top: calc(50% + 4px); left: calc(25% - 3px);",
    );
    let l = doc.tree.get(abs.0).unwrap().layout;
    assert_near(l.x, 197.0, "fixed x");
    assert_near(l.y, 304.0, "fixed y");
}

/// #1789's exact markup: a content-height relative block around a 28px
/// button. Chrome puts the menu 4px below the block's bottom edge (y = 32).
/// The block's height comes from a line holding the button, so the test
/// states the relation rather than the font-dependent height.
#[test]
fn issue_1789_repro() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = doc.create_element("div");
    doc.set_attribute(p, "style", "position: relative");
    let b = doc.create_element("button");
    doc.set_attribute(b, "style", "height: 28px");
    let t = doc.create_text("Add Component");
    doc.append_child(b, t);
    let m = doc.create_element("div");
    doc.set_attribute(m, "style", "position: absolute; left: 0; top: calc(100% + 4px)");
    let mt = doc.create_text("menu");
    doc.append_child(m, mt);
    doc.append_child(p, b);
    doc.append_child(p, m);
    doc.append_child(body, p);
    doc.resolve_layout(800.0, 600.0);
    let ph = doc.tree.get(p.0).unwrap().layout.height;
    assert!(ph >= 28.0, "parent height {ph}");
    assert_near(doc.tree.get(m.0).unwrap().layout.y, ph + 4.0, "menu y");
}

/// #1974's markup: a 28px relative flex row whose height comes from its
/// 28px button (a declared `height: 28px` was already resolved at HEAD — the
/// second half is that control). Chrome: y = 32 for both.
#[test]
fn issue_1974_repro() {
    for (row, button) in [
        ("position: relative; display: flex", "height: 28px"),
        ("position: relative; display: flex; height: 28px", ""),
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let p = doc.create_element("div");
        doc.set_attribute(p, "style", row);
        let b = doc.create_element("button");
        doc.set_attribute(b, "style", button);
        let t = doc.create_text("Add");
        doc.append_child(b, t);
        let m = doc.create_element("div");
        doc.set_attribute(m, "style", "position: absolute; left: 0; top: calc(100% + 4px);");
        doc.append_child(p, b);
        doc.append_child(p, m);
        doc.append_child(body, p);
        doc.resolve_layout(800.0, 600.0);
        assert_near(doc.tree.get(p.0).unwrap().layout.height, 28.0, row);
        assert_near(doc.tree.get(m.0).unwrap().layout.y, 32.0, row);
    }
}
