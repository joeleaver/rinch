//! #725, review round of PR #1358: what `display: list-item` must NOT change,
//! and the UA sheet's nesting rules for `list-style-type`.
//!
//! - **Layout.** `DisplayValue::ListItem` lays out as `Block` does: the same
//!   Taffy display, the same `compute_content_height` stacking, and only a
//!   *block-outside, flow-inside* `list-item` maps to it — `display: inline
//!   list-item` stays inline-level, as on main and in Chrome 153.
//! - **Nesting.** The UA rules are Chrome's own html.css spelling,
//!   `:is(dir, menu, ol, ul) :is(dir, menu, ul)` → `circle` and three levels →
//!   `square`, so an `ol` counts as a level (Chrome 153, measured with
//!   `getComputedStyle`: `ol > li > ul` circle, `ul ol ul` square).
//! - Two Chrome cases rinch does not reach yet (#1370) are pinned `#[ignore]`d.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DisplayValue;
use rinch_dom::testing::get_text_content;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.append_child(parent, e);
    e
}
fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}
fn marker_text(doc: &RinchDocument, li: NodeId) -> Option<String> {
    let node = doc.tree.get(li.0)?;
    let m = node
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)?;
    Some(get_text_content(&doc.tree, m))
}
const DISC: &str = "\u{2022}\u{2002}";
const CIRCLE: &str = "\u{25E6}\u{2002}";
const SQUARE: &str = "\u{25AA}\u{2002}";

/// Chrome 153: `<ol><li><ul><li>` is circle (html.css:
/// `:is(dir, menu, ol, ul) :is(dir, menu, ul) { list-style-type: circle }`).
#[test]
fn p1_ul_nested_in_ol_is_circle_like_chrome() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    let li = el(&mut doc, ol, "li");
    txt(&mut doc, li, "x");
    let ul = el(&mut doc, li, "ul");
    let li2 = el(&mut doc, ul, "li");
    txt(&mut doc, li2, "d");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li2).as_deref(), Some(CIRCLE));
}

/// Chrome 153: `ol > li > ol > li > ul > li` is square.
#[test]
fn p2_ul_three_lists_deep_through_ols_is_square_like_chrome() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    let li = el(&mut doc, ol, "li");
    let ol2 = el(&mut doc, li, "ol");
    let li2 = el(&mut doc, ol2, "li");
    let ul = el(&mut doc, li2, "ul");
    let li3 = el(&mut doc, ul, "li");
    txt(&mut doc, li3, "f");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li3).as_deref(), Some(SQUARE));
}

/// Dynamic: an li restyled from display:flex to list-item gains a marker,
/// and back loses it.
#[test]
fn p3_display_toggle_regenerates_marker() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = el(&mut doc, ul, "li");
    doc.set_attribute(li, "style", "display: flex;");
    txt(&mut doc, li, "a");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li), None);
    doc.set_attribute(li, "style", "");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(marker_text(&doc, li).as_deref(), Some(DISC));
    assert_eq!(
        doc.tree.get(li.0).unwrap().computed_style.display,
        DisplayValue::ListItem
    );
    doc.set_attribute(li, "style", "display: block;");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(marker_text(&doc, li), None);
}

/// Dynamic: the ul's list-style-type changes; the (inheriting) li's marker
/// follows.
#[test]
fn p4_parent_list_style_type_change_regenerates_marker() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = el(&mut doc, ul, "li");
    txt(&mut doc, li, "a");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li).as_deref(), Some(DISC));
    doc.set_attribute(ul, "style", "list-style-type: square;");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(marker_text(&doc, li).as_deref(), Some(SQUARE));
    doc.set_attribute(ul, "class", "x");
    doc.load_css(".x { list-style-type: circle !important }");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(marker_text(&doc, li).as_deref(), Some(CIRCLE));
}

/// `display: inline list-item`: Chrome lays two such items on one line.
#[test]
fn p5_inline_list_item_is_inline_level() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let a = el(&mut doc, ul, "li");
    doc.set_attribute(
        a,
        "style",
        "display: inline list-item; font: 16px/20px sans-serif;",
    );
    txt(&mut doc, a, "a");
    let b = el(&mut doc, ul, "li");
    doc.set_attribute(
        b,
        "style",
        "display: inline list-item; font: 16px/20px sans-serif;",
    );
    txt(&mut doc, b, "b");
    doc.resolve_layout(800.0, 600.0);
    let ya = doc.tree.get(a.0).unwrap().layout.y;
    let yb = doc.tree.get(b.0).unwrap().layout.y;
    eprintln!(
        "inline list-item: display={:?} ya={ya} yb={yb}",
        doc.tree.get(a.0).unwrap().computed_style.display
    );
    assert_eq!(ya, yb, "two inline list-items share a line in Chrome");
}

/// position: fixed + top + auto height: `compute_content_height` stacks
/// children only for `DisplayValue::Block` — a `display: list-item` box with
/// two 20px block children must be 40px like its `display: block` twin.
#[test]
fn p6_fixed_list_item_content_height_stacks_like_block() {
    let mut h = Vec::new();
    for disp in ["block", "list-item"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let d = el(&mut doc, body, "div");
        doc.set_attribute(
            d,
            "style",
            &format!("position: fixed; top: 0; left: 0; width: 100px; display: {disp};"),
        );
        for _ in 0..2 {
            let c = el(&mut doc, d, "div");
            doc.set_attribute(c, "style", "height: 20px;");
        }
        doc.resolve_layout(800.0, 600.0);
        h.push(doc.tree.get(d.0).unwrap().layout.height);
    }
    eprintln!("fixed heights block/list-item: {h:?}");
    assert_eq!(h[0], h[1]);
}

/// Same, with a real `<li>` — on main (li = block) this was 40.
#[test]
fn p6b_fixed_li_content_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let d = el(&mut doc, ul, "li");
    doc.set_attribute(
        d,
        "style",
        "position: fixed; top: 0; left: 0; width: 100px; list-style: none;",
    );
    for _ in 0..2 {
        let c = el(&mut doc, d, "div");
        doc.set_attribute(c, "style", "height: 20px;");
    }
    doc.resolve_layout(800.0, 600.0);
    let h = doc.tree.get(d.0).unwrap().layout.height;
    eprintln!("fixed li height: {h}");
    assert_eq!(h, 40.0);
}

/// li::after (no ::before) under ul: Chrome draws both the disc and the
/// ::after. Pre-existing: resolve_list_marker skips any pseudo child.
#[test]
#[ignore = "#1370: any pseudo child suppresses the generated marker"]
fn p7_li_after_keeps_its_marker() {
    let mut doc = RinchDocument::new();
    doc.load_css(".z::after { content: '!' }");
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = el(&mut doc, ul, "li");
    doc.set_attribute(li, "class", "z");
    txt(&mut doc, li, "a");
    doc.resolve_layout(800.0, 600.0);
    let kids: Vec<String> = doc
        .tree
        .get(li.0)
        .unwrap()
        .children
        .iter()
        .filter(|&&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .map(|&c| get_text_content(&doc.tree, c))
        .collect();
    eprintln!("pseudo kids: {kids:?}");
    assert!(kids.iter().any(|k| k == DISC));
}

/// `<menu><li>`: Chrome disc.
#[test]
#[ignore = "#1370: a marker is generated only under a ul/ol parent"]
fn p8_menu_li_has_disc() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let m = el(&mut doc, body, "menu");
    let li = el(&mut doc, m, "li");
    txt(&mut doc, li, "g");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li).as_deref(), Some(DISC));
}

/// `ul ol ul` (three list levels): Chrome square.
#[test]
fn p9_ul_ol_ul_is_square() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ul = el(&mut doc, body, "ul");
    let li = el(&mut doc, ul, "li");
    let ol = el(&mut doc, li, "ol");
    let li2 = el(&mut doc, ol, "li");
    let ul2 = el(&mut doc, li2, "ul");
    let li3 = el(&mut doc, ul2, "li");
    txt(&mut doc, li3, "q");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, li3).as_deref(), Some(SQUARE));
}

/// `<ol start=3><li><li value=10><li>` numbering survives the keyword match.
#[test]
fn p10_ol_start_and_value() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    doc.set_attribute(ol, "start", "3");
    let a = el(&mut doc, ol, "li");
    let b = el(&mut doc, ol, "li");
    doc.set_attribute(b, "value", "10");
    let c = el(&mut doc, ol, "li");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, a).as_deref(), Some("3.\u{2002}"));
    assert_eq!(marker_text(&doc, b).as_deref(), Some("10.\u{2002}"));
    assert_eq!(marker_text(&doc, c).as_deref(), Some("11.\u{2002}"));
}

/// An `<ol>` styled `list-style-type: square` draws a square (Chrome).
#[test]
fn p11_ol_square() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    doc.set_attribute(ol, "style", "list-style-type: square;");
    let a = el(&mut doc, ol, "li");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, a).as_deref(), Some(SQUARE));
}

/// F5: an `<ol>` styled `list-style-type: disc` draws a disc. Off the fixed
/// point a `<ul>` sits on, where the tag fallback is a disc too.
#[test]
fn p12_ol_styled_disc_draws_a_disc() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ol = el(&mut doc, body, "ol");
    doc.set_attribute(ol, "style", "list-style-type: disc;");
    let a = el(&mut doc, ol, "li");
    txt(&mut doc, a, "a");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(marker_text(&doc, a).as_deref(), Some(DISC));
}

/// An in-flow `<li>` holding two 20px block children stacks them, exactly
/// as its `display: block` twin does — a list item is a block container,
/// not a flex row. (`list-style: none` keeps a marker out of the geometry.)
#[test]
fn p13_li_stacks_its_block_children_like_a_block_twin() {
    let mut geo = Vec::new();
    for disp in ["list-item", "block"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let ul = el(&mut doc, body, "ul");
        let li = el(&mut doc, ul, "li");
        doc.set_attribute(
            li,
            "style",
            &format!("display: {disp}; list-style: none; width: 200px;"),
        );
        let mut kids = Vec::new();
        for _ in 0..2 {
            let c = el(&mut doc, li, "div");
            doc.set_attribute(c, "style", "height: 20px; width: 50px;");
            kids.push(c);
        }
        doc.resolve_layout(800.0, 600.0);
        let n = |id: NodeId| doc.tree.get(id.0).unwrap().layout;
        geo.push((
            n(li).width,
            n(li).height,
            n(kids[0]).x,
            n(kids[0]).y,
            n(kids[1]).x,
            n(kids[1]).y,
        ));
    }
    assert_eq!(geo[0], geo[1], "list-item vs block");
    assert_eq!(geo[0].1, 40.0, "two 20px children stacked");
    assert_eq!(geo[0].5, 20.0, "the second child sits below the first");
}

/// An `<li>` holding a text run and then a block child: the run gets an
/// anonymous block box and the child stacks below it, exactly as in its
/// `display: block` twin. `DisplayMode` is what classifies the run, so this
/// is the fixture that fails when `ListItem` stops being `DisplayMode::Block`.
/// The line box is declared (`line-height: 20px`), so no font metric enters.
#[test]
fn p14_li_with_text_then_a_block_child_matches_a_block_twin() {
    let mut geo = Vec::new();
    for disp in ["list-item", "block"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let ul = el(&mut doc, body, "ul");
        let li = el(&mut doc, ul, "li");
        doc.set_attribute(
            li,
            "style",
            &format!("display: {disp}; list-style: none; width: 200px; line-height: 20px;"),
        );
        txt(&mut doc, li, "text");
        let c = el(&mut doc, li, "div");
        doc.set_attribute(c, "style", "height: 30px; width: 50px;");
        doc.resolve_layout(800.0, 600.0);
        let n = |id: NodeId| doc.tree.get(id.0).unwrap().layout;
        geo.push((n(li).height, n(c).x, n(c).y, n(c).width));
    }
    assert_eq!(geo[0], geo[1], "list-item vs block");
    assert_eq!(
        geo[0],
        (50.0, 0.0, 20.0, 50.0),
        "one 20px line, then the 30px block"
    );
}

/// An `<li>` holding a text run and an inline `<span>` is one inline
/// formatting context: both sit on one 20px line, as in its `display: block`
/// twin. This is what `DisplayMode::Block` (a block container) buys a list
/// item — as `DisplayMode::Flex` the run and the span would not share a line.
#[test]
fn p15_li_text_and_span_share_one_line_like_a_block_twin() {
    let mut heights = Vec::new();
    for disp in ["list-item", "block"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let ul = el(&mut doc, body, "ul");
        let li = el(&mut doc, ul, "li");
        doc.set_attribute(
            li,
            "style",
            &format!("display: {disp}; list-style: none; width: 200px; line-height: 20px;"),
        );
        txt(&mut doc, li, "a ");
        let s = el(&mut doc, li, "span");
        txt(&mut doc, s, "b");
        doc.resolve_layout(800.0, 600.0);
        heights.push(doc.tree.get(li.0).unwrap().layout.height);
    }
    assert_eq!(heights, vec![20.0, 20.0], "list-item vs block");
}
