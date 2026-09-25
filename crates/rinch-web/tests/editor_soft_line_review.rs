//! Visual-line edges on the web (#301), from the review of PR #1019. The oracle is
//! Chrome 153's native behaviour, measured with CDP editing commands in a
//! `contenteditable`. `r1`-`r3` pin Home's caret affinity at a wrap point; `r4`
//! is #1025. `r5` pins the probe
//! under `transform` and CSS `zoom`, `r6` the line's vertical middle.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_soft_line_review
//! ```
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

const HOST: &str = "data-test-host-r1019f";
const TEXT: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    sel: String,
    base: usize,
}

impl F {
    fn new(html: &str, host_style: &str, sel: &str, base: usize) -> Self {
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
            &format!("font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 180px; {host_style}"),
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
        Self {
            root,
            host,
            handle,
            sel: sel.into(),
            base,
        }
    }
    fn focus(&self) {
        let r = self.char_rect(0);
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        let cap = document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap();
        assert!(
            document().active_element().as_ref() == Some(&cap),
            "positive control: focused"
        );
    }
    fn block(&self) -> web_sys::Element {
        document()
            .query_selector(&format!("[data-pm-editor] {}", self.sel))
            .unwrap()
            .expect("block")
    }
    fn tnode(&self) -> web_sys::Node {
        let b = self.block();
        let mut n = b.first_child();
        while let Some(c) = n {
            if c.node_type() == 3 {
                return c;
            }
            n = c.next_sibling();
        }
        panic!("no text node")
    }
    fn char_rect(&self, i: u32) -> web_sys::DomRect {
        let t = self.tnode();
        let r = document().create_range().unwrap();
        r.set_start(&t, i).unwrap();
        r.set_end(&t, i + 1).unwrap();
        r.get_bounding_client_rect()
    }
    fn line_starts(&self) -> Vec<u32> {
        let n = self.tnode().text_content().unwrap().chars().count() as u32;
        let mut starts = vec![0];
        let mut top = self.char_rect(0).top();
        for i in 1..n {
            let t = self.char_rect(i).top();
            if t > top + 1.0 {
                starts.push(i);
                top = t;
            }
        }
        starts
    }
    fn cap(&self) -> web_sys::Element {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
    }
    fn bi(&self, t: &str) -> bool {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type(t);
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        self.cap().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }
    fn key(&self, k: &str, shift: bool) -> bool {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(k);
        init.set_code(k);
        init.set_shift_key(shift);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.cap().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }
    fn at(&self, i: u32) {
        self.handle
            .set_selection(Selection::cursor(Pos(i as usize + self.base)));
    }
    fn head(&self) -> i64 {
        self.handle.selection().head().0 as i64 - self.base as i64
    }
    fn sel_s(&self) -> String {
        let s = self.handle.selection();
        format!(
            "anchor={} head={}",
            s.anchor().0 as i64 - self.base as i64,
            s.head().0 as i64 - self.base as i64
        )
    }
    fn caret_line(&self, starts: &[u32]) -> i64 {
        let y = self
            .handle
            .caret_rect(self.handle.selection().head())
            .map(|r| r.y as f64 + r.height as f64 / 2.0);
        let Some(y) = y else { return -1 };
        for (li, s) in starts.iter().enumerate() {
            let r = self.char_rect(*s);
            if y >= r.top() - 4.0 && y <= r.bottom() + 4.0 {
                return li as i64;
            }
        }
        -2
    }
    fn text(&self) -> String {
        self.block().text_content().unwrap_or_default()
    }
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).expect("hit");
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

fn middle(style: &str) -> (F, Vec<u32>, u32) {
    let f = F::new(&format!("<p>{TEXT}</p>"), style, "p", 1);
    let st = f.line_starts();
    assert!(
        st.len() >= 4,
        "positive control: wraps over 4+ lines: {st:?}"
    );
    let caret = st[1] + 3;
    f.focus();
    (f, st, caret)
}

/// Chrome: Home puts the caret at the START of the line it was pressed on and
/// draws it there; a second Home stays. At PR head the wrap point draws at the
/// end of the line above and a second Home goes to line 1's start.
#[wasm_bindgen_test]
fn r1_home_draws_on_its_own_line_and_is_idempotent() {
    let (f, st, caret) = middle("");
    f.at(caret);
    assert!(f.key("Home", false));
    assert_eq!(f.head(), st[1] as i64);
    assert_eq!(
        f.caret_line(&st),
        1,
        "Home's caret is drawn on the line Home was pressed on"
    );
    assert!(f.key("Home", false));
    assert_eq!(
        f.head(),
        st[1] as i64,
        "a second Home stays (Chrome: 12 -> 12)"
    );
    f.done();
}

/// Chrome: Home then End reaches the END of the same line; Shift+Home then
/// Shift+End extends to that end.
#[wasm_bindgen_test]
fn r2_home_then_end_round_trips_on_one_line() {
    let (f, st, caret) = middle("");
    f.at(caret);
    f.key("Home", false);
    f.key("End", false);
    assert_eq!(
        f.head(),
        st[2] as i64,
        "Home, End ends on the line both were pressed on"
    );
    f.at(caret);
    f.key("Home", true);
    f.key("End", true);
    assert_eq!(f.sel_s(), format!("anchor={caret} head={}", st[2]));
    f.done();
}

/// Chrome: at the start of a soft-wrapped line (reached with Home),
/// deleteSoftLineBackward deletes ONE character — the hanging space — not the
/// line above.
#[wasm_bindgen_test]
fn r3_soft_back_at_a_wrapped_line_start_deletes_one_char() {
    let (f, st, caret) = middle("");
    f.at(caret);
    f.key("Home", false);
    assert!(f.bi("deleteSoftLineBackward"));
    let s = st[1] as usize;
    let want: String = TEXT
        .chars()
        .take(s - 1)
        .chain(TEXT.chars().skip(s))
        .collect();
    assert_eq!(f.text(), want);
    f.done();
}

/// Chrome keeps a hard break: deleteSoftLineBackward on the line after a
/// Shift+Enter deletes that line's prefix only (`alpha bravo<br>rlie ...`).
#[wasm_bindgen_test]
#[ignore = "#1025: a DOM point after a hard break maps before it"]
fn r4_soft_back_after_a_hard_break_keeps_the_break() {
    let f = F::new(
        "<p>alpha bravo<br>charlie delta echo foxtrot golf hotel india</p>",
        "",
        "p",
        1,
    );
    f.focus();
    f.at(15);
    assert!(f.bi("deleteSoftLineBackward"));
    assert_eq!(
        f.block().inner_html(),
        "alpha bravo<br>rlie delta echo foxtrot golf hotel india"
    );
    f.done();
}

/// A scaled or zoomed editor: getBoundingClientRect is scaled, clientWidth is
/// not, so the content-box arithmetic probes off the line.
#[wasm_bindgen_test]
fn r5_scaled_and_zoomed_editors_find_the_line_edges() {
    for style in [
        "transform: scale(0.5); transform-origin: 0 0;",
        "transform: scale(1.5); transform-origin: 0 0;",
        "zoom: 1.5;",
    ] {
        let (f, st, caret) = middle(style);
        f.at(caret);
        f.key("Home", false);
        assert_eq!(f.head(), st[1] as i64, "Home under {style}");
        f.at(caret);
        f.key("End", false);
        assert_eq!(f.head(), st[2] as i64, "End under {style}");
        f.done();
    }
}

/// A heading at `line-height: 1`: the caret rect's top pixel sits above the
/// line box, so the probe must be the rect's middle. Passes at PR head; kills the
/// `hy + 1` mutant (Home -> 0, End -> 77).
#[wasm_bindgen_test]
fn r6_tight_line_height_heading() {
    let f = F::new(&format!("<h1>{TEXT}</h1>"), "line-height: 1;", "h1", 1);
    let st = f.line_starts();
    assert!(st.len() >= 4, "{st:?}");
    f.focus();
    let caret = st[1] + 3;
    f.at(caret);
    f.key("Home", false);
    assert_eq!(f.head(), st[1] as i64);
    f.at(caret);
    f.key("End", false);
    assert_eq!(f.head(), st[2] as i64);
    f.done();
}
