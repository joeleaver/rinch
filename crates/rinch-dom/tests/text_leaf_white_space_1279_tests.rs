//! A flex or grid item's own text — a text **leaf**, measured through
//! `NodeContext::Text` rather than an inline formatting context — takes the
//! white-space rules an IFC does (#1279, #1672).
//!
//! An IFC collapses its text per `white-space` ([`IfcText`], #1192), hangs
//! preserved spaces at a soft wrap (`break_lines_hanging_spaces`), keeps
//! `break-spaces` spaces on the line as content (#1043/#1671), and lays out
//! no line after a text-final newline (#1172). A leaf did none of the first
//! three: its text went to parley exactly as written. So a leaf ending in a
//! space was measured with the space (it is removed at a line's end in CSS),
//! and broken at that width the space wrapped onto a line of its own —
//! `<div style="display:flex">Use </div>` was 50px tall at a 25px line, and
//! so was every `Text` whose children are split at a space.
//!
//! Every expected number is Chrome 155's, measured with the same markup and
//! the bundled Inter (registered under an override name, so the host's fonts
//! cannot answer). Heights are the `display: flex` container's in a block of
//! the given width; widths are a `display: inline-flex` container's (the
//! item's max-content width — the second leaf measure site, the atomic
//! inline's own compute), which Taffy rounds to whole pixels.
//!
//! [`IfcText`]: rinch_dom::ifc

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// `(flex container height in a block `width` wide, inline-flex width,
/// inline-flex height)` for `text` under `white-space: ws`.
fn measure(ws: &str, text: &str, font: (f32, f32), width: f32) -> (f32, f32, f32) {
    let mut d = doc();
    let body = d.body();
    let root = el(&mut d, body, "div", "margin: 0; font-family: ProbeFace");
    let typo = format!(
        "white-space: {ws}; font-size: {}px; line-height: {}px",
        font.0, font.1
    );
    let block = el(&mut d, root, "div", &format!("width: {width}px"));
    let flex = el(&mut d, block, "div", &format!("display: flex; {typo}"));
    let t = d.create_text(text);
    d.append_child(flex, t);
    let line = el(&mut d, root, "div", "");
    let inline = el(
        &mut d,
        line,
        "div",
        &format!("display: inline-flex; {typo}"),
    );
    let t = d.create_text(text);
    d.append_child(inline, t);
    d.resolve_layout(800.0, 600.0);
    let f = d.tree.get(flex.0).unwrap().layout;
    let i = d.tree.get(inline.0).unwrap().layout;
    (f.height, i.width, i.height)
}

/// One row of a Chrome 155 table: `(white-space, text, (font-size,
/// line-height), block width, flex height, inline-flex width, inline-flex
/// height)`.
type Row = (&'static str, &'static str, (f32, f32), f32, f32, f32, f32);

fn check(rows: &[Row]) {
    let mut wrong = Vec::new();
    for &(ws, text, font, width, h, iw, ih) in rows {
        let got = measure(ws, text, font, width);
        let want = (h, iw.round(), ih);
        if got != want {
            wrong.push(format!(
                "{ws} {text:?} in {width}px: rinch {got:?}, Chrome {want:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// The issue's case and its neighbours: a trailing collapsible space is
/// removed at the line's end, so it neither widens the item nor makes a line.
#[test]
fn a_collapsible_space_at_the_end_makes_no_line_1279() {
    check(&[
        ("normal", "Use ", (16.0, 25.0), 300.0, 25.0, 29.6875, 25.0),
        (
            "normal",
            "Use  x ",
            (16.0, 25.0),
            300.0,
            25.0,
            42.921875,
            25.0,
        ),
        (
            "normal",
            "  a  b  ",
            (16.0, 25.0),
            300.0,
            25.0,
            23.28125,
            25.0,
        ),
        ("normal", "Use ", (16.0, 25.0), 20.0, 25.0, 29.6875, 25.0),
        ("normal", "aa bb ", (16.0, 25.0), 30.0, 50.0, 42.0625, 25.0),
        ("nowrap", "a b ", (16.0, 25.0), 300.0, 25.0, 23.28125, 25.0),
        // All white space: measured 0x0, preserved or not. (Chrome makes no
        // item of it at all; rinch's is still an item and takes a `gap`, #1760.)
        ("normal", "   ", (16.0, 25.0), 300.0, 0.0, 0.0, 0.0),
        ("pre-wrap", "   ", (16.0, 25.0), 300.0, 0.0, 0.0, 0.0),
    ]);
}

/// The issue's comment's table (Chrome 155, 10px/20px, 300px wide) and more
/// shapes of preserved white space at a leaf's end.
#[test]
fn preserved_white_space_at_the_end_1279() {
    check(&[
        ("pre-wrap", "a\n  ", (10.0, 20.0), 300.0, 40.0, 5.625, 40.0),
        (
            "pre-wrap",
            "a\n  \n",
            (10.0, 20.0),
            300.0,
            40.0,
            5.625,
            40.0,
        ),
        ("pre-line", "a\n ", (10.0, 20.0), 300.0, 20.0, 5.625, 20.0),
        ("pre-line", "a\n  ", (10.0, 20.0), 300.0, 20.0, 5.625, 20.0),
        (
            "pre-line",
            "a\n  \n",
            (10.0, 20.0),
            300.0,
            40.0,
            5.625,
            40.0,
        ),
        (
            "break-spaces",
            "a\n  ",
            (10.0, 20.0),
            300.0,
            40.0,
            5.625,
            40.0,
        ),
        (
            "break-spaces",
            "a\n  \n",
            (10.0, 20.0),
            300.0,
            40.0,
            5.625,
            40.0,
        ),
        (
            "pre-wrap",
            "a   ",
            (16.0, 25.0),
            300.0,
            25.0,
            22.484375,
            25.0,
        ),
        (
            "pre-wrap",
            "aa   bb",
            (16.0, 25.0),
            30.0,
            50.0,
            51.0625,
            25.0,
        ),
        // Spaces at a soft wrap hang, all of them: parley hangs one and
        // commits the line, and the rest made a line of their own.
        (
            "pre-wrap",
            "aa      ",
            (16.0, 25.0),
            30.0,
            25.0,
            44.96875,
            25.0,
        ),
        (
            "pre-wrap",
            "aa      bb",
            (16.0, 25.0),
            30.0,
            50.0,
            64.5625,
            25.0,
        ),
        (
            "pre-wrap",
            "aa  \n  bb",
            (16.0, 25.0),
            30.0,
            50.0,
            28.59375,
            50.0,
        ),
        ("pre-wrap", "Use ", (16.0, 25.0), 300.0, 25.0, 34.1875, 25.0),
        (
            "break-spaces",
            "a   ",
            (16.0, 25.0),
            300.0,
            25.0,
            22.484375,
            25.0,
        ),
        (
            "pre-line",
            "a  b  \nc ",
            (16.0, 25.0),
            300.0,
            50.0,
            23.28125,
            50.0,
        ),
        (
            "pre-line",
            "  a\n  b",
            (16.0, 25.0),
            300.0,
            50.0,
            9.796875,
            50.0,
        ),
        ("pre", "a  ", (16.0, 25.0), 300.0, 25.0, 17.984375, 25.0),
    ]);
}

/// #1672: a `break-spaces` leaf's spaces do not hang. Chrome: `bullet ` /
/// `text  x` at 75px, and two lines at 40px too (the second overflowing).
#[test]
fn break_spaces_spaces_do_not_hang_in_a_leaf_1672() {
    check(&[
        (
            "break-spaces",
            "bullet text  x",
            (16.0, 25.0),
            75.0,
            50.0,
            92.140625,
            25.0,
        ),
        (
            "break-spaces",
            "bullet text  x",
            (16.0, 25.0),
            40.0,
            50.0,
            92.140625,
            25.0,
        ),
    ]);
}

/// The issue's second row: `Use <span>inline</span> within text.` in a
/// wrapping flex row. The `Use ` item was 30x50 and the row 50 tall; Chrome
/// lays the row out one line tall.
#[test]
fn a_split_text_component_is_one_line_tall_1279() {
    let mut d = doc();
    let body = d.body();
    let root = el(
        &mut d,
        body,
        "div",
        "margin: 0; font-family: ProbeFace; width: 300px",
    );
    let flex = el(
        &mut d,
        root,
        "div",
        "display: flex; flex-wrap: wrap; font-size: 16px; line-height: 25px",
    );
    let t = d.create_text("Use ");
    d.append_child(flex, t);
    let span = el(&mut d, flex, "span", "");
    let inner = d.create_text("inline");
    d.append_child(span, inner);
    let t2 = d.create_text(" within text.");
    d.append_child(flex, t2);
    d.resolve_layout(800.0, 600.0);
    assert_eq!(d.tree.get(flex.0).unwrap().layout.height, 25.0);
    let item = d.tree.get(t.0).unwrap().layout;
    assert_eq!((item.width, item.height), (30.0, 25.0), "the `Use ` item");
}

/// #1268's table: a leaf lays U+2028 out as a space-wide character with a
/// break opportunity after it (not a forced break) and U+0085 as nothing,
/// as an IFC does (`ifc::laid_out_as`). Chrome 153 (bundled Inter, 16px/25px):
/// 21.97 x 25 and 17.47 x 25.
#[test]
fn line_and_paragraph_separators_in_a_leaf_1268() {
    check(&[
        (
            "normal",
            "x\u{2028}x",
            (16.0, 25.0),
            300.0,
            25.0,
            21.97,
            25.0,
        ),
        ("pre", "x\u{2028}x", (16.0, 25.0), 300.0, 25.0, 21.97, 25.0),
        ("normal", "x\u{85}x", (16.0, 25.0), 300.0, 25.0, 17.47, 25.0),
    ]);
}

/// A blank leaf reports no first baseline, so it moves no baseline-aligned
/// item (review of #1759). Chrome 155, 40px/50px container, two 10px/12px
/// spans around `"   "` under `align-items: baseline`: the spans at y 0, the
/// row 12 tall, under `normal` and `pre-wrap` alike. With the blank leaf's
/// 40px first line as a baseline, rinch put them 29px down.
#[test]
fn a_blank_leaf_has_no_baseline() {
    for ws in ["normal", "pre-wrap"] {
        let mut d = doc();
        let body = d.body();
        let root = el(&mut d, body, "div", "margin: 0; font: 40px/50px ProbeFace");
        let flex = el(
            &mut d,
            root,
            "div",
            &format!("display: flex; align-items: baseline; white-space: {ws}"),
        );
        let a = el(&mut d, flex, "span", "font: 10px/12px ProbeFace");
        let t = d.create_text("a");
        d.append_child(a, t);
        let blank = d.create_text("   ");
        d.append_child(flex, blank);
        let b = el(&mut d, flex, "span", "font: 10px/12px ProbeFace");
        let t = d.create_text("b");
        d.append_child(b, t);
        d.resolve_layout(800.0, 600.0);
        let ay = d.tree.get(a.0).unwrap().layout.y;
        let h = d.tree.get(flex.0).unwrap().layout.height;
        assert_eq!((ay, h), (0.0, 12.0), "{ws}");
    }
}

/// A preserved tab in a leaf stays a tab (its stops are #1585): pinned at
/// the widths rinch drew before #1279, so that a tab turned into a space by
/// the collapsing pass is seen. Not Chrome's numbers.
#[test]
fn a_preserved_tab_in_a_leaf_stays_a_tab() {
    check(&[
        ("pre", "a\tb\tc", (16.0, 25.0), 300.0, 25.0, 49.0, 25.0),
        ("pre-wrap", "a\t\tb", (16.0, 25.0), 300.0, 25.0, 40.0, 25.0),
    ]);
}
