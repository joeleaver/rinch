//! A downstream caret at a soft-hyphen wrap is drawn on the lower line (#1115).
//!
//! After a hyphenated break at a soft hyphen (U+00AD) the range over the first
//! character of the next line also covers the break, so its **bounding** rect
//! spans both lines and starts on the upper one. `text_caret_viewport_rect`
//! read that as "the character after the position is on the same line" and
//! answered the upstream caret: the end of the line above. It takes the glyph's
//! own box (the last client rect) now.
//!
//! Nothing here pins a font's advances: the wraps are found from the glyphs'
//! own boxes on whatever monospace face the host has, and every soft-hyphen
//! wrap the text makes is checked (a positive control asserts there is one).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test caret_soft_hyphen_wrap_1115
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{CaretAffinity, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-shy-1115";

/// The issue's text: words joined by soft hyphens, wrapped at 250px.
const TEXT: &str = "extra\u{AD}ordinary super\u{AD}cali\u{AD}fragilistic\u{AD}expi\u{AD}ali\u{AD}docious \
                    hyper\u{AD}ventil\u{AD}ation counter\u{AD}revolu\u{AD}tionary";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn mounted() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(el) = stale.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; \
             padding: 20px; width: 250px; hyphens: manual;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(&format!("<p>{TEXT}</p>")));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self { root, host, handle }
    }

    fn text_node(&self) -> web_sys::Node {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("a paragraph")
            .first_child()
            .expect("a text node")
    }

    /// The glyph box of char `i`: the LAST client rect of its range.
    fn glyph(&self, i: u32) -> web_sys::DomRect {
        let range = document().create_range().unwrap();
        let text = self.text_node();
        range.set_start(&text, i).unwrap();
        range.set_end(&text, i + 1).unwrap();
        let list = range.get_client_rects().unwrap();
        assert!(list.length() > 0, "char {i} has a box");
        list.item(list.length() - 1).unwrap()
    }

    /// The bounding rect of char `i`'s range.
    fn bounding(&self, i: u32) -> web_sys::DomRect {
        let range = document().create_range().unwrap();
        let text = self.text_node();
        range.set_start(&text, i).unwrap();
        range.set_end(&text, i + 1).unwrap();
        range.get_bounding_client_rect()
    }

    /// Char offsets that start a visual line right after a soft hyphen.
    fn shy_wraps(&self) -> Vec<u32> {
        let chars: Vec<char> = TEXT.chars().collect();
        assert_eq!(
            self.text_node().text_content().unwrap().chars().count(),
            chars.len(),
            "the paragraph holds the text, soft hyphens included"
        );
        (1..chars.len() as u32)
            .filter(|&i| {
                chars[i as usize - 1] == '\u{AD}'
                    && self.glyph(i).top() > self.glyph(i - 2).top() + 1.0
            })
            .collect()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[wasm_bindgen_test]
fn a_downstream_caret_at_a_soft_hyphen_wrap_is_on_the_lower_line() {
    let f = Fixture::mounted();
    let wraps = f.shy_wraps();
    assert!(
        !wraps.is_empty(),
        "positive control: the text wraps at a soft hyphen at 250px"
    );
    for &i in &wraps {
        let g = f.glyph(i);
        // Positive control: this is the shape of the bug — the bounding rect of
        // the line's first character starts on the line above.
        assert!(
            f.bounding(i).top() < g.top() - 1.0,
            "char {i}: its bounding rect spans the break (the #1115 shape)"
        );
        let pos = Pos(i as usize + 1);
        let down = f
            .handle
            .caret_rect_with_affinity(pos, CaretAffinity::Downstream)
            .expect("a downstream caret rect");
        assert!(
            (down.y as f64 - g.top()).abs() < 1.0 && (down.x as f64 - g.left()).abs() < 1.0,
            "char {i}: the downstream caret is at the start of the lower line \
             ({}, {}), got ({}, {})",
            g.left(),
            g.top(),
            down.x,
            down.y
        );
        let up = f
            .handle
            .caret_rect_with_affinity(pos, CaretAffinity::Upstream)
            .expect("an upstream caret rect");
        assert!(
            (up.y as f64) < g.top() - 1.0,
            "char {i}: the upstream caret stays on the upper line (y {}, lower line {})",
            up.y,
            g.top()
        );
    }
    f.teardown();
}

/// Home from inside the line after a soft-hyphen wrap lands at that line's
/// first character, not one in (the #1113 consequence the issue names).
#[wasm_bindgen_test]
fn home_on_the_line_after_a_soft_hyphen_wrap_lands_at_its_start() {
    let f = Fixture::mounted();
    let wraps = f.shy_wraps();
    assert!(!wraps.is_empty(), "positive control: a soft-hyphen wrap");
    // Focus by a real press on the first character.
    let r = f.glyph(0);
    let (x, y) = (
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
    );
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    let capture: web_sys::HtmlTextAreaElement = document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .expect("a capture textarea")
        .dyn_into()
        .unwrap();
    assert!(
        document().active_element().as_deref() == Some(capture.as_ref()),
        "positive control: the press focused the editor"
    );
    for &i in &wraps {
        // Three characters into the line: off the line start and off the wrap.
        let caret = Pos(i as usize + 1 + 3);
        f.handle.set_selection(Selection::cursor(caret));
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key("Home");
        init.set_code("Home");
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        capture.dispatch_event(&ev).unwrap();
        assert!(ev.default_prevented(), "the editor took Home");
        assert_eq!(
            f.handle.selection().head(),
            Pos(i as usize + 1),
            "Home from {caret:?} lands at the start of the line that begins at char {i}"
        );
    }
    f.teardown();
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
