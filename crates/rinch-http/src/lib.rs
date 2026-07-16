//! Cross-platform, callback-based HTTP client for rinch.
//!
//! One [`fetch`] function works on both targets:
//!
//! - **Native** (`cfg(not(target_arch = "wasm32"))`): a blocking [`ureq`] request
//!   runs on a spawned `std::thread`.
//! - **Web** (`cfg(target_arch = "wasm32")`): `web_sys::fetch` runs on the single
//!   browser thread.
//!
//! On **both** platforms the `on_done` callback is invoked on the main (UI)
//! thread, so consumers can update rinch [`Signal`](rinch_core::Signal)s directly
//! with `.set()` from inside the callback. On native this is arranged via
//! [`rinch_core::run_on_main_thread`]; on web the callback already runs on the UI
//! thread.
//!
//! # Example
//!
//! ```ignore
//! use rinch_http::{fetch, Request};
//!
//! fetch(Request::get("https://example.com/api/thing"), move |result| {
//!     match result {
//!         Ok(resp) if resp.ok() => { /* resp.text() */ }
//!         Ok(resp) => { /* non-2xx: read error body from resp.text() */ }
//!         Err(e) => { /* transport failure */ }
//!     }
//! });
//! ```

mod error;

pub use error::HttpError;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{clear_cookies, fetch, fetch_blocking};

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::fetch;

/// HTTP request method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl Method {
    /// The uppercase HTTP method token (`"GET"`, `"POST"`, ...).
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
        }
    }
}

/// How credentials (cookies, HTTP auth) are sent with a request.
///
/// This mirrors the Fetch API's `credentials` mode and only has meaning on the
/// **web** target. On native it is accepted but ignored (ureq's cookie handling
/// is governed by its own agent configuration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credentials {
    /// Never send credentials.
    Omit,
    /// Send credentials only for same-origin requests (the default).
    SameOrigin,
    /// Always send credentials, including cross-origin.
    Include,
}

/// An HTTP request to be issued by [`fetch`].
///
/// Build one with the [`Request::get`] / [`post`](Request::post) /
/// [`put`](Request::put) / [`delete`](Request::delete) / [`patch`](Request::patch)
/// constructors, then chain [`header`](Request::header), [`body`](Request::body) /
/// [`body_str`](Request::body_str), and [`credentials`](Request::credentials).
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub credentials: Credentials,
}

impl Request {
    fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            credentials: Credentials::SameOrigin,
        }
    }

    /// A `GET` request to `url`.
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    /// A `POST` request to `url`.
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    /// A `PUT` request to `url`.
    pub fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }

    /// A `DELETE` request to `url`.
    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(Method::Delete, url)
    }

    /// A `PATCH` request to `url`.
    pub fn patch(url: impl Into<String>) -> Self {
        Self::new(Method::Patch, url)
    }

    /// Add a request header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    /// Set the request body from raw bytes.
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set the request body from a string (UTF-8 bytes).
    pub fn body_str(mut self, body: &str) -> Self {
        self.body = Some(body.as_bytes().to_vec());
        self
    }

    /// Set the credentials mode (web-only meaning; ignored on native).
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = credentials;
        self
    }
}

/// An HTTP response returned to the [`fetch`] callback.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// Whether the status code is in the 2xx success range.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The response body decoded as UTF-8 (lossily).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The first value of the response header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The result callback for [`fetch`].
///
/// It is invoked on the main (UI) thread on both targets and never crosses a
/// thread boundary (on native it is parked main-thread-side while only the result
/// is dispatched back — see the `native` module), so it does **not** need to be
/// `Send`. This lets callbacks capture `!Send` UI state such as `Rc`-based editor
/// handles or `Signal`s over them.
pub trait HttpCallback: FnOnce(Result<Response, HttpError>) + 'static {}
impl<F> HttpCallback for F where F: FnOnce(Result<Response, HttpError>) + 'static {}
