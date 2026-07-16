//! HTTP client error type.

use std::fmt;

/// Errors from an HTTP request.
///
/// Each variant carries a human-readable message. Note that a non-2xx HTTP
/// response (e.g. 404, 500) is **not** an error — it is returned as an
/// `Ok(Response { status, .. })` so callers can read error bodies. Only a
/// failure to complete the request (transport, malformed request, or an
/// unreadable body) surfaces as an `HttpError`.
#[derive(Debug, Clone)]
pub enum HttpError {
    /// A transport-level failure: DNS resolution, connection refused, TLS,
    /// timeout, or (on web) a rejected fetch promise.
    Network(String),
    /// The request could not be constructed (e.g. an invalid URL or header).
    InvalidRequest(String),
    /// The response was received but its body could not be read/decoded.
    Body(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Network(msg) => write!(f, "http network error: {msg}"),
            HttpError::InvalidRequest(msg) => write!(f, "http invalid request: {msg}"),
            HttpError::Body(msg) => write!(f, "http body error: {msg}"),
        }
    }
}

impl std::error::Error for HttpError {}
