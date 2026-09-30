//! #1172 — a forced break at the very end of a block makes no line box after
//! it.
//!
//! A line box after the last forced break — a `<br>`, or a preserved newline
//! under `pre`/`pre-wrap`/`pre-line`/`break-spaces` — exists only when
//! something comes after that break. rinch's IFC walk pushes a `<br>` into
//! parley as `"\n"`, and parley 0.11.1 always lays out one more, empty line
//! after a text-final newline (`line_break.rs`, "copy metrics from previous
//! line … an empty line following a newline at the end of a layout"), and
//! counts it: `<div>a<br></div>` was 40px tall where Chrome makes it 20.
//!
//! Every number is Chrome 153's on the same markup, measured with the
//! container at `width: 300px; font-size: 10px; line-height: 20px` and
//! `* { margin: 0 }`. Heights only — the line box is the declared
//! `line-height`, so no number here depends on the host's fonts (the bundled
//! Inter is registered anyway, so a wrap in the narrow fixture does not
//! either). A `<textarea>` is a different thing and not here: its value
//! ending in `"\n"` DOES show an empty line in Chrome (`scrollHeight` 40 for
//! `"a\n"`, one row of 20), because that is where its caret goes.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CONTAINER: &str =
    "width: 300px; font-family: ProbeFace; font-size: 10px; line-height: 20px; margin: 0";

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

/// One piece of a fixture's content.
#[derive(Clone, Copy)]
enum P<'a> {
    Text(&'a str),
    Br,
    /// An element with its own style, holding these pieces.
    El(&'a str, &'a str, &'a [P<'a>]),
}

fn build(d: &mut RinchDocument, parent: NodeId, pieces: &[P]) {
    for p in pieces {
        match *p {
            P::Text(s) => {
                let t = d.create_text(s);
                d.append_child(parent, t);
            }
            P::Br => {
                let b = d.create_element("br");
                d.append_child(parent, b);
            }
            P::El(tag, style, kids) => {
                let e = d.create_element(tag);
                if !style.is_empty() {
                    d.set_attribute(e, "style", style);
                }
                d.append_child(parent, e);
                build(d, e, kids);
            }
        }
    }
}

/// The container's height, laid out: `tag` styled `CONTAINER; extra`.
fn height_of(tag: &str, extra: &str, pieces: &[P]) -> f32 {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element(tag);
    d.set_attribute(c, "style", &format!("{CONTAINER}; {extra}"));
    d.append_child(body, c);
    build(&mut d, c, pieces);
    d.resolve_layout(800.0, 600.0);
    d.tree.get(c.0).unwrap().layout.height
}

fn check(tag: &str, extra: &str, pieces: &[P], chrome: f32, what: &str) {
    let h = height_of(tag, extra, pieces);
    assert!(
        (h - chrome).abs() < 0.5,
        "{what}: Chrome 153 makes it {chrome}px tall, rinch {h}px"
    );
}

use P::{Br, El, Text};

#[test]
fn a_trailing_br_makes_no_line_after_it() {
    check("div", "", &[Text("a"), Br], 20.0, "a<br>");
}

#[test]
fn a_lone_br_is_one_line() {
    check("div", "", &[Br], 20.0, "<br>");
}

#[test]
fn two_trailing_brs_make_one_empty_line_between() {
    check("div", "", &[Br, Br], 40.0, "<br><br>");
    check("div", "", &[Text("a"), Br, Br], 40.0, "a<br><br>");
}

#[test]
fn a_trailing_br_inside_an_inline_element() {
    check(
        "div",
        "",
        &[Text("a"), El("span", "", &[Br])],
        20.0,
        "a<span><br></span>",
    );
    check(
        "div",
        "",
        &[Text("a"), Br, El("b", "", &[Text("b"), Br])],
        40.0,
        "a<br><b>b<br></b>",
    );
}

#[test]
fn an_empty_element_or_a_collapsed_space_after_it_is_not_content() {
    check(
        "div",
        "",
        &[Text("a"), Br, El("span", "", &[])],
        20.0,
        "a<br><span></span>",
    );
    check("div", "", &[Text("a"), Br, Text(" ")], 20.0, "a<br> ");
}

#[test]
fn an_out_of_flow_box_after_it_is_not_content() {
    check(
        "div",
        "",
        &[
            Text("a"),
            Br,
            El("span", "position: absolute", &[Text("x")]),
        ],
        20.0,
        "a<br><span abs>x</span>",
    );
}

#[test]
fn a_final_preserved_newline_makes_no_line_after_it() {
    check(
        "div",
        "white-space: pre-wrap",
        &[Text("a\n")],
        20.0,
        "pre-wrap a\\n",
    );
    check(
        "div",
        "white-space: pre-wrap",
        &[Text("\n")],
        20.0,
        "pre-wrap \\n",
    );
    check(
        "div",
        "white-space: pre-wrap",
        &[Text("a"), Br],
        20.0,
        "pre-wrap a<br>",
    );
    check(
        "div",
        "white-space: pre-line",
        &[Text("a\n")],
        20.0,
        "pre-line a\\n",
    );
    check(
        "div",
        "white-space: pre-line",
        &[Text("a \n")],
        20.0,
        "pre-line a \\n",
    );
    check(
        "div",
        "white-space: break-spaces",
        &[Text("a\n")],
        20.0,
        "break-spaces a\\n",
    );
    check("pre", "", &[Text("a\n")], 20.0, "<pre>a\\n</pre>");
    check("pre", "", &[Text("\n")], 20.0, "<pre>\\n</pre>");
    check("pre", "", &[Text("a\n\n")], 40.0, "<pre>a\\n\\n</pre>");
}

#[test]
fn a_final_newline_after_wrapped_lines() {
    // Both words overflow a 20px box, so each is a line of its own; the final
    // newline adds none.
    check(
        "div",
        "white-space: pre-wrap; width: 20px",
        &[Text("aaaa aaaa\n")],
        40.0,
        "narrow pre-wrap `aaaa aaaa\\n`",
    );
}

#[test]
fn a_flex_items_ifc_ending_in_a_br() {
    check(
        "div",
        "display: flex",
        &[El("span", "", &[Text("a"), Br])],
        20.0,
        "flex <span>a<br></span>",
    );
}

/// Controls — content after the last break is a line, and stays one. Each
/// kills a repair that drops the last line after a forced break without
/// asking whether it holds anything.
#[test]
fn content_after_the_last_break_keeps_its_line() {
    check("div", "", &[Text("a"), Br, Text("b")], 40.0, "a<br>b");
    check(
        "div",
        "",
        &[Text("a"), Br, Text("\u{a0}")],
        40.0,
        "a<br>&nbsp;",
    );
    check(
        "div",
        "",
        &[El("span", "", &[Text("a"), Br]), Text("b")],
        40.0,
        "<span>a<br></span>b",
    );
    check(
        "div",
        "white-space: pre-wrap",
        &[Text("a\n ")],
        40.0,
        "pre-wrap `a\\n `",
    );
}

/// An atomic inline after the break is content even when it is 0x0: Chrome
/// makes `a<br><span ib 0x0>` 40px, a line for the box. rinch's height there
/// is 20 — at HEAD and after this change — because a line holding only an
/// atomic inline gets no strut (#624, #663), so the line is pinned by its
/// count, not by its height.
#[test]
fn an_empty_inline_block_after_the_break_keeps_its_line() {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", CONTAINER);
    d.append_child(body, c);
    build(
        &mut d,
        c,
        &[
            Text("a"),
            Br,
            El("span", "display: inline-block; width: 0; height: 0", &[]),
        ],
    );
    d.resolve_layout(800.0, 600.0);
    let lines = d
        .tree
        .get(c.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("an IFC root")
        .layout
        .len();
    assert_eq!(lines, 2, "a line for `a`, a line for the 0x0 box");
}

/// A flex or grid container's own text is an anonymous item measured as a
/// text leaf, not an IFC — a second route to the same parley behaviour.
#[test]
fn a_flex_or_grid_containers_own_text_ending_in_a_newline() {
    check(
        "div",
        "display: flex; white-space: pre-wrap",
        &[Text("a\n")],
        20.0,
        "flex pre-wrap a\\n",
    );
    check(
        "div",
        "display: flex; white-space: pre",
        &[Text("a\n\n")],
        40.0,
        "flex pre a\\n\\n",
    );
    check(
        "div",
        "display: grid; white-space: pre-wrap",
        &[Text("a\n")],
        20.0,
        "grid pre-wrap a\\n",
    );
}

/// The same inside an `inline-flex`, whose text leaf is measured by the
/// detached atomic-inline compute rather than the root compute — the third
/// route. Chrome 153: 20.
#[test]
fn an_inline_flexs_own_text_ending_in_a_newline() {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", CONTAINER);
    d.append_child(body, c);
    build(
        &mut d,
        c,
        &[El(
            "span",
            "display: inline-flex; white-space: pre-wrap",
            &[Text("a\n")],
        )],
    );
    d.resolve_layout(800.0, 600.0);
    let chip = d.tree.get(c.0).unwrap().children[0];
    let h = d.tree.get(chip).unwrap().layout.height;
    assert!(
        (h - 20.0).abs() < 0.5,
        "inline-flex pre-wrap a\\n: Chrome 20, rinch {h}"
    );
}

/// More shapes, from the review of PR #1197, each Chrome 153's number on the
/// same markup: a trailing break reached through a `display: contents`
/// wrapper, an `inline-block`'s own IFC, a newline inside a span, and things
/// after the break that are not content (a collapsed space in a positioned
/// span, empty spans and wrappers, a float).
#[test]
fn more_shapes_of_a_final_forced_break() {
    let cases: Vec<(&str, &str, Vec<P>, f32)> = vec![
        (
            "a<span contents>b<br></span>",
            "",
            vec![Text("a"), El("span", "display: contents", &[Text("b"), Br])],
            20.0,
        ),
        (
            "x<span ib>a<br></span>y",
            "",
            vec![
                Text("x"),
                El("span", "display: inline-block", &[Text("a"), Br]),
                Text("y"),
            ],
            20.0,
        ),
        (
            "a<br><span relative>  </span>",
            "",
            vec![
                Text("a"),
                Br,
                El("span", "position: relative", &[Text("  ")]),
            ],
            20.0,
        ),
        (
            "<span>a<br></span><span></span>",
            "",
            vec![El("span", "", &[Text("a"), Br]), El("span", "", &[])],
            20.0,
        ),
        (
            "a<br><span contents></span>",
            "",
            vec![Text("a"), Br, El("span", "display: contents", &[])],
            20.0,
        ),
        (
            "a<br><span float></span>",
            "",
            vec![Text("a"), Br, El("span", "float: left", &[])],
            20.0,
        ),
        (
            "pre-wrap a<span>\\n</span>",
            "white-space: pre-wrap",
            vec![Text("a"), El("span", "", &[Text("\n")])],
            20.0,
        ),
        (
            "pre-wrap a\\n  (control)",
            "white-space: pre-wrap",
            vec![Text("a\n  ")],
            40.0,
        ),
    ];
    for (what, extra, pieces, chrome) in &cases {
        check("div", extra, pieces, *chrome, what);
    }
}
