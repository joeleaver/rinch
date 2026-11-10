//! Which line boxes are "empty" — zero height, no strut — and which get the
//! block's strut (CSS 2.2 §9.4.2, §10.8.1). #624 / #1258's residual after
//! parley 0.12: a line holding only a **zero-size** atomic inline (a 0x0
//! tracking-pixel `<img>`, an empty `inline-block`) was 0 tall, where Chrome
//! gives it the block's line height.
//!
//! parley's line breaker counted an in-flow inline box as content only when it
//! had a non-zero ascent or descent, and made a line with no content an
//! invisible line box. CSS counts "images, inline blocks or inline tables" with
//! no size condition, and so does Blink: `InlineItem::ComputeBoxProperties`
//! leaves an atomic inline non-empty whatever its size, and an inline box's
//! open/close tag is non-empty when its inline-start/end border, padding or
//! margin is non-zero (block-axis ones are not looked at). rinch hands an
//! inline element's inline-start/end margin+border+padding to parley as a
//! zero-height in-flow box (`ifc::IfcText::push_edge_box`, #1426), so the same
//! rule in parley gave both (rinch's parley fork, `rinch-patch`:
//! "an in-flow inline box is content whatever its size").
//!
//! # Chrome 156, headless, standards mode
//!
//! Every container is `width: 400px; font-size: 12px; line-height: 23px`
//! (off the 20px a default would give, and not a multiple of the font size).
//! The heights are declarations: a line's strut is exactly its `line-height`
//! here, and nothing on these lines reaches past it, so no number depends on
//! the local fonts.
//!
//! | content of the container | Chrome | rinch before |
//! |---|---|---|
//! | `<div h10></div><img width=0 height=0>` (the mail's shape) | 33 | **10** |
//! | `<img width=0 height=0>` | 23 | **0** |
//! | `<img style="width:0;height:0">` | 23 | **0** |
//! | empty `inline-block` 0x0 / 30x0 / auto | 23 | **0** |
//! | empty `inline-flex` / `inline-grid` 0x0 | 23 | **0** |
//! | 0x0 `inline-block`, `vertical-align: top` | 23 | **0** |
//! | 0x0 `inline-block`, `visibility: hidden` | 23 | **0** |
//! | 10x5 `inline-block`, `margin-bottom: -5px` (0 tall margin box) | 23 | **0** |
//! | 0x5 `inline-block` | 23 | 23 |
//! | `<span><img 0x0></span>` | 23 | **0** |
//! | spaces around a 0x0 `inline-block` | 23 | **0** |
//! | `A<br><img 0x0>` | 46 | **23** |
//! | `<div h10></div><img 0x0><div h10></div>` | 43 | **20** |
//! | empty `<span>` with `padding-left: 5px` / `padding-right` / `border-left: 1px` / `margin-left: 5px` | 23 | **0** |
//! | `<span padding-left:5px><inline-block 0x0></span>` | 23 | **0** |
//! | `<br>` / `<br><br>` | 23 / 46 | 23 / 46 |
//! | collapsible spaces only | 0 | 0 |
//! | empty `<span>` | 0 | 0 |
//! | empty `<span>` with `padding-top: 5px` / `border-top: 3px` | 0 | 0 |
//! | an absolutely positioned / floated `<span>` only | 0 | 0 |
//! | empty `<span>` with `margin-left: -5px` / `margin-right: -5px` | 23 | **0** |
//! | empty `<span>` with `margin-left: -5px; padding-left: 5px` (sums to 0) | 23 | **0** |
//!
//! The last two rows were #2406: rinch pushed an edge box only for a positive
//! edge, so a negative one left nothing on the line to count. A side counts
//! when any of its margin, border or padding is non-zero, not when their sum
//! is, as in Blink. The zero rows are here so the fix cannot be bought by
//! giving every line a strut.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const IMG00: &str = r#"<img width="0" height="0" alt="" src="x.png">"#;

/// Lay `inner` out in one `width: 400px; font-size: 12px; line-height: 23px`
/// container and return the container's height.
fn height(inner: &str) -> f32 {
    let mut doc = RinchDocument::new();
    doc.load_css("body { margin: 0 } .c { width: 400px; font-size: 12px; line-height: 23px }");
    let body = doc.body();
    doc.set_inner_html(body, &format!(r#"<div class="c" id="c">{inner}</div>"#));
    doc.resolve_layout(720.0, 800.0);
    let c = rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, "#c")[0];
    doc.tree.nodes[c].layout.height
}

fn check(cases: &[(&str, &str, f32)]) {
    let mut wrong = Vec::new();
    for &(name, inner, chrome) in cases {
        let got = height(inner);
        if got != chrome {
            wrong.push(format!("{name}: rinch {got}, Chrome {chrome}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The mail's shape: a 0x0 image on a line after the last block. Chrome 33
/// (10 + one 23px line); rinch was 10, the line dropped.
#[test]
fn a_line_holding_only_a_zero_size_image_after_a_block_gets_the_strut() {
    check(&[(
        "div h10 + img 0x0",
        &format!(r#"<div style="height:10px"></div>{IMG00}"#),
        33.0,
    )]);
}

/// Every spelling of a zero-size atomic inline alone on a line: Chrome 23,
/// the line height, for all of them.
#[test]
fn a_line_holding_only_a_zero_size_atomic_inline_gets_the_strut() {
    let ib = |style: &str| format!(r#"<span style="display:inline-block;{style}"></span>"#);
    check(&[
        ("img 0x0 attributes", IMG00, 23.0),
        (
            "img 0x0 style",
            r#"<img style="width:0;height:0" alt="" src="x.png">"#,
            23.0,
        ),
        ("inline-block 0x0", &ib("width:0;height:0"), 23.0),
        ("inline-block 30x0", &ib("width:30px;height:0"), 23.0),
        ("inline-block auto, empty", &ib(""), 23.0),
        (
            "inline-flex 0x0",
            r#"<span style="display:inline-flex;width:0;height:0"></span>"#,
            23.0,
        ),
        (
            "inline-grid 0x0",
            r#"<span style="display:inline-grid;width:0;height:0"></span>"#,
            23.0,
        ),
        (
            "inline-block 0x0 vertical-align top",
            &ib("width:0;height:0;vertical-align:top"),
            23.0,
        ),
        (
            "inline-block 0x0 vertical-align bottom",
            &ib("width:0;height:0;vertical-align:bottom"),
            23.0,
        ),
        (
            "inline-block 0x0 visibility hidden",
            &ib("width:0;height:0;visibility:hidden"),
            23.0,
        ),
        (
            "inline-block 10x5 margin-bottom -5px",
            &ib("width:10px;height:5px;margin-bottom:-5px"),
            23.0,
        ),
        (
            "inline-block 0x5 (control)",
            &ib("width:0;height:5px"),
            23.0,
        ),
        ("img 0x0 in a span", &format!("<span>{IMG00}</span>"), 23.0),
        (
            "spaces around inline-block 0x0",
            &format!("  {}   ", ib("width:0;height:0")),
            23.0,
        ),
    ]);
}

/// A zero-size image on a line of its own among other lines: each line still
/// counts. Chrome 46 and 43.
#[test]
fn a_zero_size_image_line_counts_among_other_lines() {
    check(&[
        ("A<br>img 0x0", &format!("A<br>{IMG00}"), 46.0),
        (
            "div h10 + img 0x0 + div h10",
            &format!(r#"<div style="height:10px"></div>{IMG00}<div style="height:10px"></div>"#),
            43.0,
        ),
    ]);
}

/// An empty inline element with an inline-start or inline-end margin, border
/// or padding makes its line non-empty (Blink's `IsInlineBoxStartEmpty` /
/// `IsInlineBoxEndEmpty`): Chrome 23.
#[test]
fn an_empty_span_with_an_inline_edge_gets_the_strut() {
    check(&[
        (
            "span padding-left",
            r#"<span style="padding-left:5px"></span>"#,
            23.0,
        ),
        (
            "span padding-right",
            r#"<span style="padding-right:5px"></span>"#,
            23.0,
        ),
        (
            "span border-left",
            r#"<span style="border-left:1px solid red"></span>"#,
            23.0,
        ),
        (
            "span margin-left",
            r#"<span style="margin-left:5px"></span>"#,
            23.0,
        ),
        (
            "span margin-left -5 (#2406)",
            r#"<span style="margin-left:-5px"></span>"#,
            23.0,
        ),
        (
            "span margin-right -5 (#2406)",
            r#"<span style="margin-right:-5px"></span>"#,
            23.0,
        ),
        (
            "span margin-left -5 padding-left 5, summing to 0 (#2406)",
            r#"<span style="margin-left:-5px;padding-left:5px"></span>"#,
            23.0,
        ),
        (
            "span padding-left around inline-block 0x0",
            r#"<span style="padding-left:5px"><span style="display:inline-block;width:0;height:0"></span></span>"#,
            23.0,
        ),
    ]);
}

/// What stays an empty line in Chrome, and must stay one: nothing here is
/// in-flow content, so the line is 0 tall (or a `<br>` line, the strut, as
/// before). These hold the fix to "in-flow boxes", not "every line".
#[test]
fn lines_with_no_in_flow_content_stay_empty() {
    check(&[
        ("collapsible spaces", "    ", 0.0),
        ("empty span", "<span></span>", 0.0),
        (
            "span padding-top only",
            r#"<span style="padding-top:5px"></span>"#,
            0.0,
        ),
        (
            "span border-top only",
            r#"<span style="border-top:3px solid red"></span>"#,
            0.0,
        ),
        (
            "absolutely positioned span",
            r#"<span style="position:absolute;width:10px;height:10px"></span>"#,
            0.0,
        ),
        (
            "floated span",
            r#"<span style="float:left;width:10px;height:10px"></span>"#,
            0.0,
        ),
        ("br", "<br>", 23.0),
        ("br br", "<br><br>", 46.0),
    ]);
}
