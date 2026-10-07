#![cfg(target_arch = "wasm32")]
//! Emulation fixtures for #1466 (added in review, 2026-10-07).
//!
//! 1. `dump_*`: every caret position in a set of shapes, both affinities, as
//!    exact `f32` bits, so Chrome's answer can be compared byte for byte with
//!    the src hunk reverted. Panics with the dump (that is how it prints).
//! 2. `emulated_gecko_*`: Firefox's answer forced in Chrome by patching
//!    `Range.prototype.getBoundingClientRect` so a collapsed range right after
//!    a `"\n"` answers that newline's own box (the end of the line it closes),
//!    as the PR measured in Firefox 155/157. Pins the new branch in Chrome.

use rinch_core::dom::{CaretAffinity, DomDocument, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::Pos;
use rinch_web::{EditorHandle, RootHandle, WebDocument, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-review-ff";

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

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl F {
    fn new(html: &str, width: u32) -> Self {
        clear_stale();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST, "").unwrap();
        host.set_attribute(
            "style",
            &format!("font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: {width}px;"),
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
    fn origin(&self) -> (f64, f64) {
        let r = self.host.get_bounding_client_rect();
        (r.x(), r.y())
    }
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn dump_editor(out: &mut String, html: &str, width: u32) {
    let f = F::new(html, width);
    let (ox, oy) = f.origin();
    out.push_str(&format!("== {html} @{width}\n"));
    for p in 0..40usize {
        for a in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
            let r = f.handle.caret_rect_with_affinity(Pos(p), a);
            let s = match r {
                Some(r) => format!(
                    "{:.4} {:.4} {:.4}",
                    r.x as f64 - ox,
                    r.y as f64 - oy,
                    r.height
                ),
                None => "None".into(),
            };
            out.push_str(&format!("{p} {a:?}: {s}\n"));
        }
    }
    f.done();
}

fn dump_doc(out: &mut String, style: &str, texts: &[&str]) {
    clear_stale();
    let mut doc = WebDocument::new(document());
    let body = doc.body();
    let block = doc.create_element("div");
    doc.set_attribute(block, HOST, "");
    doc.set_attribute(
        block,
        "style",
        &format!(
            "font-family: monospace; font-size: 16px; line-height: 24px; width: 120px; {style}"
        ),
    );
    doc.append_child(body, block);
    for t in texts {
        let n = doc.create_text(t);
        doc.append_child(block, n);
    }
    let el: web_sys::Element = document()
        .query_selector(&format!("[{HOST}]"))
        .unwrap()
        .unwrap();
    let r = el.get_bounding_client_rect();
    out.push_str(&format!("== doc {style} {texts:?}\n"));
    let total: usize = texts.iter().map(|t| t.len()).sum();
    for b in 0..=total {
        for a in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
            let s = match doc.query_caret_rect_with_affinity(block.0 as u64, b, a) {
                Some((x, y, h)) => {
                    format!("{:.4} {:.4} {:.4}", x as f64 - r.x(), y as f64 - r.y(), h)
                }
                None => "None".into(),
            };
            out.push_str(&format!("{b} {a:?}: {s}\n"));
        }
    }
    el.remove();
    if let Some(root) = document().get_element_by_id("rinch-root") {
        root.remove();
    }
}

#[wasm_bindgen_test]
#[ignore = "review probe: prints every caret by panicking; diff two runs"]
fn dump_chrome_carets() {
    let mut out = String::new();
    dump_editor(&mut out, "<p>x</p><pre>ab\n\ncd</pre>", 400);
    dump_editor(&mut out, "<pre>ab\n</pre>", 400);
    dump_editor(&mut out, "<pre>ab\n\n</pre>", 400);
    dump_editor(&mut out, "<pre>\nab\n\n\ncd\nef</pre>", 400);
    dump_editor(&mut out, "<p>ab<br>cd</p><p>ab<br></p>", 400);
    dump_editor(&mut out, "<p>aaaa bbbb cccc dddd eeee</p>", 60);
    dump_editor(&mut out, "<pre>aaaaaaaaaaaaaaaa\nbb</pre>", 60);
    dump_doc(&mut out, "white-space: pre", &["ab\n\ncd"]);
    dump_doc(&mut out, "white-space: pre-wrap", &["a\n \nb"]);
    dump_doc(&mut out, "white-space: pre-wrap", &["aaaaaaaaaaaaaaaaa\nb"]);
    dump_doc(&mut out, "white-space: pre-line", &["a  \n  b"]);
    dump_doc(
        &mut out,
        "white-space: normal",
        &["aaaa bbb\ncccc dddd eeee"],
    );
    dump_doc(&mut out, "white-space: break-spaces", &["ab\n\ncd"]);
    dump_doc(&mut out, "white-space: pre", &["ab\r\n\r\ncd"]);
    dump_doc(&mut out, "white-space: pre; direction: rtl", &["אבג\n\nדה"]);
    dump_doc(&mut out, "white-space: pre", &["ab\n", "\ncd"]);
    panic!("DUMP-BEGIN\n{out}DUMP-END");
}

// ── Firefox's answer, forced in Chrome ──────────────────────────────────

fn eval(js: &str) -> wasm_bindgen::JsValue {
    js_sys::eval(js).expect("eval")
}

/// Install (true) / remove (false) the Gecko emulation. Returns whether the
/// patched function changed the answer for `ab\n\ncd` offset 3 (control).
fn gecko_emulation(on: bool) {
    if on {
        eval(
            r#"(function(){
  if (window.__rv_ff_orig) return;
  const orig = Range.prototype.getBoundingClientRect;
  window.__rv_ff_orig = orig;
  Range.prototype.getBoundingClientRect = function () {
    const n = this.startContainer;
    if (this.collapsed && n.nodeType === 3 && this.startOffset > 0 &&
        n.data[this.startOffset - 1] === '\n') {
      const r = document.createRange();
      r.setStart(n, this.startOffset - 1);
      r.setEnd(n, this.startOffset);
      const rects = r.getClientRects();
      if (rects.length > 0) {
        const b = rects[0];
        return new DOMRect(b.x, b.y, 0, b.height);
      }
    }
    return orig.call(this);
  };
})()"#,
        );
    } else {
        eval(
            r#"(function(){ if (window.__rv_ff_orig) { Range.prototype.getBoundingClientRect = window.__rv_ff_orig; delete window.__rv_ff_orig; } })()"#,
        );
    }
}

fn pre_text(f: &F) -> web_sys::Node {
    let pre = f.host.query_selector("pre").unwrap().unwrap();
    let kids = pre.child_nodes();
    (0..kids.length())
        .filter_map(|i| kids.item(i))
        .find(|c| c.node_type() == 3)
        .unwrap()
}

fn line_of(f: &F, pos: usize, a: CaretAffinity) -> f64 {
    let pre = f.host.query_selector("pre").unwrap().unwrap();
    let pr = pre.get_bounding_client_rect();
    let r = f
        .handle
        .caret_rect_with_affinity(Pos(pos), a)
        .expect("a rect");
    // 24px lines; the caret's middle, from the pre's top (its padding is
    // whatever the editor sheet says, so subtract the first line's caret).
    r.y as f64 + r.height as f64 / 2.0 - pr.y()
}

#[wasm_bindgen_test]
fn emulated_gecko_the_caret_after_a_newline_is_on_the_next_line() {
    let f = F::new("<p>x</p><pre>ab\n\ncd</pre>", 400);
    let base = line_of(&f, 4, CaretAffinity::Downstream); // before `a`: line 0
    gecko_emulation(true);
    // Positive control: the emulated collapsed range after the first newline
    // is on line 0 (the newline's box), where the caret belongs on line 1.
    let t = pre_text(&f);
    let r = document().create_range().unwrap();
    r.set_start(&t, 3).unwrap();
    r.set_end(&t, 3).unwrap();
    let own = r.get_bounding_client_rect();
    let pre = f.host.query_selector("pre").unwrap().unwrap();
    let own_mid = own.y() + own.height() / 2.0 - pre.get_bounding_client_rect().y();
    let mut errs = vec![];
    if !(own.height() > 0.0 && (own_mid - base).abs() < 12.0) {
        errs.push(format!(
            "CONTROL: emulated rect not on line 0: h {} mid {own_mid} base {base}",
            own.height()
        ));
    }
    for (pos, n) in [(7usize, 1.0f64), (8, 2.0)] {
        for a in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
            let mid = line_of(&f, pos, a);
            if (mid - (base + 24.0 * n)).abs() > 6.0 {
                errs.push(format!(
                    "pos {pos} {a:?}: mid {mid}, want line {n} (base {base})"
                ));
            }
        }
    }
    gecko_emulation(false);
    f.done();
    let f = F::new("<pre>ab\n</pre>", 400);
    let base = line_of(&f, 1, CaretAffinity::Downstream);
    gecko_emulation(true);
    for a in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
        let mid = line_of(&f, 4, a);
        if (mid - (base + 24.0)).abs() > 6.0 {
            errs.push(format!(
                "ab\\n end {a:?}: mid {mid}, want line 1 (base {base})"
            ));
        }
    }
    gecko_emulation(false);
    f.done();
    assert!(errs.is_empty(), "{errs:#?}");
}

/// In collapsing text a `"\n"` is a space: the caret after it at a soft wrap
/// is an ordinary wrap-point caret, `Upstream` at the end of the upper line.
/// With Firefox's newline-box answer emulated, the white-space guard of
/// `follows_a_preserved_newline` is what keeps it there (kills the mutant that
/// drops that guard, which Chrome's own answers cannot see).
#[wasm_bindgen_test]
fn emulated_gecko_a_collapsible_newline_at_a_wrap_keeps_its_upstream_caret() {
    clear_stale();
    let mut doc = WebDocument::new(document());
    let body = doc.body();
    let block = doc.create_element("div");
    doc.set_attribute(block, HOST, "");
    doc.set_attribute(
        block,
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; width: 120px; white-space: normal",
    );
    doc.append_child(body, block);
    let t = doc.create_text("aaaa bbb\ncccc dddd eeee");
    doc.append_child(block, t);
    let el: web_sys::Element = document()
        .query_selector(&format!("[{HOST}]"))
        .unwrap()
        .unwrap();
    let top = el.get_bounding_client_rect().y();
    let left = el.get_bounding_client_rect().x();
    gecko_emulation(true);
    let up = doc.query_caret_rect_with_affinity(block.0 as u64, 9, CaretAffinity::Upstream);
    let down = doc.query_caret_rect_with_affinity(block.0 as u64, 9, CaretAffinity::Downstream);
    gecko_emulation(false);
    el.remove();
    let (_, uy, _) = up.expect("upstream rect");
    let (dx, dy, _) = down.expect("downstream rect");
    assert!(
        (uy as f64 - top) < 20.0,
        "Upstream after a collapsible newline at a wrap: the upper line, got {}",
        uy as f64 - top
    );
    assert!(
        (dy as f64 - top) > 20.0 && (dx as f64 - left).abs() < 1.0,
        "Downstream: the start of the lower line"
    );
}
