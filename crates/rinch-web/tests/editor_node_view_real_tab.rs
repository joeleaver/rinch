//! A **real** Tab out of a node view's inputs, in Chrome (review of #1595, F5). A
//! synthetic `keydown` never moves focus, so the other fixtures can only show that
//! rinch leaves Tab to the browser (`!defaultPrevented`); this one has the browser
//! do it. The keys are trusted ones, sent by `support/real_keys.mjs` over the
//! DevTools protocol (`Input.dispatchKeyEvent`) when the page asks for them through
//! its title, so it runs only through `support/run_real_keys.sh`, which builds the
//! test with `RINCH_REAL_KEYS=1`, starts Chrome with a debugging port and runs the
//! helper beside it:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver crates/rinch-web/tests/support/run_real_keys.sh
//! ```
//!
//! An ordinary run reports it **ignored**, never `ok`: nothing would send the keys.
//! CI's Chrome job runs it through the script.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/real_input.rs"]
mod real_input;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-real-tab";

thread_local! {
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

/// Ask the helper for one trusted key press, and wait until it says it sent it.
async fn real_key(n: u32, key: &str, shift: bool) {
    real_input::ask_helper(
        &format!("RINCH-REAL-KEY:{n}:{key}:{}", u8::from(shift)),
        &format!("RINCH-REAL-KEY-SENT:{n}"),
        key,
    )
    .await;
    // The focus move runs in the task that handled the key.
    sleep(50).await;
}

fn describe(el: Option<web_sys::Element>) -> String {
    el.map(|e| {
        format!(
            "<{} id={:?} in-view={}>",
            e.tag_name().to_lowercase(),
            e.id(),
            e.closest("[data-pm-node-view]").ok().flatten().is_some()
        )
    })
    .unwrap_or_else(|| "nothing".into())
}

#[wasm_bindgen_test]
#[ignore = "needs trusted keys: run tests/support/run_real_keys.sh"]
async fn a_real_tab_leaves_the_view_from_its_last_input_and_shift_tab_from_its_first() {
    assert!(
        option_env!("RINCH_REAL_KEYS").is_some(),
        "built without RINCH_REAL_KEYS: run tests/support/run_real_keys.sh, which sends the keys"
    );
    let before = document().create_element("button").unwrap();
    before.set_id("before");
    before.set_text_content(Some("before"));
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    let after = document().create_element("button").unwrap();
    after.set_id("after");
    after.set_text_content(Some("after"));
    let body = document().body().unwrap();
    for el in [&before, &host, &after] {
        body.append_child(el).unwrap();
    }
    let handle = create_editor();
    assert!(handle.load_html(r#"<p>ab</p><div data-pm-type="embed" data-x="1"></div><p>cd</p>"#));
    handle
        .register_node_view(rinch_web::NodeViewSpec::new(
            "embed",
            |s: &mut RenderScope, _ctx| {
                let root = s.create_element("div");
                for id in ["first", "last"] {
                    let input = s.create_element("input");
                    input.set_attribute("id", id);
                    root.append_child(&input);
                }
                root
            },
        ))
        .keep();
    let h = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| h.mount(scope),
    );
    LIVE_ROOT.with(|r| r.set(Some(root)));
    sleep(50).await;
    let input = |id: &str| -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap()
            .dyn_into()
            .unwrap()
    };

    // Tab inside the view goes from its first input to its last: the browser's.
    input("first").focus().unwrap();
    real_key(1, "Tab", false).await;
    let now = document().active_element();
    assert_eq!(
        now.as_ref().map(|e| e.id()),
        Some("last".into()),
        "Tab from the first input: {}",
        describe(now.clone())
    );

    // From the last input, out of the view.
    real_key(2, "Tab", false).await;
    let now = document().active_element();
    console_log!("REAL-TAB from the last input: {}", describe(now.clone()));
    let left = now
        .as_ref()
        .is_some_and(|e| e.closest("[data-pm-node-view]").ok().flatten().is_none());
    assert!(
        left,
        "Tab from the last input leaves the view: {}",
        describe(now)
    );

    // Shift+Tab from the first input, out of it backwards.
    input("first").focus().unwrap();
    real_key(3, "Tab", true).await;
    let now = document().active_element();
    console_log!(
        "REAL-TAB shift from the first input: {}",
        describe(now.clone())
    );
    let left = now
        .as_ref()
        .is_some_and(|e| e.closest("[data-pm-node-view]").ok().flatten().is_none());
    assert!(
        left,
        "Shift+Tab from the first input leaves the view: {}",
        describe(now)
    );
    assert_eq!(handle.doc().child_count(), 3, "no key edited the document");
    before.remove();
    after.remove();
}
