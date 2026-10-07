//! Home / End beside the zero-byte inline leaves and at a soft-hyphen wrap
//! (#1026, review of PR #1113). The web finds a visual line's edges from caret
//! rects (`visual_line_bound`), and two kinds of rect are wrong for that: the
//! view gives an image no bytes, so the positions on its two sides share one
//! caret rect, and the character after a hyphenated soft-hyphen break has a
//! bounding rect spanning both lines. Each expectation is Chrome's own
//! `Selection.modify(.., "lineboundary")` on a contenteditable clone of the
//! rendered paragraph, noted beside it.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_line_edge_leaves
//! ```
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

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR4nGM4ISdHFcQwatCoQaMGjRo0atCoQQNvEADGtUkfvb3KfwAAAABJRU5ErkJggg==";

struct F {
    root: RootHandle,
    host: web_sys::Element,
    sheet: web_sys::Element,
    handle: EditorHandle,
}

impl F {
    fn new(html: &str, host_style: &str, css: &str) -> Self {
        for sel in ["[data-line-edge-host]", "[data-line-edge-sheet]"] {
            let stale = document().query_selector_all(sel).unwrap();
            for i in 0..stale.length() {
                stale
                    .item(i)
                    .unwrap()
                    .dyn_into::<web_sys::Element>()
                    .unwrap()
                    .remove();
            }
        }
        let sheet = document().create_element("style").unwrap();
        sheet.set_attribute("data-line-edge-sheet", "").unwrap();
        sheet.set_text_content(Some(css));
        document().head().unwrap().append_child(&sheet).unwrap();
        let host = document().create_element("div").unwrap();
        host.set_attribute("data-line-edge-host", "").unwrap();
        host.set_attribute(
            "style",
            &format!(
                "font-family: monospace; font-size: 16px; line-height: 24px; \
                 padding: 20px; width: 180px; {host_style}"
            ),
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
        let f = Self {
            root,
            host,
            sheet,
            handle,
        };
        f.focus();
        f
    }
    fn para(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .unwrap()
    }
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }
    fn focus(&self) {
        let r = self.para().get_bounding_client_rect();
        let (x, y) = ((r.x() + 3.0) as f32, (r.y() + 5.0) as f32);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(self.capture().as_ref()),
            "focus control"
        );
    }
    fn key(&self, key: &str, shift: bool) -> bool {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        init.set_code(key);
        init.set_shift_key(shift);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }
    fn at(&self, p: usize, k: &str) -> usize {
        self.handle.set_selection(Selection::cursor(Pos(p)));
        assert!(self.key(k, false), "{k} is taken");
        self.handle.selection().head().0
    }
    fn img_rect(&self) -> web_sys::DomRect {
        document()
            .query_selector("[data-pm-editor] p img")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect()
    }
    fn text_rect(&self, text: &web_sys::Node, i: u32) -> web_sys::DomRect {
        let r = document().create_range().unwrap();
        r.set_start(text, i).unwrap();
        r.set_end(text, i + 1).unwrap();
        r.get_bounding_client_rect()
    }
    /// Chrome's own Home/End: a contenteditable clone of the rendered paragraph,
    /// the caret at char `off` of its `node`-th child, moved by lineboundary.
    /// Answers "(child index of the focus node or 'P', offset)".
    fn native(&self, node: u32, off: u32, forward: bool) -> String {
        let para = self.para();
        let w = para.client_width();
        let twin = para
            .clone_node_with_deep(true)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap();
        twin.set_attribute("contenteditable", "true").unwrap();
        twin.set_attribute("data-line-edge-host", "").unwrap();
        let cs = web_sys::window()
            .unwrap()
            .get_computed_style(&para)
            .unwrap()
            .unwrap();
        let mut style = format!("width: {w}px; box-sizing: border-box; margin: 0; ");
        for p in [
            "font-family",
            "font-size",
            "line-height",
            "white-space",
            "padding-left",
            "padding-right",
            "word-break",
            "overflow-wrap",
        ] {
            style.push_str(&format!("{p}: {}; ", cs.get_property_value(p).unwrap()));
        }
        twin.set_attribute("style", &style).unwrap();
        if let Ok(Some(img)) = twin.query_selector("img") {
            let r = self.img_rect();
            img.set_attribute(
                "style",
                &format!("width: {}px; height: {}px;", r.width(), r.height()),
            )
            .unwrap();
        }
        self.host.append_child(&twin).unwrap();
        let start = twin.child_nodes().item(node).unwrap();
        let sel = web_sys::window().unwrap().get_selection().unwrap().unwrap();
        sel.collapse_with_offset(Some(&start), off).unwrap();
        let modify: js_sys::Function = js_sys::Reflect::get(&sel, &"modify".into())
            .unwrap()
            .dyn_into()
            .unwrap();
        modify
            .apply(
                &sel,
                &js_sys::Array::of3(
                    &"move".into(),
                    &(if forward { "forward" } else { "backward" }).into(),
                    &"lineboundary".into(),
                ),
            )
            .unwrap();
        let fnode = sel.focus_node().unwrap();
        let kids = twin.child_nodes();
        let mut name = String::from("P");
        for i in 0..kids.length() {
            if kids.item(i).unwrap().is_same_node(Some(&fnode)) {
                name = format!("child{i}");
            }
        }
        let out = format!("({name}, {})", sel.focus_offset());
        sel.remove_all_ranges().unwrap();
        twin.remove();
        // restore focus to the editor capture
        self.capture().focus().ok();
        out
    }
    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        self.sheet.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).unwrap();
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

const IMG_CSS: &str =
    "[data-pm-editor] p img { width: 120px !important; height: 20px !important; }";

/// "aaaa bbbb " (1..11), image (11..12), " cccc" (12..17). The image does not fit
/// after "aaaa bbbb " (96px + 120px > 180px), so it starts line 2.
#[wasm_bindgen_test]
fn end_stops_before_an_image_that_starts_the_next_line() {
    let f = F::new(
        &format!(r#"<p>aaaa bbbb <img src="{PNG}"> cccc</p>"#),
        "",
        IMG_CSS,
    );
    let text = f.para().first_child().unwrap();
    let a = f.text_rect(&text, 0);
    let img = f.img_rect();
    assert!(
        img.top() > a.bottom() - 1.0,
        "control: the image starts line 2"
    );
    let end = f.at(2, "End");
    let home = f.at(14, "Home");
    assert_eq!(f.native(0, 1, true), "(child0, 10)", "the oracle: End");
    assert_eq!(f.native(2, 2, false), "(child2, 1)", "the oracle: Home");
    assert_eq!(
        end, 11,
        "End on line 1 stops before the image (line 2's first item) — Chrome: (child0, 10)"
    );
    assert_eq!(
        home, 13,
        "Home on line 3 (\" cccc\" wraps after the image) — Chrome: (child2, 1)"
    );
    f.teardown();
}

/// "aaaa " (1..6), image (6..7), "bbbb cccc" (7..16). Line 1 = "aaaa [img]"
/// (48 + 120 = 168px), line 2 = "bbbb cccc".
#[wasm_bindgen_test]
fn home_and_end_around_an_image_on_a_line_of_its_own() {
    let f = F::new(
        &format!(r#"<p>aaaa <img src="{PNG}">bbbb cccc</p>"#),
        "",
        IMG_CSS,
    );
    let first = f.para().first_child().unwrap();
    let after = f.para().last_child().unwrap();
    let a = f.text_rect(&first, 0);
    let b = f.text_rect(&after, 0);
    let img = f.img_rect();
    assert!(
        img.top() > a.bottom() - 1.0 && b.top() > img.bottom() - 1.0,
        "control: aaaa / image / bbbb on three lines"
    );
    let home = f.at(9, "Home");
    let end = f.at(2, "End");
    // One point, spelled two ways: before the paragraph's third child
    // (Chrome), or at offset 0 of that child, the text after the image
    // (Firefox).
    let after_the_image = if engine::is_gecko() {
        "(child2, 0)"
    } else {
        "(P, 2)"
    };
    assert_eq!(f.native(2, 2, false), after_the_image, "the oracle: Home");
    assert_eq!(f.native(0, 1, true), "(child0, 5)", "the oracle: End");
    assert_eq!(
        home, 7,
        "Home on bbbb's line goes to after the image — Chrome: (P, 2)"
    );
    assert_eq!(
        end, 6,
        "End on line 1 stops before the image (alone on line 2) — Chrome: (child0, 5)"
    );
    f.teardown();
}

/// "aaaa " (1..6), a 170px image alone on line 2 (6..7), " bbbb" (7..12).
#[wasm_bindgen_test]
fn end_stops_before_a_wide_image_alone_on_the_next_line() {
    let css = "[data-pm-editor] p img { width: 170px !important; height: 20px !important; }";
    let f = F::new(&format!(r#"<p>aaaa <img src="{PNG}"> bbbb</p>"#), "", css);
    let first = f.para().first_child().unwrap();
    let a = f.text_rect(&first, 0);
    let img = f.img_rect();
    assert!(
        img.top() > a.bottom() - 1.0,
        "control: the image is below line 1"
    );
    let end = f.at(2, "End");
    assert_eq!(f.native(0, 1, true), "(child0, 5)", "the oracle: End");
    assert_eq!(end, 6, "End on line 1 stops before the image");
    f.teardown();
}

/// End then Home on a glyph wrap: End lands on the wrap point drawn upstream;
/// Home from there goes back to line 1's start, not line 2's.
#[wasm_bindgen_test]
fn home_after_end_at_a_wrap_returns_to_the_line_start() {
    let t = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";
    let f = F::new(&format!("<p>{t}</p>"), "", "");
    let text = f.para().first_child().unwrap();
    let top0 = f.text_rect(&text, 0).top();
    let l2 = (1..t.len() as u32)
        .find(|&i| f.text_rect(&text, i).top() > top0 + 1.0)
        .unwrap();
    let l3 = (l2 + 1..t.len() as u32)
        .find(|&i| f.text_rect(&text, i).top() > f.text_rect(&text, l2).top() + 1.0)
        .unwrap();
    // caret on line 2
    let c = l2 as usize + 1 + 2;
    let end = f.at(c, "End");
    assert_eq!(end, l3 as usize + 1, "End: wrap point");
    assert!(f.key("End", false));
    assert_eq!(f.handle.selection().head().0, end, "End again stays");
    assert!(f.key("Home", false));
    let home = f.handle.selection().head().0;
    assert_eq!(home, l2 as usize + 1, "Home after End: line 2's start");
    f.teardown();
}

/// A soft-hyphen wrap: Home on the line after it goes to that line's start, the
/// position right after the SHY (Chrome agrees), not one character in.
#[wasm_bindgen_test]
fn home_after_a_soft_hyphen_wrap_lands_at_the_line_start() {
    let text = "extra\u{00AD}ordinary super\u{00AD}cali\u{00AD}fragilistic expi\u{00AD}ali\u{00AD}docious words";
    let f = F::new(&format!("<p>{text}</p>"), "width: 250px;", "");
    let t = f.para().first_child().unwrap();
    assert!(
        f.text_rect(&t, 27).top() > f.text_rect(&t, 24).bottom() - 1.0,
        "control: the line breaks at the SHY at 25"
    );
    let home = f.at(31, "Home");
    let chrome = native_u16(&f, 30, false);
    assert_eq!(chrome, 26, "the oracle");
    assert_eq!(home, 27, "Home: Pos 27 (char 26, after the SHY)");
    f.teardown();
}

/// Chrome native lineboundary from UTF-16 `off` in a clone of the rendered
/// single-text-node paragraph, as a UTF-16 offset.
fn native_u16(f: &F, off: u32, forward: bool) -> u32 {
    let s = f.native(0, off, forward);
    // "(child0, N)" or "(P, k)"
    let inner = s.trim_start_matches('(').trim_end_matches(')');
    let mut it = inner.split(", ");
    let who = it.next().unwrap().to_string();
    let n: u32 = it.next().unwrap().parse().unwrap();
    if who == "child0" {
        n
    } else if n == 0 {
        0
    } else {
        u32::MAX
    }
}

/// A KNOWN divergence, pinned: End on a line whose last run is right-to-left.
/// rinch answers the line's logical end — the wrap point, after the trailing
/// space — as on a Latin line, where Chrome agrees; Chrome stops before the
/// space on this one. If this starts failing because rinch now matches
/// Chrome, update the guide's mixed-direction caveat.
///
/// Firefox's own End takes the wrap point here (measured, 155 and 157), so in
/// that engine rinch and the browser agree.
#[wasm_bindgen_test]
fn end_on_a_line_ending_in_rtl_text_takes_the_wrap_point() {
    let text = "alpha bravo שלום עולם אחד שניים charlie delta echo foxtrot golf";
    let f = F::new(&format!("<p>{text}</p>"), "width: 300px;", "");
    let t = f.para().first_child().unwrap();
    let top0 = f.text_rect(&t, 0).top();
    let wrap = (1..text.encode_utf16().count() as u32)
        .find(|&i| f.text_rect(&t, i).top() > top0 + 1.0)
        .expect("control: the paragraph wraps");
    assert_eq!(
        &text[..text.char_indices().nth(wrap as usize).unwrap().0],
        "alpha bravo שלום עולם אחד ",
        "control: line 1 ends in the Hebrew run and its space"
    );
    let end = f.at(3, "End");
    let chrome = native_u16(&f, 2, true);
    if engine::is_gecko() {
        assert_eq!(chrome, wrap, "the oracle (Firefox) takes the wrap point");
    } else {
        assert_eq!(
            chrome,
            wrap - 1,
            "the oracle stops before the trailing space"
        );
    }
    assert_eq!(end, wrap as usize + 1, "rinch: the wrap point");
    f.teardown();
}

/// Not an assertion: the cost of End, Home and (for scale) ArrowRight on a
/// 5000-character paragraph, the caret in its middle.
/// `-- --include-ignored --nocapture` prints the timings.
#[wasm_bindgen_test]
#[ignore = "a timing, not a test"]
fn line_edge_cost_on_a_long_paragraph() {
    let mut s = String::new();
    let words = [
        "alpha",
        "bravo",
        "charlie",
        "de",
        "echo",
        "foxtrotting",
        "g",
    ];
    let mut i = 0;
    while s.len() < 5000 {
        s.push_str(words[i % words.len()]);
        s.push(' ');
        i += 1;
    }
    let f = F::new(&format!("<p>{s}</p>"), "width: 600px;", "");
    for round in 0..3 {
        for k in ["End", "Home", "ArrowRight"] {
            let n = 300;
            let t0 = js_sys::Date::now();
            for _ in 0..n {
                f.handle.set_selection(Selection::cursor(Pos(2500)));
                f.key(k, false);
            }
            let dt = (js_sys::Date::now() - t0) / n as f64;
            console_log!("COST r{round} {k}: {dt:.3} ms per key");
        }
    }
    f.teardown();
}
