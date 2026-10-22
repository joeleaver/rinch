//! `copy_text_then` / `copy_html_then` on the web (#1780): the callback carries
//! the browser's answer to the system-clipboard write, where `copy_text` answers
//! `Ok` before the promise settles.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test clipboard_copy_then
//! ```
//!
//! A refusal is made deterministic with a real browser rule rather than a stub:
//! the clipboard of a same-origin `<iframe allow="clipboard-write 'none'">`
//! (handed to the crate through its doc-hidden `__set_clipboard_source`), whose
//! write Chrome rejects with `NotAllowedError` whatever the page's own
//! permission state (Firefox rejects it too, for want of a user activation).
//! An *accepted* write needs a user activation in a headless browser, so that
//! case is a trusted click sent over the DevTools protocol: it is `#[ignore]`d
//! in an ordinary run and runs through `support/run_real_keys.sh` (Chrome only,
//! in CI's Chrome job). Each case waits for its callback to have run (the
//! positive control), since an `on_done` never called would otherwise read as
//! no answer at all.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_clipboard::{ClipboardError, ClipboardResult};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Let pending promises and `spawn_local` tasks run: a macrotask (`setTimeout`
/// 0) is queued behind every microtask, a few of them behind a settled write.
async fn settle() {
    for _ in 0..20 {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }
}

type Answer = Arc<Mutex<Option<ClipboardResult<()>>>>;

/// A slot an `on_done` fills, and the callback that fills it.
fn answer_slot() -> (Answer, impl FnOnce(ClipboardResult<()>) + Send + 'static) {
    let slot: Answer = Arc::new(Mutex::new(None));
    let fill = slot.clone();
    (slot, move |result| {
        *fill.lock().unwrap() = Some(result);
    })
}

/// Wait for `slot` to be filled, failing if the callback never ran.
async fn answer(slot: &Answer) -> ClipboardResult<()> {
    for _ in 0..50 {
        if let Some(result) = slot.lock().unwrap().take() {
            return result;
        }
        settle().await;
    }
    panic!("on_done was never called");
}

/// A same-origin iframe whose permissions policy denies `clipboard-write`, and
/// its `navigator.clipboard`.
async fn refusing_clipboard() -> (web_sys::HtmlIFrameElement, web_sys::Clipboard) {
    let frame: web_sys::HtmlIFrameElement = document()
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    frame
        .set_attribute("allow", "clipboard-write 'none'")
        .unwrap();
    frame
        .set_attribute("srcdoc", "<p>refusing frame</p>")
        .unwrap();
    document().body().unwrap().append_child(&frame).unwrap();
    for _ in 0..50 {
        if let Some(win) = frame.content_window() {
            let nav = js_sys::Reflect::get(&win, &"navigator".into()).unwrap();
            let cb = js_sys::Reflect::get(&nav, &"clipboard".into()).unwrap();
            let loaded = win
                .document()
                .and_then(|d| d.body())
                .is_some_and(|b| b.text_content().unwrap_or_default().contains("refusing"));
            if loaded && !cb.is_undefined() {
                return (frame, cb.unchecked_into());
            }
        }
        settle().await;
    }
    panic!("the refusing iframe never loaded");
}

/// The browser itself refuses the frame's write — the fact the refusal cases
/// rest on, asked of the browser with no rinch code in between.
#[wasm_bindgen_test]
async fn the_browser_refuses_a_write_from_the_policy_denied_frame() {
    let (frame, clipboard) = refusing_clipboard().await;
    let outcome = wasm_bindgen_futures::JsFuture::from(clipboard.write_text("x")).await;
    frame.remove();
    let reason = outcome.expect_err("the frame's write is refused");
    let name = js_sys::Reflect::get(&reason, &"name".into()).unwrap();
    assert_eq!(name.as_string().as_deref(), Some("NotAllowedError"));
}

/// The defect: the fire-and-forget `copy_text` answers `Ok` for a write the
/// browser refuses. Kept as it is (its `Ok` is the in-app buffer's); pinned so
/// its documentation stays true.
#[wasm_bindgen_test]
async fn copy_text_answers_ok_for_a_refused_write() {
    let (frame, clipboard) = refusing_clipboard().await;
    rinch_clipboard::__set_clipboard_source(Some(Some(clipboard)));
    let result = rinch_clipboard::copy_text("refused but ok");
    rinch_clipboard::__set_clipboard_source(None);
    frame.remove();
    assert!(result.is_ok());
    assert_eq!(
        rinch_clipboard::paste_text().ok().as_deref(),
        Some("refused but ok"),
        "the in-app buffer is written"
    );
}

#[wasm_bindgen_test]
async fn copy_text_then_reports_a_refused_write_as_an_error() {
    let (frame, clipboard) = refusing_clipboard().await;
    rinch_clipboard::__set_clipboard_source(Some(Some(clipboard)));
    let (slot, on_done) = answer_slot();
    rinch_clipboard::copy_text_then("refused", on_done);
    rinch_clipboard::__set_clipboard_source(None);
    let result = answer(&slot).await;
    frame.remove();
    match result {
        Err(ClipboardError::AccessFailed(message)) => {
            assert!(message.starts_with("NotAllowedError"), "{message}")
        }
        other => panic!("expected AccessFailed(NotAllowedError..), got {other:?}"),
    }
}

#[wasm_bindgen_test]
async fn copy_html_then_reports_a_refused_write_as_an_error() {
    let (frame, clipboard) = refusing_clipboard().await;
    rinch_clipboard::__set_clipboard_source(Some(Some(clipboard)));
    let (slot, on_done) = answer_slot();
    rinch_clipboard::copy_html_then("<b>refused</b>", Some("refused"), on_done);
    rinch_clipboard::__set_clipboard_source(None);
    let result = answer(&slot).await;
    frame.remove();
    assert!(
        matches!(result, Err(ClipboardError::AccessFailed(ref m)) if m.starts_with("NotAllowedError")),
        "{result:?}"
    );
}

/// A context with no `navigator.clipboard` (an insecure origin) answers
/// `NotAvailable` before the call returns.
#[wasm_bindgen_test]
fn copy_text_then_without_a_clipboard_is_not_available_at_once() {
    rinch_clipboard::__set_clipboard_source(Some(None));
    let (slot, on_done) = answer_slot();
    rinch_clipboard::copy_text_then("nowhere", on_done);
    rinch_clipboard::__set_clipboard_source(None);
    let result = slot
        .lock()
        .unwrap()
        .take()
        .expect("called back synchronously");
    assert!(
        matches!(result, Err(ClipboardError::NotAvailable)),
        "{result:?}"
    );
}

thread_local! {
    static CLICK_N: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Ask `support/real_keys.mjs` for one trusted primary click at the centre of
/// `el` and wait until it says it sent it.
async fn real_click(el: &web_sys::Element) {
    let n = CLICK_N.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let r = el.get_bounding_client_rect();
    let (x, y) = (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
    document().set_title(&format!("RINCH-REAL-CLICK:{n}:{},{}", x.round(), y.round()));
    let sent = format!("RINCH-REAL-CLICK-SENT:{n}");
    for _ in 0..200 {
        settle().await;
        if document().title() == sent {
            return;
        }
    }
    panic!("the helper never sent click #{n} (is support/real_keys.mjs running?)");
}

/// Two buttons the way an app writes them: each copies from its click and
/// shows, through a signal written by the callback, what the browser said.
/// `refuse`, while set, is the clipboard the copies go to.
struct CopyButtons {
    root: RootHandle,
    host: web_sys::Element,
    refuse: Rc<RefCell<Option<web_sys::Clipboard>>>,
}

impl CopyButtons {
    fn mount() -> Self {
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let refuse: Rc<RefCell<Option<web_sys::Clipboard>>> = Rc::new(RefCell::new(None));
        let refuse_in = refuse.clone();
        let root = rinch_web::mount_into(&host, Default::default(), move |__scope| {
            let status = Signal::new(String::from("idle"));
            let (text_source, html_source) = (refuse_in.clone(), refuse_in.clone());
            let report = move |what: &'static str| {
                move |r: ClipboardResult<()>| {
                    status.set(match r {
                        Ok(()) => format!("Copied {what}"),
                        Err(_) => format!("Copy {what} failed"),
                    });
                }
            };
            rsx! {
                div {
                    button {
                        id: "copy-text-1780",
                        style: "display: block; width: 160px; height: 40px",
                        onclick: move || {
                            rinch_clipboard::__set_clipboard_source(
                                text_source.borrow().clone().map(Some),
                            );
                            rinch_clipboard::copy_text_then("from a button", report("text"));
                            rinch_clipboard::__set_clipboard_source(None);
                        },
                        "Copy text"
                    }
                    button {
                        id: "copy-html-1780",
                        style: "display: block; width: 160px; height: 40px",
                        onclick: move || {
                            rinch_clipboard::__set_clipboard_source(
                                html_source.borrow().clone().map(Some),
                            );
                            rinch_clipboard::copy_html_then(
                                "<b>from a button</b>",
                                Some("from a button"),
                                report("html"),
                            );
                            rinch_clipboard::__set_clipboard_source(None);
                        },
                        "Copy html"
                    }
                    span { id: "status-1780", {|| status.get()} }
                }
            }
        });
        CopyButtons { root, host, refuse }
    }

    fn button(&self, which: &str) -> web_sys::Element {
        document()
            .get_element_by_id(&format!("copy-{which}-1780"))
            .unwrap()
    }

    fn shown(&self) -> String {
        document()
            .get_element_by_id("status-1780")
            .unwrap()
            .text_content()
            .unwrap_or_default()
    }

    /// Wait until the status is no longer `before`: the callback ran.
    async fn shown_after(&self, before: &str) -> String {
        for _ in 0..50 {
            let now = self.shown();
            if now != before {
                return now;
            }
            settle().await;
        }
        panic!("the copy's callback never ran (status still {before:?})");
    }

    fn unmount(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// A synthetic click on a mounted copy button whose write the browser refuses
/// shows the refusal (and, by showing anything, that the callback ran).
#[wasm_bindgen_test]
async fn a_copy_button_shows_a_refused_write() {
    let buttons = CopyButtons::mount();
    assert_eq!(buttons.shown(), "idle", "mounted");
    let (frame, clipboard) = refusing_clipboard().await;
    *buttons.refuse.borrow_mut() = Some(clipboard);

    buttons
        .button("text")
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    let text = buttons.shown_after("idle").await;
    buttons
        .button("html")
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    let html = buttons.shown_after(&text).await;
    frame.remove();
    buttons.unmount();
    assert_eq!(
        (text.as_str(), html.as_str()),
        ("Copy text failed", "Copy html failed")
    );
}

/// A trusted click — a user activation, which is what lets a page write the
/// clipboard — on the mounted copy buttons: the browser accepts both writes and
/// the buttons say so; with the policy-denied frame's clipboard, the same
/// trusted click is still refused.
#[wasm_bindgen_test]
#[ignore = "needs trusted clicks: run tests/support/run_real_keys.sh"]
async fn a_trusted_click_on_a_copy_button_shows_the_browsers_answer() {
    assert!(
        option_env!("RINCH_REAL_KEYS").is_some(),
        "built without RINCH_REAL_KEYS: run tests/support/run_real_keys.sh, which sends the clicks"
    );
    let buttons = CopyButtons::mount();
    assert_eq!(buttons.shown(), "idle", "mounted");

    real_click(&buttons.button("text")).await;
    let text = buttons.shown_after("idle").await;
    real_click(&buttons.button("html")).await;
    let html = buttons.shown_after(&text).await;

    let (frame, clipboard) = refusing_clipboard().await;
    *buttons.refuse.borrow_mut() = Some(clipboard);
    real_click(&buttons.button("text")).await;
    let refused = buttons.shown_after(&html).await;
    frame.remove();
    buttons.unmount();

    assert_eq!(
        (text.as_str(), html.as_str(), refused.as_str()),
        ("Copied text", "Copied html", "Copy text failed")
    );
}
