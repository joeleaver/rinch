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
        "{ic:?}"
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

/// The offset `caretPositionFromPoint` answers at `(x, y)`, as
/// `"<id>:<offset>"` — the browser's own caret-from-point, the oracle a
/// caret rect has to agree with.
fn caret_from_point(x: f32, y: f32) -> String {
    let f = js_sys::Function::new_with_args(
        "x,y",
        "const p=document.caretPositionFromPoint(x,y); \
         return p ? ((p.offsetNode.id||p.offsetNode.nodeName)+':'+p.offset) : 'null'",
    );
    f.call2(
        &wasm_bindgen::JsValue::NULL,
        &(x as f64).into(),
        &(y as f64).into(),
    )
    .unwrap()
    .as_string()
    .unwrap()
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
                input { id: "c6", r#type: "password", style: "width: 200px; height: 40px; font: 16px sans-serif", value: "abcdef" }
                // Last, so the scaled box paints over nothing else.
                div { style: "transform: scale(2); transform-origin: 0 0; width: 300px",
                    input { id: "c3", style: "width: 120px; height: 30px; font: 16px sans-serif", value: "abcdef" }
                }
            }
        };
        let k = r.children();
        let scaled = k[6].children();
        (
            r,
            vec![
                k[0].clone(),
                k[1].clone(),
                k[2].clone(),
                scaled[0].clone(),
                k[3].clone(),
                k[4].clone(),
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
        for &o in offs {
            let r = h.caret_rect_at(o).expect("a rendered control");
            let got = caret_from_point(r.x, r.y + r.height / 2.0);
            if got != format!("{id}:{o}") {
                bad.push(format!("{id}@{o} -> {got} ({r:?})"));
            }
        }
    };
    check(&hs[0], "c0", &[0, 1, 5, 6, 11]);
    let tail: Vec<usize> = (n - 40..n).step_by(3).collect();
    check(&hs[1], "c1", &tail);
    check(&hs[2], "c2", &[0, 1, 3]);
    check(&hs[3], "c3", &[0, 3, 6]);
    check(&hs[4], "c4", &[n - 3, n]);
    check(&hs[5], "c5", &[1, 3, 5]);
    check(&hs[6], "c6", &[0, 2, 6]);
    assert!(bad.is_empty(), "{bad:#?}");
    root.unmount();
    host.remove();
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

/// Safari's composition-confirming Enter: `isComposing` false, `keyCode`
/// 229. Not offered.
#[wasm_bindgen_test]
fn a_key_code_229_key_is_not_offered() {
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
    assert!(
        !ev.default_prevented(),
        "neither onkeydown nor onsubmit took it"
    );
    assert!(f.take_log().is_empty(), "no handler ran");
    f.teardown();
}
