//! Margin anchors and the `Editor` overlay slot in a browser (#1677). Desktop's
//! twin is `rinch/src/app/editor_margin_anchors_1677_tests.rs`.
//!
//! The oracle for "where is the line" is the client rect of a paragraph whose
//! line box is the declared `line-height: 24px` (one line: its top; the last
//! of several: its bottom less 24), and for "in which frame" a child of the
//! overlay slot placed at the reported `top`: it must sit level with the line.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_margin_anchors_1677
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::{Pos, Selection};
use rinch_web::{Editor, EditorHandle, MarginAnchor, MarginLine, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/engine.rs"]
mod engine;

const HOST_MARKER: &str = "data-test-host-margin-anchors";

/// Wraps to several lines in the 300px editor.
const LONG: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    reports: Rc<RefCell<Vec<Vec<MarginLine>>>>,
    clicks: Rc<Cell<usize>>,
}

impl Fixture {
    /// `<p>top</p><p>{body}</p><p>end</p>` in an `Editor` whose slot holds a
    /// marker (moved by the callback to the line of the anchor keyed `mark`)
    /// and a button. `editor_style` is added to the editor's style.
    fn mount(body: &str, editor_style: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute("style", "padding: 20px;").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(&format!("<p>top</p><p>{body}</p><p>end</p>")));
        let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
        let clicks: Rc<Cell<usize>> = Rc::default();
        let (h, r, c) = (handle.clone(), reports.clone(), clicks.clone());
        let editor_style = editor_style.to_string();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let marker = scope.create_element("div");
                marker.set_attribute("data-marker", "");
                marker.set_attribute(
                    "style",
                    "position: absolute; left: 0; top: 0; width: 6px; height: 6px",
                );
                let button = scope.create_element("button");
                button.set_attribute(
                    "style",
                    "position: absolute; left: 250px; top: 30px; width: 30px; height: 20px",
                );
                let c = c.clone();
                let rid = scope.register_handler(move || c.set(c.get() + 1));
                button.set_attribute("data-rid", &rid.0.to_string());
                let (m, r) = (marker.clone(), r.clone());
                h.on_margin_layout(move |lines: &[MarginLine]| {
                    if let Some(line) = lines.iter().find(|l| l.key == "mark") {
                        m.set_style("top", &format!("{}px", line.top));
                    }
                    r.borrow_mut().push(lines.to_vec());
                });
                let editor = Editor {
                    editor: Some(h.clone()),
                    ..Default::default()
                }
                .render(scope, &[marker, button]);
                editor.set_attribute(
                    "style",
                    &format!(
                        "width: 300px; font-family: monospace; font-size: 16px; \
                         line-height: 24px; {editor_style}"
                    ),
                );
                editor
            },
        );
        Self {
            root,
            host,
            handle,
            reports,
            clicks,
        }
    }

    fn el(&self, selector: &str) -> web_sys::Element {
        self.host
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
    }

    fn para(&self, i: u32) -> web_sys::DomRect {
        let list = self.host.query_selector_all("p").unwrap();
        list.item(i)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap()
            .get_bounding_client_rect()
    }

    /// Client y of the reporting frame's origin: the overlay slot's.
    fn frame_y(&self) -> f64 {
        self.el("[data-pm-overlay]")
            .get_bounding_client_rect()
            .top()
    }

    fn last(&self) -> Vec<MarginLine> {
        self.reports.borrow().last().cloned().expect("a report")
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn line(lines: &[MarginLine], key: &str) -> (f64, f64) {
    let l = lines
        .iter()
        .find(|l| l.key == key)
        .unwrap_or_else(|| panic!("{key} reported: {lines:?}"));
    (l.top as f64, l.height as f64)
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 0.5, "{what}: {a} vs {b}");
}

/// The end of the second paragraph (`top` 1..4, the body from 6).
fn body_end(body: &str) -> Pos {
    Pos(6 + body.chars().count())
}

fn anchors(body: &str) -> Vec<MarginAnchor> {
    vec![
        MarginAnchor::at("a", Pos(2)),
        MarginAnchor::at("b", Pos(6)),
        MarginAnchor::at("mark", body_end(body)),
        MarginAnchor::at("d", Pos(body_end(body).0 - 1)),
    ]
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

/// Each anchor reports its line box in the slot's frame, and a slot child
/// placed at that `top` sits level with the line.
#[wasm_bindgen_test]
async fn each_anchor_reports_its_line_and_a_slot_child_there_sits_on_it() {
    let f = Fixture::mount(LONG, "");
    f.handle.set_margin_anchors(anchors(LONG));
    next_task().await;
    let lines = f.last();
    let fy = f.frame_y();
    let (p0, p1) = (f.para(0), f.para(1));
    assert!(p1.height() >= 72.0, "precondition: the body wraps");
    close(line(&lines, "a").0, p0.top() - fy, "a's line");
    close(line(&lines, "b").0, p1.top() - fy, "b's line");
    close(
        line(&lines, "mark").0,
        p1.bottom() - 24.0 - fy,
        "mark's line",
    );
    for key in ["a", "b", "mark"] {
        close(line(&lines, key).1, 24.0, "a line box is 24px");
    }
    assert_eq!(line(&lines, "d"), line(&lines, "mark"), "one line, one box");
    let marker = f.el("[data-marker]").get_bounding_client_rect();
    close(
        marker.top(),
        p1.bottom() - 24.0,
        "the marker sits on its line",
    );
    f.teardown();
}

/// A line holding text in two sizes (a `code` run beside plain text) is one
/// line: both report one box.
#[wasm_bindgen_test]
async fn a_line_of_mixed_sizes_reports_one_box() {
    let body = "plain <code>code</code> more";
    let f = Fixture::mount(body, "");
    // `plain ` 6..12, `code` 12..16.
    f.handle.set_margin_anchors(vec![
        MarginAnchor::at("plain", Pos(8)),
        MarginAnchor::at("code", Pos(14)),
    ]);
    next_task().await;
    let lines = f.last();
    assert_eq!(line(&lines, "plain"), line(&lines, "code"));
    close(
        line(&lines, "plain").0,
        f.para(1).top() - f.frame_y(),
        "the line",
    );
    f.teardown();
}

/// With `line-height: normal` the browser's line is measured from the caret
/// rects, which differ across sizes on one line: still one box for the line.
#[wasm_bindgen_test]
async fn a_line_of_mixed_sizes_at_normal_line_height_reports_one_box() {
    let body = "plain <code>code</code> more";
    let f = Fixture::mount(body, "line-height: normal");
    f.handle.set_margin_anchors(vec![
        MarginAnchor::at("plain", Pos(8)),
        MarginAnchor::at("code", Pos(14)),
    ]);
    next_task().await;
    let lines = f.last();
    assert_eq!(line(&lines, "plain"), line(&lines, "code"));
    let p1 = f.para(1);
    let (top, height) = line(&lines, "plain");
    let fy = f.frame_y();
    assert!(
        top >= p1.top() - fy - 0.5 && top + height <= p1.bottom() - fy + 0.5,
        "inside the paragraph's one line: {top}+{height} in {}..{}",
        p1.top() - fy,
        p1.bottom() - fy
    );
    f.teardown();
}

/// An app's edit that moves lines and not the selection still reports.
#[wasm_bindgen_test]
async fn an_edit_from_app_code_reports_the_moved_lines() {
    let f = Fixture::mount(LONG, "");
    f.handle.set_selection(Selection::cursor(Pos(2)));
    f.handle.set_margin_anchors(anchors(LONG));
    next_task().await;
    let before = f.reports.borrow().len();
    // A paragraph between `top` and the body: the selection stays put.
    assert!(f.handle.update(|state| {
        let mut tr = state.tr();
        let s = state.schema();
        let p = s
            .branch("paragraph", Fragment::from_node(s.text("new").ok()?))
            .ok()?;
        tr.replace(5, 5, Slice::from_fragment(Fragment::from_node(p)))
            .ok()?;
        Some(tr)
    }));
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(2)));
    next_task().await;
    assert_eq!(f.reports.borrow().len(), before + 1, "one report");
    let p2 = f.para(2);
    close(
        line(&f.last(), "b").0,
        p2.top() - f.frame_y(),
        "b moved down with its text",
    );
    f.teardown();
}

/// The lines are the content's: with the editor itself the scroller, a
/// scroll and a pass after it report nothing, the marker scrolls with its
/// line, and the caret drawn after the scroll is on its line too.
#[wasm_bindgen_test]
async fn a_scrolled_editor_reports_nothing_and_its_overlays_stay_on_their_lines() {
    let f = Fixture::mount(LONG, "height: 90px; overflow: auto");
    f.handle.set_margin_anchors(anchors(LONG));
    f.handle.focus();
    next_task().await;
    let before = f.reports.borrow().len();
    let editor = f.el("[data-pm-editor]");
    editor.set_scroll_top(30);
    assert_eq!(editor.scroll_top(), 30, "precondition: it scrolls");
    // A pass after the scroll (a caret moved by app code).
    f.handle.set_selection(Selection::cursor(body_end(LONG)));
    next_task().await;
    assert_eq!(f.reports.borrow().len(), before, "a scroll moves no line");
    let line_top = f.para(1).bottom() - 24.0;
    let marker = f.el("[data-marker]").get_bounding_client_rect();
    close(marker.top(), line_top, "the marker scrolled with its line");
    let caret = f.el("[data-pm-caret]").get_bounding_client_rect();
    assert!(
        caret.top() >= line_top - 0.5 && caret.bottom() <= line_top + 24.5,
        "the caret is on its line: {} in {line_top}..{}",
        caret.top(),
        line_top + 24.0
    );
    f.teardown();
}

/// A reflow the editor did not make (its width changed) reports.
#[wasm_bindgen_test]
async fn a_resize_that_rewraps_reports() {
    let f = Fixture::mount(LONG, "");
    f.handle.set_margin_anchors(anchors(LONG));
    next_task().await;
    let before = f.reports.borrow().len();
    let editor = f.el("[data-pm-editor]");
    let style = editor.get_attribute("style").unwrap();
    editor
        .set_attribute("style", &style.replace("width: 300px", "width: 200px"))
        .unwrap();
    next_frame().await;
    next_frame().await;
    next_task().await;
    assert!(f.reports.borrow().len() > before, "the resize reported");
    close(
        line(&f.last(), "mark").0,
        f.para(1).bottom() - 24.0 - f.frame_y(),
        "mark's new line",
    );
    f.teardown();
}

/// A press on a slot child is the child's: its handler runs, the editor's
/// selection stays.
#[wasm_bindgen_test]
async fn a_press_on_a_slot_child_is_the_childs() {
    let f = Fixture::mount(LONG, "");
    f.handle.set_selection(Selection::cursor(Pos(2)));
    next_task().await;
    let button = f.el("button");
    let r = button.get_bounding_client_rect();
    let (x, y) = (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
    let hit = document().element_from_point(x as f32, y as f32).unwrap();
    assert!(
        hit.is_same_node(Some(&button)),
        "precondition: the button is hit"
    );
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    for name in ["pointerdown", "mousedown"] {
        let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
        button.dispatch_event(&ev).unwrap();
    }
    init.set_buttons(0);
    // No synthetic `click`: an untrusted one activates a `data-rid` on its
    // own (#240), as a second press would.
    for name in ["pointerup", "mouseup"] {
        let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
        button.dispatch_event(&ev).unwrap();
    }
    next_task().await;
    assert_eq!(f.clicks.get(), 1, "the button's handler ran");
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(2)));
    f.teardown();
}

/// A line a 60px picture makes taller than `line-height` is reported whole,
/// and the line after it where it is (review of #1683, F2).
#[wasm_bindgen_test]
async fn a_line_taller_than_its_line_height_is_reported_whole() {
    let style = document().create_element("style").unwrap();
    style.set_text_content(Some(
        "[data-pm-editor] img { width: 12px; height: 60px; vertical-align: baseline; }",
    ));
    document().head().unwrap().append_child(&style).unwrap();
    // The body: a picture 6..7, `xyz` 7..10, a hard break 10..11, `second` 11..17.
    let f = Fixture::mount(
        r#"<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=">xyz<br>second"#,
        "",
    );
    f.handle.set_margin_anchors(vec![
        MarginAnchor::at("tall", Pos(8)),
        MarginAnchor::at("mark", Pos(13)),
    ]);
    next_frame().await;
    next_task().await;
    let lines = f.last();
    let fy = f.frame_y();
    let p1 = f.para(1);
    let img = f.el("img").get_bounding_client_rect();
    assert!(
        p1.height() > 24.0 + 60.0 - 0.5,
        "precondition: a tall first line, {} {}",
        p1.height(),
        img.height()
    );
    let second_top = p1.bottom() - 24.0;
    let (top, height) = line(&lines, "tall");
    close(top, p1.top() - fy, "the tall line's top");
    close(top + height, second_top - fy, "the tall line's bottom");
    assert!(top <= img.top() - fy + 0.5, "the picture is in it");
    close(line(&lines, "mark").0, second_top - fy, "the line after it");
    close(line(&lines, "mark").1, 24.0, "a 24px line");
    let marker = f.el("[data-marker]").get_bounding_client_rect();
    close(marker.top(), second_top, "the marker sits on its line");
    f.teardown();
    style.remove();
}

/// Anchors and a callback set before the editor mounts report once it does,
/// and again after a re-mount (review of #1683, F1).
#[wasm_bindgen_test]
async fn anchors_report_after_the_mount_and_after_a_re_mount() {
    let handle = create_editor();
    assert!(handle.load_html("<p>top</p><p>two</p>"));
    let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
    let r = reports.clone();
    handle.on_margin_layout(move |lines: &[MarginLine]| r.borrow_mut().push(lines.to_vec()));
    handle.set_margin_anchors(vec![MarginAnchor::at("a", Pos(7))]);
    let mount = |handle: &EditorHandle| {
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let h = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                Editor {
                    editor: Some(h.clone()),
                    ..Default::default()
                }
                .render(scope, &[])
            },
        );
        (root, host)
    };
    let (root, host) = mount(&handle);
    next_frame().await;
    next_task().await;
    let first = reports.borrow().len();
    assert!(first > 0, "a report after the mount");
    assert_eq!(reports.borrow().last().unwrap()[0].key, "a");
    root.unmount();
    host.remove();
    let (root, host) = mount(&handle);
    next_frame().await;
    next_task().await;
    assert!(
        reports.borrow().len() > first,
        "a report after the re-mount"
    );
    assert_eq!(reports.borrow().last().unwrap().len(), 1);
    root.unmount();
    host.remove();
}

/// The leading split the line box is rebuilt with is measured, and is each
/// engine's own: Chrome 155 floors the half above a text run to a whole pixel
/// (17px of monospace text in a 24px line starts 3px down); Firefox puts the
/// exact half there (3.5px — measured in CI's Firefox, where the floored
/// rebuild was 0.5px low on every line).
#[wasm_bindgen_test]
fn the_leading_split_is_the_engines_own() {
    assert_eq!(rinch_web::__leading_floors(), !engine::is_gecko());
}
