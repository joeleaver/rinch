//! Page Up / Page Down in the rich-text editor on the web (#2020).
//!
//! The browser's own Page Down went to the capture textarea, which moved no
//! caret. Chrome 155's `contenteditable` (measured headless; the desktop
//! fixtures, `rinch/src/app/editor_page_keys_2020_tests.rs`, say how): the
//! caret moves 0.875 of the scrollport's height, keeping its x; Shift extends;
//! a page reaching the last (first) line goes to the document's end (start); a
//! read-only field scrolls a page and leaves the caret.
//!
//! Off the fixed points: the scroller has 13px padding (a 226px padding box),
//! is much shorter than the document, mixes 20px paragraphs with 30px
//! headings, and the caret starts mid-line in the third block.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-r2020";
/// The scroller's `clientHeight`: 200px plus 2 × 13px padding.
const PAGE_BOX: f64 = 226.0;
/// Chrome's page step, 0.875 of it, in whole pixels as `scrollTop` takes it.
const STEP: f64 = 198.0;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

fn content() -> String {
    (0..40)
        .map(|i| {
            if i % 3 == 1 {
                format!("<h2>abcdefghij {i:02}</h2>")
            } else {
                format!("<p>abcdefghij {i:02}</p>")
            }
        })
        .collect()
}

impl Fixture {
    fn mount() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        // A number: a paragraph's line is 20px, a heading's 30px.
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 1.25; padding: 10px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(&content()));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let scroller = scope.create_element("div");
                scroller.set_attribute("data-r2020-scroller", "");
                scroller.set_attribute(
                    "style",
                    "box-sizing: content-box; height: 200px; padding: 13px; width: 500px; \
                     overflow-y: auto",
                );
                scroller.append_child(&mounted.mount(scope));
                scroller
            },
        );
        // The runner's page has its log above: the fixture at the top.
        host.scroll_into_view();
        let f = Self { root, host, handle };
        assert_eq!(f.scroller().client_height() as f64, PAGE_BOX, "control: the page box");
        f
    }

    fn scroller(&self) -> web_sys::Element {
        document()
            .query_selector("[data-r2020-scroller]")
            .unwrap()
            .unwrap()
    }

    /// Focus with a genuine press on the first line, then put the caret six
    /// characters into block 2.
    fn start(&self) {
        let r = self.scroller().get_bounding_client_rect();
        let (x, y) = ((r.x() + 30.0) as f32, (r.y() + 20.0) as f32);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        let ta = document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("capture");
        assert!(
            document().active_element().as_deref() == Some(ta.as_ref()),
            "positive control: the press focused the capture textarea"
        );
        self.handle
            .set_selection(Selection::cursor(pos_in_block(&self.handle, 2, 6)));
        self.scroller().set_scroll_top(0);
    }

    /// The caret's `(x, y, height)` in viewport px.
    fn caret(&self) -> (f64, f64, f64) {
        let r = self
            .handle
            .caret_rect(self.handle.selection().head())
            .expect("the caret has geometry");
        (r.x as f64, r.y as f64, r.height as f64)
    }

    /// Each block's `(top, bottom)` in viewport px.
    fn bands(&self) -> Vec<(f64, f64)> {
        let blocks = document()
            .query_selector_all("[data-pm-editor] > [data-pm-type]")
            .unwrap();
        (0..blocks.length())
            .map(|i| {
                let el: web_sys::Element = blocks.item(i).unwrap().dyn_into().unwrap();
                let r = el.get_bounding_client_rect();
                (r.top(), r.bottom())
            })
            .collect()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn pos_in_block(handle: &EditorHandle, i: usize, off: usize) -> Pos {
    let doc = handle.doc();
    let before: usize = (0..i).map(|j| doc.child(j).node_size()).sum();
    Pos(before + 1 + off)
}

fn block_of(handle: &EditorHandle, pos: Pos) -> usize {
    let doc = handle.doc();
    let mut start = 0;
    for j in 0..doc.child_count() {
        let size = doc.child(j).node_size();
        if pos.0 < start + size {
            return j;
        }
        start += size;
    }
    doc.child_count()
}

/// The block holding `y`, or both neighbours of the gap it falls in.
fn expected_blocks(bands: &[(f64, f64)], y: f64) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, &(t, b)) in bands.iter().enumerate() {
        if (t..b).contains(&y) {
            return vec![i];
        }
        if b <= y && bands.get(i + 1).is_some_and(|&(nt, _)| y < nt) {
            out = vec![i, i + 1];
        }
    }
    out
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// A key on the capture textarea; whether rinch consumed it (the positive
/// control that the key reached the editor at all).
fn key(name: &str, shift: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(name);
    init.set_code(name);
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let ta = document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .unwrap();
    !ta.dispatch_event(&ev).unwrap()
}

#[wasm_bindgen_test]
fn page_down_moves_the_caret_a_page_and_the_scroller_with_it() {
    let f = Fixture::mount();
    f.start();
    let (cx, cy, ch) = f.caret();
    let want = expected_blocks(&f.bands(), cy + ch * 0.5 + STEP);
    assert!(!want.is_empty(), "control: a block a page down");

    assert!(key("PageDown", false), "rinch took Page Down");
    let got = block_of(&f.handle, f.handle.selection().head());
    assert!(
        want.contains(&got),
        "Page Down put the caret in block {got}; a page down is {want:?}"
    );
    assert!(f.handle.selection().is_empty());
    let sc = f.scroller();
    assert_eq!(f64::from(sc.scroll_top()), STEP, "the scroller moved a page");
    let (nx, ny, _) = f.caret();
    assert!((nx - cx).abs() < 8.0, "the column held: x {cx} -> {nx}");
    assert!((ny - cy).abs() < 30.0, "the caret kept its place: y {cy} -> {ny}");

    assert!(key("PageUp", false), "rinch took Page Up");
    assert_eq!(sc.scroll_top(), 0, "Page Up scrolled back");
    assert_eq!(block_of(&f.handle, f.handle.selection().head()), 2);
    f.teardown();
}

#[wasm_bindgen_test]
fn page_down_at_the_end_goes_to_the_document_end_and_page_up_to_its_start() {
    let f = Fixture::mount();
    f.start();
    let end = Selection::at_end(&f.handle.doc()).head();
    for _ in 0..12 {
        key("PageDown", false);
    }
    assert_eq!(f.handle.selection().head(), end, "Page Down ends at the end");
    let sc = f.scroller();
    assert_eq!(
        sc.scroll_top(),
        sc.scroll_height() - sc.client_height(),
        "the scroller is at its end"
    );
    for _ in 0..12 {
        key("PageUp", false);
    }
    assert_eq!(f.handle.selection().head(), Pos(1), "Page Up ends at the start");
    assert_eq!(sc.scroll_top(), 0);
    f.teardown();
}

#[wasm_bindgen_test]
fn shift_page_down_extends_the_selection() {
    let f = Fixture::mount();
    f.start();
    let anchor = f.handle.selection().head();
    let (_, cy, ch) = f.caret();
    let want = expected_blocks(&f.bands(), cy + ch * 0.5 + STEP);
    assert!(key("PageDown", true));
    let sel = f.handle.selection();
    assert_eq!(sel.anchor(), anchor, "Shift keeps the anchor");
    assert!(want.contains(&block_of(&f.handle, sel.head())));
    assert!(key("PageUp", true));
    let sel = f.handle.selection();
    assert_eq!(sel.anchor(), anchor);
    assert_eq!(block_of(&f.handle, sel.head()), 2);
    f.teardown();
}

/// Chrome's read-only `<textarea>`: a page of scroll, the caret left.
#[wasm_bindgen_test]
fn a_read_only_editor_scrolls_a_page_and_keeps_the_caret() {
    let f = Fixture::mount();
    f.start();
    f.handle.set_read_only(true);
    let before = f.handle.selection();
    assert!(key("PageDown", false), "rinch took Page Down");
    assert_eq!(f.handle.selection(), before, "the caret stays");
    assert_eq!(f64::from(f.scroller().scroll_top()), STEP, "a page of scroll");
    f.teardown();
}

/// A page that lands on the document's last line goes to the document's end
/// (Chrome), not to the goal column on that line. The scroller is resized so
/// that a page from block `n - 7`'s caret lands mid-way down the last line.
#[wasm_bindgen_test]
fn a_page_that_lands_on_the_last_line_goes_to_the_end() {
    let f = Fixture::mount();
    f.start();
    let bands = f.bands();
    let n = bands.len();
    let k = n - 7;
    f.handle
        .set_selection(Selection::cursor(pos_in_block(&f.handle, k, 3)));
    let (_, cy, ch) = f.caret();
    let (lt, lb) = bands[n - 1];
    // 0.875 × clientHeight = from the caret's middle to the last line's.
    let client = (((lt + lb) / 2.0 - (cy + ch * 0.5)) / 0.875).round();
    let sc = f.scroller();
    sc.set_attribute(
        "style",
        &format!(
            "box-sizing: content-box; height: {}px; padding: 13px; width: 500px; \
             overflow-y: auto",
            client - 26.0
        ),
    )
    .unwrap();
    // Block `k` at the scroller's top, the last line in view below it.
    let k_top = f.bands()[k].0 - sc.get_bounding_client_rect().top() - 13.0;
    sc.set_scroll_top(k_top.round() as i32);
    let before = sc.scroll_top();
    let (_, cy, ch) = f.caret();
    let (lt, lb) = f.bands()[n - 1];
    let page = (f64::from(sc.client_height()) * 0.875).round();
    let max = sc.scroll_height() - sc.client_height();
    // How far the page's own scroll will get (the rest of the page is the
    // caret's move on screen).
    let scrolled = f64::from((before + page as i32).min(max) - before);
    let target = cy + ch * 0.5 + page;
    assert!(
        target > lt + 2.0 && target < lb - 2.0,
        "control: a page from block {k} lands on the last line ({target} in {lt}..{lb})"
    );
    let window = web_sys::window().unwrap();
    let view = window.inner_height().unwrap().as_f64().unwrap();
    assert!(
        target - scrolled < view,
        "control: the last line is on screen ({} < {view}), so the page \
         resolves a hit there rather than finding none",
        target - scrolled
    );
    assert!(key("PageDown", false));
    assert_eq!(
        f.handle.selection().head(),
        Selection::at_end(&f.handle.doc()).head(),
        "the document's end, not the column on its last line"
    );
    f.teardown();
}

/// The scroller runs past the bottom of the window, and the page's scroll is
/// cut short at the scroller's end: the line a page down is below the window,
/// where `caretPositionFromPoint` finds nothing. The page still lands on that
/// line (the window is scrolled to it), not at the document's end.
#[wasm_bindgen_test]
fn a_page_landing_below_the_window_finds_its_line() {
    let f = Fixture::mount();
    f.start();
    let window = web_sys::window().unwrap();
    let view = window.inner_height().unwrap().as_f64().unwrap();
    let sc = f.scroller();
    // The scroller's top 60px above the window's bottom.
    sc.set_attribute(
        "style",
        &format!(
            "box-sizing: content-box; height: 200px; padding: 13px; width: 500px; \
             overflow-y: auto; margin-top: {view}px"
        ),
    )
    .unwrap();
    let top = sc.get_bounding_client_rect().top() + window.scroll_y().unwrap();
    window.scroll_to_with_x_and_y(0.0, top - (view - 60.0));
    // Short of the scroller's end by `short` px, so the page's scroll stops
    // after `short`, with the caret on a block on screen whose page lands
    // below the window and before the last line.
    let max = sc.scroll_height() - sc.client_height();
    let last = f.bands().len() - 1;
    let mut found = None;
    'search: for short in (30..=160).step_by(10) {
        sc.set_scroll_top(max - short);
        let sc_top = sc.get_bounding_client_rect().top();
        let bands = f.bands();
        for (i, &(t, b)) in bands.iter().enumerate() {
            if t < sc_top + 13.0 || b > view {
                continue;
            }
            f.handle
                .set_selection(Selection::cursor(pos_in_block(&f.handle, i, 6)));
            let (_, cy, ch) = f.caret();
            let target = cy + ch * 0.5 + STEP;
            let want = expected_blocks(&bands, target);
            if target - f64::from(short) > view + 5.0 && !want.is_empty() && !want.contains(&last)
            {
                found = Some((short, want));
                break 'search;
            }
        }
    }
    let (short, want) = found.expect("control: a caret whose page lands below the window");
    assert!(key("PageDown", false));
    assert_eq!(sc.scroll_top(), max, "the scroller went to its end ({short}px)");
    let got = block_of(&f.handle, f.handle.selection().head());
    assert!(
        want.contains(&got),
        "the caret landed in block {got}; a page down is {want:?}"
    );
    window.scroll_to_with_x_and_y(0.0, 0.0);
    f.teardown();
}
