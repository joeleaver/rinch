//! Review round 2 of #1683: the web line box against Chrome's own lines.
//! Oracle: a one-line paragraph's border box IS its line box (no padding on
//! the editor's `p`), so the reported (top, height) must equal it.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::Pos;
use rinch_web::{Editor, EditorHandle, MarginAnchor, MarginLine, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    sheet: web_sys::Element,
    handle: EditorHandle,
    reports: Rc<RefCell<Vec<Vec<MarginLine>>>>,
}

fn mount(html: &str, css: &str, editor_style: &str) -> F {
    let sheet = document().create_element("style").unwrap();
    sheet.set_text_content(Some(css));
    document().head().unwrap().append_child(&sheet).unwrap();
    let host = document().create_element("div").unwrap();
    host.set_attribute("style", "padding: 20px;").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(html));
    let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
    let (h, r) = (handle.clone(), reports.clone());
    let es = editor_style.to_string();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let r = r.clone();
            h.on_margin_layout(move |lines: &[MarginLine]| r.borrow_mut().push(lines.to_vec()));
            let kid = scope.create_element("div");
            let editor = Editor {
                editor: Some(h.clone()),
                ..Default::default()
            }
            .render(scope, &[kid]);
            editor.set_attribute("style", &format!("width: 600px; font-size: 16px; {es}"));
            editor
        },
    );
    F {
        root,
        host,
        sheet,
        handle,
        reports,
    }
}

impl F {
    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        self.sheet.remove();
    }
    fn frame_y(&self) -> f64 {
        self.host
            .query_selector("[data-pm-overlay]")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect()
            .top()
    }
    fn paras(&self) -> Vec<web_sys::DomRect> {
        let list = self.host.query_selector_all("[data-pm-editor] p").unwrap();
        (0..list.length())
            .map(|i| {
                list.item(i)
                    .unwrap()
                    .dyn_into::<web_sys::Element>()
                    .unwrap()
                    .get_bounding_client_rect()
            })
            .collect()
    }
}

/// The textblocks' start positions (inside), in document order.
fn textblock_starts(h: &EditorHandle) -> Vec<(usize, usize)> {
    let doc = h.doc();
    let mut out = Vec::new();
    let size = doc.content_size();
    doc.nodes_between(0, size, &mut |n, pos, _| {
        if n.is_textblock() {
            out.push((pos + 1, n.content_size()));
            return false;
        }
        true
    });
    out
}

/// Every one-line case paragraph: (label, html).
const CASES: &[(&str, &str)] = &[
    ("plain", "<p>plain text</p>"),
    ("sup", "<p>aa<sup>bb</sup>cc</p>"),
    ("sub", "<p>aa<sub>bb</sub>cc</p>"),
    ("bigcode", "<p>aa<code>BIG</code>cc</p>"),
    (
        "img-middle",
        "<p>aa<img src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\">cc</p>",
    ),
    ("empty", "<p></p>"),
    ("rtl", "<p>שלום עולם hello</p>"),
    (
        "cell",
        "<table><tr><td><p>cell</p></td><td><p>c2</p></td></tr></table>",
    ),
    ("list", "<ul><li><p>item</p></li></ul>"),
    ("quote", "<blockquote><p>quoted</p></blockquote>"),
];

const CSS: &str = "[data-pm-editor] code { font-size: 32px; } \
     [data-pm-editor] img { width: 12px; height: 60px; vertical-align: middle; }";

async fn run(editor_style: &str) -> Vec<String> {
    let html: String = CASES.iter().map(|c| c.1).collect();
    let f = mount(&html, CSS, editor_style);
    next_frame().await;
    next_task().await;
    let starts = textblock_starts(&f.handle);
    let labels: Vec<&str> = CASES
        .iter()
        .flat_map(|c| {
            if c.0 == "cell" {
                vec!["cell", "cell2"]
            } else {
                vec![c.0]
            }
        })
        .collect();
    assert_eq!(starts.len(), labels.len(), "{starts:?}");
    let mut anchors = Vec::new();
    for ((start, len), label) in starts.iter().zip(&labels) {
        anchors.push(MarginAnchor::at(*label, Pos(start + (*len).min(1))));
    }
    // An anchor right before the picture.
    let img = labels.iter().position(|l| *l == "img-middle").unwrap();
    anchors.push(MarginAnchor::at("before-img", Pos(starts[img].0 + 2)));
    f.handle.set_margin_anchors(anchors);
    next_frame().await;
    next_task().await;
    let lines = f.reports.borrow().last().cloned().expect("a report");
    let fy = f.frame_y();
    let paras = f.paras();
    let mut out = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let p = &paras[i];
        let truth = (p.top() - fy, p.height());
        let got = lines
            .iter()
            .find(|l| l.key == *label)
            .map(|l| (l.top as f64, l.height as f64));
        // Under `line-height: normal` the browser's line is the font's own
        // metrics, which the content area only approximates (a pixel).
        let tol = if editor_style.contains("normal") {
            1.01
        } else {
            0.5
        };
        let ok = got.is_some_and(|g| (g.0 - truth.0).abs() < tol && (g.1 - truth.1).abs() < tol);
        out.push(format!(
            "[{editor_style}] {label:12} truth ({:.2}, {:.2}) got {:?} {}",
            truth.0,
            truth.1,
            got,
            if ok { "OK" } else { "DIFF" }
        ));
    }
    let bi = lines.iter().find(|l| l.key == "before-img");
    let p = &paras[img];
    out.push(format!(
        "[{editor_style}] before-img   truth ({:.2}, {:.2}) got {:?}",
        p.top() - fy,
        p.height(),
        bi.map(|l| (l.top, l.height))
    ));
    f.teardown();
    out
}

/// Every case's line box is the one-line paragraph's border box, under a px,
/// a unitless and a `normal` line-height (review 2 of #1683, G3: a `sup`, a
/// `sub` and a 32px `code` run under 1.65 are grown by their own
/// line-height).
#[wasm_bindgen_test]
async fn the_line_box_is_chromes_under_every_line_height() {
    let mut all = Vec::new();
    for style in [
        "line-height: 24px",
        "line-height: 1.65",
        "line-height: normal",
    ] {
        all.extend(run(style).await);
    }
    let bad: Vec<&String> = all.iter().filter(|l| l.ends_with("DIFF")).collect();
    assert!(bad.is_empty(), "{}", all.join("\n"));
}

/// A keystroke in one editor measures nothing of another's anchors.
#[wasm_bindgen_test]
async fn a_keystroke_in_one_editor_measures_no_other_editors_anchors() {
    let other = mount("<p>one</p><p>two</p><p>three</p>", "", "line-height: 24px");
    other.handle.set_margin_anchors(vec![
        MarginAnchor::at("a", Pos(2)),
        MarginAnchor::at("b", Pos(7)),
        MarginAnchor::at("c", Pos(12)),
    ]);
    let typing = mount("<p>type here</p>", "", "line-height: 24px");
    next_frame().await;
    next_task().await;
    assert!(!other.reports.borrow().is_empty(), "precondition: reported");
    typing
        .handle
        .set_selection(rinch_editor_core::Selection::cursor(Pos(2)));
    next_task().await;
    let before = rinch_editor_view::__margin_counters();
    for _ in 0..3 {
        assert!(typing.handle.insert_text("x"));
        next_task().await;
    }
    assert_eq!(
        rinch_editor_view::__margin_counters(),
        before,
        "nothing measured"
    );
    typing.teardown();
    other.teardown();
}

/// Cost: 500 one-line paragraphs, one anchor each; a keystroke's pass.
#[wasm_bindgen_test]
#[ignore = "a measurement, reported by panic"]
async fn cost_500_anchors() {
    let html: String = (0..500)
        .map(|i| format!("<p>line {i} of text</p>"))
        .collect();
    let f = mount(&html, "", "line-height: 24px");
    next_frame().await;
    next_task().await;
    let starts = textblock_starts(&f.handle);
    let mut res = Vec::new();
    for with in [false, true, false, true] {
        let anchors: Vec<MarginAnchor> = if with {
            starts
                .iter()
                .enumerate()
                .map(|(i, (s, _))| MarginAnchor::at(format!("k{i}"), Pos(s + 1)))
                .collect()
        } else {
            Vec::new()
        };
        f.handle.set_margin_anchors(anchors);
        next_frame().await;
        next_task().await;
        f.handle
            .set_selection(rinch_editor_core::Selection::cursor(Pos(starts[250].0 + 1)));
        next_task().await;
        let (p0, m0) = rinch_editor_view::__margin_counters();
        let mut times = Vec::new();
        for _ in 0..10 {
            let t0 = js_sys::Date::now();
            assert!(f.handle.insert_text("x"));
            // The owed pass runs in a microtask; settle it.
            next_task().await;
            times.push(js_sys::Date::now() - t0);
        }
        let (p1, m1) = rinch_editor_view::__margin_counters();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        res.push(format!(
            "anchors={with} median {:.2} ms min {:.2} max {:.2} passes {} measures {}",
            times[5],
            times[0],
            times[9],
            p1 - p0,
            m1 - m0
        ));
    }
    f.teardown();
    panic!("COST\n{}", res.join("\n"));
}

/// Cost: one long paragraph (~400 lines), 500 anchors in it.
#[wasm_bindgen_test]
#[ignore = "a measurement, reported by panic"]
async fn cost_one_block() {
    let word = "alpha bravo charlie delta echo foxtrot golf hotel ";
    let text: String = (0..800).map(|_| word).collect();
    let f = mount(&format!("<p>{text}</p>"), "", "line-height: 24px");
    next_frame().await;
    next_task().await;
    let (start, len) = textblock_starts(&f.handle)[0];
    let mut res = Vec::new();
    for with in [false, true] {
        let anchors: Vec<MarginAnchor> = if with {
            (0..500)
                .map(|i| MarginAnchor::at(format!("k{i}"), Pos(start + i * (len / 500))))
                .collect()
        } else {
            Vec::new()
        };
        f.handle.set_margin_anchors(anchors);
        next_frame().await;
        next_task().await;
        f.handle
            .set_selection(rinch_editor_core::Selection::cursor(Pos(start + 5)));
        next_task().await;
        let mut times = Vec::new();
        for _ in 0..5 {
            let t0 = js_sys::Date::now();
            assert!(f.handle.insert_text("x"));
            next_task().await;
            times.push(js_sys::Date::now() - t0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        res.push(format!(
            "one-block anchors={with} median {:.1} ms max {:.1}",
            times[2], times[4]
        ));
    }
    f.teardown();
    panic!("COST1\n{}", res.join("\n"));
}
