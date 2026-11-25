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

#[path = "element_key_1678/caret_oracle.rs"]
mod caret_oracle;
#[path = "element_key_1678/key_handlers.rs"]
mod key_handlers;
#[path = "element_key_1678/selection_and_caret_rect.rs"]
mod selection_and_caret_rect;
