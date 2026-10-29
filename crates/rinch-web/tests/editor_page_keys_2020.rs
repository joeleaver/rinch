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

#[path = "support/engine.rs"]
mod engine;

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

/// A failed test leaves its host behind: take every one off the page.
fn remove_stale_hosts() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

impl Fixture {
    fn mount() -> Self {
        Self::mount_with(false)
    }

    /// `lock_other`: a sibling of the scroller holds a scroll lock
    /// (`lock_scroll`), which locks the scroller out.
    fn mount_with(lock_other: bool) -> Self {
        remove_stale_hosts();
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
                if !lock_other {
                    return scroller;
                }
                let wrap = scope.create_element("div");
                wrap.append_child(&scroller);
                let other = scope.create_element("div");
                wrap.append_child(&other);
                other.set_scroll_locked(true);
                wrap
            },
        );
        // The runner's page has its log above: the fixture at the top.
        host.scroll_into_view();
        let f = Self { root, host, handle };
        assert_eq!(
            f.scroller().client_height() as f64,
            PAGE_BOX,
            "control: the page box"
        );
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

/// `scrollTop` as the browser answers it, unrounded: Firefox 158's scroll
/// positions are fractional, and web-sys's `scroll_top()` truncates them.
fn scroll_top(sc: &web_sys::Element) -> f64 {
    js_sys::Reflect::get(sc, &"scrollTop".into())
        .ok()
        .and_then(|v| v.as_f64())
        .expect("control: scrollTop is a number")
}

/// Set `scrollTop` unrounded (web-sys's `set_scroll_top` takes an `i32`).
fn set_scroll_top(sc: &web_sys::Element, top: f64) {
    js_sys::Reflect::set(sc, &"scrollTop".into(), &top.into()).unwrap();
}

/// The scroller's end: the largest `scrollTop` the browser lets it take, asked
/// of the browser (set past the end, read back unrounded, put back). Not
/// `scrollHeight - clientHeight` in every engine. Chrome's end is
/// `scrollHeight - clientHeight` (155 and 156). Firefox's is its
/// `scrollTopMax`, and in Firefox 158 (CI's, batch #2042) both are
/// fractional: 1463.6 here where `scrollHeight - clientHeight` is 1464, and
/// 1409.6 for the last-line test's scroller (Firefox 155 answered whole
/// pixels, as Chrome does). `scrollTopMax` came back as 1463.5999755859375, a
/// float's precision, so it is compared to within one app unit (1/60px).
fn scroll_end(sc: &web_sys::Element) -> f64 {
    let was = scroll_top(sc);
    sc.set_scroll_top(i32::MAX);
    let end = scroll_top(sc);
    set_scroll_top(sc, was);
    let span = f64::from(sc.scroll_height() - sc.client_height());
    if engine::is_gecko() {
        let top_max = js_sys::Reflect::get(sc, &"scrollTopMax".into())
            .ok()
            .and_then(|v| v.as_f64())
            .expect("control: Firefox has scrollTopMax");
        assert!(
            (top_max - end).abs() <= 1.0 / 60.0,
            "control: Firefox's scrollTopMax ({top_max}) is the end ({end})"
        );
        assert!(
            (0.0..1.0).contains(&(span - end)),
            "control: Firefox's end {end} is scrollHeight - clientHeight ({span}) \
             or less than a pixel short of it"
        );
    } else {
        assert_eq!(
            end, span,
            "control: Chrome's end is scrollHeight - clientHeight"
        );
    }
    end
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
    assert_eq!(
        f64::from(sc.scroll_top()),
        STEP,
        "the scroller moved a page"
    );
    let (nx, ny, _) = f.caret();
    assert!((nx - cx).abs() < 8.0, "the column held: x {cx} -> {nx}");
    assert!(
        (ny - cy).abs() < 30.0,
        "the caret kept its place: y {cy} -> {ny}"
    );

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
    assert_eq!(
        f.handle.selection().head(),
        end,
        "Page Down ends at the end"
    );
    let sc = f.scroller();
    assert_eq!(
        scroll_top(&sc),
        scroll_end(&sc),
        "the scroller is at its end"
    );
    for _ in 0..12 {
        key("PageUp", false);
    }
    assert_eq!(
        f.handle.selection().head(),
        Pos(1),
        "Page Up ends at the start"
    );
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
    assert_eq!(
        f64::from(f.scroller().scroll_top()),
        STEP,
        "a page of scroll"
    );
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
    let before = scroll_top(&sc);
    let (_, cy, ch) = f.caret();
    let (lt, lb) = f.bands()[n - 1];
    let page = (f64::from(sc.client_height()) * 0.875).round();
    let max = scroll_end(&sc);
    // How far the page's own scroll will get (the rest of the page is the
    // caret's move on screen).
    let scrolled = (before + page).min(max) - before;
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
    let max = scroll_end(&sc);
    let last = f.bands().len() - 1;
    let mut found = None;
    'search: for short in (30..=160).step_by(10) {
        set_scroll_top(&sc, max - f64::from(short));
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
            if target - f64::from(short) > view + 5.0 && !want.is_empty() && !want.contains(&last) {
                found = Some((short, want));
                break 'search;
            }
        }
    }
    let (short, want) = found.expect("control: a caret whose page lands below the window");
    assert!(key("PageDown", false));
    assert_eq!(
        scroll_top(&sc),
        max,
        "the scroller went to its end ({short}px)"
    );
    let got = block_of(&f.handle, f.handle.selection().head());
    assert!(
        want.contains(&got),
        "the caret landed in block {got}; a page down is {want:?}"
    );
    window.scroll_to_with_x_and_y(0.0, 0.0);
    f.teardown();
}

/// Where Page Down (Up) from `(block, 6)` lands with the caret in view: a
/// fresh fixture, the scroller placed so the caret is in its view.
fn in_view_landing(block: usize, page_key: &str) -> Pos {
    let f = Fixture::mount();
    f.start();
    f.handle
        .set_selection(Selection::cursor(pos_in_block(&f.handle, block, 6)));
    let sc = f.scroller();
    let top = f.bands()[block].0 - sc.get_bounding_client_rect().top() - 13.0;
    sc.set_scroll_top((top - 60.0).max(0.0).round() as i32);
    assert!(key(page_key, false));
    let head = f.handle.selection().head();
    f.teardown();
    head
}

/// The caret's line is inside the scroller's padding box.
fn assert_caret_in_view(f: &Fixture, what: &str) {
    let (_, y, h) = f.caret();
    let r = f.scroller().get_bounding_client_rect();
    assert!(
        y >= r.top() - 0.5 && y + h <= r.bottom() + 0.5,
        "{what}: the caret ({y}..{}) is in the scroller's view ({}..{})",
        y + h,
        r.top(),
        r.bottom()
    );
}

/// Review of #2028, F1 (web): the caret scrolled out of the scroller's view
/// above (the wheel), then Page Down. Chrome 155 and Firefox 155 move the
/// caret a page from where it is, wherever the view is; the page used to
/// resolve on screen, miss, and jump to the document's end (scroll 1418).
#[wasm_bindgen_test]
fn page_down_with_the_caret_scrolled_above_the_view_pages_from_the_caret() {
    let want = in_view_landing(2, "PageDown");
    let f = Fixture::mount();
    f.start();
    let sc = f.scroller();
    sc.set_scroll_top(400);
    let (_, cy, _) = f.caret();
    assert!(
        cy < sc.get_bounding_client_rect().top(),
        "control: the caret ({cy}) is above the view"
    );
    assert!(
        (3..30).contains(&block_of(&f.handle, want)),
        "control: an in-view page lands mid-document ({want:?})"
    );
    assert!(key("PageDown", false), "rinch took Page Down");
    assert_eq!(
        f.handle.selection().head(),
        want,
        "the page from the off-view caret lands where it does in view"
    );
    assert_caret_in_view(&f, "PageDown");
    f.teardown();
}

/// F1 (web): the caret below the view, then Page Up.
#[wasm_bindgen_test]
fn page_up_with_the_caret_scrolled_below_the_view_pages_from_the_caret() {
    let want = in_view_landing(30, "PageUp");
    let f = Fixture::mount();
    f.start();
    f.handle
        .set_selection(Selection::cursor(pos_in_block(&f.handle, 30, 6)));
    let sc = f.scroller();
    sc.set_scroll_top(0);
    let (_, cy, _) = f.caret();
    assert!(
        cy > sc.get_bounding_client_rect().bottom(),
        "control: the caret ({cy}) is below the view"
    );
    assert!(
        (3..30).contains(&block_of(&f.handle, want)),
        "control: {want:?}"
    );
    assert!(key("PageUp", false));
    assert_eq!(
        f.handle.selection().head(),
        want,
        "the page from the off-view caret lands where it does in view"
    );
    assert_caret_in_view(&f, "PageUp");
    f.teardown();
}

/// A host holding an editor (10 blocks in a fixed box, else 60) in a box —
/// `fixed`: `position: fixed`
/// (a modal's root) — over a page made 3000px taller, with — `lock`: a scroll
/// lock held by another element (rinch's `lock_scroll`).
fn mount_over_tall_page(fixed: bool, lock: bool) -> (web_sys::Element, RootHandle, EditorHandle) {
    remove_stale_hosts();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 1.25;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    // Short in the fixed box, so a page runs past it; long on the page, so a
    // page lands inside it.
    let n = if fixed { 10 } else { 60 };
    let ten: String = (0..n)
        .map(|i| format!("<p>abcdefghij {i:02}</p>"))
        .collect();
    assert!(handle.load_html(&ten));
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let page = scope.create_element("div");
            let boxed = scope.create_element("div");
            if fixed {
                boxed.set_attribute(
                    "style",
                    "position: fixed; top: 20px; left: 20px; width: 400px; \
                     background: white",
                );
            }
            boxed.append_child(&mounted.mount(scope));
            page.append_child(&boxed);
            let tall = scope.create_element("div");
            tall.set_attribute("style", "height: 3000px");
            page.append_child(&tall);
            if lock {
                tall.set_scroll_locked(true);
            }
            page
        },
    );
    web_sys::window().unwrap().scroll_to_with_x_and_y(0.0, 0.0);
    (host, root, handle)
}

fn focus_editor_at(handle: &EditorHandle, block: usize) {
    let ed = document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-editor]"))
        .unwrap()
        .unwrap();
    let r = ed.get_bounding_client_rect();
    let (x, y) = ((r.x() + 30.0) as f32, (r.y() + 8.0) as f32);
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    handle.set_selection(Selection::cursor(pos_in_block(handle, block, 6)));
}

/// Review of #2028, F2 (web): an editor in a `position: fixed` box (a modal)
/// over a page that scrolls. The walk stops at the fixed box, so Page Down
/// leaves the page and takes the short editor's caret to its end — it used to
/// page the window and leave the caret where it was.
#[wasm_bindgen_test]
fn page_down_in_a_fixed_box_leaves_the_page_behind_it() {
    let (host, root, handle) = mount_over_tall_page(true, false);
    let window = web_sys::window().unwrap();
    focus_editor_at(&handle, 2);
    assert_eq!(window.scroll_y().unwrap(), 0.0, "control: the page at 0");
    let doc_el = document().document_element().unwrap();
    assert!(
        doc_el.scroll_height() > doc_el.client_height() + 1000,
        "control: the page scrolls"
    );
    assert!(key("PageDown", false), "rinch took Page Down");
    assert_eq!(window.scroll_y().unwrap(), 0.0, "the page did not move");
    assert_eq!(
        handle.selection().head(),
        Selection::at_end(&handle.doc()).head(),
        "the caret went to the short editor's end"
    );
    root.unmount();
    host.remove();
}

/// F2 (web): the scroll lock alone, no fixed box: an editor on a page that a
/// `lock_scroll` on another element holds. Page Down leaves the page (the
/// drag auto-scroll's rule) and moves the caret a page.
#[wasm_bindgen_test]
fn page_down_on_a_locked_page_leaves_the_page() {
    let (host, root, handle) = mount_over_tall_page(false, true);
    let window = web_sys::window().unwrap();
    assert_eq!(
        rinch_web::__scroll_lock_depth(),
        1,
        "control: the lock is held"
    );
    focus_editor_at(&handle, 0);
    let before = handle.selection().head();
    assert!(key("PageDown", false), "rinch took Page Down");
    assert_eq!(
        window.scroll_y().unwrap(),
        0.0,
        "the locked page did not move"
    );
    assert_ne!(handle.selection().head(), before, "the caret moved");
    root.unmount();
    host.remove();
    rinch_web::__reset_scroll_lock();
}

/// F2 (web): a scroller the lock leaves out is not paged either. Read-only,
/// so the page is the scroll alone: a moved caret would be revealed, and the
/// reveal is not the page key's (it ignores the lock, as an arrow key's does).
#[wasm_bindgen_test]
async fn page_down_leaves_a_locked_out_scroller() {
    let f = Fixture::mount_with(true);
    f.start();
    f.handle.set_read_only(true);
    settle_reveal().await;
    let depth = rinch_web::__scroll_lock_depth();
    let took = key("PageDown", false);
    let top = f.scroller().scroll_top();
    // Released before asserting: a failure must not leave the page locked
    // for the tests after it.
    f.teardown();
    rinch_web::__reset_scroll_lock();
    assert_eq!(depth, 1, "control: the lock was held");
    assert!(took, "rinch took Page Down");
    assert_eq!(top, 0, "the locked-out scroller kept still");
}

/// The control for both: with neither, Page Down pages the window.
#[wasm_bindgen_test]
async fn control_page_down_on_an_open_page_pages_the_window() {
    let (host, root, handle) = mount_over_tall_page(false, false);
    let window = web_sys::window().unwrap();
    focus_editor_at(&handle, 0);
    settle_reveal().await;
    assert!(key("PageDown", false));
    let y = window.scroll_y().unwrap();
    // The page's scrollport is the scrolling element's `clientHeight` (the
    // window less any horizontal scrollbar).
    let port = f64::from(document().document_element().unwrap().client_height());
    assert!(
        (y - (port * 0.875).round()).abs() <= 1.0,
        "a page of the window: {y} (scrollport {port})"
    );
    window.scroll_to_with_x_and_y(0.0, 0.0);
    root.unmount();
    host.remove();
}

/// Let the caret reveal a selection change queued for a microtask run, as
/// it has long before a user presses a key.
async fn settle_reveal() {
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL))
        .await
        .unwrap();
}

fn key_alt(name: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(name);
    init.set_code(name);
    init.set_alt_key(true);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let ta = document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .unwrap();
    !ta.dispatch_event(&ev).unwrap()
}

/// macOS (through the test seam, `__set_mac_platform`): the plain key scrolls
/// a page and leaves the caret; Option moves it — Chrome's bindings there.
#[wasm_bindgen_test]
async fn on_macos_page_down_scrolls_and_option_page_down_moves() {
    rinch_web::__set_mac_platform(Some(true));
    let f = Fixture::mount();
    f.start();
    // `start`'s selection queues the caret's reveal for a microtask: let it
    // run, or the key's caret refresh runs it after the page and scrolls
    // back to the caret.
    settle_reveal().await;
    assert_eq!(
        f.scroller().scroll_top(),
        0,
        "control: the caret is in view at 0"
    );
    let before = f.handle.selection();
    assert!(key("PageDown", false), "rinch took Page Down");
    assert_eq!(
        f.handle.selection(),
        before,
        "the plain key leaves the caret"
    );
    assert_eq!(
        f64::from(f.scroller().scroll_top()),
        STEP,
        "and scrolls a page"
    );
    f.scroller().set_scroll_top(0);
    assert!(key_alt("PageDown"), "rinch took Option+Page Down");
    assert_ne!(
        block_of(&f.handle, f.handle.selection().head()),
        2,
        "Option moves the caret"
    );
    rinch_web::__set_mac_platform(Some(false));
    // Control: off macOS, Alt+Page Down is the browser's, and the plain key moves.
    f.handle
        .set_selection(Selection::cursor(pos_in_block(&f.handle, 2, 6)));
    f.scroller().set_scroll_top(0);
    assert!(
        !key_alt("PageDown"),
        "off macOS Alt+Page Down is not rinch's"
    );
    rinch_web::__set_mac_platform(None);
    f.teardown();
}
