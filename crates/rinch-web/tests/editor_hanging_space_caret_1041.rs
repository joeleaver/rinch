//! The caret after preserved spaces that hang past a line's end is drawn at
//! the textblock's border-box edge on the web too (#1041).
//!
//! The browser hangs the spaces (CSS Text 3 §4.1.3) and a collapsed `Range`
//! after them reports their true x, past the block; the editor draws its
//! caret overlay from that, so it was painted outside the paragraph. Chrome
//! 155 draws its own caret at the block's border-box edge less the caret's
//! width, except in a block that is itself a scroll container (measured with
//! headless screenshots, see the desktop twin
//! `rinch/src/app/editor_hanging_space_caret_1041_tests.rs`). Only the
//! overlay is kept in: `caret_rect` keeps the Range's x.
//!
//! The spaces hanging past the paragraph is the positive control, measured
//! in Chrome; Firefox's Range there is unmeasured, so in Gecko a Range that
//! does not report the hanging spaces past the block leaves only the "caret
//! inside the block" half to check.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_hanging_space_caret_1041
//! ```
#![cfg(target_arch = "wasm32")]

#[path = "support/engine.rs"]
mod engine;

use rinch_core::dom::{CARET_WIDTH, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-hanging-caret";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    style: web_sys::Element,
}

impl Fixture {
    /// One editor 130px wide over `<p>bullet text</p>` with the extra author
    /// `css`, focused by a real press, the caret at the text's end.
    fn new(css: &str) -> Self {
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
        let style = document().create_element("style").unwrap();
        style.set_attribute(HOST_MARKER, "").unwrap();
        style.set_text_content(Some(css));
        document().head().unwrap().append_child(&style).unwrap();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 20px; width: 130px; \
             margin-left: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html("<p>bullet text</p>"));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self {
            root,
            host,
            handle,
            style,
        };
        let r = f.para().get_bounding_client_rect();
        let (x, y) = ((r.x() + 4.0) as f32, (r.y() + r.height() / 2.0) as f32);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press focuses the capture textarea"
        );
        f.handle.set_selection(Selection::cursor(Pos(12)));
        f
    }

    fn para(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("the paragraph")
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea")
            .dyn_into()
            .unwrap()
    }

    fn caret(&self) -> web_sys::DomRect {
        document()
            .query_selector("[data-pm-editor] [data-pm-caret]")
            .unwrap()
            .expect("a caret overlay")
            .get_bounding_client_rect()
    }

    /// A real Space keydown on the capture textarea.
    fn space(&self) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(" ");
        init.set_code("Space");
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
    }

    /// Twelve spaces; the caret overlay's `(x, right, y)` before and after each.
    fn type_spaces(&self) -> Vec<(f64, f64, f64)> {
        let at = |r: web_sys::DomRect| (r.x(), r.right(), r.y());
        let mut out = vec![at(self.caret())];
        for _ in 0..12 {
            self.space();
            out.push(at(self.caret()));
        }
        assert_eq!(
            self.para().text_content().unwrap(),
            format!("bullet text{}", " ".repeat(12)),
            "positive control: the spaces were typed"
        );
        out
    }

    /// Where the typed spaces end: a collapsed Range after them.
    fn spaces_end(&self) -> f64 {
        let text = self.para().first_child().unwrap();
        let range = document().create_range().unwrap();
        range.set_start(&text, 23).unwrap();
        range.set_end(&text, 23).unwrap();
        range.get_bounding_client_rect().x()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        self.style.remove();
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

#[wasm_bindgen_test]
fn the_caret_after_spaces_hanging_past_a_paragraphs_end_stays_at_its_end() {
    let f = Fixture::new("");
    let carets = f.type_spaces();
    let right = f.para().get_bounding_client_rect().right();
    let spaces_end = f.spaces_end();
    let hangs = spaces_end > right + 20.0;
    assert!(
        hangs || engine::is_gecko(),
        "the spaces hang well past the paragraph ({spaces_end} vs {right})"
    );
    assert!(
        carets[1].0 > carets[0].0 && carets[2].0 > carets[1].0,
        "the first spaces move the caret: {carets:?}"
    );
    for (n, c) in carets.iter().enumerate() {
        assert!(
            c.2 == carets[0].2,
            "space {n}: the caret stays on the line: {carets:?}"
        );
        assert!(
            c.1 <= right + 0.5,
            "space {n}: the caret {c:?} is painted inside the paragraph (right {right})"
        );
    }
    let last = carets.last().unwrap();
    assert!(
        !hangs
            || ((last.1 - right).abs() <= 0.5
                && (last.1 - last.0 - CARET_WIDTH as f64).abs() < 0.01),
        "the last caret ends at the paragraph's right edge {right}: {last:?}"
    );
    let rect = f.handle.caret_rect(Pos(24)).expect("caret rect");
    assert!(
        !hangs || (rect.x as f64 - spaces_end).abs() <= 1.0,
        "caret_rect {rect:?} keeps the Range's x {spaces_end}"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_paragraph_that_clips_its_overflow_does_not_keep_its_caret_in() {
    let f = Fixture::new("[data-pm-editor] p { overflow: hidden; }");
    let carets = f.type_spaces();
    let right = f.para().get_bounding_client_rect().right();
    let spaces_end = f.spaces_end();
    if spaces_end <= right + 20.0 && engine::is_gecko() {
        f.teardown();
        return;
    }
    let last = carets.last().unwrap();
    assert!(
        last.0 > right + 20.0 && (last.0 - spaces_end).abs() <= 1.0,
        "the caret goes on to where the spaces end ({spaces_end}), past {right}: {carets:?}"
    );
    f.teardown();
}
