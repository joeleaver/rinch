//! #2054: what a scroll container's kept descendant walk holds, and what it
//! costs, as boxes come and go: a record goes with its box (taken out,
//! moved or freed), a walk goes with its container, and a box resized among
//! many siblings is brought up to date without visiting them.
//! (`testing::deep_overflow_held`, `deep_overflow_walks`,
//! `deep_overflow_measured`.)

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::{deep_overflow_held, deep_overflow_measured, deep_overflow_walks};

fn kept_doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

fn range_of(d: &RinchDocument, id: &str) -> (f64, f64) {
    let s = by_id(d, id);
    (d.scroll_width(s), d.scroll_height(s))
}

/// A layout, a read of `#s`'s range and a paint consuming the tree: one
/// frame.
fn frame(d: &mut RinchDocument) -> (f64, f64) {
    d.resolve_layout(800.0, 600.0);
    let range = range_of(d, "s");
    d.tree.consume_paint_dirty();
    range
}

/// How the tests take a box out.
#[derive(Debug, Clone, Copy)]
enum Out {
    /// `remove_child`: detached, alive, may be put back.
    Remove,
    /// `discard_node`: detached and freed.
    Discard,
}

fn take_out(d: &mut RinchDocument, parent: NodeId, child: NodeId, how: Out) {
    match how {
        Out::Remove => d.remove_child(parent, child),
        Out::Discard => d.discard_node(child),
    }
}

/// A row of two boxes, the inner one past the row's width.
fn row(d: &mut RinchDocument, width: &str) -> NodeId {
    let row = d.create_element("div");
    d.set_attribute(row, "style", "height:20px");
    let inner = d.create_element("div");
    d.set_attribute(inner, "style", &format!("width:{width};height:10px"));
    d.append_child(row, inner);
    row
}

const LIST: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="l" style="width:150px"><div style="height:20px"><div style="width:300px;height:10px"></div></div></div></div>"#;

/// A row appended, read and taken out, over and over: the memo holds what
/// it held before, not a record of every row it ever met (each stayed
/// until a whole-document restyle).
#[test]
fn rows_taken_out_leave_no_records() {
    for how in [Out::Remove, Out::Discard] {
        let mut d = kept_doc(LIST);
        assert_eq!(frame(&mut d), (300.0, 20.0), "control: the inner box");
        let held = deep_overflow_held(&d.tree);
        assert_eq!(
            held,
            (2, 4),
            "control: the root scroller's walk (#s's record) and #s's (#l, its row and the row's box)"
        );
        let l = by_id(&d, "l");
        for _ in 0..200 {
            let r = row(&mut d, "420px");
            d.append_child(l, r);
            assert_eq!(frame(&mut d), (420.0, 40.0), "{how:?}: the row read");
            take_out(&mut d, l, r, how);
            assert_eq!(frame(&mut d), (300.0, 20.0), "{how:?}: the row gone");
        }
        assert_eq!(deep_overflow_held(&d.tree), held, "{how:?}");
    }
}

/// A scroll container added beside `#s`, read, and taken out, over and
/// over: its walk and its records go with it.
#[test]
fn a_scroller_taken_out_leaves_no_walk() {
    for how in [Out::Remove, Out::Discard] {
        let mut d = kept_doc(&format!(r#"{LIST}<div id="b"></div>"#));
        frame(&mut d);
        let held = deep_overflow_held(&d.tree);
        let b = by_id(&d, "b");
        for _ in 0..50 {
            let x = d.create_element("div");
            d.set_attribute(x, "style", "width:100px;height:50px;overflow:auto");
            for _ in 0..10 {
                let r = row(&mut d, "230px");
                d.append_child(x, r);
            }
            d.append_child(b, x);
            frame(&mut d);
            assert_eq!(d.scroll_width(x), 230.0, "{how:?}: control, its range");
            assert!(
                deep_overflow_held(&d.tree).0 > held.0,
                "{how:?}: its walk kept"
            );
            take_out(&mut d, b, x, how);
            frame(&mut d);
        }
        assert_eq!(deep_overflow_held(&d.tree), held, "{how:?}");
    }
}

/// A row taken out and put back where it was is recorded again, and the
/// range follows each step.
#[test]
fn a_row_put_back_is_recorded_again() {
    let mut d = kept_doc(LIST);
    frame(&mut d);
    let held = deep_overflow_held(&d.tree);
    let l = by_id(&d, "l");
    let r = row(&mut d, "420px");
    d.append_child(l, r);
    assert_eq!(frame(&mut d), (420.0, 40.0));
    d.remove_child(l, r);
    assert_eq!(frame(&mut d), (300.0, 20.0));
    assert_eq!(deep_overflow_held(&d.tree), held, "the row's records went");
    d.append_child(l, r);
    assert_eq!(frame(&mut d), (420.0, 40.0), "put back, it counts again");
    assert_eq!(deep_overflow_held(&d.tree), (held.0, held.1 + 2));
}

const MOVE: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="p" style="width:150px;height:40px"></div><div id="q" style="width:150px;height:40px"></div></div>"#;

/// A box moved to another parent under the scroller is brought up to date
/// where it is, not walked again: its records are dropped (it was met under
/// its old parent) and it is recorded under the new one. Measured: the two
/// parents and the moved box with its child. (Its records kept, it would be
/// met under another parent than recorded, and the whole scroller walked
/// again.)
///
/// The box is moved before any paint consumed it: a painted box's move
/// drops its records when it is made (`record_pixels_left_by_move`, through
/// `forget_detached`), so only an unpainted one reaches the check's own
/// "under another parent" branch (`Marker::note`), which this pins.
#[test]
fn a_box_moved_to_another_parent_is_recorded_where_it_is() {
    let mut d = kept_doc(MOVE);
    assert_eq!(frame(&mut d), (150.0, 80.0), "control: p and q");
    let (p, q) = (by_id(&d, "p"), by_id(&d, "q"));
    let x = row(&mut d, "330px");
    d.set_attribute(x, "style", "width:20px;height:10px");
    d.append_child(p, x);
    d.resolve_layout(800.0, 600.0);
    assert_eq!(range_of(&d, "s"), (330.0, 80.0), "control: x's child read");
    let held = deep_overflow_held(&d.tree);
    d.append_child(q, x);
    d.resolve_layout(800.0, 600.0);
    let (walks, measured) = (
        deep_overflow_walks(&d.tree),
        deep_overflow_measured(&d.tree),
    );
    assert_eq!(range_of(&d, "s"), (330.0, 80.0));
    assert_eq!(deep_overflow_walks(&d.tree), walks, "not walked again");
    assert_eq!(
        deep_overflow_measured(&d.tree) - measured,
        4,
        "p, q, and x with its child recorded under q"
    );
    assert_eq!(deep_overflow_held(&d.tree), held);
}

/// Of a box's many children, one resized: the box's span is brought up to
/// date from that child alone — joined where it grows or shrinks inside the
/// span, summed from all of them where it held the span's edge and no
/// longer reaches it. Each step against a fresh document; measured: the
/// child and its parent.
#[test]
fn one_child_resized_among_many_is_brought_up_to_date_alone() {
    let rows: String = (0..50)
        .map(|i| {
            let w = if i == 7 { 400 } else { 100 + i };
            format!(r#"<div style="width:{w}px;height:10px"></div>"#)
        })
        .collect();
    let html = format!(
        r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="l" style="position:relative;width:150px;height:60px">{rows}<div id="g" style="position:absolute;left:0;top:0;width:300px;height:10px"></div></div></div>"#
    );
    let fresh = |width: &str| {
        let mut d = kept_doc(&html);
        let g = by_id(&d, "g");
        d.set_style(g, "width", width);
        d.resolve_layout(800.0, 600.0);
        range_of(&d, "s")
    };
    let mut d = kept_doc(&html);
    assert_eq!(frame(&mut d), (400.0, 500.0), "control: the widest row");
    let g = by_id(&d, "g");
    // Inside the span, past it, and back from its edge.
    for width in ["280px", "450px", "120px"] {
        d.set_style(g, "width", width);
        d.resolve_layout(800.0, 600.0);
        let (walks, measured) = (
            deep_overflow_walks(&d.tree),
            deep_overflow_measured(&d.tree),
        );
        assert_eq!(range_of(&d, "s"), fresh(width), "the gauge at {width}");
        assert_eq!(deep_overflow_walks(&d.tree), walks, "{width}: not walked");
        assert_eq!(
            deep_overflow_measured(&d.tree) - measured,
            2,
            "{width}: the gauge and its parent"
        );
        d.tree.consume_paint_dirty();
    }
    assert_eq!(
        range_of(&d, "s"),
        (400.0, 500.0),
        "control: back to the row"
    );
}
