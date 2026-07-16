//! Web (wasm32) HTTP implementation backed by `web_sys::fetch`.
//!
//! The request is issued against the browser's Fetch API. Everything runs on the
//! single web thread, so `on_done` is invoked directly (NOT via
//! `rinch_core::run_on_main_thread`, whose `Send` bound a JS-capturing callback
//! cannot satisfy) — it is already on the UI thread.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Request as WebRequest, RequestCredentials, RequestInit, Response as WebResponse};

use crate::{Credentials, HttpCallback, HttpError, Request, Response};

/// Issue `request` via `web_sys::fetch` and deliver the result to `on_done`.
///
/// `on_done` runs on the single web thread (the UI thread), so it can update
/// rinch signals directly.
pub fn fetch(request: Request, on_done: impl HttpCallback) {
    spawn_local(async move {
        let result = do_fetch(request).await;
        on_done(result);
    });
}

async fn do_fetch(request: Request) -> Result<Response, HttpError> {
    let init = RequestInit::new();
    init.set_method(request.method.as_str());
    init.set_credentials(match request.credentials {
        Credentials::Omit => RequestCredentials::Omit,
        Credentials::SameOrigin => RequestCredentials::SameOrigin,
        Credentials::Include => RequestCredentials::Include,
    });

    if let Some(body) = &request.body {
        // Copy the bytes into a JS Uint8Array (a valid fetch BufferSource body).
        let array = js_sys::Uint8Array::from(&body[..]);
        init.set_body(&array.into());
    }

    let web_request = WebRequest::new_with_str_and_init(&request.url, &init)
        .map_err(|e| HttpError::InvalidRequest(js_err(&e)))?;

    let headers = web_request.headers();
    for (k, v) in &request.headers {
        headers
            .set(k, v)
            .map_err(|e| HttpError::InvalidRequest(js_err(&e)))?;
    }

    let window = web_sys::window()
        .ok_or_else(|| HttpError::Network("no global `window` available".to_string()))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&web_request))
        .await
        .map_err(|e| HttpError::Network(js_err(&e)))?;

    let resp: WebResponse = resp_value
        .dyn_into()
        .map_err(|_| HttpError::Network("fetch result is not a Response".to_string()))?;

    let status = resp.status();

    let headers = collect_headers(&resp.headers());

    // Read the body as raw bytes via `array_buffer()` so `Response.body: Vec<u8>`
    // is byte-accurate — a binary payload (image, gzip, protobuf) survives intact,
    // matching the native target. (Text/JSON callers just `resp.text()` on top.)
    let buffer_promise = resp
        .array_buffer()
        .map_err(|e| HttpError::Body(js_err(&e)))?;
    let buffer_value = JsFuture::from(buffer_promise)
        .await
        .map_err(|e| HttpError::Body(js_err(&e)))?;
    let body = js_sys::Uint8Array::new(&buffer_value).to_vec();

    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Collect a `web_sys::Headers` into a `Vec<(name, value)>`.
///
/// `Headers` is a JS-iterable of `[name, value]` pairs; we walk it via
/// `js_sys::try_iter`. Any malformed entry is skipped rather than failing the
/// whole request.
fn collect_headers(headers: &web_sys::Headers) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(Some(iter)) = js_sys::try_iter(headers) {
        for entry in iter.flatten() {
            // Each entry is a 2-element array: [name, value].
            let pair: js_sys::Array = match entry.dyn_into() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let name = pair.get(0).as_string();
            let value = pair.get(1).as_string();
            if let (Some(name), Some(value)) = (name, value) {
                out.push((name, value));
            }
        }
    }
    out
}

/// Render a `JsValue` error into a debug string.
fn js_err(e: &JsValue) -> String {
    format!("{e:?}")
}
