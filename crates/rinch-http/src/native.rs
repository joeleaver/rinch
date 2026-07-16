//! Native (non-wasm) HTTP implementation backed by blocking `ureq`.
//!
//! [`fetch_blocking`] is the core: a synchronous request on the **calling** thread
//! using the shared process-wide agent (cookie jar + connection pool). It is the
//! right entry point for code already running off the main thread — e.g. rinch's
//! `NetworkImageLoader`, which runs on the image-decode thread and shares this
//! agent so authenticated image URLs carry the session cookie.
//!
//! The asynchronous [`fetch`] layers UI-thread delivery on top: it runs
//! `fetch_blocking` on a spawned thread and resumes the callback on the main (UI)
//! thread via rinch-core's main-thread callback parking
//! ([`park_main_callback`](rinch_core::park_main_callback)). The callback is parked
//! main-thread-side and only the (`Send`) id + result cross the thread boundary, so
//! it need not be `Send` and can capture `!Send` UI state (an `Rc`-based editor
//! handle, a `Signal`).

use std::sync::OnceLock;
use std::time::Duration;

use rinch_core::{park_main_callback, resume_main_callback, run_on_main_thread};

use crate::{Credentials, HttpCallback, HttpError, Method, Request, Response};

/// Issue `request` off the calling thread and deliver the result to `on_done` on
/// the main (UI) thread.
///
/// Call from the main thread (the one the rinch runtime registered via
/// `register_main_thread`): `on_done` is parked main-thread-side and resumed there,
/// so it is invoked on the main thread and need not be `Send`. The result is hopped
/// back via the runtime's cross-thread dispatcher, which must be registered.
pub fn fetch(request: Request, on_done: impl HttpCallback) {
    // Park the (!Send) callback on the main thread; only the id + Send result
    // travel to the worker and back.
    let id = park_main_callback::<Result<Response, HttpError>>(on_done);
    std::thread::spawn(move || {
        let result = fetch_blocking(request);
        run_on_main_thread(move || resume_main_callback(id, result));
    });
}

/// Issue `request` **synchronously on the calling thread** and return the result.
///
/// The blocking core [`fetch`] builds on. Because it neither spawns a thread nor
/// touches the main-thread machinery, it is safe (and intended) to call from a
/// worker thread that wants to share the process-wide agent — its cookie jar,
/// connection pool and timeouts. rinch's `NetworkImageLoader` uses this so image
/// loads reuse the app's session instead of a fresh, cookie-less agent per call.
pub fn fetch_blocking(request: Request) -> Result<Response, HttpError> {
    match request.credentials {
        // `Omit`: use a throwaway agent with its own empty cookie jar, so no
        // cookies are sent or persisted — matching the web `credentials: "omit"`
        // mode rather than silently attaching the shared jar's cookies.
        Credentials::Omit => do_request(&ureq::Agent::new_with_config(agent_config()), request),
        // `SameOrigin` (default) / `Include` share the process-wide agent + jar.
        Credentials::SameOrigin | Credentials::Include => do_request(agent(), request),
    }
}

/// Clear the shared agent's cookie jar — e.g. on logout / session reset.
///
/// A no-op if no request has been made yet (the agent is created lazily). Only
/// affects the shared agent used by non-`Omit` requests.
pub fn clear_cookies() {
    if let Some(agent) = AGENT.get() {
        agent.cookie_jar_lock().clear();
    }
}

/// The process-wide agent.
///
/// Shared (rather than built per request) for two reasons: ureq's **cookie jar
/// lives on the agent**, so session cookies only persist across requests if the
/// agent does — and a fresh agent per request also throws away connection
/// pooling/keep-alive. `Agent` is internally `Arc`-based, so sharing it across the
/// request threads is cheap and its jar is shared.
static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| ureq::Agent::new_with_config(agent_config()))
}

fn agent_config() -> ureq::config::Config {
    ureq::config::Config::builder()
        // Do NOT treat non-2xx as an error: a 404/500 with a body round-trips into
        // `Ok(Response { .. })` so callers can read error JSON out of 4xx/5xx bodies.
        .http_status_as_error(false)
        // ureq defaults to NO timeouts. A general HTTP primitive must not let a
        // hung/half-open server block a request thread (and leak its parked
        // callback) forever, so impose sane connect + overall ceilings.
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
}

fn do_request(agent: &ureq::Agent, request: Request) -> Result<Response, HttpError> {
    let url = request.url.as_str();

    // `get`/`delete` produce a bodyless builder (`.call()`); `post`/`put`/`patch`
    // produce a body builder (`.send(..)`). Both yield `Result<http::Response<Body>>`.
    let response = match request.method {
        Method::Get => {
            let mut builder = agent.get(url);
            for (k, v) in &request.headers {
                builder = builder.header(k, v);
            }
            builder.call()
        }
        Method::Delete => {
            let mut builder = agent.delete(url);
            for (k, v) in &request.headers {
                builder = builder.header(k, v);
            }
            builder.call()
        }
        Method::Post | Method::Put | Method::Patch => {
            let mut builder = match request.method {
                Method::Post => agent.post(url),
                Method::Put => agent.put(url),
                _ => agent.patch(url),
            };
            for (k, v) in &request.headers {
                builder = builder.header(k, v);
            }
            match request.body {
                Some(bytes) => builder.send(&bytes[..]),
                None => builder.send_empty(),
            }
        }
    };

    let response = response.map_err(map_ureq_error)?;

    let status = response.status().as_u16();

    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();

    let body = response
        .into_body()
        .read_to_vec()
        .map_err(|e| HttpError::Body(e.to_string()))?;

    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Map a ureq error to [`HttpError`].
///
/// With `http_status_as_error(false)`, a non-2xx status never reaches here. URL /
/// request-construction failures map to [`HttpError::InvalidRequest`] (matching the
/// web target, which reports the same class as `InvalidRequest`); everything else
/// is a transport/protocol failure ([`HttpError::Network`]).
fn map_ureq_error(err: ureq::Error) -> HttpError {
    match err {
        ureq::Error::BadUri(msg) => HttpError::InvalidRequest(msg),
        ureq::Error::Http(e) => HttpError::InvalidRequest(e.to_string()),
        other => HttpError::Network(other.to_string()),
    }
}
