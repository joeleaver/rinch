//! The web half of "scroll the caret into view" (#837): a local caret move
//! scrolls the caret's scroller minimally, a second editor below the fold does
//! not pull the page, and a peer's edit above the caret does not pull a user who
//! scrolled away back to it. Run with `--features collaboration` for that last
//! fixture.
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
fn window() -> web_sys::Window {
    web_sys::window().unwrap()
}

const HOST_MARKER: &str = "data-test-host-r837";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handles: Vec<EditorHandle>,
}

fn content() -> String {
    (0..40).map(|i| format!("<p>line {i}</p>")).collect()
}

impl Fixture {
    /// Editors, each inside its own 120px scroller, separated by `gap` px of
    /// spacer (so the second can sit below the page fold).
    fn mount(n: usize, gap: u32) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        window().scroll_to_with_x_and_y(0.0, 0.0);
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handles: Vec<EditorHandle> = (0..n)
            .map(|_| {
                let h = create_editor();
                assert!(h.load_html(&content()));
                h
            })
            .collect();
        let mounted = handles.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrapper = scope.create_element("div");
                for (i, handle) in mounted.iter().enumerate() {
                    if i > 0 {
                        let spacer = scope.create_element("div");
                        spacer.set_attribute("style", &format!("height: {gap}px"));
                        wrapper.append_child(&spacer);
                    }
                    let scroller = scope.create_element("div");
                    scroller.set_attribute("data-r837-scroller", "");
                    scroller
                        .set_attribute("style", "height: 120px; width: 300px; overflow-y: auto");
                    scroller.append_child(&handle.mount(scope));
                    wrapper.append_child(&scroller);
                }
                wrapper
            },
        );
        Self {
            root,
            host,
            handles,
        }
    }

    fn scroller(&self, i: u32) -> web_sys::Element {
        document()
            .query_selector_all("[data-r837-scroller]")
            .unwrap()
            .item(i)
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    /// Focus editor 0 with a genuine press on its first line.
    fn focus_first(&self) {
        let r = self.scroller(0).get_bounding_client_rect();
        let (x, y) = ((r.x() + 30.0) as f32, (r.y() + 30.0) as f32);
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
    }

    fn caret(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-caret]")
            .unwrap()
            .expect("a caret overlay")
    }

    fn caret_visible_in(&self, scroller: &web_sys::Element) -> bool {
        let c = self.caret().get_bounding_client_rect();
        let s = scroller.get_bounding_client_rect();
        c.top() >= s.top() - 0.5 && c.bottom() <= s.bottom() + 0.5
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        window().scroll_to_with_x_and_y(0.0, 0.0);
    }
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

fn keydown(key: &str, code: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let ta = document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .unwrap();
    ta.dispatch_event(&ev).unwrap();
}

/// The PR's claim on the web: a caret moved below the fold is scrolled into
/// view, minimally (the scroller only; the page does not move).
#[wasm_bindgen_test]
fn a_caret_moved_below_the_fold_is_scrolled_into_view() {
    let f = Fixture::mount(1, 0);
    f.focus_first();
    let sc = f.scroller(0);
    assert_eq!(sc.scroll_top(), 0);
    let end = f.handles[0].doc().content_size() - 1;
    f.handles[0].set_selection(Selection::cursor(Pos(end)));
    keydown("ArrowLeft", "ArrowLeft"); // an input event: refresh_caret runs
    assert!(
        sc.scroll_top() > 0,
        "the scroller scrolled (got {})",
        sc.scroll_top()
    );
    assert!(f.caret_visible_in(&sc), "and the caret is inside it");
    assert_eq!(
        window().scroll_y().unwrap(),
        0.0,
        "the page itself did not move"
    );
    // Gate: a further no-op refresh (Shift alone) does not move anything.
    let before = sc.scroll_top();
    keydown("Shift", "ShiftLeft");
    assert_eq!(sc.scroll_top(), before);
    f.teardown();
}

/// Two editors, the second 3000px below the page fold. Focusing and moving the
/// caret in the first must never scroll the page to the second.
#[wasm_bindgen_test]
fn a_second_editor_below_the_fold_does_not_pull_the_page() {
    let f = Fixture::mount(2, 3000);
    f.focus_first();
    keydown("ArrowDown", "ArrowDown");
    assert_eq!(window().scroll_y().unwrap(), 0.0, "page stayed at the top");
    f.teardown();
}

/// A peer's edit above the local caret moves the caret's geometry but not the
/// user's position. A user who scrolled away must not be yanked back — a
/// contenteditable in a browser is not scrolled by a DOM mutation elsewhere.
#[cfg(feature = "collaboration")]
#[wasm_bindgen_test]
fn a_remote_edit_above_the_caret_does_not_yank_a_scrolled_away_user() {
    let f = Fixture::mount(1, 0);
    f.focus_first();
    let sc = f.scroller(0);
    let local = f.handles[0].clone();
    let end = local.doc().content_size() - 1;
    local.set_selection(Selection::cursor(Pos(end)));
    keydown("ArrowLeft", "ArrowLeft");
    assert!(
        sc.scroll_top() > 0,
        "positive control: caret scrolled into view"
    );

    // Peer hosts; the mounted editor joins.
    let peer = create_editor();
    assert!(peer.load_html(&content()));
    let local_in = local.clone();
    let snapshot = peer
        .start_collaboration_host(move |delta| {
            local_in.collab_receive(&delta);
        })
        .unwrap();
    local.start_collaboration_guest(&snapshot, |_| {}).unwrap();
    // Re-place the local caret at the end (the guest join reloaded the doc).
    let end = local.doc().content_size() - 1;
    local.set_selection(Selection::cursor(Pos(end)));
    keydown("ArrowLeft", "ArrowLeft");

    // The user scrolls the editor back to the top.
    sc.set_scroll_top(0);
    assert_eq!(sc.scroll_top(), 0);

    // Peer types a long run into the FIRST paragraph (above the caret).
    peer.set_selection(Selection::cursor(Pos(1)));
    assert!(peer.insert_text(&"peer text that wraps ".repeat(6)));
    let first = document()
        .query_selector("[data-pm-editor] p")
        .unwrap()
        .unwrap();
    assert!(
        first.text_content().unwrap().contains("peer text"),
        "positive control: the remote edit arrived"
    );

    assert_eq!(
        sc.scroll_top(),
        0,
        "a remote edit above the caret must not scroll the user back (got {})",
        sc.scroll_top()
    );
    local.stop_collaboration();
    f.teardown();
}

/// review-846: a click in a task item's checkbox gutter at the TOP of an
/// unfocused editor toggles the box and does not move the caret. It must not
/// scroll the editor to the old caret at the bottom.
#[wasm_bindgen_test]
fn r846_a_focusing_checkbox_click_does_not_yank_to_the_old_caret() {
    let f = Fixture::mount(1, 0);
    // An ordinary text field outside the editor, to take focus away.
    let other = document().create_element("input").unwrap();
    other
        .set_attribute(
            "style",
            "position: fixed; left: 500px; top: 10px; width: 100px; height: 20px",
        )
        .unwrap();
    f.host.append_child(&other).unwrap();
    f.focus_first();
    let h = f.handles[0].clone();
    h.set_selection(Selection::cursor(Pos(1)));
    for c in ["[", " ", "]", " "] {
        assert!(h.insert_text(c));
    }
    assert_eq!(
        h.doc().child(0).type_name(),
        "task_list",
        "control: a task list"
    );
    let sc = f.scroller(0);
    let end = h.doc().content_size() - 1;
    h.set_selection(Selection::cursor(Pos(end)));
    keydown("ArrowLeft", "ArrowLeft");
    assert!(sc.scroll_top() > 0, "control: caret scrolled into view");
    // Focus the other field (a genuine press on it).
    let r = other.get_bounding_client_rect();
    mouse("mousedown", (r.x() + 5.0) as f32, (r.y() + 5.0) as f32);
    mouse("mouseup", (r.x() + 5.0) as f32, (r.y() + 5.0) as f32);
    // Any later refresh (e.g. a mousedown anywhere) runs the caret pass.
    sc.set_scroll_top(0);
    assert_eq!(sc.scroll_top(), 0);
    let item = document()
        .query_selector("[data-pm-type='task_item']")
        .unwrap()
        .expect("a task item");
    let content = item
        .first_element_child()
        .unwrap()
        .get_bounding_client_rect();
    let (x, y) = (
        (content.left() - 6.0) as f32,
        (content.top() + content.height() / 2.0) as f32,
    );
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    let checked = h
        .doc()
        .child(0)
        .child(0)
        .attrs()
        .get_bool("checked")
        .unwrap_or(false);
    assert!(checked, "control: the click toggled the box");
    let top = sc.scroll_top();
    other.remove();
    f.teardown();
    assert_eq!(
        top, 0,
        "the checkbox click yanked the scroller to the old caret (got {top})"
    );
}

/// review-846: the same, with focus taken away by a SECOND editor (whose
/// caret pass hides the first editor's overlays, so the first's re-focus is a
/// "focus gained" to the handle).
#[wasm_bindgen_test]
fn r846_a_checkbox_click_refocusing_from_another_editor_does_not_yank() {
    let f = Fixture::mount(2, 0);
    f.focus_first();
    let h = f.handles[0].clone();
    h.set_selection(Selection::cursor(Pos(1)));
    for c in ["[", " ", "]", " "] {
        assert!(h.insert_text(c));
    }
    assert_eq!(
        h.doc().child(0).type_name(),
        "task_list",
        "control: a task list"
    );
    let sc = f.scroller(0);
    let end = h.doc().content_size() - 1;
    h.set_selection(Selection::cursor(Pos(end)));
    keydown("ArrowLeft", "ArrowLeft");
    assert!(sc.scroll_top() > 0, "control: caret scrolled into view");
    // Focus editor 1 with a press on its first line.
    let r1 = f.scroller(1).get_bounding_client_rect();
    let (x1, y1) = ((r1.x() + 40.0) as f32, (r1.y() + 12.0) as f32);
    mouse("mousedown", x1, y1);
    mouse("mouseup", x1, y1);
    sc.set_scroll_top(0);
    assert_eq!(sc.scroll_top(), 0);
    let item = document()
        .query_selector("[data-pm-type='task_item']")
        .unwrap()
        .expect("a task item");
    let content = item
        .first_element_child()
        .unwrap()
        .get_bounding_client_rect();
    let (x, y) = (
        (content.left() - 6.0) as f32,
        (content.top() + content.height() / 2.0) as f32,
    );
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    let checked = h
        .doc()
        .child(0)
        .child(0)
        .attrs()
        .get_bool("checked")
        .unwrap_or(false);
    let top = sc.scroll_top();
    f.teardown();
    assert!(checked, "control: the click toggled the box");
    assert_eq!(
        top, 0,
        "the checkbox click yanked the scroller to the old caret (got {top})"
    );
}
