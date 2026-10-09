//! Review fixtures for PR #1494 (static position of out-of-flow boxes; issues
//! #633, #632, #634) — shapes the PR's own 176 rows do not hold.
//!
//! Every number is Chrome 153's, measured the way `static_position_tests`
//! measures (`--headless=new`, standards mode, `* { box-sizing: border-box }`,
//! 800x600, DSF 1, the bundled Inter through `@font-face`, declared 20px
//! lines), relative to the container's border box. Names: `f_` fixed, `a_`
//! absolute, `fi_` / `ai_` the same on a `<span>` (inline-level before
//! `position` blockified it).
//!
//! - `ROWS`: rinch agrees with Chrome.
//! - `SCROLLED`: before and after a scroll, and after a relayout in that
//!   scrolled state.
//! - `KNOWN`: rinch does **not** agree; the row pins what rinch gives and
//!   names the review finding. A fix flips the row into `ROWS`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";

struct Row {
    name: &'static str,
    c: &'static str,
    html: &'static str,
    want: [f32; 2],
}

struct Known {
    name: &'static str,
    why: &'static str,
    c: &'static str,
    html: &'static str,
    chrome: [f32; 2],
    rinch: [f32; 2],
}

struct Scrolled {
    name: &'static str,
    c: &'static str,
    html: &'static str,
    before: [f32; 2],
    after: [f32; 2],
    rinch_after: [f32; 2],
}

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    doc
}

fn build(c: &str, html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(r#"<div data-m="c" style="{C}{c}">{html}</div>"#),
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector}");
    found[0]
}

fn screen(doc: &RinchDocument, selector: &str) -> [f32; 2] {
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, one(doc, selector), 1.0);
    [x as f32, y as f32]
}

fn rel(doc: &RinchDocument) -> [f32; 2] {
    let [x, y] = screen(doc, "[data-m=abs]");
    let [cx, cy] = screen(doc, "[data-m=c]");
    [x - cx, y - cy]
}

fn close(got: [f32; 2], want: [f32; 2]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55)
}

#[test]
fn more_shapes_match_chrome() {
    let mut wrong = Vec::new();
    for row in ROWS {
        let mut doc = build(row.c, row.html);
        let got = rel(&doc);
        // A second layout (another viewport height, so it is not skipped)
        // leaves the box where it is.
        doc.resolve_layout(800.0, 640.0);
        let again = rel(&doc);
        if !close(got, row.want) || got != again {
            wrong.push(format!(
                "{}: got {got:?} (again {again:?}), Chrome 153 {:?}",
                row.name, row.want
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(ROWS.len() >= 100, "the table is here");
}

#[test]
fn scrolled_shapes() {
    let mut wrong = Vec::new();
    for row in SCROLLED {
        let mut doc = build(row.c, row.html);
        let before = rel(&doc);
        if !close(before, row.before) {
            wrong.push(format!(
                "{} before: {before:?}, Chrome {:?}",
                row.name, row.before
            ));
        }
        if row.name.contains("nested_scroll") {
            let (s1, s2) = (one(&doc, "[data-m=s1]"), one(&doc, "[data-m=s2]"));
            doc.set_scroll_top(NodeId(s1), 15.0);
            doc.set_scroll_top(NodeId(s2), 25.0);
        } else if row.name.contains("page_scroll") {
            let body = doc.body();
            let tall = doc.create_element("div");
            doc.set_attribute(tall, "style", "height: 2000px");
            doc.append_child(body, tall);
            doc.resolve_layout(800.0, 600.0);
            doc.set_scroll_top(body, 50.0);
        } else {
            let s1 = one(&doc, "[data-m=s1]");
            doc.set_scroll_top(NodeId(s1), 25.0);
        }
        let scrolled = rel(&doc);
        doc.resolve_layout(800.0, 680.0);
        let relaid = rel(&doc);
        // `rinch_after` is Chrome's answer except in the two sticky rows the
        // module doc names.
        if !close(scrolled, row.rinch_after) || scrolled != relaid {
            wrong.push(format!(
                "{} after the scroll: {scrolled:?}, after a relayout {relaid:?}, pinned {:?} (Chrome {:?})",
                row.name, row.rinch_after, row.after
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where rinch is not Chrome's. Each row pins rinch's answer, so a fix has
/// to move the row to `ROWS`.
#[test]
fn known_differences_found_by_the_review() {
    let mut wrong = Vec::new();
    for row in KNOWN {
        let doc = build(row.c, row.html);
        let got = rel(&doc);
        if !close(got, row.rinch) {
            wrong.push(format!(
                "{} ({}): got {got:?}, pinned {:?}, Chrome 153 {:?}",
                row.name, row.why, row.rinch, row.chrome
            ));
        }
        assert!(
            !close(row.rinch, row.chrome),
            "{} is a difference",
            row.name
        );
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A box shown later among the **second** run of lines of a container is
/// found by that run's lines, as a fresh layout finds it.
#[test]
fn a_box_shown_later_in_a_second_run_finds_its_place() {
    let html = |display: &str| {
        format!(
            r#"one<div style="height: 10px"></div>two<span data-m="abs" style="position: absolute; width: 40px; height: 30px; {display}"></span>three"#
        )
    };
    let fresh = rel(&build("", &html("")));
    assert!(close(fresh, [38.875, 37.0]), "Chrome 153: {fresh:?}");
    let mut doc = build("", &html("display: none;"));
    let id = one(&doc, "[data-m=abs]");
    doc.set_attribute(
        NodeId(id),
        "style",
        "position: absolute; width: 40px; height: 30px;",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(rel(&doc), fresh);
}

const ROWS: &[Row] = &[
    Row {
        name: "a_after_block_margin",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "a_after_block_text",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "a_before_block_then_run",
        c: r##""##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_before_block_then_run_center",
        c: r##"text-align:center;"##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_between_blocks",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style='height:10px;margin-top:30px'></div>"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "a_border",
        c: r##"border:5px solid black;"##,
        html: r##"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        want: [16.0, 32.0],
    },
    Row {
        name: "a_border_first",
        c: r##"border:5px solid black;"##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        want: [16.0, 12.0],
    },
    Row {
        name: "a_center_first_then_block",
        c: r##"text-align:center;"##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>text<div style='height:10px'></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_center_run_first",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "a_center_two_first",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div data-m="abs2" style="position: absolute; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "a_contents",
        c: r##""##,
        html: r##"text<div style='display:contents'><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_contents_span",
        c: r##""##,
        html: r##"text<span style='display:contents'>mid<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div></span>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_empty_then",
        c: r##""##,
        html: r##"<div></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_flex_between",
        c: r##"display:flex;justify-content:space-between;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_flex_col_center",
        c: r##"display:flex;flex-direction:column;justify-content:center;height:100px;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 38.5],
    },
    Row {
        name: "a_flex_gap",
        c: r##"display:flex;gap:9px;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_flex_rowrev",
        c: r##"display:flex;flex-direction:row-reverse;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [360.0, 7.0],
    },
    Row {
        name: "a_flex_wrap",
        c: r##"display:flex;flex-wrap:wrap;"##,
        html: r##"<div style='width:300px;height:10px'></div><div style='width:300px;height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_grid_placed",
        c: r##"display:grid;grid-template-columns:100px 100px;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;grid-column:2;grid-row:2;"></div>"##,
        want: [111.0, 17.0],
    },
    Row {
        name: "a_in_abs_in_rel",
        c: r##""##,
        html: r##"<div style='position:absolute;left:30px;top:40px;width:200px'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [30.0, 60.0],
    },
    Row {
        name: "a_in_rel_offset",
        c: r##""##,
        html: r##"<div style='position:relative;left:20px;top:10px'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [31.0, 37.0],
    },
    Row {
        name: "a_inner_border_pad",
        c: r##""##,
        html: r##"<div style='border:3px solid black;padding:4px 0 0 6px;margin-left:9px'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [29.0, 34.0],
    },
    Row {
        name: "a_letter",
        c: r##"letter-spacing:3px;"##,
        html: r##"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_li_mid",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li>item<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>more</li></ul>"##,
        want: [51.0, 27.0],
    },
    Row {
        name: "a_pct_width",
        c: r##""##,
        html: r##"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;width:50%;"></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_pre_newline",
        c: r##"white-space:pre;"##,
        html: r##"a
<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>b"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_prewrap_spaces",
        c: r##"white-space:pre-wrap;width:131px;"##,
        html: r##"alpha beta   <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>gamma delta"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_right_first_then_block",
        c: r##"text-align:right;"##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>text<div style='height:10px'></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_right_run_first",
        c: r##"text-align:right;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "a_second_run",
        c: r##""##,
        html: r##"one<div style='height:10px'></div>two<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>three"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "a_space_both",
        c: r##""##,
        html: r##"text <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div> tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_split1",
        c: r##""##,
        html: r##"<span>aa<div style='height:10px'></div>bb<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>cc</span>"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "a_split2",
        c: r##""##,
        html: r##"aa<span>bb<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style='height:10px'></div>cc</span>"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_wrap_long",
        c: r##"width:131px;"##,
        html: r##"alpha beta gamma delta epsilon <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>zeta eta theta"##,
        want: [11.0, 67.0],
    },
    Row {
        name: "ai_after_block_margin",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>text"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "ai_after_block_text",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [39.328, 37.0],
    },
    Row {
        name: "ai_before_block_then_run",
        c: r##""##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "ai_between_blocks",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><div style='height:10px;margin-top:30px'></div>"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "ai_border",
        c: r##"border:5px solid black;"##,
        html: r##"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        want: [44.328, 12.0],
    },
    Row {
        name: "ai_border_first",
        c: r##"border:5px solid black;"##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        want: [16.0, 12.0],
    },
    Row {
        name: "ai_contents",
        c: r##""##,
        html: r##"text<div style='display:contents'><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span></div>tail"##,
        want: [39.328, 7.0],
    },
    Row {
        name: "ai_contents_span",
        c: r##""##,
        html: r##"text<span style='display:contents'>mid<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span></span>tail"##,
        want: [67.016, 7.0],
    },
    Row {
        name: "ai_empty_then",
        c: r##""##,
        html: r##"<div></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "ai_flex_between",
        c: r##"display:flex;justify-content:space-between;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "ai_flex_col_center",
        c: r##"display:flex;flex-direction:column;justify-content:center;height:100px;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 38.5],
    },
    Row {
        name: "ai_flex_gap",
        c: r##"display:flex;gap:9px;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "ai_flex_rowrev",
        c: r##"display:flex;flex-direction:row-reverse;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [360.0, 7.0],
    },
    Row {
        name: "ai_flex_wrap",
        c: r##"display:flex;flex-wrap:wrap;"##,
        html: r##"<div style='width:300px;height:10px'></div><div style='width:300px;height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "ai_grid_placed",
        c: r##"display:grid;grid-template-columns:100px 100px;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;grid-column:2;grid-row:2;"></span>"##,
        want: [111.0, 17.0],
    },
    Row {
        name: "ai_ib_text_abs",
        c: r##""##,
        html: r##"<span style='display:inline-block;width:30px;height:20px'></span>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        want: [69.328, 7.0],
    },
    Row {
        name: "ai_in_abs_in_rel",
        c: r##""##,
        html: r##"<div style='position:absolute;left:30px;top:40px;width:200px'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [58.328, 40.0],
    },
    Row {
        name: "ai_in_rel_offset",
        c: r##""##,
        html: r##"<div style='position:relative;left:20px;top:10px'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [59.328, 17.0],
    },
    Row {
        name: "ai_inner_border_pad",
        c: r##""##,
        html: r##"<div style='border:3px solid black;padding:4px 0 0 6px;margin-left:9px'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [57.328, 14.0],
    },
    Row {
        name: "ai_letter",
        c: r##"letter-spacing:3px;"##,
        html: r##"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        want: [51.328, 7.0],
    },
    Row {
        name: "ai_pct_width",
        c: r##""##,
        html: r##"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;width:50%;"></span>tail"##,
        want: [39.328, 7.0],
    },
    Row {
        name: "ai_pre_newline",
        c: r##"white-space:pre;"##,
        html: r##"a
<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>b"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "ai_second_run",
        c: r##""##,
        html: r##"one<div style='height:10px'></div>two<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>three"##,
        want: [38.875, 37.0],
    },
    Row {
        name: "ai_space_both",
        c: r##""##,
        html: r##"text <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span> tail"##,
        want: [43.828, 7.0],
    },
    Row {
        name: "ai_split1",
        c: r##""##,
        html: r##"<span>aa<div style='height:10px'></div>bb<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>cc</span>"##,
        want: [30.594, 37.0],
    },
    Row {
        name: "ai_split2",
        c: r##""##,
        html: r##"aa<span>bb<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><div style='height:10px'></div>cc</span>"##,
        want: [48.563, 7.0],
    },
    Row {
        name: "ai_wrap_long",
        c: r##"width:131px;"##,
        html: r##"alpha beta gamma delta epsilon <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>zeta eta theta"##,
        want: [69.875, 47.0],
    },
    Row {
        name: "f_after_block_margin",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "f_after_block_text",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "f_before_block_then_run",
        c: r##""##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_before_block_then_run_center",
        c: r##"text-align:center;"##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_between_blocks",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style='height:10px;margin-top:30px'></div>"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "f_border",
        c: r##"border:5px solid black;"##,
        html: r##"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        want: [16.0, 32.0],
    },
    Row {
        name: "f_border_first",
        c: r##"border:5px solid black;"##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        want: [16.0, 12.0],
    },
    Row {
        name: "f_center_first_then_block",
        c: r##"text-align:center;"##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>text<div style='height:10px'></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_center_run_first",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "f_center_two_first",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div data-m="abs2" style="position: fixed; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "f_contents",
        c: r##""##,
        html: r##"text<div style='display:contents'><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_contents_span",
        c: r##""##,
        html: r##"text<span style='display:contents'>mid<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div></span>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_empty_then",
        c: r##""##,
        html: r##"<div></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_flex_between",
        c: r##"display:flex;justify-content:space-between;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_flex_col_center",
        c: r##"display:flex;flex-direction:column;justify-content:center;height:100px;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 38.5],
    },
    Row {
        name: "f_flex_gap",
        c: r##"display:flex;gap:9px;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_flex_rowrev",
        c: r##"display:flex;flex-direction:row-reverse;"##,
        html: r##"<div style='width:50px;height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [360.0, 7.0],
    },
    Row {
        name: "f_flex_wrap",
        c: r##"display:flex;flex-wrap:wrap;"##,
        html: r##"<div style='width:300px;height:10px'></div><div style='width:300px;height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_grid_placed",
        c: r##"display:grid;grid-template-columns:100px 100px;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;grid-column:2;grid-row:2;"></div>"##,
        want: [111.0, 17.0],
    },
    Row {
        name: "f_in_abs_in_rel",
        c: r##""##,
        html: r##"<div style='position:absolute;left:30px;top:40px;width:200px'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [30.0, 60.0],
    },
    Row {
        name: "f_in_rel_offset",
        c: r##""##,
        html: r##"<div style='position:relative;left:20px;top:10px'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [31.0, 37.0],
    },
    Row {
        name: "f_inner_border_pad",
        c: r##""##,
        html: r##"<div style='border:3px solid black;padding:4px 0 0 6px;margin-left:9px'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</div>"##,
        want: [29.0, 34.0],
    },
    Row {
        name: "f_letter",
        c: r##"letter-spacing:3px;"##,
        html: r##"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_li_mid",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li>item<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>more</li></ul>"##,
        want: [51.0, 27.0],
    },
    Row {
        name: "f_pct_width",
        c: r##""##,
        html: r##"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;width:50%;"></div>tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_pre_newline",
        c: r##"white-space:pre;"##,
        html: r##"a
<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>b"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_prewrap_spaces",
        c: r##"white-space:pre-wrap;width:131px;"##,
        html: r##"alpha beta   <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>gamma delta"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_right_first_then_block",
        c: r##"text-align:right;"##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>text<div style='height:10px'></div>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_right_run_first",
        c: r##"text-align:right;"##,
        html: r##"<div style='height:10px'></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>text"##,
        want: [11.0, 17.0],
    },
    Row {
        name: "f_second_run",
        c: r##""##,
        html: r##"one<div style='height:10px'></div>two<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>three"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "f_space_both",
        c: r##""##,
        html: r##"text <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div> tail"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_split1",
        c: r##""##,
        html: r##"<span>aa<div style='height:10px'></div>bb<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>cc</span>"##,
        want: [11.0, 57.0],
    },
    Row {
        name: "f_split2",
        c: r##""##,
        html: r##"aa<span>bb<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style='height:10px'></div>cc</span>"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_wrap_long",
        c: r##"width:131px;"##,
        html: r##"alpha beta gamma delta epsilon <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>zeta eta theta"##,
        want: [11.0, 67.0],
    },
    Row {
        name: "fi_after_block_margin",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>text"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "fi_after_block_text",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [39.328, 37.0],
    },
    Row {
        name: "fi_before_block_then_run",
        c: r##""##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><div style='height:10px'></div>text"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "fi_between_blocks",
        c: r##""##,
        html: r##"<div style='height:10px;margin-bottom:20px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><div style='height:10px;margin-top:30px'></div>"##,
        want: [11.0, 37.0],
    },
    Row {
        name: "fi_border",
        c: r##"border:5px solid black;"##,
        html: r##"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        want: [44.328, 12.0],
    },
    Row {
        name: "fi_border_first",
        c: r##"border:5px solid black;"##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        want: [16.0, 12.0],
    },
    Row {
        name: "fi_contents",
        c: r##""##,
        html: r##"text<div style='display:contents'><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span></div>tail"##,
        want: [39.328, 7.0],
    },
    Row {
        name: "fi_contents_span",
        c: r##""##,
        html: r##"text<span style='display:contents'>mid<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span></span>tail"##,
        want: [67.016, 7.0],
    },
    Row {
        name: "fi_empty_then",
        c: r##""##,
        html: r##"<div></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "fi_flex_between",
        c: r##"display:flex;justify-content:space-between;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "fi_flex_col_center",
        c: r##"display:flex;flex-direction:column;justify-content:center;height:100px;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 38.5],
    },
    Row {
        name: "fi_flex_gap",
        c: r##"display:flex;gap:9px;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "fi_flex_rowrev",
        c: r##"display:flex;flex-direction:row-reverse;"##,
        html: r##"<div style='width:50px;height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [360.0, 7.0],
    },
    Row {
        name: "fi_flex_wrap",
        c: r##"display:flex;flex-wrap:wrap;"##,
        html: r##"<div style='width:300px;height:10px'></div><div style='width:300px;height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"##,
        want: [11.0, 7.0],
    },
    Row {
        name: "fi_grid_placed",
        c: r##"display:grid;grid-template-columns:100px 100px;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;grid-column:2;grid-row:2;"></span>"##,
        want: [111.0, 17.0],
    },
    Row {
        name: "fi_ib_text_abs",
        c: r##""##,
        html: r##"<span style='display:inline-block;width:30px;height:20px'></span>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        want: [69.328, 7.0],
    },
    Row {
        name: "fi_in_abs_in_rel",
        c: r##""##,
        html: r##"<div style='position:absolute;left:30px;top:40px;width:200px'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [58.328, 40.0],
    },
    Row {
        name: "fi_in_rel_offset",
        c: r##""##,
        html: r##"<div style='position:relative;left:20px;top:10px'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [59.328, 17.0],
    },
    Row {
        name: "fi_inner_border_pad",
        c: r##""##,
        html: r##"<div style='border:3px solid black;padding:4px 0 0 6px;margin-left:9px'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</div>"##,
        want: [57.328, 14.0],
    },
    Row {
        name: "fi_letter",
        c: r##"letter-spacing:3px;"##,
        html: r##"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        want: [51.328, 7.0],
    },
    Row {
        name: "fi_pct_width",
        c: r##""##,
        html: r##"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;width:50%;"></span>tail"##,
        want: [39.328, 7.0],
    },
    Row {
        name: "fi_pre_newline",
        c: r##"white-space:pre;"##,
        html: r##"a
<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>b"##,
        want: [11.0, 27.0],
    },
    Row {
        name: "fi_second_run",
        c: r##""##,
        html: r##"one<div style='height:10px'></div>two<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>three"##,
        want: [38.875, 37.0],
    },
    Row {
        name: "fi_space_both",
        c: r##""##,
        html: r##"text <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span> tail"##,
        want: [43.828, 7.0],
    },
    Row {
        name: "fi_split1",
        c: r##""##,
        html: r##"<span>aa<div style='height:10px'></div>bb<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>cc</span>"##,
        want: [30.594, 37.0],
    },
    Row {
        name: "fi_split2",
        c: r##""##,
        html: r##"aa<span>bb<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><div style='height:10px'></div>cc</span>"##,
        want: [48.563, 7.0],
    },
    Row {
        name: "fi_wrap_long",
        c: r##"width:131px;"##,
        html: r##"alpha beta gamma delta epsilon <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>zeta eta theta"##,
        want: [69.875, 47.0],
    },
];

const KNOWN: &[Known] = &[
    Known {
        name: "a_li_first",
        why: "F3",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>item</li></ul>"##,
        chrome: [51.0, 7.0],
        rinch: [51.0, 27.0],
    },
    Known {
        name: "a_ol_first",
        why: "F3",
        c: r##""##,
        html: r##"<ol style='margin:0;padding-left:40px'><li><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>item</li></ol>"##,
        chrome: [51.0, 7.0],
        rinch: [51.0, 27.0],
    },
    Known {
        name: "a_rtl",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        chrome: [360.0, 27.0],
        rinch: [11.0, 27.0],
    },
    Known {
        name: "a_rtl_first",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        chrome: [360.0, 7.0],
        rinch: [11.0, 7.0],
    },
    Known {
        name: "ai_before_block_then_run_center",
        why: "F1, left: a box with a block between it and the text, where rinch builds no line",
        c: r##"text-align:center;"##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><div style='height:10px'></div>text"##,
        chrome: [205.5, 7.0],
        rinch: [11.0, 7.0],
    },
    Known {
        name: "ai_center_first_then_block",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:center;"##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>text<div style='height:10px'></div>"##,
        chrome: [191.328, 7.0],
        rinch: [192.0, 7.0],
    },
    Known {
        name: "ai_center_run_first",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>text"##,
        chrome: [191.328, 17.0],
        rinch: [192.0, 17.0],
    },
    Known {
        name: "ai_center_two_first",
        why: "F1, left: two boxes before the run's text; Chrome puts the first 14px right of the line's start",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><span data-m="abs2" style="position: absolute; width: 40px; height: 30px;"></span>text"##,
        chrome: [205.5, 17.0],
        rinch: [192.0, 17.0],
    },
    Known {
        name: "ai_li_first",
        why: "F3",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>item</li></ul>"##,
        chrome: [51.0, 7.0],
        rinch: [68.0, 7.0],
    },
    Known {
        name: "ai_li_mid",
        why: "F3 (marker is inline content, #1356)",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li>item<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>more</li></ul>"##,
        chrome: [83.297, 7.0],
        rinch: [100.0, 7.0],
    },
    Known {
        name: "ai_ol_first",
        why: "F3",
        c: r##""##,
        html: r##"<ol style='margin:0;padding-left:40px'><li><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>item</li></ol>"##,
        chrome: [51.0, 7.0],
        rinch: [70.0, 7.0],
    },
    Known {
        name: "ai_prewrap_spaces",
        why: "F4",
        c: r##"white-space:pre-wrap;width:131px;"##,
        html: r##"alpha beta   <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>gamma delta"##,
        chrome: [103.531, 7.0],
        rinch: [90.0, 7.0],
    },
    Known {
        name: "ai_right_first_then_block",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:right;"##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>text<div style='height:10px'></div>"##,
        chrome: [371.672, 7.0],
        rinch: [373.0, 7.0],
    },
    Known {
        name: "ai_right_run_first",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:right;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>text"##,
        chrome: [371.672, 17.0],
        rinch: [373.0, 17.0],
    },
    Known {
        name: "ai_rtl",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        chrome: [377.953, 7.0],
        rinch: [39.0, 7.0],
    },
    Known {
        name: "ai_rtl_first",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        chrome: [360.0, 7.0],
        rinch: [11.0, 7.0],
    },
    Known {
        name: "f_li_first",
        why: "F3",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>item</li></ul>"##,
        chrome: [51.0, 7.0],
        rinch: [51.0, 27.0],
    },
    Known {
        name: "f_ol_first",
        why: "F3",
        c: r##""##,
        html: r##"<ol style='margin:0;padding-left:40px'><li><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>item</li></ol>"##,
        chrome: [51.0, 7.0],
        rinch: [51.0, 27.0],
    },
    Known {
        name: "f_rtl",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        chrome: [360.0, 27.0],
        rinch: [11.0, 27.0],
    },
    Known {
        name: "f_rtl_first",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        chrome: [360.0, 7.0],
        rinch: [11.0, 7.0],
    },
    Known {
        name: "fi_before_block_then_run_center",
        why: "F1, left: a box with a block between it and the text, where rinch builds no line",
        c: r##"text-align:center;"##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><div style='height:10px'></div>text"##,
        chrome: [205.5, 7.0],
        rinch: [11.0, 7.0],
    },
    Known {
        name: "fi_center_first_then_block",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:center;"##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>text<div style='height:10px'></div>"##,
        chrome: [191.328, 7.0],
        rinch: [192.0, 7.0],
    },
    Known {
        name: "fi_center_run_first",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>text"##,
        chrome: [191.328, 17.0],
        rinch: [192.0, 17.0],
    },
    Known {
        name: "fi_center_two_first",
        why: "F1, left: two boxes before the run's text; Chrome puts the first 14px right of the line's start",
        c: r##"text-align:center;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><span data-m="abs2" style="position: fixed; width: 40px; height: 30px;"></span>text"##,
        chrome: [205.5, 17.0],
        rinch: [192.0, 17.0],
    },
    Known {
        name: "fi_li_first",
        why: "F3",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>item</li></ul>"##,
        chrome: [51.0, 7.0],
        rinch: [68.0, 7.0],
    },
    Known {
        name: "fi_li_mid",
        why: "F3 (marker is inline content, #1356)",
        c: r##""##,
        html: r##"<ul style='margin:0;padding-left:40px'><li>item<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>more</li></ul>"##,
        chrome: [83.297, 7.0],
        rinch: [100.0, 7.0],
    },
    Known {
        name: "fi_ol_first",
        why: "F3",
        c: r##""##,
        html: r##"<ol style='margin:0;padding-left:40px'><li><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>item</li></ol>"##,
        chrome: [51.0, 7.0],
        rinch: [70.0, 7.0],
    },
    Known {
        name: "fi_prewrap_spaces",
        why: "F4",
        c: r##"white-space:pre-wrap;width:131px;"##,
        html: r##"alpha beta   <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>gamma delta"##,
        chrome: [103.531, 7.0],
        rinch: [90.0, 7.0],
    },
    Known {
        name: "fi_right_first_then_block",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:right;"##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>text<div style='height:10px'></div>"##,
        chrome: [371.672, 7.0],
        rinch: [373.0, 7.0],
    },
    Known {
        name: "fi_right_run_first",
        why: "F1, fixed: the box starts the anonymous box's first line, which is ~1px from Chrome's",
        c: r##"text-align:right;"##,
        html: r##"<div style='height:10px'></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>text"##,
        chrome: [371.672, 17.0],
        rinch: [373.0, 17.0],
    },
    Known {
        name: "fi_rtl",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        chrome: [377.953, 7.0],
        rinch: [39.0, 7.0],
    },
    Known {
        name: "fi_rtl_first",
        why: "direction: rtl is not modelled",
        c: r##"direction: rtl;"##,
        html: r##"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        chrome: [360.0, 7.0],
        rinch: [11.0, 7.0],
    },
];

const SCROLLED: &[Scrolled] = &[
    Scrolled {
        name: "a_anc_abs_scrolled",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:absolute;left:30px;top:40px;width:200px'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</div><div style='height:300px'></div></div>"##,
        before: [30.0, 60.0],
        after: [30.0, 60.0],
        rinch_after: [30.0, 60.0],
    },
    Scrolled {
        name: "a_nested_scroll",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div data-m='s2' style='overflow:auto;height:80px'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail<div style='height:300px'></div></div><div style='height:300px'></div></div>"##,
        before: [11.0, 27.0],
        after: [11.0, 27.0],
        rinch_after: [11.0, 27.0],
    },
    Scrolled {
        name: "a_page_scroll_tall",
        c: r##""##,
        html: r##"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"##,
        before: [11.0, 27.0],
        after: [11.0, 27.0],
        rinch_after: [11.0, 27.0],
    },
    Scrolled {
        name: "a_sticky_parent",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:sticky;top:0'>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</div><div style='height:300px'></div></div>"##,
        before: [11.0, 27.0],
        after: [11.0, 27.0],
        rinch_after: [11.0, 2.0],
    },
    Scrolled {
        name: "ai_anc_abs_scrolled",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:absolute;left:30px;top:40px;width:200px'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</div><div style='height:300px'></div></div>"##,
        before: [58.328, 40.0],
        after: [58.328, 40.0],
        rinch_after: [58.0, 40.0],
    },
    Scrolled {
        name: "ai_nested_scroll",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div data-m='s2' style='overflow:auto;height:80px'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail<div style='height:300px'></div></div><div style='height:300px'></div></div>"##,
        before: [39.328, 7.0],
        after: [39.328, 7.0],
        rinch_after: [39.0, 7.0],
    },
    Scrolled {
        name: "ai_page_scroll_tall",
        c: r##""##,
        html: r##"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"##,
        before: [39.328, 7.0],
        after: [39.328, 7.0],
        rinch_after: [39.0, 7.0],
    },
    Scrolled {
        name: "ai_sticky_parent",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:sticky;top:0'>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</div><div style='height:300px'></div></div>"##,
        before: [39.328, 7.0],
        after: [39.328, 7.0],
        rinch_after: [39.0, -18.0],
    },
    Scrolled {
        name: "f_anc_abs_scrolled",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:absolute;left:30px;top:40px;width:200px'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</div><div style='height:300px'></div></div>"##,
        before: [30.0, 60.0],
        after: [30.0, 60.0],
        rinch_after: [30.0, 60.0],
    },
    Scrolled {
        name: "f_nested_scroll",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div data-m='s2' style='overflow:auto;height:80px'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail<div style='height:300px'></div></div><div style='height:300px'></div></div>"##,
        before: [11.0, 27.0],
        after: [11.0, 27.0],
        rinch_after: [11.0, 27.0],
    },
    Scrolled {
        name: "f_page_scroll_tall",
        c: r##""##,
        html: r##"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"##,
        before: [11.0, 27.0],
        after: [11.0, 77.0],
        rinch_after: [11.0, 77.0],
    },
    Scrolled {
        name: "f_sticky_parent",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:sticky;top:0'>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</div><div style='height:300px'></div></div>"##,
        before: [11.0, 27.0],
        after: [11.0, 27.0],
        rinch_after: [11.0, 27.0],
    },
    Scrolled {
        name: "fi_anc_abs_scrolled",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:absolute;left:30px;top:40px;width:200px'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</div><div style='height:300px'></div></div>"##,
        before: [58.328, 40.0],
        after: [58.328, 40.0],
        rinch_after: [58.0, 40.0],
    },
    Scrolled {
        name: "fi_nested_scroll",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div data-m='s2' style='overflow:auto;height:80px'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail<div style='height:300px'></div></div><div style='height:300px'></div></div>"##,
        before: [39.328, 7.0],
        after: [39.328, 7.0],
        rinch_after: [39.0, 7.0],
    },
    Scrolled {
        name: "fi_page_scroll_tall",
        c: r##""##,
        html: r##"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"##,
        before: [39.328, 7.0],
        after: [39.328, 57.0],
        rinch_after: [39.0, 57.0],
    },
    Scrolled {
        name: "fi_sticky_parent",
        c: r##""##,
        html: r##"<div data-m='s1' style='overflow:auto;height:60px'><div style='position:sticky;top:0'>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</div><div style='height:300px'></div></div>"##,
        before: [39.328, 7.0],
        after: [39.328, 7.0],
        rinch_after: [39.0, 7.0],
    },
];

// ── Two bugs older than the PR that its territory meets (both on main 66373ff1) ──

fn plain(html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `position: absolute` → `position: fixed` on a box with no insets changes
/// no Taffy value, so no layout runs and the box's parent-relative `layout`
/// is read as a viewport position until something else lays out: `(11, 27)`
/// where a fresh layout gives `(28, 40)`. `static_position_tests`'
/// `position_and_inset_changes_return_to_the_static_position` cannot see it:
/// its `relayout()` changes the viewport height, which forces the layout the
/// restyle did not ask for. The review's suggested F5 patch (the cascade sets
/// `layout_dirty` for a `position` change on an out-of-flow box) passes this.
#[test]
fn absolute_to_fixed_with_no_insets_owes_a_layout() {
    let html = |position: &str| {
        format!(
            r#"<div data-m="c" style="width: 300px; margin: 13px 0 0 17px; padding: 7px 0 0 11px; font: 16px/20px ProbeFace">text<div data-m="abs" style="width: 40px; height: 30px; position: {position};"></div>tail</div>"#
        )
    };
    let fresh = screen(&plain(&html("fixed")), "[data-m=abs]");
    assert_eq!(fresh, [28.0, 40.0], "the static position, in the viewport");
    let mut doc = plain(&html("absolute"));
    let id = one(&doc, "[data-m=abs]");
    doc.set_attribute(
        NodeId(id),
        "style",
        "width: 40px; height: 30px; position: fixed;",
    );
    // The same viewport: only what the restyle owes runs.
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(screen(&doc, "[data-m=abs]"), fresh);
}

/// A `text-align` written on a block container does not reach the lines of
/// its **anonymous block boxes** (text beside a block child): an
/// `inline-block` there stays at the left edge, and another layout does not
/// move it (fresh: 271). A static position found in those lines is stale the
/// same way. Root inline formatting contexts are fine.
#[test]
#[ignore = "review of #1494: older than the PR; see issue-draft-review-1494-1.md"]
fn a_text_align_restyle_reaches_an_anonymous_box() {
    let html = |align: &str| {
        format!(
            r#"<div data-m="c" style="width: 300px; font: 16px/20px ProbeFace; text-align: {align}"><div style="height: 10px"></div><span data-m="abs" style="display: inline-block; width: 30px; height: 20px"></span></div>"#
        )
    };
    let fresh = screen(&plain(&html("right")), "[data-m=abs]");
    assert!(fresh[0] > 260.0, "right-aligned: {fresh:?}");
    let mut doc = plain(&html("left"));
    let c = one(&doc, "[data-m=c]");
    doc.set_style(NodeId(c), "text-align", "right");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(screen(&doc, "[data-m=abs]"), fresh);
}
