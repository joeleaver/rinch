//! The static position of an out-of-flow box: where it would have been in
//! the flow (CSS 2.1 §10.3.7, §10.6.4) — issues #633, #632 and #634.
//!
//! - **#633** — `position: fixed` with both insets of an axis `auto` keeps
//!   its static position on that axis. rinch put it at the viewport origin.
//! - **#632** — a block-level out-of-flow box among inline content goes below
//!   the line holding the content before it, or at that line's top when
//!   nothing precedes it there. rinch put it below the whole run of lines.
//! - **#634** — an out-of-flow box whose `display` was inline-level before
//!   `position` blockified it sits **in** its line: at the x the content
//!   before it ends at, at the line's top. rinch gave it a block's place.
//!
//! Every number is Chrome 153's (`--headless=new`, standards mode, `* {
//! box-sizing: border-box }`, 800x600, device scale factor 1),
//! `getBoundingClientRect` relative to the container's border box, with this
//! crate's bundled Inter loaded through `@font-face` under the name the
//! fixtures register it with. The scaffold is `abs_inline_containing_block_tests`':
//!
//! ```html
//! <div C="position: relative; width: 400px; font: 16px/20px ProbeFace;
//!         margin: 13px 0 0 17px; padding: 7px 0 0 11px">
//! ```
//!
//! so the content box starts at `(11, 7)` and no expectation sits on the
//! fixed point where the container's corner, its content corner and the
//! viewport's coincide. Line boxes are the declared 20px.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
/// The same box with no `position`: an absolute box inside resolves against
/// the initial containing block.
const C_STATIC: &str = "width: 400px; font: 16px/20px ProbeFace; \
                        margin: 13px 0 0 17px; padding: 7px 0 0 11px;";

struct Case {
    doc: RinchDocument,
    c: usize,
    relayouts: u32,
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
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

impl Case {
    /// `inner` in the container `base` + `c_extra`, with `post` after it.
    fn build(base: &str, c_extra: &str, inner: &str, post: &str) -> Self {
        let mut doc = document();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(r#"<div data-m="c" style="{base}{c_extra}">{inner}</div>{post}"#),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let c = one(&doc, "[data-m=c]");
        Case {
            doc,
            c,
            relayouts: 0,
        }
    }

    fn new(inner: &str) -> Self {
        Self::build(C, "", inner, "")
    }

    fn id(&self, m: &str) -> usize {
        one(&self.doc, &format!("[data-m={m}]"))
    }

    /// `m`'s on-screen position.
    fn screen(&self, m: &str) -> [f32; 2] {
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, self.id(m), 1.0);
        [x as f32, y as f32]
    }

    /// `m`'s on-screen position relative to the container's border box.
    fn rel(&self, m: &str) -> [f32; 2] {
        let [x, y] = self.screen(m);
        let [cx, cy] = self.screen("c");
        [x - cx, y - cy]
    }

    /// Lay out again at a viewport height no earlier pass used, so the pass
    /// is not skipped while no width in the fixture moves.
    fn relayout(&mut self) {
        self.relayouts += 1;
        self.doc
            .resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0 * self.relayouts as f32);
    }

    fn set_style(&mut self, m: &str, style: &str) {
        let id = self.id(m);
        self.doc.set_attribute(NodeId(id), "style", style);
    }

    fn scroll(&mut self, m: &str, top: f32) {
        let id = self.id(m);
        self.doc.set_scroll_top(NodeId(id), f64::from(top));
        assert_eq!(
            self.doc.tree.get(id).unwrap().scroll_offset.1,
            f64::from(top),
            "precondition: {m} scrolled"
        );
    }

    fn frame(&mut self) -> rinch_dom::perf::FrameStats {
        self.doc.tree.perf.end_frame();
        self.relayout();
        self.doc.tree.perf.frame()
    }
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

const BOX: &str = "width: 40px; height: 30px;";

fn boxed(position: &str, tag: &str, style: &str, inner: &str) -> String {
    format!(r#"<{tag} data-m="abs" style="position: {position}; {BOX}{style}">{inner}</{tag}>"#)
}

/// Chrome's position is fractional wherever text decides it (`text` is
/// 28.328px). rinch puts every box on the pixel grid, so a position is
/// Chrome's to the nearest pixel.
fn close(got: [f32; 2], want: [f32; 2]) -> bool {
    got.iter().all(|g| g.fract() == 0.0) && got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55)
}

#[track_caller]
fn assert_at(got: [f32; 2], want: [f32; 2], what: &str) {
    assert!(
        close(got, want),
        "{what}: got {got:?}, Chrome 153 gives {want:?}"
    );
}

/// One measured shape: `html` in the container (`C`, or `C_STATIC`) with `c`
/// appended to its style, and where Chrome 153 puts `[data-m=abs]` relative
/// to the container's border box.
struct Row {
    name: &'static str,
    static_c: bool,
    c: &'static str,
    html: &'static str,
    want: [f32; 2],
}

/// The table, as measured (`f_` fixed, `a_` absolute; `_il_` an inline-level
/// box, a `<span style="position: ...">`).
const ROWS: &[Row] = &[
    Row {
        name: "f_mid",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_first",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_last",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_ws_first",
        static_c: false,
        c: "",
        html: r#" <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>beta gamma delta epsilon zeta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_3line_mid",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>delta epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_3line_mid_nospace",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gam<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>ma delta epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_br",
        static_c: false,
        c: "",
        html: r#"text<br><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_br_after",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><br>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;margin: 3px 0 0 5px;"></div>tail"#,
        want: [16.0, 30.0],
    },
    Row {
        name: "f_block_ctx",
        static_c: false,
        c: "",
        html: r#"<div style="height:25px"></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style="height:10px"></div>"#,
        want: [11.0, 32.0],
    },
    Row {
        name: "f_mixed",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail<div style="height:25px"></div>more"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_mixed_after",
        static_c: false,
        c: "",
        html: r#"text<div style="height:25px"></div>more<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 72.0],
    },
    Row {
        name: "f_in_span",
        static_c: false,
        c: "",
        html: r#"lead <span>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_in_span_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<span>alpha beta gamma <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>delta</span> epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_center",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_static_c",
        static_c: true,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_flex",
        static_c: false,
        c: "display:flex;",
        html: r#"<div style="width:50px;height:20px"></div><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style="width:50px;height:20px"></div>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_il_mid",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_first",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_il_last",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_space",
        static_c: false,
        c: "",
        html: r#"text <span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span> tail"#,
        want: [43.828, 7.0],
    },
    Row {
        name: "f_il_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma <span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>delta epsilon zeta"#,
        want: [71.313, 27.0],
    },
    Row {
        name: "f_il_3line_nospace",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gam<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>ma delta epsilon zeta"#,
        want: [43.813, 27.0],
    },
    Row {
        name: "f_il_3line_end",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span> delta epsilon zeta"#,
        want: [66.813, 27.0],
    },
    Row {
        name: "f_il_br",
        static_c: false,
        c: "",
        html: r#"text<br><span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_il_br_before",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span><br>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_center",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [208.641, 7.0],
    },
    Row {
        name: "f_il_right",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [377.953, 7.0],
    },
    Row {
        name: "f_il_ib",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;display:inline-block;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_iflex",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;display:inline-flex;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_in_span",
        static_c: false,
        c: "",
        html: r#"lead <span>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail</span>"#,
        want: [75.969, 7.0],
    },
    Row {
        name: "f_il_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;margin: 3px 0 0 5px;">box</span>tail"#,
        want: [44.328, 10.0],
    },
    Row {
        name: "f_il_block_ctx",
        static_c: false,
        c: "",
        html: r#"<div style="height:25px"></div><span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span><div style="height:10px"></div>"#,
        want: [11.0, 32.0],
    },
    Row {
        name: "f_il_mixed",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail<div style="height:25px"></div>more"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_tall_line",
        static_c: false,
        c: "",
        html: r#"text<span style="display:inline-block;width:10px;height:50px"></span><span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [49.328, 7.0],
    },
    Row {
        name: "f_il_bigfont",
        static_c: false,
        c: "",
        html: r#"te<span style="font-size:32px">xt</span><span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [53.969, 7.0],
    },
    Row {
        name: "f_il_after_ib",
        static_c: false,
        c: "",
        html: r#"<span style="display:inline-block;width:50px;height:26px"></span><span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [61.0, 7.0],
    },
    Row {
        name: "f_il_lineheight",
        static_c: false,
        c: "line-height:36px;",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_mid",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_first",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_last",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_ws_first",
        static_c: false,
        c: "",
        html: r#" <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>beta gamma delta epsilon zeta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_3line_mid",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>delta epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_3line_mid_nospace",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gam<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>ma delta epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_br",
        static_c: false,
        c: "",
        html: r#"text<br><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_br_after",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><br>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_top_only",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;top: 5px;"></div>tail"#,
        want: [11.0, 5.0],
    },
    Row {
        name: "a_left_only",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;left: 5px;"></div>tail"#,
        want: [5.0, 27.0],
    },
    Row {
        name: "a_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;margin: 3px 0 0 5px;"></div>tail"#,
        want: [16.0, 30.0],
    },
    Row {
        name: "a_block_ctx",
        static_c: false,
        c: "",
        html: r#"<div style="height:25px"></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style="height:10px"></div>"#,
        want: [11.0, 32.0],
    },
    Row {
        name: "a_mixed",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail<div style="height:25px"></div>more"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_mixed_after",
        static_c: false,
        c: "",
        html: r#"text<div style="height:25px"></div>more<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 72.0],
    },
    Row {
        name: "a_in_span",
        static_c: false,
        c: "",
        html: r#"lead <span>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_in_span_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<span>alpha beta gamma <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>delta</span> epsilon zeta"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_center",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_static_c",
        static_c: true,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_flex",
        static_c: false,
        c: "display:flex;",
        html: r#"<div style="width:50px;height:20px"></div><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style="width:50px;height:20px"></div>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_il_mid",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_first",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_il_last",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_space",
        static_c: false,
        c: "",
        html: r#"text <span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span> tail"#,
        want: [43.828, 7.0],
    },
    Row {
        name: "a_il_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma <span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>delta epsilon zeta"#,
        want: [71.313, 27.0],
    },
    Row {
        name: "a_il_3line_nospace",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gam<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>ma delta epsilon zeta"#,
        want: [43.813, 27.0],
    },
    Row {
        name: "a_il_3line_end",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta gamma<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span> delta epsilon zeta"#,
        want: [66.813, 27.0],
    },
    Row {
        name: "a_il_br",
        static_c: false,
        c: "",
        html: r#"text<br><span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_il_br_before",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span><br>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_center",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [208.641, 7.0],
    },
    Row {
        name: "a_il_right",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [377.953, 7.0],
    },
    Row {
        name: "a_il_ib",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;display:inline-block;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_iflex",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;display:inline-flex;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_in_span",
        static_c: false,
        c: "",
        html: r#"lead <span>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail</span>"#,
        want: [75.969, 7.0],
    },
    Row {
        name: "a_il_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;margin: 3px 0 0 5px;">box</span>tail"#,
        want: [44.328, 10.0],
    },
    Row {
        name: "a_il_top_only",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;top: 5px;">box</span>tail"#,
        want: [39.328, 5.0],
    },
    Row {
        name: "a_il_left_only",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;left: 5px;">box</span>tail"#,
        want: [5.0, 7.0],
    },
    Row {
        name: "a_il_block_ctx",
        static_c: false,
        c: "",
        html: r#"<div style="height:25px"></div><span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span><div style="height:10px"></div>"#,
        want: [11.0, 32.0],
    },
    Row {
        name: "a_il_mixed",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail<div style="height:25px"></div>more"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_tall_line",
        static_c: false,
        c: "",
        html: r#"text<span style="display:inline-block;width:10px;height:50px"></span><span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [49.328, 7.0],
    },
    Row {
        name: "a_il_bigfont",
        static_c: false,
        c: "",
        html: r#"te<span style="font-size:32px">xt</span><span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [53.969, 7.0],
    },
    Row {
        name: "a_il_after_ib",
        static_c: false,
        c: "",
        html: r#"<span style="display:inline-block;width:50px;height:26px"></span><span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [61.0, 7.0],
    },
    Row {
        name: "a_il_lineheight",
        static_c: false,
        c: "line-height:36px;",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;">box</span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_wrap_after_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>gamma delta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_wrap_before_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div> gamma delta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_before_ib",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_br_last",
        static_c: false,
        c: "",
        html: r#"text<br><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_br_br",
        static_c: false,
        c: "",
        html: r#"text<br><br><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_only",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_two",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div data-m="abs2" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_center_first",
        static_c: false,
        c: "text-align:center;",
        html: r#"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_center_br",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<br><div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_right_last",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_pct_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;margin-left:10%;margin-top:5%;"></div>tail"#,
        want: [91.0, 67.0],
    },
    Row {
        name: "f_neg_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;margin-left:-6px;margin-top:-4px;"></div>tail"#,
        want: [5.0, 23.0],
    },
    Row {
        name: "f_in_rel_span",
        static_c: false,
        c: "",
        html: r#"lead <span style="position:relative">text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail</span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_nowrap_overflow",
        static_c: false,
        c: "width:131px;",
        html: r#"alphabetagammadeltaepsilon<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>zeta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_trailing_space_last",
        static_c: false,
        c: "",
        html: r#"text <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_il_wrap_after_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>gamma delta"#,
        want: [90.031, 7.0],
    },
    Row {
        name: "f_il_wrap_before_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span> gamma delta"#,
        want: [90.031, 7.0],
    },
    Row {
        name: "f_il_wrap_ib_next",
        static_c: false,
        c: "width:131px;",
        html: r#"<span style="display:inline-block;width:50px;height:20px"></span><span style="display:inline-block;width:50px;height:20px"></span><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>"#,
        want: [111.0, 7.0],
    },
    Row {
        name: "f_il_before_ib",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_il_text_before_ib",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_after_ib_mid",
        static_c: false,
        c: "",
        html: r#"text<span style="display:inline-block;width:50px;height:20px"></span><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [89.328, 7.0],
    },
    Row {
        name: "f_il_br_last",
        static_c: false,
        c: "",
        html: r#"text<br><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_il_br_br",
        static_c: false,
        c: "",
        html: r#"text<br><br><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_il_only",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_il_two",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><span data-m="abs2" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_center_first",
        static_c: false,
        c: "text-align:center;",
        html: r#"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [194.469, 7.0],
    },
    Row {
        name: "f_il_center_br",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<br><span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [194.469, 27.0],
    },
    Row {
        name: "f_il_right_last",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"#,
        want: [400.0, 7.0],
    },
    Row {
        name: "f_il_pct_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;margin-left:10%;margin-top:5%;"></span>tail"#,
        want: [119.328, 47.0],
    },
    Row {
        name: "f_il_neg_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;margin-left:-6px;margin-top:-4px;"></span>tail"#,
        want: [33.328, 3.0],
    },
    Row {
        name: "f_il_in_rel_span",
        static_c: false,
        c: "",
        html: r#"lead <span style="position:relative">text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail</span>"#,
        want: [75.969, 7.0],
    },
    Row {
        name: "f_il_nowrap_overflow",
        static_c: false,
        c: "width:131px;",
        html: r#"alphabetagammadeltaepsilon<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>zeta"#,
        want: [233.234, 7.0],
    },
    Row {
        name: "f_il_trailing_space_last",
        static_c: false,
        c: "",
        html: r#"text <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_wrap_after_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>gamma delta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_wrap_before_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div> gamma delta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_before_ib",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_br_last",
        static_c: false,
        c: "",
        html: r#"text<br><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_br_br",
        static_c: false,
        c: "",
        html: r#"text<br><br><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_only",
        static_c: false,
        c: "",
        html: r#"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_two",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div data-m="abs2" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_center_first",
        static_c: false,
        c: "text-align:center;",
        html: r#"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_center_br",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<br><div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_right_last",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_pct_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;margin-left:10%;margin-top:5%;"></div>tail"#,
        want: [51.0, 47.0],
    },
    Row {
        name: "a_neg_margin",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;margin-left:-6px;margin-top:-4px;"></div>tail"#,
        want: [5.0, 23.0],
    },
    Row {
        name: "a_in_rel_span",
        static_c: false,
        c: "",
        html: r#"lead <span style="position:relative">text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail</span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_nowrap_overflow",
        static_c: false,
        c: "width:131px;",
        html: r#"alphabetagammadeltaepsilon<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>zeta"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_trailing_space_last",
        static_c: false,
        c: "",
        html: r#"text <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_il_wrap_after_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>gamma delta"#,
        want: [90.031, 7.0],
    },
    Row {
        name: "a_il_wrap_before_space",
        static_c: false,
        c: "width:131px;",
        html: r#"alpha beta<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span> gamma delta"#,
        want: [90.031, 7.0],
    },
    Row {
        name: "a_il_wrap_ib_next",
        static_c: false,
        c: "width:131px;",
        html: r#"<span style="display:inline-block;width:50px;height:20px"></span><span style="display:inline-block;width:50px;height:20px"></span><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>"#,
        want: [111.0, 7.0],
    },
    Row {
        name: "a_il_before_ib",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_il_text_before_ib",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><span style="display:inline-block;width:50px;height:20px"></span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_after_ib_mid",
        static_c: false,
        c: "",
        html: r#"text<span style="display:inline-block;width:50px;height:20px"></span><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [89.328, 7.0],
    },
    Row {
        name: "a_il_br_last",
        static_c: false,
        c: "",
        html: r#"text<br><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_il_br_br",
        static_c: false,
        c: "",
        html: r#"text<br><br><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_il_only",
        static_c: false,
        c: "",
        html: r#"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_il_two",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><span data-m="abs2" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_center_first",
        static_c: false,
        c: "text-align:center;",
        html: r#"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [194.469, 7.0],
    },
    Row {
        name: "a_il_center_br",
        static_c: false,
        c: "text-align:center;",
        html: r#"text<br><span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [194.469, 27.0],
    },
    Row {
        name: "a_il_right_last",
        static_c: false,
        c: "text-align:right;",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"#,
        want: [400.0, 7.0],
    },
    Row {
        name: "a_il_pct_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;margin-left:10%;margin-top:5%;"></span>tail"#,
        want: [79.328, 27.0],
    },
    Row {
        name: "a_il_neg_margin",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;margin-left:-6px;margin-top:-4px;"></span>tail"#,
        want: [33.328, 3.0],
    },
    Row {
        name: "a_il_in_rel_span",
        static_c: false,
        c: "",
        html: r#"lead <span style="position:relative">text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail</span>"#,
        want: [75.969, 7.0],
    },
    Row {
        name: "a_il_nowrap_overflow",
        static_c: false,
        c: "width:131px;",
        html: r#"alphabetagammadeltaepsilon<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>zeta"#,
        want: [233.234, 7.0],
    },
    Row {
        name: "a_il_trailing_space_last",
        static_c: false,
        c: "",
        html: r#"text <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_space_br",
        static_c: false,
        c: "",
        html: r#"alpha <br>beta<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "f_run_end",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div><div style="height:25px"></div>more"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_ellipsis_mid",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha beta gamma delta<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>epsilon"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_ellipsis_first",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>alpha beta gamma delta epsilon"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_ellipsis_short",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div> beta gamma delta epsilon"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "f_below_block",
        static_c: false,
        c: "",
        html: r#"<div style="height:33px"></div>text<div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 60.0],
    },
    Row {
        name: "f_run_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<div style="height:25px"></div>alpha beta gamma <div data-m="abs" style="position: fixed; width: 40px; height: 30px;"></div>delta epsilon zeta<div style="height:10px"></div>"#,
        want: [11.0, 72.0],
    },
    Row {
        name: "f_il_space_br",
        static_c: false,
        c: "",
        html: r#"alpha <br>beta<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [44.422, 27.0],
    },
    Row {
        name: "f_il_run_end",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span><div style="height:25px"></div>more"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "f_il_ellipsis_mid",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha beta gamma delta<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>epsilon"#,
        want: [192.375, 7.0],
    },
    Row {
        name: "f_il_ellipsis_first",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>alpha beta gamma delta epsilon"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "f_il_ellipsis_short",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span> beta gamma delta epsilon"#,
        want: [52.109, 7.0],
    },
    Row {
        name: "f_il_below_block",
        static_c: false,
        c: "",
        html: r#"<div style="height:33px"></div>text<span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>tail"#,
        want: [39.328, 40.0],
    },
    Row {
        name: "f_il_run_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<div style="height:25px"></div>alpha beta gamma <span data-m="abs" style="position: fixed; width: 40px; height: 30px;"></span>delta epsilon zeta<div style="height:10px"></div>"#,
        want: [71.313, 52.0],
    },
    Row {
        name: "a_space_br",
        static_c: false,
        c: "",
        html: r#"alpha <br>beta<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 47.0],
    },
    Row {
        name: "a_run_end",
        static_c: false,
        c: "",
        html: r#"text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div><div style="height:25px"></div>more"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_ellipsis_mid",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha beta gamma delta<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>epsilon"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_ellipsis_first",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>alpha beta gamma delta epsilon"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_ellipsis_short",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div> beta gamma delta epsilon"#,
        want: [11.0, 27.0],
    },
    Row {
        name: "a_below_block",
        static_c: false,
        c: "",
        html: r#"<div style="height:33px"></div>text<div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>tail"#,
        want: [11.0, 60.0],
    },
    Row {
        name: "a_run_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<div style="height:25px"></div>alpha beta gamma <div data-m="abs" style="position: absolute; width: 40px; height: 30px;"></div>delta epsilon zeta<div style="height:10px"></div>"#,
        want: [11.0, 72.0],
    },
    Row {
        name: "a_il_space_br",
        static_c: false,
        c: "",
        html: r#"alpha <br>beta<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [44.422, 27.0],
    },
    Row {
        name: "a_il_run_end",
        static_c: false,
        c: "",
        html: r#"text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span><div style="height:25px"></div>more"#,
        want: [39.328, 7.0],
    },
    Row {
        name: "a_il_ellipsis_mid",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha beta gamma delta<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>epsilon"#,
        want: [192.375, 7.0],
    },
    Row {
        name: "a_il_ellipsis_first",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>alpha beta gamma delta epsilon"#,
        want: [11.0, 7.0],
    },
    Row {
        name: "a_il_ellipsis_short",
        static_c: false,
        c: "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        html: r#"alpha<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span> beta gamma delta epsilon"#,
        want: [52.109, 7.0],
    },
    Row {
        name: "a_il_below_block",
        static_c: false,
        c: "",
        html: r#"<div style="height:33px"></div>text<span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>tail"#,
        want: [39.328, 40.0],
    },
    Row {
        name: "a_il_run_3line",
        static_c: false,
        c: "width:131px;",
        html: r#"<div style="height:25px"></div>alpha beta gamma <span data-m="abs" style="position: absolute; width: 40px; height: 30px;"></span>delta epsilon zeta<div style="height:10px"></div>"#,
        want: [71.313, 52.0],
    },
];

/// Every row whose name `pick` accepts, laid out and compared.
#[track_caller]
fn check(pick: impl Fn(&str) -> bool) {
    let mut ran = 0;
    let mut wrong = Vec::new();
    for row in ROWS.iter().filter(|r| pick(r.name)) {
        ran += 1;
        let base = if row.static_c { C_STATIC } else { C };
        let case = Case::build(base, row.c, row.html, "");
        let got = case.rel("abs");
        if !close(got, row.want) {
            wrong.push(format!(
                "  {}: got {:?}, Chrome 153 gives {:?}",
                row.name, got, row.want
            ));
        }
    }
    assert!(ran > 0, "the filter names no row");
    assert!(
        wrong.is_empty(),
        "{} of {ran} rows:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

fn inline_level(name: &str) -> bool {
    name.contains("_il_")
}

// ── The measured table ──────────────────────────────────────────────────────

/// #633 (and, for the rows among lines, #632): a fixed block-level box.
/// Before: every one of them at the viewport's origin, `(-17, -13)` here.
#[test]
fn fixed_block_level_rows() {
    check(|n| n.starts_with("f_") && !inline_level(n));
}

/// #632: an absolute block-level box. Before: below the whole run of lines
/// (`a_3line` at `(11, 67)`), and below the first line when it came first
/// (`a_first` at `(11, 27)`).
#[test]
fn absolute_block_level_rows() {
    check(|n| n.starts_with("a_") && !inline_level(n));
}

/// #634 on a fixed box.
#[test]
fn fixed_inline_level_rows() {
    check(|n| n.starts_with("f_") && inline_level(n));
}

/// #634: a `<span style="position: absolute">` sits in its line. Before: a
/// block's place, `(11, 27)` for `a_il_mid`.
#[test]
fn absolute_inline_level_rows() {
    check(|n| n.starts_with("a_") && inline_level(n));
}

// ── One inset: the other axis is static ─────────────────────────────────────

/// `top: 5px` alone: 5px down the **viewport**, at the static x. `left: 5px`
/// alone: the reverse. (Compared on screen for the inset axis: the page's
/// own margin is not the fixture's business.)
#[test]
fn a_fixed_box_with_one_inset_keeps_the_other_axis_static() {
    for (tag, x) in [("div", 11.0), ("span", 39.328)] {
        let c = Case::new(&format!("text{}tail", boxed("fixed", tag, "top: 5px;", "")));
        assert!(
            close([c.rel("abs")[0], c.screen("abs")[1]], [x, 5.0]),
            "{tag} top: {:?} {:?}",
            c.rel("abs"),
            c.screen("abs")
        );
        let y = if tag == "div" { 27.0 } else { 7.0 };
        let c = Case::new(&format!(
            "text{}tail",
            boxed("fixed", tag, "left: 5px;", "")
        ));
        assert!(
            close([c.screen("abs")[0], c.rel("abs")[1]], [5.0, y]),
            "{tag} left: {:?} {:?}",
            c.screen("abs"),
            c.rel("abs")
        );
    }
}

// ── Inside other boxes ──────────────────────────────────────────────────────

/// A box among the lines of an `inline-block`: its place in **those** lines.
/// Chrome puts the inline-block at `(47.641, 7)` and the box at `(52.641,
/// 30)` — `(5, 23)` inside it, past its padding and below its one line; an
/// inline-level one at `(33.328, 3)` inside it. (Compared with the
/// inline-block's own corner: rinch stands that 2px higher on the line.)
#[test]
fn a_box_in_an_inline_block_takes_its_place_in_that_blocks_lines() {
    for position in ["absolute", "fixed"] {
        for (tag, want) in [("div", [5.0, 23.0]), ("span", [33.328, 3.0])] {
            let c = Case::new(&format!(
                r#"lead <span data-m="ib" style="display: inline-block; width: 100px; padding: 3px 0 0 5px">text{}tail</span>"#,
                boxed(position, tag, "", "")
            ));
            let (b, ib) = (c.screen("abs"), c.screen("ib"));
            let got = [b[0] - ib[0], b[1] - ib[1]];
            assert!(
                got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55),
                "{position} {tag}: got {got:?}, Chrome 153 gives {want:?}"
            );
        }
    }
}

/// A fixed box inside a fixed box that has insets: the outer one's `layout`
/// is already a viewport position, so the sum of the boxes above stops
/// there. Chrome: `(6, 24)` inside the outer box (its padding, one line).
#[test]
fn a_static_fixed_box_inside_a_fixed_box_is_measured_from_it() {
    for position in ["absolute", "fixed"] {
        for (tag, want) in [("div", [6.0, 24.0]), ("span", [34.328, 4.0])] {
            let c = Case::new(&format!(
                r#"<div data-m="fp" style="position: fixed; top: 50px; left: 60px; width: 200px; height: 100px; padding: 4px 0 0 6px">text{}tail</div>"#,
                boxed(position, tag, "", "")
            ));
            assert_at(c.screen("fp"), [60.0, 50.0], "the outer box");
            let (b, fp) = (c.screen("abs"), c.screen("fp"));
            assert_at(
                [b[0] - fp[0], b[1] - fp[1]],
                want,
                &format!("{position} {tag}"),
            );
        }
    }
}

/// In a grid container the static position is the padding box's corner, for
/// both kinds: Chrome's `(0, 0)`, and Taffy's.
#[test]
fn a_grid_containers_static_position_is_its_padding_corner() {
    let item = r#"<div style="width: 50px; height: 20px"></div>"#;
    for position in ["absolute", "fixed"] {
        let c = Case::build(
            C,
            "display: grid; grid-template-columns: 100px 100px;",
            &format!("{item}{}{item}", boxed(position, "div", "", "")),
            "",
        );
        assert_at(c.rel("abs"), [0.0, 0.0], position);
    }
}

// ── Scrolling ───────────────────────────────────────────────────────────────

const SCROLLER: &str = r#"<div data-m="sc" style="overflow: auto; height: 60px;">"#;
const TALL: &str = r#"<div style="height: 300px"></div>"#;

/// A scroller between the box and its containing block does not carry it:
/// after `scrollTop = 40` Chrome has both a fixed and an absolute box where
/// they were, `(11, 27)`. And a layout run while scrolled puts them there
/// again.
#[test]
fn a_scroller_between_the_box_and_its_containing_block_does_not_carry_it() {
    for base in [C, C_STATIC] {
        for position in ["absolute", "fixed"] {
            let mut c = Case::build(
                base,
                "",
                &format!(
                    "{SCROLLER}text{}tail{TALL}</div>",
                    boxed(position, "div", "", "")
                ),
                "",
            );
            let what = format!(
                "{position} in {}",
                if base == C { "C" } else { "a static C" }
            );
            assert_at(c.rel("abs"), [11.0, 27.0], &what);
            c.scroll("sc", 40.0);
            assert_at(c.rel("abs"), [11.0, 27.0], &format!("{what}, scrolled"));
            c.relayout();
            assert_at(
                c.rel("abs"),
                [11.0, 27.0],
                &format!("{what}, scrolled and laid out"),
            );
        }
    }
}

/// The containing block's **own** scroll carries an absolute box (Chrome:
/// `(11, -13)` at `scrollTop = 40`) and not a fixed one, whose containing
/// block is the viewport (`(11, 27)` still).
#[test]
fn the_containers_own_scroll_carries_an_absolute_box_and_not_a_fixed_one() {
    for (position, after) in [("absolute", [11.0, -13.0]), ("fixed", [11.0, 27.0])] {
        let mut c = Case::build(
            C,
            "overflow: auto; height: 60px;",
            &format!("text{}tail{TALL}", boxed(position, "div", "", "")),
            "",
        );
        assert_at(c.rel("abs"), [11.0, 27.0], position);
        c.scroll("c", 40.0);
        assert_at(c.rel("abs"), after, &format!("{position}, scrolled"));
        c.relayout();
        assert_at(
            c.rel("abs"),
            after,
            &format!("{position}, scrolled and laid out"),
        );
    }
}

/// The page's scroll: a fixed box stays on screen where it was, an absolute
/// one goes with the page — against the initial containing block too.
#[test]
fn the_pages_scroll_moves_an_absolute_box_and_not_a_fixed_one() {
    for base in [C, C_STATIC] {
        for position in ["absolute", "fixed"] {
            let mut c = Case::build(
                base,
                "",
                &format!("text{}tail", boxed(position, "div", "", "")),
                r#"<div style="height: 2000px"></div>"#,
            );
            let body = c.doc.body();
            let before = c.screen("abs");
            assert_at(c.rel("abs"), [11.0, 27.0], position);
            c.doc.set_scroll_top(body, 50.0);
            let moved = if position == "fixed" { 0.0 } else { 50.0 };
            assert_eq!(
                c.screen("abs"),
                [before[0], before[1] - moved],
                "{position}, scrolled"
            );
            c.relayout();
            assert_eq!(
                c.screen("abs"),
                [before[0], before[1] - moved],
                "{position}, laid out"
            );
        }
    }
}

/// The static position is measured with nothing scrolled. A box shown inside
/// a scroller already at `scrollTop = 40`, after 100px of content and one
/// line, is at `(11, 127)` in Chrome — not 40px higher.
#[test]
fn a_box_shown_in_a_scrolled_scroller_is_placed_as_if_unscrolled() {
    for position in ["absolute", "fixed"] {
        let mut c = Case::new(&format!(
            r#"{SCROLLER}<div style="height: 100px"></div>text{}tail{TALL}</div>"#,
            boxed(position, "div", "display: none;", "")
        ));
        c.scroll("sc", 40.0);
        c.set_style("abs", &format!("position: {position}; {BOX}"));
        c.relayout();
        assert_at(c.rel("abs"), [11.0, 127.0], position);
    }
}

/// A fixed box inside an absolute one that is itself placed against the
/// initial containing block from inside a scrolled scroller. The outer box
/// stays at `(11, 107)` whatever the scroll, and Chrome has the inner one at
/// `(14, 129)`: `(3, 22)` inside it. The outer box's `layout` carries the
/// scroller's offset; the fixed box's sum takes it off again.
#[test]
fn a_fixed_box_in_an_absolute_box_placed_through_a_scrolled_scroller() {
    let mut c = Case::build(
        C_STATIC,
        "",
        &format!(
            r#"{SCROLLER}<div style="height: 100px"></div><div data-m="ia" style="position: absolute; width: 100px; height: 50px; padding: 2px 0 0 3px">text{}tail</div>{TALL}</div>"#,
            boxed("fixed", "div", "", "")
        ),
        "",
    );
    assert_at(c.rel("ia"), [11.0, 107.0], "the outer box");
    assert_at(c.rel("abs"), [14.0, 129.0], "unscrolled");
    c.scroll("sc", 40.0);
    assert_at(c.rel("ia"), [11.0, 107.0], "the outer box, scrolled");
    assert_at(c.rel("abs"), [14.0, 129.0], "scrolled");
    c.relayout();
    assert_at(c.rel("abs"), [14.0, 129.0], "scrolled and laid out");
}

// ── What moves the box afterwards ───────────────────────────────────────────

/// The lines are rebuilt when the container narrows; the box follows the
/// content before it onto its new line. 400px: one line, the box below it.
/// 131px (Chrome's `a_3line_mid`): `alpha beta` / `gamma delta` / …, and the
/// box below the second.
#[test]
fn the_box_follows_its_place_when_the_lines_are_broken_again() {
    for (position, tag, wide, narrow) in [
        ("absolute", "div", [11.0, 27.0], [11.0, 47.0]),
        ("fixed", "div", [11.0, 27.0], [11.0, 47.0]),
        ("absolute", "span", [154.844, 7.0], [71.313, 27.0]),
        ("fixed", "span", [154.844, 7.0], [71.313, 27.0]),
    ] {
        let mut c = Case::new(&format!(
            "alpha beta gamma {}delta epsilon zeta",
            boxed(position, tag, "", "")
        ));
        assert_at(c.rel("abs"), wide, &format!("{position} {tag}, 400px"));
        c.set_style("c", &format!("{C}width: 131px;"));
        c.relayout();
        assert_at(c.rel("abs"), narrow, &format!("{position} {tag}, 131px"));
        c.set_style("c", C);
        c.relayout();
        assert_at(
            c.rel("abs"),
            wide,
            &format!("{position} {tag}, 400px again"),
        );
    }
}

/// A relayout that rebuilds no line reads every kind of box back to the
/// place its lines hold: an absolute box in its own containing block, one
/// placed against the initial containing block, and a fixed one.
#[test]
fn a_relayout_that_rebuilds_no_line_keeps_the_box_in_its_line() {
    for base in [C, C_STATIC] {
        for position in ["absolute", "fixed"] {
            let mut c = Case::build(
                base,
                "width: 131px;",
                &format!(
                    "alpha beta gamma {}delta epsilon zeta",
                    boxed(position, "span", "", "")
                ),
                "",
            );
            let what = format!("{position}, static container: {}", base == C_STATIC);
            assert_at(c.rel("abs"), [71.313, 27.0], &what);
            let frame = c.frame();
            assert_eq!(
                frame.get(Counter::ShapeIfcBuild),
                0,
                "{what}: no line is rebuilt"
            );
            assert_at(
                c.rel("abs"),
                [71.313, 27.0],
                &format!("{what}, laid out again"),
            );
        }
    }
}

/// `text-align` moves a line and no box (the layout is skipped but for the
/// lines): an inline-level box goes with its line, a block-level one stays
/// at the content box's edge. Chrome: `(208.641, 7)` centred.
#[test]
fn text_align_moves_an_inline_level_box_with_its_line() {
    for position in ["absolute", "fixed"] {
        for (tag, left, centred) in [
            ("span", [39.328, 7.0], [208.641, 7.0]),
            ("div", [11.0, 27.0], [11.0, 27.0]),
        ] {
            let mut c = Case::new(&format!("text{}tail", boxed(position, tag, "", "")));
            assert_at(c.rel("abs"), left, &format!("{position} {tag}"));
            c.set_style("c", &format!("{C}text-align: center;"));
            c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            assert_at(c.rel("abs"), centred, &format!("{position} {tag}, centred"));
        }
    }
}

/// A box appended after the first layout, as the container's last child —
/// no inline content follows it, so nothing about the container's members
/// changed — still finds its place: below the second line of three.
#[test]
fn a_box_appended_later_finds_its_place() {
    for position in ["absolute", "fixed"] {
        let mut c = Case::build(C, "width: 131px;", "alpha beta gamma", "");
        let abs = c.doc.create_element("div");
        c.doc.set_attribute(abs, "data-m", "abs");
        c.doc
            .set_attribute(abs, "style", &format!("position: {position}; {BOX}"));
        c.doc.append_child(NodeId(c.c), abs);
        c.relayout();
        // `alpha beta` / `gamma`: below the second line.
        assert_at(c.rel("abs"), [11.0, 47.0], position);
        // …and leaves with no trace when it goes.
        c.doc.remove_child(NodeId(c.c), abs);
        c.relayout();
        assert!(
            c.doc
                .tree
                .get(c.c)
                .unwrap()
                .text_layout
                .as_ref()
                .unwrap()
                .layout
                .len()
                == 2,
            "precondition: two lines"
        );
    }
}

/// A box that was `display: none` is no part of the lines; shown, it is — in
/// a block whose members did not change at all. Chrome: below the second of
/// three lines, `(11, 47)`; in the line for a `<span>`, `(71.313, 27)`.
#[test]
fn a_box_shown_later_finds_its_place() {
    for position in ["absolute", "fixed"] {
        for (tag, display, want) in [
            ("div", "block", [11.0, 47.0]),
            ("span", "inline", [71.313, 27.0]),
        ] {
            let mut c = Case::build(
                C,
                "width: 131px;",
                &format!(
                    "alpha beta gamma {}delta epsilon zeta",
                    boxed(position, tag, "display: none;", "")
                ),
                "",
            );
            c.set_style(
                "abs",
                &format!("position: {position}; {BOX} display: {display};"),
            );
            c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            assert_at(c.rel("abs"), want, &format!("{position} {tag}, shown"));
        }
    }
}

/// A box moved out of the lines into a block that has none keeps nothing of
/// its old place: on the very layout after the move it is where a fresh
/// layout puts it, below the block before it.
#[test]
fn a_box_moved_out_of_the_lines_forgets_its_place() {
    for position in ["absolute", "fixed"] {
        let other = r#"<div data-m="other" style="position: relative; margin-left: 70px"><div style="height: 25px"></div>"#;
        let b = boxed(position, "span", "", "");
        let mut c = Case::build(C, "", &format!("text{b}tail"), &format!("{other}</div>"));
        assert_at(
            c.rel("abs"),
            [39.328, 7.0],
            &format!("{position}, in the line"),
        );
        let (abs, to) = (c.id("abs"), c.id("other"));
        c.doc.append_child(NodeId(to), NodeId(abs));
        c.relayout();
        let fresh = Case::build(C, "", "texttail", &format!("{other}{b}</div>"));
        assert_eq!(c.rel("abs"), fresh.rel("abs"), "{position}, moved");
        assert_eq!(
            c.rel("abs")[0],
            70.0 - 17.0,
            "{position}: at the other block's edge"
        );
    }
}

/// The text before the box goes: the box is first on its line, at the top.
#[test]
fn removing_the_content_before_the_box_moves_it_up() {
    let mut c = Case::new(&format!("text{}tail", boxed("absolute", "div", "", "")));
    assert_at(c.rel("abs"), [11.0, 27.0], "after text");
    let text = c.doc.tree.get(c.c).unwrap().children[0];
    c.doc.remove_child(NodeId(c.c), NodeId(text));
    c.relayout();
    assert_at(c.rel("abs"), [11.0, 7.0], "first");
}

/// Whether the box was inline-level is its own style, and changing it
/// changes no Taffy value (`position` makes both a block): a `<span>` given
/// `display: block` drops below its line, and comes back.
#[test]
fn a_change_of_the_boxs_own_display_moves_it_between_the_two_places() {
    for position in ["absolute", "fixed"] {
        let mut c = Case::new(&format!("text{}tail", boxed(position, "span", "", "")));
        assert_at(c.rel("abs"), [39.328, 7.0], &format!("{position}, a span"));
        c.set_style(
            "abs",
            &format!("position: {position}; {BOX} display: block;"),
        );
        c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        assert_at(
            c.rel("abs"),
            [11.0, 27.0],
            &format!("{position}, display: block"),
        );
        c.set_style(
            "abs",
            &format!("position: {position}; {BOX} display: inline-block;"),
        );
        c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        assert_at(
            c.rel("abs"),
            [39.328, 7.0],
            &format!("{position}, inline-block"),
        );
    }
}

/// `absolute` ↔ `fixed` keeps the static position, and an inset written and
/// taken back returns the box to it.
#[test]
fn position_and_inset_changes_return_to_the_static_position() {
    let mut c = Case::new(&format!("text{}tail", boxed("absolute", "span", "", "")));
    assert_at(c.rel("abs"), [39.328, 7.0], "absolute");
    c.set_style("abs", &format!("position: fixed; {BOX}"));
    c.relayout();
    assert_at(c.rel("abs"), [39.328, 7.0], "fixed");
    c.set_style(
        "abs",
        &format!("position: absolute; {BOX} left: 5px; top: 3px;"),
    );
    c.relayout();
    assert_at(c.rel("abs"), [5.0, 3.0], "insets");
    c.set_style("abs", &format!("position: absolute; {BOX}"));
    c.relayout();
    assert_at(c.rel("abs"), [39.328, 7.0], "auto again");
}

/// A block drawing a `text-overflow: ellipsis` "…" has its lines rebuilt as
/// flat text; a box keeps the place the lines they replace gave it (the
/// `*_ellipsis_*` rows: Chrome puts an inline-level one after the whole uncut
/// text, `(192.375, 7)`). And it is still there after a relayout that
/// rebuilds no line.
#[test]
fn a_box_in_a_block_cut_with_an_ellipsis_keeps_its_place() {
    check(|n| n.contains("ellipsis"));
    let mut c = Case::build(
        C,
        "width: 131px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        &format!(
            "alpha beta gamma delta{}epsilon",
            boxed("absolute", "span", "", "")
        ),
        "",
    );
    c.relayout();
    assert_at(c.rel("abs"), [192.375, 7.0], "after a relayout");
}

/// A box placed from its lines is where a **fresh** layout of the same
/// document puts it, after a change that moves the lines without rebuilding
/// them — the block above an anonymous box grows — or that moves the box
/// holding them along its own line.
#[test]
fn a_moved_line_block_takes_its_boxes_along() {
    for base in [C, C_STATIC] {
        for position in ["absolute", "fixed"] {
            for tag in ["div", "span"] {
                let what = format!("{position} {tag}, static container: {}", base == C_STATIC);
                let b = boxed(position, tag, "", "");

                // Text beside a block child: the lines are an anonymous box's.
                let page = |h: u32| {
                    format!(r#"<div data-m="blk" style="height: {h}px"></div>text{b}tail"#)
                };
                let mut c = Case::build(base, "", &page(10), "");
                c.set_style("blk", "height: 33px");
                c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
                let fresh = Case::build(base, "", &page(33), "");
                assert_eq!(
                    c.rel("abs"),
                    fresh.rel("abs"),
                    "{what}: the block above grew"
                );
                let x = if tag == "div" { 11.0 } else { 39.328 };
                let y = if tag == "div" { 60.0 } else { 40.0 };
                assert_at(fresh.rel("abs"), [x, y], &what);

                // Lines inside an inline-block that the text before it moves.
                let page = |lead: &str| {
                    format!(
                        r#"<span data-m="lead">{lead}</span> <span data-m="ib" style="display: inline-block; width: 100px">text{b}tail</span>"#
                    )
                };
                let mut c = Case::build(base, "", &page("lead"), "");
                let lead = c.doc.tree.get(c.id("lead")).unwrap().children[0];
                c.doc.set_text_content(NodeId(lead), "a longer lead");
                c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
                let fresh = Case::build(base, "", &page("a longer lead"), "");
                assert_eq!(
                    c.rel("abs"),
                    fresh.rel("abs"),
                    "{what}: the inline-block moved"
                );
                assert_ne!(
                    fresh.rel("abs"),
                    Case::build(base, "", &page("lead"), "").rel("abs"),
                    "{what}: the fixture moves the box"
                );
            }
        }
    }
}

// ── Cost ────────────────────────────────────────────────────────────────────

/// What a relayout looks at. An absolute box whose layout parent is its
/// containing block is in no pass, static position or not (0); a fixed box
/// is looked at once when an axis is static, and not at all with insets on
/// both. None of them costs a second compute.
#[test]
fn a_relayout_visits_only_the_boxes_placed_from_outside_their_parent() {
    for (style, visited) in [
        ("position: absolute;", 0),
        ("position: absolute; top: 2px; right: 2px;", 0),
        ("position: fixed;", 1),
        ("position: fixed; top: 2px;", 1),
        ("position: fixed; top: 2px; left: 2px;", 0),
    ] {
        let mut c = Case::new(&format!(
            r#"text<div data-m="abs" style="{style} {BOX}"></div>tail"#
        ));
        let frame = c.frame();
        assert_eq!(
            frame.get(Counter::AbsBoxesVisited),
            visited,
            "{style}: boxes visited"
        );
        assert_eq!(
            frame.get(Counter::TaffyRootComputes),
            1,
            "{style}: computes"
        );
        assert_eq!(
            frame.get(Counter::ShapeIfcBuild),
            0,
            "{style}: no line is rebuilt"
        );
    }
}

// ── Not Chrome's, pinned ────────────────────────────────────────────────────

/// rinch's current answers where they are not Chrome's, each with Chrome
/// 153's beside it. None is the static-position rule of this file: they are
/// what the rule is applied **to** — a line box's height, Taffy's static
/// position in a flex container, a fixed box's auto size (#893).
#[test]
fn known_differences_from_chrome() {
    // A line holding a 26px inline-block and text is 26px tall in rinch and
    // 30 in Chrome (the text's descent below the box's baseline), so a
    // block-level box below it is 4px high: Chrome (11, 37). Issue #663.
    let ib = r#"<span style="display: inline-block; width: 50px; height: 26px"></span>"#;
    let c = Case::new(&format!("{ib}{}tail", boxed("absolute", "div", "", "")));
    assert_eq!(
        c.rel("abs"),
        [11.0, 33.0],
        "below a line with a tall inline-block"
    );

    // A flex container's static position is Taffy's: the main axis follows
    // `justify-content`, the cross axis does not follow `align-items`.
    // Chrome: (185.5, 28.5) centred, (360, 50) at the end. Issue #1492.
    let item = r#"<div style="width: 50px; height: 20px"></div>"#;
    for (align, got) in [("center", [186.0, 7.0]), ("flex-end", [360.0, 7.0])] {
        let c = Case::build(
            C,
            &format!(
                "display: flex; justify-content: {align}; align-items: {align}; height: 80px;"
            ),
            &format!("{item}{}{item}", boxed("absolute", "div", "", "")),
            "",
        );
        assert_eq!(c.rel("abs"), got, "flex, {align}");
    }

    // A fixed box with no insets and no size fills the viewport; Chrome
    // shrinks it to its content, 27.766 x 20. Issue #893.
    let c = Case::new(r#"text<div data-m="abs" style="position: fixed">box</div>tail"#);
    let l = c.doc.tree.get(c.id("abs")).unwrap().layout;
    assert_eq!(
        [l.width, l.height],
        [800.0, 600.0],
        "an auto-sized fixed box"
    );
    assert_at(
        c.rel("abs"),
        [11.0, 27.0],
        "…at its static position all the same",
    );
}
