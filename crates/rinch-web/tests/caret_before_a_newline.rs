//! The caret Chrome gives no rect, on the web (#1202).
//!
//! In preserved-newline text (`pre`, `pre-wrap`: every code block) Chrome 153
//! gives a collapsed range **no rect** — zero height, no client rects — right
//! before a segment break (`"\n"`, `"\r"`), at the end of a text node that
//! ends in `"\n"`, and (in any `white-space`) in an empty text node, though not
//! at every such position. Measured on a `<pre>`: `ab\n\ncd` at offset 3 (the
//! empty middle line), `\nab` at 0, `\n\nab` at 0 and 1, `ab\n\n` at 3 and 4,
//! and in `pre-wrap` `a\n \nb` at 3 (after the space, before the second
//! newline; `pre`'s has a rect there); at the other positions of those texts
//! the collapsed range has a rect of its own. The range over the `"\n"` does
//! have one: a zero-width rect at the character's start, on the line the caret
//! belongs to — the empty line itself, the start of a line that begins with a
//! newline, or just after the space.
//!
//! `WebDocument::text_caret_viewport_rect` answered `None` for every such
//! caret except the one #1197 added (after a text-final newline, placed from
//! what follows), and `query_caret_rect_with_affinity` fell back to the
//! **block's own box**: the caret on an empty line inside a code block was
//! drawn at the top of the block, one block tall. It now draws a caret with no
//! rect of its own where whatever follows it starts: the next character of the
//! same text node, else the next text or `<br>` in the block.
//!
//! **Firefox** (155 and 157) answers differently: there the collapsed range
//! always has a rect, and right after a preserved `"\n"` it is that newline's
//! own box — the end of the line *above* the caret's. The caret after a
//! newline is drawn where what follows starts in that engine too. Each
//! positive control below states both engines' measured answer
//! ([`engine::is_gecko`]).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test caret_before_a_newline
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{CaretAffinity, DomDocument, NodeId, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::Pos;
use rinch_web::{EditorHandle, RootHandle, WebDocument, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/engine.rs"]
mod engine;

const HOST: &str = "data-test-host-1202";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn clear_stale() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
}

// ── The editor ──────────────────────────────────────────────────────────

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl F {
    /// An editor holding `html` in a 400px monospace host, 16px / 24px.
    fn new(html: &str) -> Self {
        clear_stale();
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
        Self { root, host, handle }
    }
    fn pre(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] pre")
            .unwrap()
            .expect("the code block")
    }
    /// The code block's line height: its computed `line-height`.
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
    /// The block's first text node.
    fn text(&self) -> web_sys::Node {
        let kids = self.pre().child_nodes();
        (0..kids.length())
            .filter_map(|i| kids.item(i))
            .find(|c| c.node_type() == 3)
            .expect("a text node")
    }
    /// The browser's own rect for UTF-16 range `from..to` of the block's text.
    fn range_rect(&self, from: u32, to: u32) -> web_sys::DomRect {
        let r = document().create_range().unwrap();
        r.set_start(&self.text(), from).unwrap();
        r.set_end(&self.text(), to).unwrap();
        r.get_bounding_client_rect()
    }
    /// Positive control for a caret the browser's own collapsed range does
    /// not place, at UTF-16 `off`, which belongs on line `n`. Chrome gives it
    /// no rect. Firefox gives it one on line `gecko_line` (`n` where Firefox's
    /// own answer is right, the line above where it is the newline's box).
    fn assert_own_caret(&self, off: u32, n: f64, gecko_line: f64, what: &str) {
        let own = self.range_rect(off, off);
        if !engine::is_gecko() {
            assert!(
                own.height() <= 0.0,
                "positive control: Chrome gives the collapsed range {what} no rect"
            );
            return;
        }
        let line = self.line();
        let top = own.y() - self.content_origin().1;
        assert!(
            own.height() > 0.0 && top >= gecko_line * line - 1.0 && top < (gecko_line + 1.0) * line,
            "positive control: Firefox draws the collapsed range {what} on line \
             {gecko_line} (the caret's is {n}): top {top}, height {}, line {line}",
            own.height()
        );
    }
    /// `caret_rect` at `pos`, as `(x, top, height)` relative to the block's
    /// content box.
    fn caret(&self, pos: usize) -> (f64, f64, f64) {
        let r = self.handle.caret_rect(Pos(pos)).expect("a caret rect");
        let b = self.content_origin();
        (r.x as f64 - b.0, r.y as f64 - b.1, r.height as f64)
    }
    /// The block's content-box origin in the viewport.
    fn content_origin(&self) -> (f64, f64) {
        let pre = self.pre();
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
                .unwrap_or(0.0)
        };
        let r = pre.get_bounding_client_rect();
        (
            r.x() + px("border-left-width") + px("padding-left"),
            r.y() + px("border-top-width") + px("padding-top"),
        )
    }
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// `x` is Pos 1..2, the paragraph closes at 3, and the code block's text
/// starts at Pos 4: char `i` of it starts at `4 + i`.
const CODE: usize = 4;

/// Line `n` (0-based) of the code block: its caret is at the content box's
/// left edge, on that line, and one line — not the whole block — tall.
fn assert_line_start(f: &F, pos: usize, n: f64, what: &str) {
    let line = f.line();
    let (x, top, h) = f.caret(pos);
    // The caret rect is the glyph box, inside the line box.
    assert!(
        top >= n * line - 1.0 && top + h <= (n + 1.0) * line + 1.0,
        "{what}: the caret is on line {n}: top {top}, height {h}, line {line}"
    );
    assert!(
        x.abs() < 1.0,
        "{what}: the caret starts the line: x {x} from the content box"
    );
    assert!(
        h > 0.0 && h < line + 1.0,
        "{what}: one line tall, not the block: height {h}, line {line}"
    );
}

/// The caret on the empty middle line of `ab\n\ncd` (#1202). Chrome gives the
/// collapsed range after the first newline no rect; the caret used to be the
/// code block's own box — top of the block, three lines tall.
///
/// Kills the mutant that sends every such caret to the "what follows the text
/// node" walk (#1197's `off == end` guard, dropped: W2 of its review), which
/// finds nothing after this text node and answered `None` — the block again.
#[wasm_bindgen_test]
fn the_caret_on_an_empty_line_inside_a_code_block_is_on_that_line() {
    let f = F::new("<p>x</p><pre>ab\n\ncd</pre>");
    // Positive control: this is a caret the browser gives no rect.
    f.assert_own_caret(3, 1.0, 0.0, "at the empty line");
    let line = f.line();
    let (_, t2, _) = f.caret(CODE + 2);
    assert!(t2.abs() < line * 0.5, "after `ab` is line 0: {t2}");
    assert_line_start(&f, CODE + 3, 1.0, "the empty line");
    assert_line_start(&f, CODE + 4, 2.0, "before `cd`");
    f.done();
}

/// Two empty lines in a row: each caret on its own line.
#[wasm_bindgen_test]
fn each_of_two_empty_lines_has_its_own_caret() {
    let f = F::new("<p>x</p><pre>ab\n\n\ncd</pre>");
    assert_line_start(&f, CODE + 3, 1.0, "the first empty line");
    assert_line_start(&f, CODE + 4, 2.0, "the second empty line");
    assert_line_start(&f, CODE + 5, 3.0, "before `cd`");
    f.done();
}

/// A code block that begins with a newline: the caret at its start is the
/// start of line 0, one line tall — it was the whole block's box.
#[wasm_bindgen_test]
fn the_caret_at_the_start_of_a_code_block_beginning_with_a_newline() {
    let f = F::new("<p>x</p><pre>\nab</pre>");
    f.assert_own_caret(0, 0.0, 0.0, "at the start");
    assert_line_start(&f, CODE, 0.0, "before the leading newline");
    assert_line_start(&f, CODE + 1, 1.0, "before `ab`");
    f.done();
}

/// `ab\n\n`: the empty middle line has no rect (next is `"\n"`) and the last
/// line is the trailing-break placeholder's (#1172). Three distinct lines.
#[wasm_bindgen_test]
fn a_code_block_ending_in_two_newlines_has_three_caret_lines() {
    let f = F::new("<p>x</p><pre>ab\n\n</pre>");
    assert_eq!(
        f.pre()
            .query_selector_all("br[data-pm-trailing-break]")
            .unwrap()
            .length(),
        1,
        "the placeholder makes the last line"
    );
    assert_line_start(&f, CODE + 3, 1.0, "the empty middle line");
    assert_line_start(&f, CODE + 4, 2.0, "after the final newline");
    f.done();
}

/// A caret the browser does give a rect is unchanged: mid-line, and at the
/// start of the line after a newline.
///
/// In Firefox the second is not such a caret: its own collapsed range after
/// the newline is on the empty line above `cd` (the positive control of
/// [`the_caret_after_a_newline_is_on_the_next_line_with_either_affinity`]),
/// so there only the mid-line carets are compared.
#[wasm_bindgen_test]
fn carets_with_a_rect_of_their_own_are_unchanged() {
    let f = F::new("<p>x</p><pre>ab\n\ncd</pre>");
    let after_a_newline = if engine::is_gecko() {
        None
    } else {
        Some((4, CODE + 4))
    };
    for (i, pos) in [(1u32, CODE + 1), (5, CODE + 5)]
        .into_iter()
        .chain(after_a_newline)
    {
        let own = f.range_rect(i, i);
        assert!(own.height() > 0.0, "a rect of its own at {i}");
        let r = f.handle.caret_rect(Pos(pos)).unwrap();
        assert!(
            (r.x as f64 - own.x()).abs() < 0.5 && (r.y as f64 - own.y()).abs() < 0.5,
            "the browser's own caret at {i}: {:?} vs ({}, {})",
            (r.x, r.y),
            own.x(),
            own.y()
        );
    }
    f.done();
}

/// The caret right after a preserved newline has one line, whatever its
/// affinity: a hard line break is not a soft wrap, so there is no "end of the
/// upper line" for an `Upstream` caret to take.
///
/// Firefox's collapsed range there is the newline's own box, on the line
/// above; `Upstream` answered it as it stood (the caret after the first
/// newline of `ab\n\ncd` at the end of `ab`), and after a text-final newline
/// both affinities did: the caret on the last, empty line of a code block was
/// drawn at the end of the line before it. Chrome's own range is right or
/// absent at each of these, so this passed there before the fix too.
#[wasm_bindgen_test]
fn the_caret_after_a_newline_is_on_the_next_line_with_either_affinity() {
    let on_line = |f: &F, pos: usize, n: f64, what: &str| {
        let line = f.line();
        let origin = f.content_origin();
        for affinity in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
            let r = f
                .handle
                .caret_rect_with_affinity(Pos(pos), affinity)
                .expect("a caret rect");
            let (x, top, h) = (
                r.x as f64 - origin.0,
                r.y as f64 - origin.1,
                r.height as f64,
            );
            assert!(
                top >= n * line - 1.0 && top + h <= (n + 1.0) * line + 1.0 && x.abs() < 1.0,
                "{what}, {affinity:?}: the start of line {n}: x {x}, top {top}, height {h}, line {line}"
            );
        }
    };
    let f = F::new("<p>x</p><pre>ab\n\ncd</pre>");
    if engine::is_gecko() {
        // Positive control: Firefox's own caret after the second newline is
        // on the empty line, not on `cd`'s.
        let line = f.line();
        let top = f.range_rect(4, 4).y() - f.content_origin().1;
        assert!(
            top >= line - 1.0 && top < 2.0 * line,
            "positive control: Firefox's collapsed range after the second newline is on line 1: {top}"
        );
    }
    on_line(&f, CODE + 3, 1.0, "the empty line");
    on_line(&f, CODE + 4, 2.0, "before `cd`");
    f.done();
    let f = F::new("<p>x</p><pre>ab\n</pre>");
    on_line(&f, CODE + 3, 1.0, "after the final newline");
    f.done();
}

// ── The document, directly: shapes the editor does not make ───────────

/// A `WebDocument` with one block holding `texts` as sibling text nodes (and
/// `<b>` wrappers for the ones marked `true`), under `style`.
struct D {
    doc: WebDocument,
    block: NodeId,
}

impl D {
    fn new(style: &str, texts: &[(&str, bool)]) -> Self {
        clear_stale();
        let mut doc = WebDocument::new(document());
        let body = doc.body();
        let block = doc.create_element("div");
        doc.set_attribute(block, HOST, "");
        doc.set_attribute(
            block,
            "style",
            &format!(
                "font-family: monospace; font-size: 16px; line-height: 24px; width: 300px; {style}"
            ),
        );
        doc.append_child(body, block);
        for (t, wrapped) in texts {
            let text = doc.create_text(t);
            if *wrapped {
                let b = doc.create_element("b");
                doc.append_child(b, text);
                doc.append_child(block, b);
            } else {
                doc.append_child(block, text);
            }
        }
        Self { doc, block }
    }
    fn el(&self) -> web_sys::Element {
        document()
            .query_selector(&format!("[{HOST}]"))
            .unwrap()
            .expect("the block")
    }
    /// The caret at `byte` as `(x, top, height)` from the block's top-left.
    fn caret(&self, byte: usize) -> (f64, f64, f64) {
        let (x, y, h) = self
            .doc
            .query_caret_rect_with_affinity(self.block.0 as u64, byte, CaretAffinity::Downstream)
            .expect("a caret rect");
        let r = self.el().get_bounding_client_rect();
        (x as f64 - r.x(), y as f64 - r.y(), h as f64)
    }
    fn done(self) {
        self.el().remove();
        if let Some(root) = document().get_element_by_id("rinch-root") {
            root.remove();
        }
    }
}

/// `pre-wrap` `a\n \nb`: after the space, before the second newline, Chrome
/// gives no rect. The caret is right after the space on line 1 — the start
/// of the newline that follows it — where it was the block's own box.
#[wasm_bindgen_test]
fn pre_wrap_the_caret_after_a_space_before_a_newline() {
    let d = D::new("white-space: pre-wrap", &[("a\n \nb", false)]);
    let (x3, t3, h3) = d.caret(3);
    // The caret before that space: line 1, the start of the line.
    let (x2, t2, _) = d.caret(2);
    assert!(t2 > 20.0 && t2 < 28.0, "the space is on line 1: {t2}");
    assert!(
        (t3 - t2).abs() < 0.5 && x3 > x2 + 5.0,
        "after the space, on its line: ({x3}, {t3}) vs the space at ({x2}, {t2})"
    );
    assert!(h3 > 0.0 && h3 < 25.0, "one line tall: {h3}");
    d.done();
}

/// A text node ending in a newline, followed by one that begins with one
/// (marks split a code block this way): the caret between them is on the
/// empty line between, where #1197's walk found the second node's collapsed
/// start — itself a caret with no rect — and gave up.
#[wasm_bindgen_test]
fn the_caret_between_a_newline_and_a_newline_in_the_next_node() {
    let d = D::new("white-space: pre", &[("ab\n", false), ("\ncd", true)]);
    let (x, t, h) = d.caret(3);
    assert!(
        t > 20.0 && t < 28.0 && x.abs() < 0.5,
        "the empty line, at its start: ({x}, {t})"
    );
    assert!(h > 0.0 && h < 25.0, "one line tall: {h}");
    let (_, t4, _) = d.caret(4);
    assert!(t4 > t + 20.0, "`cd` is the line after: {t4} vs {t}");
    d.done();
}

impl D {
    /// Positive control for a caret on line 1 (24px lines) that the browser's
    /// own collapsed range, at UTF-16 `off` of the block's first child, does
    /// not place: Chrome gives it no rect, and Firefox the box of the newline
    /// before it, on line 0.
    fn assert_own_caret_is_not_on_line_1(&self, off: u32, what: &str) {
        if !engine::is_gecko() {
            assert!(
                self.native_height(0, off) <= 0.0,
                "positive control: no rect {what}"
            );
            return;
        }
        let t = self.el().child_nodes().item(0).expect("the child");
        let r = document().create_range().unwrap();
        r.set_start(&t, off).unwrap();
        r.set_end(&t, off).unwrap();
        let own = r.get_bounding_client_rect();
        let top = own.y() - self.el().get_bounding_client_rect().y();
        assert!(
            own.height() > 0.0 && top < 20.0,
            "positive control: Firefox draws the collapsed range {what} on line 0: top {top}"
        );
    }
    /// The browser's own collapsed-range height at UTF-16 `off` of the
    /// block's `n`th child (a text node).
    fn native_height(&self, n: u32, off: u32) -> f64 {
        let t = self.el().child_nodes().item(n).expect("the child");
        let r = document().create_range().unwrap();
        r.set_start(&t, off).unwrap();
        r.set_end(&t, off).unwrap();
        r.get_bounding_client_rect().height()
    }
}

/// An empty text node at the block's start (review of #1207): Chrome gives
/// the collapsed range in it no rect, in `normal` text as in `pre-wrap`. The
/// caret is where `ab` starts. #1197's walk was guarded by the node ending
/// in `"\n"`, which an empty node does not, so it answered the block's own
/// box; that guard is gone, and this pins its going.
#[wasm_bindgen_test]
fn the_caret_in_an_empty_text_node_is_where_what_follows_starts() {
    let d = D::new("white-space: normal", &[("", false), ("ab", false)]);
    assert!(
        d.native_height(0, 0) <= 0.0,
        "positive control: no rect in an empty text node"
    );
    // The browser's own caret at the start of `ab`: the block's box is one
    // line here, so only this tells the two answers apart.
    let t = d.el().child_nodes().item(1).unwrap();
    let r = document().create_range().unwrap();
    r.set_start(&t, 0).unwrap();
    r.set_end(&t, 0).unwrap();
    let own = r.get_bounding_client_rect();
    let o = d.el().get_bounding_client_rect();
    let (x, top, h) = d.caret(0);
    assert!(
        (x - (own.x() - o.x())).abs() < 0.5
            && (top - (own.y() - o.y())).abs() < 0.5
            && (h - own.height()).abs() < 0.5,
        "the start of `ab`: ({x}, {top}, {h}) vs ({}, {}, {})",
        own.x() - o.x(),
        own.y() - o.y(),
        own.height()
    );
    d.done();
}

/// Before a `"\r"` in `pre` (`ab\r\n\r\ncd`, after the first `\n`): no rect
/// either, and the caret is the start of the empty line 1.
#[wasm_bindgen_test]
fn the_caret_before_a_carriage_return_is_on_its_line() {
    let d = D::new("white-space: pre", &[("ab\r\n\r\ncd", false)]);
    d.assert_own_caret_is_not_on_line_1(4, "before the second `\\r`");
    let (x, t, h) = d.caret(4);
    assert!(
        x.abs() < 0.5 && t > 20.0 && t < 28.0,
        "the start of line 1: ({x}, {t})"
    );
    assert!(h > 0.0 && h < 25.0, "one line tall: {h}");
    d.done();
}

/// Right-to-left: the empty line's caret is at the block's RIGHT edge — its
/// start edge — where the old block-box fallback, like every left-to-right
/// fixture above, put x at the left.
#[wasm_bindgen_test]
fn a_right_to_left_empty_line_caret_is_at_the_right_edge() {
    let d = D::new("white-space: pre; direction: rtl", &[("אבג\n\nדה", false)]);
    d.assert_own_caret_is_not_on_line_1(4, "on the empty line");
    let w = d.el().get_bounding_client_rect().width();
    // `אבג` is 6 bytes, the first `\n` byte 6: the empty line is byte 7.
    let (x, t, h) = d.caret(7);
    assert!(
        (x - w).abs() < 1.0 && t > 20.0 && t < 28.0,
        "the right edge of line 1: ({x}, {t}), block width {w}"
    );
    assert!(h > 0.0 && h < 25.0, "one line tall: {h}");
    d.done();
}
