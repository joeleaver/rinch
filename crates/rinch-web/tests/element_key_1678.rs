//! Browser-driven tests for issue #1678 on the web backend: an element's own
//! `onkeydown` / `onkeyup` (prevent-default, order against the interceptor
//! and the dismiss stack, ancestors, composition), the selection read in
//! UTF-16 units, and the caret rect in client px.
//!
//! A synthetic `keydown` has no default action, so "Enter inserts no line
//! break" is read as the event's `defaultPrevented` — the call that cancels a
//! trusted Enter's line break.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test element_key_1678
//! ```
#![cfg(target_arch = "wasm32")]

#[path = "support/engine.rs"]
mod engine;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-element-key-1678-host";

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    textarea: NodeHandle,
    input: NodeHandle,
    log: Log,
}

impl Fixture {
    fn mount() -> Self {
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
        document().body().unwrap().append_child(&host).unwrap();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let handles: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
        let (log_in, handles_in) = (log.clone(), handles.clone());
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |__scope: &mut RenderScope| {
                let l = log_in.clone();
                let (l1, l2, l3, l4) = (l.clone(), l.clone(), l.clone(), l.clone());
                let node = rsx! {
                    div {
                        style: "padding: 10px",
                        onkeydown: move |k| {
                            l1.borrow_mut().push(format!("outer:{}", k.key));
                            false
                        },
                        onsubmit: move || l2.borrow_mut().push("submit".into()),
                        textarea {
                            id: "k-ta",
                            style: "width: 300px; height: 80px; font: 16px/20px monospace; padding: 4px 6px; border: 2px solid black; margin: 0",
                            onkeydown: move |k: &KeyEventData| {
                                l3.borrow_mut().push(format!(
                                    "down:{}{}",
                                    if k.shift { "S-" } else { "" },
                                    k.key
                                ));
                                (k.key == "Enter" && !k.shift) || k.key == "Escape"
                            },
                            onkeyup: move |k| {
                                l4.borrow_mut().push(format!("up:{}", k.key));
                                true
                            },
                        }
                        input {
                            id: "k-in",
                            style: "width: 200px; height: 30px; font: 16px monospace; padding: 0 8px; border: 1px solid black; margin: 0",
                        }
                    }
                };
                let kids = node.children();
                *handles_in.borrow_mut() = Some((kids[0].clone(), kids[1].clone()));
                node
            },
        );
        let (textarea, input) = handles.borrow_mut().take().expect("handles");
        Self {
            root,
            host,
            textarea,
            input,
            log,
        }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// A key event where the browser sends it: the focused element. Answers
/// whether it was cancelled.
fn key(kind: &str, key: &str, code: &str, shift: bool, composing: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    init.set_shift_key(shift);
    init.set_is_composing(composing);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
    let target: web_sys::EventTarget = document().active_element().unwrap().into();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// Positive control and the shape: a genuine keydown at the focused textarea
/// reaches its `onkeydown` (the delegation is installed by the mount), a
/// consumed Enter is cancelled and reaches no ancestor and no `onsubmit`,
/// and Shift+Enter goes on to the ancestor and is not cancelled.
#[wasm_bindgen_test]
fn onkeydown_consumes_enter_and_shift_enter_goes_on() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();

    assert!(key("keydown", "Enter", "Enter", false, false), "cancelled");
    assert_eq!(f.take_log(), ["down:Enter"], "no ancestor, no submit");

    assert!(
        !key("keydown", "Enter", "Enter", true, false),
        "not cancelled"
    );
    assert_eq!(f.take_log(), ["down:S-Enter", "outer:Enter"]);
    f.teardown();
}

/// Before the interceptor and the dismiss stack, as on desktop: a field's
/// handler takes Escape and the overlay stays; an unconsumed key reaches the
/// interceptor.
#[wasm_bindgen_test]
fn onkeydown_runs_before_the_interceptor_and_the_dismiss_stack() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let s1 = seen.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        s1.borrow_mut().push(k.key.clone());
        false
    });
    let dismissed = Rc::new(Cell::new(0));
    let d1 = dismissed.clone();
    let entry = push_dismiss_handler(0, move || {
        d1.set(d1.get() + 1);
        true
    });

    assert!(key("keydown", "Escape", "Escape", false, false));
    assert_eq!(dismissed.get(), 0, "the field took Escape");
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());

    assert!(!key("keydown", "x", "KeyX", false, false));
    assert_eq!(*seen.borrow(), ["x"]);
    drop(entry);
    rinch_core::clear_keyboard_interceptor();
    f.teardown();
}

/// `onkeyup` hears the release, and a consuming one keeps it from the
/// ancestors (there is no ancestor `onkeyup` here; the release still reaches
/// the interceptor).
#[wasm_bindgen_test]
fn onkeyup_hears_the_release_and_the_interceptor_still_does() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let ups = Rc::new(Cell::new(0));
    let u1 = ups.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        if k.is_up() {
            u1.set(u1.get() + 1);
        }
        false
    });
    key("keyup", "q", "KeyQ", false, false);
    assert_eq!(f.take_log(), ["up:q"]);
    assert_eq!(ups.get(), 1);
    rinch_core::clear_keyboard_interceptor();
    f.teardown();
}

/// A key the input method is composing with is not offered.
#[wasm_bindgen_test]
fn a_composing_key_is_not_offered() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    assert!(!key("keydown", "Enter", "Enter", false, true));
    assert!(!key("keydown", "Process", "KeyN", false, false));
    assert!(
        !f.take_log().iter().any(|e| e.starts_with("down:")),
        "not offered while composing"
    );
    f.teardown();
}

/// The selection reads back as the browser holds it, in UTF-16 units, with
/// its direction; a fresh control reads `(0, 0, …)` as desktop answers it.
#[wasm_bindgen_test]
fn selection_range_reads_the_controls_own_selection() {
    let f = Fixture::mount();
    let (s, e, d) = f.textarea.selection_range().expect("a text control");
    assert_eq!((s, e), (0, 0), "a fresh control");
    // Chrome's direction for a caret nobody set; desktop answers the same.
    assert_eq!(d, SelectionDirection::Forward, "fresh direction");

    let ta: web_sys::HtmlTextAreaElement = f.el("k-ta").dyn_into().unwrap();
    ta.set_value("a😀bc");
    ta.set_selection_range_with_direction(3, 5, "backward")
        .unwrap();
    assert_eq!(
        f.textarea.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );
    f.textarea
        .set_selection_range(1, 3, SelectionDirection::Forward);
    assert_eq!(
        f.textarea.selection_range(),
        Some((1, 3, SelectionDirection::Forward))
    );
    f.teardown();
}

/// The caret rect is in client px, at the control's content box: offset 0
/// at the box's left + border + padding, offset 2 two monospace cells
/// further, the start of line 2 one `line-height` (20px) lower; a
/// single-line field's caret is inside its box.
#[wasm_bindgen_test]
fn caret_rect_is_in_client_px_at_the_offset() {
    let f = Fixture::mount();
    let ta: web_sys::HtmlTextAreaElement = f.el("k-ta").dyn_into().unwrap();
    ta.set_value("ab\ncd");
    let r = f.el("k-ta").get_bounding_client_rect();

    let c0 = f.textarea.caret_rect_at(0).expect("laid out");
    let c2 = f.textarea.caret_rect_at(2).unwrap();
    let c3 = f.textarea.caret_rect_at(3).unwrap();
    assert!(
        (c0.x as f64 - (r.left() + 2.0 + 6.0)).abs() < 0.5,
        "{c0:?} {}",
        r.left()
    );
    // The caret is the text's own height (the font's content area, a little
    // less than the 20px line), centred in its line.
    assert!(c0.height > 15.0 && c0.height <= 20.0, "{c0:?}");
    let line_top = r.top() + 2.0 + 4.0;
    assert!(
        (c0.y as f64 - (line_top + (20.0 - c0.height as f64) / 2.0)).abs() < 1.0,
        "{c0:?} {line_top}"
    );

    let probe = document().create_element("span").unwrap();
    probe
        .set_attribute("style", "font: 16px/20px monospace; white-space: pre")
        .unwrap();
    probe.set_text_content(Some("ab"));
    document().body().unwrap().append_child(&probe).unwrap();
    let ab = probe.get_bounding_client_rect().width() as f32;
    probe.remove();
    assert!((c2.x - c0.x - ab).abs() < 0.5, "{c0:?} {c2:?} ab={ab}");
    assert!(
        (c3.x - c0.x).abs() < 0.5 && (c3.y - c0.y - 20.0).abs() < 0.5,
        "{c0:?} {c3:?}"
    );

    // The head: a caret set at 4 is drawn where offset 4 is.
    f.textarea
        .set_selection_range(4, 4, SelectionDirection::Forward);
    let head = f.textarea.caret_rect().unwrap();
    let at4 = f.textarea.caret_rect_at(4).unwrap();
    assert_eq!((head.x, head.y), (at4.x, at4.y));

    let ir = f.el("k-in").get_bounding_client_rect();
    let ic = f.input.caret_rect().expect("an empty field has a caret");
    assert!(
        (ic.x as f64 - (ir.left() + 1.0 + 8.0)).abs() < 0.5,
        "{ic:?} box left {} clientLeft {} padding-left {:?}",
        ir.left(),
        f.el("k-in").client_left(),
        web_sys::window()
            .unwrap()
            .get_computed_style(&f.el("k-in"))
            .unwrap()
            .unwrap()
            .get_property_value("padding-left")
    );
    assert!(
        (ic.y as f64) > ir.top() && ((ic.y + ic.height) as f64) < ir.bottom(),
        "{ic:?} in {}..{}",
        ir.top(),
        ir.bottom()
    );
    f.teardown();
}

/// Mount `build`'s tree in a fresh host; answers the root, the host and the
/// handles `build` hands back.
fn mount_with(
    build: impl Fn(&mut RenderScope) -> (NodeHandle, Vec<NodeHandle>) + 'static,
) -> (RootHandle, web_sys::Element, Vec<NodeHandle>) {
    // A host a failed test left behind would put its controls under this
    // test's points.
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
    document().body().unwrap().append_child(&host).unwrap();
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let (r, hs) = build(s);
        *o.borrow_mut() = hs;
        r
    });
    let hs = out.borrow().clone();
    (root, host, hs)
}

/// What the browser's own caret-from-point answers at `(x, y)` for control
/// `id` — the oracle a caret rect has to agree with.
#[derive(Debug, PartialEq)]
enum Probe {
    /// `caretPositionFromPoint` names the control and this offset.
    At(usize),
    /// Another element is on top at that point (`elementFromPoint`): the
    /// point says nothing about this control.
    Covered,
    /// The control is on top, but caret-from-point names a node inside its
    /// anonymous content (Firefox 158 answers its inner `<div>` with a child
    /// index, `DIV:1`, for some points) or nothing: no offset to compare.
    Unanswered(String),
}

fn caret_from_point(id: &str, x: f32, y: f32) -> Probe {
    let f = js_sys::Function::new_with_args(
        "id,x,y",
        "const el = document.getElementById(id); \
         if (document.elementFromPoint(x, y) !== el) return 'covered'; \
         const p = document.caretPositionFromPoint(x, y); \
         if (!p) return 'none'; \
         if (p.offsetNode === el) return 'at:' + p.offset; \
         return (p.offsetNode.id || p.offsetNode.nodeName) + ':' + p.offset",
    );
    let got = f
        .call3(
            &wasm_bindgen::JsValue::NULL,
            &id.into(),
            &(x as f64).into(),
            &(y as f64).into(),
        )
        .unwrap()
        .as_string()
        .unwrap();
    match got.strip_prefix("at:") {
        Some(o) => Probe::At(o.parse().unwrap()),
        None if got == "covered" => Probe::Covered,
        None => Probe::Unanswered(got),
    }
}

/// Probe each `(offset, rect)` and fold the answers: the mismatches, and
/// how many points were compared and how many the oracle could not answer.
/// A control passes when nothing compared disagrees, at least `min` points
/// were compared, and the unanswered ones are under a quarter of those
/// asked — so most points are always checked.
#[derive(Default)]
struct Tally {
    /// Points answered with the other side of a space a soft wrap hangs at a
    /// line's end (see `probe`), reported with the verdict.
    excluded: usize,
    compared: usize,
    unanswered: usize,
    covered: usize,
    wrong: Vec<String>,
}

/// Whether the character at UTF-16 offset `at` of control `id`'s value is a
/// space that ends its line at a soft wrap: the carets either side of it are
/// on one line and the caret after it is not on the next offset's line.
fn hanging_space(h: &NodeHandle, id: &str, at: usize) -> bool {
    let el = document().get_element_by_id(id).unwrap();
    let value = if let Some(t) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        t.value()
    } else {
        el.dyn_ref::<web_sys::HtmlInputElement>().unwrap().value()
    };
    let units: Vec<u16> = value.encode_utf16().collect();
    if units.get(at) != Some(&u16::from(b' ')) {
        return false;
    }
    let y = |o: usize| h.caret_rect_at(o).map(|r| r.y);
    match (y(at), y(at + 1), y(at + 2)) {
        (Some(a), Some(b), Some(c)) => (a - b).abs() <= 1.0 && (c - b).abs() > 1.0,
        _ => false,
    }
}

impl Tally {
    fn probe(&mut self, h: &NodeHandle, id: &str, o: usize, x: f32, y: f32) {
        match caret_from_point(id, x, y) {
            Probe::At(got) if got == o => self.compared += 1,
            // The two sides of the space a soft wrap hangs at the end of a
            // line: adjacent offsets on one line with the line's last
            // character, a space, between them. The engines draw a hanging
            // space differently at the line's edge (Firefox 158 squeezes it,
            // so its caret-from-point answers the space's start there, where
            // Chrome 155 and Firefox 155 answer its end), so the answer is
            // not a mismatch of the caret rect: excluded and counted.
            Probe::At(got) if got.abs_diff(o) == 1 && hanging_space(h, id, o.min(got)) => {
                self.excluded += 1
            }
            Probe::At(got) => {
                self.compared += 1;
                self.wrong.push(format!("{o} -> {got} at ({x:.1},{y:.1})"));
            }
            Probe::Covered => self.covered += 1,
            Probe::Unanswered(_) => self.unanswered += 1,
        }
    }

    fn verdict(&self, id: &str, min: usize) -> Option<String> {
        let asked = self.compared + self.unanswered;
        let ok =
            self.wrong.is_empty() && self.compared >= min && self.unanswered * 4 < asked.max(1);
        (!ok).then(|| {
            format!(
                "{id}: compared {}, excluded {}, unanswered {}, covered {}, wrong {:?}",
                self.compared,
                self.excluded,
                self.unanswered,
                self.covered,
                &self.wrong[..self.wrong.len().min(6)]
            )
        })
    }
}

/// Review of #1686, F1/F6 and the web mutants: at the centre of the rect
/// for an offset, the browser's own caret-from-point answers that offset —
/// a textarea with a vertical scrollbar scrolled to its end, a centred
/// single-line field, a field under `transform: scale(2)`, a horizontally
/// scrolled long field, a password field and spaced text.
#[wasm_bindgen_test]
fn caret_rect_agrees_with_caret_position_from_point() {
    let long: String = (0..40).map(|i| format!("word{i} ")).collect();
    let n = long.encode_utf16().count();
    let l = long.clone();
    let (root, host, hs) = mount_with(move |__scope| {
        let l1 = l.clone();
        let l2 = l.clone();
        let r = rsx! {
            div {
                textarea { id: "c0", style: "width: 300px; height: 80px; font: 16px/20px sans-serif; padding: 4px 6px; border: 2px solid black; margin: 0", "hello world" }
                textarea { id: "c1", style: "width: 200px; height: 60px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; overflow-y: scroll", {l1.clone()} }
                input { id: "c2", style: "width: 200px; height: 30px; font: 16px sans-serif; text-align: center", value: "abc" }
                input { id: "c4", style: "width: 100px; height: 30px; font: 16px sans-serif", value: {l2.clone()} }
                textarea { id: "c5", style: "width: 200px; height: 60px; font: 16px/20px sans-serif; letter-spacing: 3px", "abc def" }
                textarea { id: "c7", style: "width: 200px; height: 100px; font: 16px/20px sans-serif", "x\n\ny\nz" }
                input { id: "c6", r#type: "password", style: "width: 200px; height: 40px; font: 16px sans-serif", value: "abcdef" }
                // Last, so the scaled box paints over nothing else.
                div { style: "transform: scale(2); transform-origin: 0 0; width: 300px",
                    input { id: "c3", style: "width: 120px; height: 30px; font: 16px sans-serif", value: "abcdef" }
                }
            }
        };
        let k = r.children();
        let scaled = k[7].children();
        (
            r,
            vec![
                k[0].clone(),
                k[1].clone(),
                k[2].clone(),
                scaled[0].clone(),
                k[3].clone(),
                k[4].clone(),
                k[6].clone(),
                k[5].clone(),
            ],
        )
    });
    let by = |id: &str| -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap()
            .dyn_into()
            .unwrap()
    };
    by("c1").set_scroll_top(100_000);
    by("c4").set_scroll_left(100_000);
    assert!(by("c1").scroll_top() > 0, "positive control: c1 scrolls");
    assert!(by("c4").scroll_left() > 0, "positive control: c4 scrolls");
    let mut bad = Vec::new();
    let mut check = |h: &NodeHandle, id: &str, offs: &[usize]| {
        let mut t = Tally::default();
        for &o in offs {
            let r = h.caret_rect_at(o).expect("a rendered control");
            t.probe(h, id, o, r.x, r.y + r.height / 2.0);
        }
        bad.extend(t.verdict(id, offs.len().div_ceil(2)));
    };
    check(&hs[0], "c0", &[0, 1, 5, 6, 11]);
    let tail: Vec<usize> = (n - 40..n).step_by(3).collect();
    check(&hs[1], "c1", &tail);
    check(&hs[2], "c2", &[0, 1, 3]);
    check(&hs[3], "c3", &[0, 3, 6]);
    check(&hs[4], "c4", &[n - 3, n]);
    check(&hs[5], "c5", &[1, 3, 5]);
    check(&hs[6], "c6", &[0, 2, 6]);
    // Line starts after a break, an empty line included (Gecko puts a
    // collapsed range after a break at the end of the line before).
    check(&hs[7], "c7", &[0, 1, 2, 3, 4, 5]);
    // A single-line field's caret-from-point answers its one line at any
    // height, so its vertical centring is checked against the box itself.
    for (h, id) in [(&hs[2], "c2"), (&hs[6], "c6")] {
        let r = h.caret_rect_at(1).unwrap();
        let b = by(id).get_bounding_client_rect();
        let mid = (b.top() + b.bottom()) / 2.0;
        assert!(
            ((r.y + r.height / 2.0) as f64 - mid).abs() < 1.5,
            "{id}: {r:?} centred in {}..{}",
            b.top(),
            b.bottom()
        );
    }
    // Torn down before the verdict, so a failure leaves nothing on the page
    // for the next test's points to land on.
    root.unmount();
    host.remove();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Review of #1686, F3/F5/F7: an unfocused control with a value reads what
/// desktop reads (Chrome's answers), an unspecified direction reads
/// `Forward`, and a `display: none` field has no caret rect.
#[wasm_bindgen_test]
fn an_unfocused_controls_selection_and_a_hidden_caret() {
    let (root, host, hs) = mount_with(|__scope| {
        let r = rsx! {
            div {
                input { value: "abc" }
                input {}
                textarea {}
                input { r#type: "email", value: "a@b" }
                textarea { "hello" }
                input { style: "display: none", value: "abc" }
            }
        };
        let mut k = r.children();
        k.push(r.clone());
        (r, k)
    });
    let fwd = SelectionDirection::Forward;
    assert_eq!(
        hs[0].selection_range(),
        Some((3, 3, fwd)),
        "input value=abc"
    );
    assert_eq!(hs[1].selection_range(), Some((0, 0, fwd)), "empty input");
    hs[1].set_attribute("value", "hello");
    assert_eq!(
        hs[1].selection_range(),
        Some((5, 5, fwd)),
        "after a value write"
    );
    hs[1].set_selection_range(1, 2, fwd);
    hs[1].set_attribute("value", "hello world");
    assert_eq!(
        hs[1].selection_range(),
        Some((11, 11, fwd)),
        "a write moves it to the end"
    );
    hs[2].set_attribute("value", "xy");
    assert_eq!(
        hs[2].selection_range(),
        Some((2, 2, fwd)),
        "textarea value write"
    );
    assert_eq!(hs[3].selection_range(), None, "email");
    assert_eq!(
        hs[4].selection_range(),
        Some((0, 0, fwd)),
        "textarea text child"
    );
    hs[1].set_selection_range(2, 4, SelectionDirection::None);
    assert_eq!(
        hs[1].selection_range(),
        Some((2, 4, fwd)),
        "None reads Forward"
    );
    assert_eq!(hs[5].caret_rect(), None, "display: none");
    assert!(hs[0].caret_rect().is_some(), "positive control");
    // Parsed markup sets the value with no setter: Chrome 155 reads a caret
    // at the start, and so does desktop.
    let parent = hs[6].clone();
    parent.set_inner_html("<input value=\"abcd\">");
    let parsed = parent.children()[0].clone();
    assert_eq!(parsed.selection_range(), Some((0, 0, fwd)), "parsed input");
    root.unmount();
    host.remove();
}

/// A `keyCode` 229 Enter that is not marked composing (Safari's IME-confirm
/// Enter, and an Android soft keyboard's plain Enter) is offered, and still
/// submits: refusing it would break Enter on Android. The `Process` key is
/// not offered (`a_composing_key_is_not_offered`).
#[wasm_bindgen_test]
fn a_key_code_229_enter_is_offered() {
    let f = Fixture::mount();
    f.el("k-ta").focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    init.set_code("Enter");
    js_sys::Reflect::set(&init, &"keyCode".into(), &229.into()).unwrap();
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    assert_eq!(ev.key_code(), 229, "positive control: the init carried it");
    let target: web_sys::EventTarget = document().active_element().unwrap().into();
    target.dispatch_event(&ev).unwrap();
    assert!(ev.default_prevented(), "the field's handler took it");
    assert_eq!(f.take_log(), ["down:Enter"]);
    f.teardown();
}

/// Every offset whose rect's centre lies inside `id`'s box: the ones
/// `caretPositionFromPoint` does not answer as that offset. Answers
/// `(checked, mismatches)`.
fn oracle_in_box(h: &NodeHandle, id: &str, offs: impl Iterator<Item = usize>) -> Tally {
    let el: web_sys::HtmlElement = document()
        .get_element_by_id(id)
        .unwrap()
        .dyn_into()
        .unwrap();
    let r = el.get_bounding_client_rect();
    // The padding box (no scrollbar), scaled as the control is painted.
    let (sx, sy) = (
        r.width() / el.offset_width() as f64,
        r.height() / el.offset_height() as f64,
    );
    let left = r.left() + el.client_left() as f64 * sx;
    let top = r.top() + el.client_top() as f64 * sy;
    let right = left + el.client_width() as f64 * sx;
    let mut bottom = top + el.client_height() as f64 * sy;
    // Gecko's scrollbars overlay the content and `clientHeight` keeps the
    // strip they cover, where caret-from-point answers the scrollbar's own
    // position (Firefox 155: offset 1). Chrome's take the strip out of
    // `clientHeight`. So under Gecko the strip of a horizontally scrolling box
    // is not sampled.
    if engine::is_gecko() && el.scroll_width() > el.client_width() {
        bottom -= 12.0 * sy;
    }
    let mut t = Tally::default();
    for o in offs {
        let Some(c) = h.caret_rect_at(o) else {
            t.wrong.push(format!("{o}: None"));
            continue;
        };
        let (x, y) = (c.x as f64, (c.y + c.height / 2.0) as f64);
        let win = web_sys::window().unwrap();
        let (vw, vh) = (
            win.inner_width().unwrap().as_f64().unwrap(),
            win.inner_height().unwrap().as_f64().unwrap(),
        );
        // Inside the box and the viewport (`caretPositionFromPoint` answers
        // nothing outside the viewport).
        if x < left.max(0.0) + 3.0
            || x > right.min(vw) - 3.0
            || y < top.max(0.0) + 3.0
            || y > bottom.min(vh) - 3.0
        {
            continue;
        }
        t.probe(h, id, o, x as f32, y as f32);
    }
    t
}

/// Review 2 of #1686, G2/G3/G4: right-to-left text (Latin and Hebrew), a
/// `wrap="off"` and a `white-space: pre` textarea scrolled both ways, and a
/// transform or zoom on `<body>` / `<html>` — each against the browser's own
/// caret-from-point.
#[wasm_bindgen_test]
fn caret_rect_agrees_for_rtl_unwrapped_and_a_scaled_page() {
    let long: String = (0..40).map(|i| format!("word{i} ")).collect();
    let heb: String = (0..30).map(|_| "שלום עולם ").collect();
    let lines: String = (0..12)
        .map(|i| format!("line {i} with a long tail that does not wrap here\n"))
        .collect();
    let (n, nh, nl) = (
        long.encode_utf16().count(),
        heb.encode_utf16().count(),
        lines.encode_utf16().count(),
    );
    let (l, h, ls) = (long.clone(), heb.clone(), lines.clone());
    let (root, host, hs) = mount_with(move |__scope| {
        let (a, b, c, d, e) = (l.clone(), l.clone(), h.clone(), ls.clone(), ls.clone());
        let r = rsx! {
            div {
                input { id: "s1", style: "width: 120px; height: 30px; font: 16px sans-serif; display: inline-block; vertical-align: top", value: "abcdef" }
                textarea { id: "r1", dir: "rtl", style: "width: 200px; height: 300px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; overflow-y: hidden; display: inline-block; vertical-align: top", {a.clone()} }
                textarea { id: "r2", dir: "rtl", style: "width: 200px; height: 120px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; overflow-y: scroll; display: inline-block; vertical-align: top", {b.clone()} }
                textarea { id: "r4", dir: "rtl", style: "width: 200px; height: 300px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; overflow-y: hidden; display: inline-block; vertical-align: top", {c.clone()} }
                textarea { id: "w1", wrap: "off", style: "width: 200px; height: 80px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; display: inline-block; vertical-align: top", {d.clone()} }
                textarea { id: "w2", style: "width: 200px; height: 80px; font: 16px/20px sans-serif; padding: 4px; border: 1px solid; white-space: pre; display: inline-block; vertical-align: top", {e.clone()} }
            }
        };
        let k = r.children();
        // r1, r2, r4, w1, w2, s1, by id (s1 is first in the tree, so it stays
        // in the viewport under the page's scale).
        let pick = |id: &str| {
            k.iter()
                .find(|n| n.get_attribute("id").as_deref() == Some(id))
                .cloned()
                .unwrap_or_else(|| panic!("no #{id}"))
        };
        let hs = ["r1", "r2", "r4", "w1", "w2", "s1"].map(pick).to_vec();
        (r, hs)
    });
    let by = |id: &str| -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap()
            .dyn_into()
            .unwrap()
    };
    by("r2").set_scroll_top(100_000);
    for id in ["w1", "w2"] {
        by(id).set_scroll_left(150);
        by(id).set_scroll_top(60);
        assert!(
            by(id).scroll_left() > 0,
            "positive control: {id} scrolls across"
        );
    }
    let mut report = Vec::new();
    // In a right-to-left textarea of left-to-right text, offset 0 (the run's
    // start) and the first line's end (its trailing space, which the bidi
    // algorithm puts at the line's left) are one visual point, and
    // caret-from-point answers the latter there: offset 0 is not compared in
    // r1. The two sides of a hanging space at a soft wrap are excluded by
    // `Tally::probe` and counted.
    let mut run = |h: &NodeHandle, id: &str, len: usize, min: usize| {
        let first = usize::from(id == "r1");
        report.extend(oracle_in_box(h, id, first..=len).verdict(id, min));
    };
    // Positive control for the exclusion: r1's text wraps at spaces, so it
    // has hanging spaces (where CI's Firefox 158 answers across, 94/95,
    // 115/116, … in its fonts), and a space in the middle of a line is not one.
    let hanging = (0..n).filter(|&a| hanging_space(&hs[0], "r1", a)).count();
    assert!(hanging >= 5, "r1 has {hanging} hanging spaces");
    assert!(!hanging_space(&hs[0], "r1", 5), "r1@5 is mid-line");
    run(&hs[0], "r1", n, 100);
    run(&hs[1], "r2", n, 40);
    run(&hs[2], "r4", nh, 100);
    run(&hs[3], "w1", nl, 10);
    run(&hs[4], "w2", nl, 10);

    let body: web_sys::HtmlElement = document().body().unwrap();
    let html: web_sys::HtmlElement = document().document_element().unwrap().dyn_into().unwrap();
    body.style()
        .set_property("transform", "scale(1.5)")
        .unwrap();
    body.style()
        .set_property("transform-origin", "0 0")
        .unwrap();
    run(&hs[5], "s1", 6, 4);
    run(&hs[0], "r1", n, 50);
    body.style().remove_property("transform").unwrap();
    body.style().remove_property("transform-origin").unwrap();
    body.style().set_property("zoom", "1.5").unwrap();
    run(&hs[5], "s1", 6, 4);
    body.style().remove_property("zoom").unwrap();
    html.style().set_property("zoom", "1.25").unwrap();
    run(&hs[5], "s1", 6, 4);
    html.style().remove_property("zoom").unwrap();
    root.unmount();
    host.remove();
    assert!(report.is_empty(), "{report:#?}");
}
