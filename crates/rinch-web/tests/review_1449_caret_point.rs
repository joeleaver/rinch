//! Review of PR #1449: differential fixtures for `caret_point_from_point`.
//!
//! 1. every point of a grid over a rich editor document, pressed with both calls
//!    present and with `caretRangeFromPoint` hidden: same head, same affinity;
//! 2. the raw answers of the two calls over a page that also holds text controls,
//!    a shadow root and `user-select: none` text: where the old call answers
//!    nothing, the new one must too, or Chrome's behaviour changed (the helper
//!    falls through on a null answer);
//! 3. the generic `data-block-index` text hit (`walk_text_nodes_for_offset`),
//!    which the PR changed and its own tests never reach.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/engine.rs"]
mod engine;

const HOST: &str = "data-test-host-review-1449";
const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
const OLD_CALL: &str = "caretRangeFromPoint";
const NEW_CALL: &str = "caretPositionFromPoint";

fn log_1(v: &JsValue) {
    console_log!("{}", v.as_string().unwrap_or_default());
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn hide(name: &str) {
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(&descriptor, &"value".into(), &JsValue::UNDEFINED).unwrap();
    js_sys::Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE).unwrap();
    js_sys::Object::define_property(
        document().unchecked_ref::<js_sys::Object>(),
        &name.into(),
        &descriptor,
    );
    assert!(
        js_sys::Reflect::get(&document(), &name.into())
            .unwrap()
            .is_undefined()
    );
}

fn show(name: &str) {
    js_sys::Reflect::delete_property(document().unchecked_ref::<js_sys::Object>(), &name.into())
        .unwrap();
    assert!(
        js_sys::Reflect::get(&document(), &name.into())
            .unwrap()
            .is_function()
    );
}

/// What one of the two calls answers at a point, as text: `null`, or the node
/// (name, and a text node's text) and the offset.
fn raw(name: &str, x: f64, y: f64) -> String {
    let func: js_sys::Function = js_sys::Reflect::get(&document(), &name.into())
        .unwrap()
        .dyn_into()
        .expect("the call exists");
    let answer = func
        .call2(&document(), &JsValue::from(x), &JsValue::from(y))
        .unwrap();
    if answer.is_null() || answer.is_undefined() {
        return "null".into();
    }
    let (node, offset) = if name == OLD_CALL {
        let r: web_sys::Range = answer.dyn_into().unwrap();
        (r.start_container().unwrap(), r.start_offset().unwrap())
    } else {
        let node: web_sys::Node = js_sys::Reflect::get(&answer, &"offsetNode".into())
            .unwrap()
            .dyn_into()
            .unwrap();
        let offset = js_sys::Reflect::get(&answer, &"offset".into())
            .unwrap()
            .as_f64()
            .unwrap() as u32;
        (node, offset)
    };
    let text = if node.node_type() == web_sys::Node::TEXT_NODE {
        node.text_content().unwrap_or_default()
    } else {
        node.dyn_ref::<web_sys::Element>()
            .map(|e| {
                format!(
                    "id={} pm={}",
                    e.id(),
                    e.get_attribute("data-pm-type").unwrap_or_default()
                )
            })
            .unwrap_or_default()
    };
    let text: String = text.chars().take(16).collect();
    format!("{}[{text}]@{offset}", node.node_name())
}

fn make_host(style: &str) -> web_sys::Element {
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
        &format!(
            "position: fixed; top: 0; left: 0; z-index: 9999; background: white; \
             font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; {style}"
        ),
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn mouse(name: &str, x: f64, y: f64) {
    let Some(target) = document().element_from_point(x as f32, y as f32) else {
        return;
    };
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

struct F {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

fn editor(html: &str) -> F {
    let host = make_host("width: 300px;");
    let handle = create_editor();
    assert!(handle.load_html(html));
    let m = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| m.mount(s),
    );
    F { root, host, handle }
}

fn rich_html() -> String {
    format!(
        "<h2>Head line</h2>\
         <p>alpha <strong>bold</strong> <a href=\"https://e.test/\">link text</a> bravo<br>charlie delta<br></p>\
         <p></p>\
         <p><img src=\"{GIF}\" style=\"width:20px;height:16px\"> echo <img src=\"{GIF}\" style=\"width:20px;height:16px\"> foxtrot <img src=\"{GIF}\" style=\"width:20px;height:16px\"></p>\
         <p>a long paragraph that wraps over several visual lines in a narrow column so line ends are soft wraps</p>\
         <p>\u{5e9}\u{5dc}\u{5d5}\u{5dd} mixed \u{5e2}\u{5d5}\u{5dc}\u{5dd} text \u{1F600} emoji e\u{301}</p>\
         <ul><li><p>golf</p><ul><li><p>hotel</p></li></ul></li></ul>\
         <blockquote><p>india</p></blockquote>\
         <pre><code>code\n  line two\n</code></pre>\
         <hr>\
         <table><tr><td><p>juliet</p></td><td><p>kilo</p></td></tr></table>\
         <p>lima</p>"
    )
}

/// Press every point of a grid over the editor; `(x, y, head, affinity)`.
fn sweep(f: &F) -> Vec<(i32, i32, usize, String)> {
    let r = f.host.get_bounding_client_rect();
    let mut out = Vec::new();
    let mut y = r.y() + 2.0;
    while y < r.y() + r.height() - 1.0 {
        let mut x = r.x() + 2.0;
        while x < r.x() + r.width() - 1.0 {
            // A sentinel no press produces (a whole-document range), so a press
            // that places nothing reads the same in both sweeps.
            assert!(f.handle.command("selectAll"));
            mouse("mousedown", x, y);
            mouse("mouseup", x, y);
            let s = f.handle.selection();
            out.push((
                x as i32,
                y as i32,
                s.head().0,
                format!(
                    "{}..{} {:?}",
                    s.from().0,
                    s.to().0,
                    f.handle.caret_affinity()
                ),
            ));
            x += 7.0;
        }
        y += 5.0;
    }
    out
}

#[wasm_bindgen_test]
fn every_grid_point_of_a_rich_document_lands_alike_with_only_the_standard_call() {
    let f = editor(&rich_html());
    let both = sweep(&f);
    let mut heads: Vec<usize> = both.iter().map(|p| p.2).collect();
    heads.sort_unstable();
    heads.dedup();
    assert!(
        heads.len() > 120,
        "positive control: the sweep reached many positions ({})",
        heads.len()
    );
    hide(OLD_CALL);
    let standard = sweep(&f);
    show(OLD_CALL);
    assert_eq!(both.len(), standard.len());
    let mut differ = Vec::new();
    for (a, b) in both.iter().zip(&standard) {
        if a != b {
            let (x, y) = (a.0 as f64, a.1 as f64);
            differ.push(format!(
                "({x},{y}) both: head {} {} | standard: head {} {} | range {} | position {}",
                a.2,
                a.3,
                b.2,
                b.3,
                raw(OLD_CALL, x, y),
                raw(NEW_CALL, x, y)
            ));
        }
    }
    log_1(
        &format!(
            "REVIEW-1449 sweep: {} points, {} distinct heads, {} differ",
            both.len(),
            heads.len(),
            differ.len()
        )
        .into(),
    );
    for d in differ.iter().take(40) {
        log_1(&format!("REVIEW-1449 differ {d}").into());
    }
    f.root.unmount();
    f.host.remove();
    assert!(
        differ.is_empty(),
        "{} of {} grid points differ; first: {:?}",
        differ.len(),
        both.len(),
        differ.iter().take(8).collect::<Vec<_>>()
    );
}

/// Where `caretRangeFromPoint` answers null, the helper now goes on to ask
/// `caretPositionFromPoint`. If that ever answers there, Chrome's behaviour changed.
#[wasm_bindgen_test]
fn where_the_old_call_answers_nothing_the_standard_one_answers_nothing() {
    let host = make_host("width: 500px;");
    host.set_inner_html(
        "<p id=plain>plain text here</p>\
         <p id=nosel style='user-select: none'>unselectable text</p>\
         <p><input id=inp value='input value text' style='width: 200px'></p>\
         <p><textarea id=ta style='width: 200px; height: 50px'>textarea text\nline two</textarea></p>\
         <div id=shadow></div>\
         <p id=hidden style='visibility: hidden'>hidden text</p>\
         <p id=pe style='pointer-events: none'>no pointer events</p>\
         <p id=empty style='height: 24px'></p>\
         <p id=br>before<br>after</p>\
         <button id=btn>a button</button>\
         <select id=sel><option>option one</option></select>\
         <p id=ce contenteditable=true>editable text</p>",
    );
    let shadow_host = document().get_element_by_id("shadow").unwrap();
    js_sys::Function::new_with_args(
        "host",
        "host.attachShadow({mode: 'open'}).innerHTML = '<p>text in a shadow root</p>';",
    )
    .call1(&JsValue::NULL, &shadow_host)
    .unwrap();

    let window = web_sys::window().unwrap();
    let (w, h) = (
        window.inner_width().unwrap().as_f64().unwrap(),
        window.inner_height().unwrap().as_f64().unwrap(),
    );
    let (mut total, mut both_null, mut only_new, mut only_old, mut differ) = (0, 0, 0, 0, 0);
    // Points where the answers differ and neither names a text control.
    let mut differ_outside_controls = 0;
    let mut samples: Vec<String> = Vec::new();
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut y = -20.0;
    while y < h + 20.0 {
        let mut x = -20.0;
        while x < w + 20.0 {
            total += 1;
            let (old, new) = (raw(OLD_CALL, x, y), raw(NEW_CALL, x, y));
            let under = document()
                .element_from_point(x as f32, y as f32)
                .map(|e| format!("{}#{}", e.tag_name(), e.id()))
                .unwrap_or_else(|| "none".into());
            let kind = match (old.as_str(), new.as_str()) {
                ("null", "null") => {
                    both_null += 1;
                    None
                }
                ("null", _) => {
                    only_new += 1;
                    Some("only-standard")
                }
                (_, "null") => {
                    only_old += 1;
                    Some("only-old")
                }
                (a, b) if a != b => {
                    differ += 1;
                    let control = |r: &str| r.starts_with("INPUT[") || r.starts_with("TEXTAREA[");
                    if !control(a) && !control(b) {
                        differ_outside_controls += 1;
                    }
                    Some("differ")
                }
                _ => None,
            };
            if let Some(kind) = kind {
                let n = kinds.entry(format!("{kind} under {under}")).or_default();
                *n += 1;
                if *n <= 2 {
                    samples.push(format!(
                        "{kind} at ({x},{y}) under {under}: range {old} | position {new}"
                    ));
                }
            }
            x += 9.0;
        }
        y += 6.0;
    }
    log_1(
        &format!(
            "REVIEW-1449 raw: {total} points, both null {both_null}, only standard {only_new}, \
             only old {only_old}, differ {differ}; kinds {kinds:?}"
        )
        .into(),
    );
    for s in &samples {
        log_1(&format!("REVIEW-1449 raw {s}").into());
    }
    host.remove();
    assert!(
        total - both_null > 500,
        "positive control: the calls answered"
    );
    // Chrome 153, measured: the two calls differ only over a text control, where
    // the old one answers (parent, child index) and the standard one
    // (control, character offset) -- NOT a child index.
    assert_eq!(
        (only_new, only_old),
        (0, 0),
        "kinds {kinds:?}; samples {samples:?}"
    );
    // Every difference is over a text control, by what the answers name.
    assert_eq!(
        differ_outside_controls, 0,
        "kinds {kinds:?}; samples {samples:?}"
    );
    // By the element under the point too, in Chrome. Firefox 157 (which has
    // both calls) also differs just below the textarea, where
    // `elementFromPoint` answers the host `<div>` and both calls still name
    // the textarea, at character offsets that disagree: the old call says 0.
    if !engine::is_gecko() {
        assert!(
            kinds
                .keys()
                .all(|k| k.contains("INPUT#inp") || k.contains("TEXTAREA#ta")),
            "kinds {kinds:?}; samples {samples:?}"
        );
    }
}

/// The generic text hit: a `data-block-index` block with a click handler; the
/// press's `ClickContext::text_hit`.
#[wasm_bindgen_test]
fn the_generic_text_hit_counts_bytes_alike_and_stops_at_an_element_point() {
    let host = make_host("width: 400px;");
    let seen: Rc<RefCell<Vec<(usize, usize, bool)>>> = Rc::default();
    let s = seen.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrap = scope.create_element("div");
            let s = s.clone();
            let id = scope.register_handler(move || {
                let hit = rinch_core::events::get_click_context().text_hit;
                s.borrow_mut()
                    .push((hit.block_index, hit.byte_offset, hit.valid));
            });
            wrap.set_attribute("data-rid", &id.0.to_string());
            let block = scope.create_element("div");
            block.set_attribute("id", "blk1449");
            block.set_attribute("data-block-index", "3");
            let a = scope.create_text("h\u{e9}llo ");
            let span = scope.create_element("span");
            span.append_child(&scope.create_text("w\u{1F600}rld"));
            let br = scope.create_element("br");
            let b = scope.create_text("second line");
            let img = scope.create_element("img");
            img.set_attribute("src", GIF);
            img.set_attribute("style", "width: 40px; height: 16px");
            let c = scope.create_text("tail");
            for n in [&a, &span, &br, &b, &img, &c] {
                block.append_child(n);
            }
            wrap.append_child(&block);
            let empty = scope.create_element("div");
            empty.set_attribute("id", "empty1449");
            empty.set_attribute("data-block-index", "4");
            empty.set_attribute("style", "height: 24px");
            wrap.append_child(&empty);
            // An element point BELOW the block: (span, index), with text after it.
            let nested = scope.create_element("div");
            nested.set_attribute("id", "nested1449");
            nested.set_attribute("data-block-index", "5");
            nested.append_child(&scope.create_text("ab"));
            let holder = scope.create_element("span");
            let img2 = scope.create_element("img");
            img2.set_attribute("src", GIF);
            img2.set_attribute("style", "width: 40px; height: 16px");
            holder.append_child(&img2);
            nested.append_child(&holder);
            nested.append_child(&scope.create_text("cd"));
            wrap.append_child(&nested);
            wrap
        },
    );
    let block = document().get_element_by_id("blk1449").unwrap();
    let img = block.query_selector("img").unwrap().unwrap();
    let r = block.get_bounding_client_rect();
    let ir = img.get_bounding_client_rect();
    let er = document()
        .get_element_by_id("empty1449")
        .unwrap()
        .get_bounding_client_rect();
    let mut points: Vec<(f64, f64)> = Vec::new();
    // Both lines, every 6px, past the line ends too.
    for line in 0..2 {
        let y = r.y() + 12.0 + 24.0 * line as f64;
        let mut x = r.x() + 1.0;
        while x < r.x() + r.width() - 1.0 {
            points.push((x, y));
            x += 6.0;
        }
    }
    let image_points = [
        (ir.x() + 5.0, ir.y() + 8.0),
        (ir.x() + 35.0, ir.y() + 8.0),
        (er.x() + 50.0, er.y() + 12.0),
    ];
    points.extend(image_points);
    let nr = document()
        .get_element_by_id("nested1449")
        .unwrap()
        .query_selector("img")
        .unwrap()
        .unwrap()
        .get_bounding_client_rect();
    let nested_points = [(nr.x() + 5.0, nr.y() + 8.0), (nr.x() + 35.0, nr.y() + 8.0)];
    points.extend(nested_points);

    let run = |points: &[(f64, f64)]| -> Vec<(usize, usize, bool)> {
        seen.borrow_mut().clear();
        let mut out = Vec::new();
        for &(x, y) in points {
            let Some(target) = document().element_from_point(x as f32, y as f32) else {
                continue;
            };
            let before = seen.borrow().len();
            for (ty, bs) in [("pointerdown", 1u16), ("pointerup", 0)] {
                let init = web_sys::PointerEventInit::new();
                init.set_bubbles(true);
                init.set_cancelable(true);
                init.set_pointer_id(1);
                init.set_is_primary(true);
                init.set_pointer_type("mouse");
                init.set_button(0);
                init.set_buttons(bs);
                init.set_client_x(x as i32);
                init.set_client_y(y as i32);
                let ev = web_sys::PointerEvent::new_with_event_init_dict(ty, &init).unwrap();
                target.dispatch_event(&ev).unwrap();
            }
            let now = seen.borrow();
            assert_eq!(now.len(), before + 1, "positive control: one click a press");
            out.push(*now.last().unwrap());
        }
        out
    };
    let both = run(&points);
    let raws: Vec<String> = image_points
        .iter()
        .map(|&(x, y)| {
            format!(
                "range {} | position {}",
                raw(OLD_CALL, x.trunc(), y.trunc()),
                raw(NEW_CALL, x.trunc(), y.trunc())
            )
        })
        .collect();
    hide(OLD_CALL);
    let standard = run(&points);
    show(OLD_CALL);
    root.unmount();
    host.remove();

    let nested_raws: Vec<String> = nested_points
        .iter()
        .map(|&(x, y)| raw(OLD_CALL, x.trunc(), y.trunc()))
        .collect();
    let nested: Vec<(usize, usize, bool)> = both[both.len() - 2..].to_vec();
    let nested_standard: Vec<(usize, usize, bool)> = standard[standard.len() - 2..].to_vec();
    assert_eq!(
        (nested.clone(), nested_standard),
        (
            vec![(5, 2, true), (5, 2, true)],
            vec![(5, 2, true), (5, 2, true)]
        ),
        "an element point inside the block ends the count: `ab` is 2 bytes (raws {nested_raws:?})"
    );
    let both = both[..both.len() - 2].to_vec();
    let standard = standard[..standard.len() - 2].to_vec();
    let n = both.len();
    log_1(
        &format!(
            "REVIEW-1449 generic: both {:?} | image+empty raws {raws:?} | both tail {:?} | standard tail {:?}",
            both.iter().map(|h| h.1).collect::<Vec<_>>(),
            &both[n - 3..],
            &standard[n - 3..]
        )
        .into(),
    );
    let mut offsets: Vec<usize> = both.iter().filter(|h| h.2).map(|h| h.1).collect();
    offsets.sort_unstable();
    offsets.dedup();
    assert!(
        offsets.len() > 15,
        "positive control: many byte offsets ({offsets:?})"
    );
    assert_eq!(standard, both, "raws {raws:?}");
    // "héllo " 7 bytes + "w😀rld" 8 + "second line" 11 = 26 before the image, 30 in all.
    // A press on the image is an element point (block, child index): left half
    // before the image, right half after it; never the whole block's 30 for both.
    let (left, right) = (both[n - 3], both[n - 2]);
    assert_eq!(
        (left.1, right.1),
        (26, 26),
        "an image holds no bytes: both sides of it are byte 26 (raws {raws:?})"
    );
    assert_eq!(both[n - 1], (4, 0, true), "the empty block");
}

/// FINDING (fails at the PR head): the standard call answers a text control as
/// `(control, CHARACTER offset)`, which `walk_text_nodes_for_offset` reads as a
/// child index. A `<textarea>` with a text child inside a `data-block-index`
/// block then has its default text counted into the block's byte offset.
#[wasm_bindgen_test]
fn a_text_control_in_a_block_is_not_counted_as_block_text() {
    let host = make_host("width: 400px;");
    let seen: Rc<RefCell<Vec<(usize, usize, bool)>>> = Rc::default();
    let s = seen.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let block = scope.create_element("div");
            let s = s.clone();
            let id = scope.register_handler(move || {
                let hit = rinch_core::events::get_click_context().text_hit;
                s.borrow_mut()
                    .push((hit.block_index, hit.byte_offset, hit.valid));
            });
            block.set_attribute("data-rid", &id.0.to_string());
            block.set_attribute("data-block-index", "6");
            block.append_child(&scope.create_text("ab"));
            let ta = scope.create_element("textarea");
            ta.set_attribute("id", "ta1449");
            ta.set_attribute("style", "width: 200px; height: 40px; font: 16px monospace");
            ta.append_child(&scope.create_text("xyz and more text"));
            block.append_child(&ta);
            block.append_child(&scope.create_text("cd"));
            block
        },
    );
    let r = document()
        .get_element_by_id("ta1449")
        .unwrap()
        .get_bounding_client_rect();
    let (x, y) = ((r.x() + 100.0).trunc(), (r.y() + 12.0).trunc());
    let press = || {
        let target = document().element_from_point(x as f32, y as f32).unwrap();
        for (ty, bs) in [("pointerdown", 1u16), ("pointerup", 0)] {
            let init = web_sys::PointerEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_pointer_id(1);
            init.set_is_primary(true);
            init.set_pointer_type("mouse");
            init.set_button(0);
            init.set_buttons(bs);
            init.set_client_x(x as i32);
            init.set_client_y(y as i32);
            let ev = web_sys::PointerEvent::new_with_event_init_dict(ty, &init).unwrap();
            target.dispatch_event(&ev).unwrap();
        }
        *seen.borrow().last().expect("positive control: a click")
    };
    let both = press();
    let raws = format!(
        "range {} | position {}",
        raw(OLD_CALL, x, y),
        raw(NEW_CALL, x, y)
    );
    hide(OLD_CALL);
    let standard = press();
    show(OLD_CALL);
    root.unmount();
    host.remove();
    assert_eq!(
        both,
        (6, 2, true),
        "the old call: before the control ({raws})"
    );
    assert_eq!(standard, both, "{raws}");
}
