//! Visual-line edges on the web (issue #301): `deleteSoftLineBackward` /
//! `deleteSoftLineForward` — what an on-screen keyboard and Cmd+Backspace /
//! Cmd+Delete send — and Home / End delete to and move to the edge of the
//! **visual** line the caret is on, as every browser field does. They used to go
//! to the edge of the whole textblock, so on a paragraph wrapped over several
//! lines a soft-line delete on line 2 took line 1 with it.
//! `deleteHardLine*` keeps the textblock edge.
//!
//! The oracle is Chrome itself: a `contenteditable` twin of the paragraph, given
//! the paragraph's own computed typography and content width, is asked where
//! `Selection.modify(.., "lineboundary")` lands from the same character offset.
//! The right-to-left fixtures check against the line breaks measured off the
//! paragraph's own glyph rects instead (Chrome's `modify` answered 25 for a line
//! starting at 12 in an RTL paragraph of Latin text). Either way nothing here pins
//! a font's advance widths — the line breaks are measured, on whatever fonts the
//! host has.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_soft_line
//! ```
//!
//! Every fixture mounts through `rinch_web::mount_into`, focuses with a genuine
//! press and checks the capture textarea holds focus before asserting anything.
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

const HOST_MARKER: &str = "data-test-host-soft-line";

/// Words of uneven length, so no line break sits on an evenly spaced grid.
const TEXT: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    text: &'static str,
}

impl Fixture {
    /// One editor holding `text` in one paragraph, 180px wide so it wraps over
    /// several lines. Not focused yet: [`Fixture::focus`].
    fn mounted(text: &'static str, style: &str) -> Self {
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
        host.set_attribute(
            "style",
            &format!(
                "font-family: monospace; font-size: 16px; line-height: 24px; \
                 padding: 20px; width: 180px; {style}"
            ),
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(&format!("<p>{text}</p>")));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self {
            root,
            host,
            handle,
            text,
        }
    }

    /// Focus the editor by a real press on the first character.
    fn focus(&self) {
        let r = self.char_rect(0);
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(self.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    fn para(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("a paragraph")
    }

    fn char_rect(&self, i: u32) -> web_sys::DomRect {
        let text = self.para().first_child().expect("a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, i).unwrap();
        range.set_end(&text, i + 1).unwrap();
        range.get_bounding_client_rect()
    }

    /// The char offset each visual line of the paragraph starts at, measured.
    fn line_starts(&self) -> Vec<u32> {
        let n = self.text.chars().count() as u32;
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

    /// Chrome's answer: where `Selection.modify("move", dir, "lineboundary")`
    /// lands from char offset `at` in a contenteditable twin of the paragraph.
    fn chrome_line_boundary(&self, at: u32, forward: bool) -> u32 {
        let para = self.para();
        let cs = web_sys::window()
            .unwrap()
            .get_computed_style(&para)
            .unwrap()
            .unwrap();
        let prop = |p: &str| cs.get_property_value(p).unwrap();
        let pad = |p: &str| prop(p).trim_end_matches("px").parse::<f64>().unwrap_or(0.0);
        let width = para.client_width() as f64 - pad("padding-left") - pad("padding-right");
        let twin = document().create_element("div").unwrap();
        twin.set_attribute(HOST_MARKER, "").unwrap();
        twin.set_attribute("contenteditable", "true").unwrap();
        let mut style = format!("width: {width}px; padding: 0; margin: 0; border: 0;");
        for p in [
            "font-family",
            "font-size",
            "font-weight",
            "line-height",
            "letter-spacing",
            "word-spacing",
            "white-space",
            "word-break",
            "overflow-wrap",
            "tab-size",
            "direction",
            "text-align",
        ] {
            style.push_str(&format!("{p}: {};", prop(p)));
        }
        twin.set_attribute("style", &style).unwrap();
        twin.set_text_content(Some(self.text));
        document().body().unwrap().append_child(&twin).unwrap();
        let text = twin.first_child().unwrap();
        let sel = web_sys::window().unwrap().get_selection().unwrap().unwrap();
        sel.collapse_with_offset(Some(&text), at).unwrap();
        let modify: js_sys::Function = js_sys::Reflect::get(&sel, &"modify".into())
            .unwrap()
            .dyn_into()
            .unwrap();
        let args = js_sys::Array::of3(
            &"move".into(),
            &(if forward { "forward" } else { "backward" }).into(),
            &"lineboundary".into(),
        );
        modify.apply(&sel, &args).unwrap();
        let out = sel.focus_offset();
        sel.remove_all_ranges().unwrap();
        twin.remove();
        out
    }

    fn text(&self) -> String {
        self.para().text_content().unwrap_or_default()
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// A `beforeinput` of `input_type` on the capture textarea; answers whether
    /// the editor took it (`preventDefault`ed).
    fn before_input(&self, input_type: &str) -> bool {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type(input_type);
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
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

    /// Put the caret at char offset `i` of the paragraph (model `Pos(i + 1)`).
    fn caret_at(&self, i: u32) {
        self.handle
            .set_selection(Selection::cursor(Pos(i as usize + 1)));
    }

    /// The caret's char offset in the paragraph.
    fn head(&self) -> u32 {
        (self.handle.selection().head().0 - 1) as u32
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
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

fn slice(s: &str, from: u32, to: u32) -> String {
    s.chars()
        .skip(from as usize)
        .take((to - from) as usize)
        .collect()
}

/// Hebrew, so an RTL paragraph's logical line start is its RIGHT edge.
const HEBREW: &str = "שלום עולם אחד שניים שלושה ארבעה חמישה שישה שבעה שמונה תשעה עשרה אחת עשרה";

/// A mounted editor over `text`, wrapped over at least four lines, and the
/// paragraph's measured line starts. The caret goes on the SECOND visual line,
/// three characters in — neither the first line, whose start is the
/// textblock's, nor the last, whose end is: `(line start, caret, next start)`.
fn middle_line(text: &'static str, style: &str) -> (Fixture, u32, u32, u32) {
    let f = Fixture::mounted(text, style);
    let starts = f.line_starts();
    assert!(
        starts.len() >= 4,
        "positive control: the paragraph wraps over at least four lines, got {starts:?}"
    );
    let (start, next) = (starts[1], starts[2]);
    let caret = start + 3;
    assert!(caret < next, "the caret is on line 2: {starts:?}");
    (f, start, caret, next)
}

fn count(s: &str) -> u32 {
    s.chars().count() as u32
}

/// A soft-line delete backward takes the current visual line's prefix, and
/// nothing on the line above. It used to delete to the textblock's start.
#[wasm_bindgen_test]
fn soft_line_backward_deletes_to_the_visual_line_start() {
    let (f, start, caret, _) = middle_line(TEXT, "");
    let chrome = f.chrome_line_boundary(caret, false);
    assert_eq!(
        chrome, start,
        "the oracle agrees with the measured line start"
    );
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteSoftLineBackward"));
    assert_eq!(
        f.text(),
        format!(
            "{}{}",
            slice(TEXT, 0, start),
            slice(TEXT, caret, count(TEXT))
        )
    );
    assert_eq!(f.head(), start);
    f.teardown();
}

/// A soft-line delete forward takes the rest of the visual line — to where
/// Chrome's own line boundary is — and nothing on the line below.
#[wasm_bindgen_test]
fn soft_line_forward_deletes_to_the_visual_line_end() {
    let (f, _, caret, next) = middle_line(TEXT, "");
    let chrome = f.chrome_line_boundary(caret, true);
    assert!(
        chrome > caret && chrome <= next,
        "the oracle ends the line before the next one starts: {chrome} vs {next}"
    );
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteSoftLineForward"));
    assert_eq!(
        f.text(),
        format!(
            "{}{}",
            slice(TEXT, 0, caret),
            slice(TEXT, chrome, count(TEXT))
        )
    );
    assert_eq!(f.head(), caret);
    f.teardown();
}

/// A hard-line delete still means the textblock's edge.
#[wasm_bindgen_test]
fn hard_line_deletes_keep_the_textblock_edge() {
    let (f, _, caret, _) = middle_line(TEXT, "");
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteHardLineBackward"));
    assert_eq!(f.text(), slice(TEXT, caret, count(TEXT)));
    f.caret_at(3);
    assert!(f.before_input("deleteHardLineForward"));
    assert_eq!(f.text(), slice(TEXT, caret, caret + 3));
    f.teardown();
}

/// End from `caret` landed at `end`: the wrap point `next`, drawn on the
/// caret's own line. The model has no caret affinity, so a caret AT the wrap
/// point draws wherever the browser draws a collapsed range there — at the end
/// of the line before, after a hanging space and inside a broken word alike.
fn assert_end_on_the_line(f: &Fixture, caret: u32, end: u32, next: u32) {
    assert_eq!(end, next, "End lands at the wrap point");
    let caret_top = f.char_rect(caret).top();
    let end_rect = f.handle.caret_rect(f.handle.selection().head()).unwrap();
    assert!(
        (end_rect.y as f64 - caret_top).abs() < 12.0,
        "End's caret is drawn on the caret's own line: {} vs {caret_top}",
        end_rect.y
    );
}

/// Home and End go to the visual line's edges, as on desktop; Shift extends.
#[wasm_bindgen_test]
fn home_and_end_go_to_the_visual_line_edges() {
    let (f, start, caret, next) = middle_line(TEXT, "");
    f.focus();
    f.caret_at(caret);
    assert!(f.key("Home", false));
    assert_eq!(f.head(), start);
    assert!(f.handle.selection().is_empty());

    f.caret_at(caret);
    assert!(f.key("End", false));
    let end = f.head();
    assert_end_on_the_line(&f, caret, end, next);

    f.caret_at(caret);
    assert!(f.key("Home", true));
    assert_eq!(
        f.handle.selection(),
        Selection::text(Pos(caret as usize + 1), Pos(start as usize + 1)),
        "Shift+Home extends from the caret"
    );
    f.caret_at(caret);
    assert!(f.key("End", true));
    assert_eq!(
        f.handle.selection(),
        Selection::text(Pos(caret as usize + 1), Pos(end as usize + 1)),
        "Shift+End extends from the caret"
    );
    f.teardown();
}

/// Right-to-left Hebrew: the line's logical start is its right edge. A
/// soft-line delete backward still takes the logical prefix of the current
/// line, and forward its logical rest.
#[wasm_bindgen_test]
fn soft_line_deletes_in_an_rtl_paragraph() {
    let (f, start, caret, next) = middle_line(HEBREW, "direction: rtl;");
    assert!(
        f.char_rect(start).right() > f.char_rect(caret).right(),
        "positive control: the line runs right to left"
    );
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteSoftLineBackward"));
    let n = count(HEBREW);
    assert_eq!(
        f.text(),
        format!("{}{}", slice(HEBREW, 0, start), slice(HEBREW, caret, n))
    );
    f.teardown();

    let (f, _, caret, next2) = middle_line(HEBREW, "direction: rtl;");
    assert_eq!(next, next2);
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteSoftLineForward"));
    let got = f.text();
    assert!(
        got.starts_with(&slice(HEBREW, 0, caret)) && got.ends_with(&slice(HEBREW, next, n)),
        "only the rest of line 2 goes: {got:?}"
    );
    assert!(
        count(&got) >= n - (next - caret) && count(&got) < n,
        "at most the rest of line 2 goes, and something does: {got:?}"
    );
    f.teardown();
}

/// LTR text in an RTL paragraph: the words still run left to right, so the
/// logical line start is the LEFT edge. A rule keyed on `direction` alone would
/// take the wrong side here.
#[wasm_bindgen_test]
fn soft_line_backward_for_ltr_text_in_an_rtl_paragraph() {
    let (f, start, caret, _) = middle_line(TEXT, "direction: rtl;");
    assert!(
        f.char_rect(start).left() < f.char_rect(caret).left(),
        "positive control: the words run left to right"
    );
    f.focus();
    f.caret_at(caret);
    assert!(f.before_input("deleteSoftLineBackward"));
    assert_eq!(
        f.text(),
        format!(
            "{}{}",
            slice(TEXT, 0, start),
            slice(TEXT, caret, count(TEXT))
        )
    );
    f.teardown();
}

/// One unbroken word, broken by `overflow-wrap`: the wrap point sits between two
/// letters, with no hanging space to end the line on. End still lands on the
/// wrap point and still draws on the line it was pressed on — Chrome's own End.
#[wasm_bindgen_test]
fn end_inside_a_broken_word_stays_on_its_line() {
    let (f, _, caret, next) = middle_line(LONG_WORD, "overflow-wrap: anywhere;");
    let chrome = f.chrome_line_boundary(caret, true);
    f.focus();
    f.caret_at(caret);
    assert!(f.key("End", false));
    let end = f.head();
    assert_eq!(end, chrome, "End lands where Chrome's End does");
    assert_end_on_the_line(&f, caret, end, next);
    f.teardown();
}

const LONG_WORD: &str =
    "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghij";

/// A paragraph with padding and a border: the probes go just inside its
/// CONTENT box, not its border box, so both edges still resolve.
#[wasm_bindgen_test]
fn a_padded_paragraph_finds_both_edges() {
    let sheet = document().create_element("style").unwrap();
    // Not `HOST_MARKER`: mounting sweeps those away.
    sheet
        .set_attribute("data-test-sheet-soft-line", "")
        .unwrap();
    sheet.set_text_content(Some(
        "[data-test-host-soft-line] [data-pm-editor] p { padding: 0 60px; border: 0 solid; border-width: 0 7px; }",
    ));
    document().head().unwrap().append_child(&sheet).unwrap();
    let (f, start, caret, next) = middle_line(TEXT, "width: 300px;");
    let p = f.para();
    assert!(
        p.client_left() == 7,
        "positive control: the padding and border apply"
    );
    f.focus();
    f.caret_at(caret);
    assert!(f.key("Home", false));
    assert_eq!(f.head(), start);
    f.caret_at(caret);
    assert!(f.key("End", false));
    assert_eq!(f.head(), next);
    f.teardown();
    sheet.remove();
}
