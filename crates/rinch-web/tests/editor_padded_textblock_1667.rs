//! The editor's caret in a padded textblock (a code block), on the web
//! (#1667).
//!
//! The browser answers the block-local caret relative to the block's
//! `getBoundingClientRect` — its border box — so a caret in a code block
//! with text was always drawn on its glyphs here (the desktop half of #1667
//! was rinch-dom answering from the content box); that case is a control.
//! An **empty** code block has no text to ask, and the view placed its caret
//! at the block's border-box corner, as tall as the padded box, on both
//! backends; it now goes to the content origin
//! (`DomDocument::content_box_inset`), one line tall.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_padded_textblock_1667
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1667";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl F {
    /// `html` (starting with `<p>x</p>`) in a 400px editor, 16px / 24px,
    /// focused by a press on the `x`.
    fn new(html: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
            for i in 0..stale.length() {
                if let Some(n) = stale.item(i) {
                    n.dyn_into::<web_sys::Element>().unwrap().remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(html));
        let m = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |s: &mut RenderScope| m.mount(s),
        );
        let f = Self { root, host, handle };
        let p = document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("the paragraph");
        let r = char_rect(&p.first_child().expect("text"), 0);
        mouse("mousedown", r.x() + 1.0, r.y() + r.height() / 2.0);
        mouse("mouseup", r.x() + 1.0, r.y() + r.height() / 2.0);
        let cap = document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap();
        assert!(
            document().active_element().as_ref() == Some(&cap),
            "positive control: a press reached rinch and focused the editor"
        );
        f
    }
    fn pre(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] pre")
            .unwrap()
            .expect("the code block")
    }
    /// The code block's content origin: its border box moved in by its
    /// padding and border.
    fn content_origin(&self) -> (f64, f64) {
        let pre = self.pre();
        let r = pre.get_bounding_client_rect();
        let cs = web_sys::window()
            .unwrap()
            .get_computed_style(&pre)
            .unwrap()
            .unwrap();
        let px = |p: &str| -> f64 {
            cs.get_property_value(p)
                .unwrap()
                .trim_end_matches("px")
                .parse()
                .unwrap()
        };
        assert!(
            px("padding-left") > 10.0,
            "positive control: the code block is padded"
        );
        (
            r.x() + px("padding-left") + px("border-left-width"),
            r.y() + px("padding-top") + px("border-top-width"),
        )
    }
    /// The code block's computed `line-height` (the editor's stylesheet
    /// sets `1.5` on its `0.9em` font).
    fn line(&self) -> f64 {
        let cs = web_sys::window()
            .unwrap()
            .get_computed_style(&self.pre())
            .unwrap()
            .unwrap();
        cs.get_property_value("line-height")
            .unwrap()
            .trim_end_matches("px")
            .parse()
            .expect("a px line-height")
    }
    fn caret(&self) -> web_sys::DomRect {
        document()
            .query_selector("[data-pm-editor] [data-pm-caret]")
            .unwrap()
            .expect("a caret overlay")
            .get_bounding_client_rect()
    }
    /// The caret at `pos`, and the overlay pass the selection owes (a
    /// microtask on the web) run.
    async fn at(&self, pos: usize) {
        self.handle.set_selection(Selection::cursor(Pos(pos)));
        next_task().await;
    }
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
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

fn char_rect(t: &web_sys::Node, i: u32) -> web_sys::DomRect {
    let r = document().create_range().unwrap();
    r.set_start(t, i).unwrap();
    r.set_end(t, i + 1).unwrap();
    r.get_bounding_client_rect()
}

fn mouse(name: &str, x: f64, y: f64) {
    let target = document()
        .element_from_point(x as f32, y as f32)
        .expect("hit");
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

/// Control: in a code block with text the caret is drawn on its first
/// glyph's left edge, and `caret_rect` agrees.
#[wasm_bindgen_test]
async fn the_caret_in_a_code_block_with_text_sits_on_its_text() {
    let f = F::new("<p>x</p><pre>bullet</pre>");
    let (cx, _) = f.content_origin();
    // `<p>x</p>` is 0..3, the code block's text starts at 4.
    f.at(4).await;
    let g = char_rect(&f.pre().first_child().expect("text"), 0);
    assert!(
        (g.x() - cx).abs() < 1.0,
        "the text starts at the content origin"
    );
    let caret = f.caret();
    assert!(
        (caret.x() - g.x()).abs() <= 1.0,
        "the caret overlay is at x {}, the text starts at {}",
        caret.x(),
        g.x()
    );
    assert!(
        caret.y() >= g.y() - 4.0 && caret.y() <= g.y() + 2.0,
        "the caret overlay is at y {}, the glyph's top {}",
        caret.y(),
        g.y()
    );
    let rect = f.handle.caret_rect(Pos(4)).expect("a caret rect");
    assert!(
        (rect.x as f64 - g.x()).abs() <= 1.0,
        "caret_rect x {}",
        rect.x
    );
    f.done();
}

/// An empty code block: the caret is drawn at its content origin, one line
/// tall, and `caret_rect` answers the same.
#[wasm_bindgen_test]
async fn the_caret_in_an_empty_code_block_sits_at_its_content_origin() {
    let f = F::new("<p>x</p><pre></pre>");
    let (cx, cy) = f.content_origin();
    f.at(4).await;
    let caret = f.caret();
    assert!(
        (caret.x() - cx).abs() <= 1.0 && (caret.y() - cy).abs() <= 1.0,
        "the caret overlay is at ({}, {}), the content origin ({cx}, {cy})",
        caret.x(),
        caret.y()
    );
    assert!(
        (caret.height() - f.line()).abs() <= 1.0,
        "one line tall, not the padded box's height: {}",
        caret.height()
    );
    let rect = f.handle.caret_rect(Pos(4)).expect("a caret rect");
    assert!(
        (rect.x as f64 - cx).abs() <= 0.5
            && (rect.y as f64 - cy).abs() <= 0.5
            && (rect.height as f64 - f.line()).abs() < 0.5,
        "caret_rect answers {rect:?}, the content origin is ({cx}, {cy})"
    );
    f.done();
}
