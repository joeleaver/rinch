//! The page's half of the `support/real_keys.mjs` handshake: the test writes a
//! request into `document.title` and waits for the helper to answer in it.
//!
//! The wait is by time, not by rounds (#1839): a round of `setTimeout`s takes
//! whatever the browser gives it, which on a loaded runner can be far less
//! than the helper needs to notice a new page and poll it. While it waits it
//! writes the request again if something else replaced the title.

/// How long to wait for the helper before failing: under
/// `wasm-bindgen-test-runner`'s own 20s default, so the panic (and its title) is
/// what a failure reports rather than a runner timeout.
const WAIT_MS: f64 = 15_000.0;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn now_ms() -> f64 {
    js_sys::Date::now()
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

/// Writes `request` into the title and waits until the helper answers with a
/// title starting with `answer_prefix`; returns the rest of that title. Panics
/// after `WAIT_MS` naming `what`.
pub async fn ask_helper(request: &str, answer_prefix: &str, what: &str) -> String {
    document().set_title(request);
    let start = now_ms();
    loop {
        sleep(20).await;
        let title = document().title();
        if let Some(rest) = title.strip_prefix(answer_prefix) {
            return rest.to_string();
        }
        if title != request {
            document().set_title(request);
        }
        if now_ms() - start > WAIT_MS {
            panic!(
                "the helper never sent {what} in {WAIT_MS}ms (is support/real_keys.mjs \
                 running? run_real_keys.sh prints its log after a failure); the title \
                 is {title:?}"
            );
        }
    }
}
