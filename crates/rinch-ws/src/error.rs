//! WebSocket client error type.

use std::fmt;

/// Errors from a WebSocket connection.
///
/// Delivered to the [`on_error`](crate::WsHandle::on_error) callback for
/// asynchronous failures (a connect that never completes, a broken send, a
/// protocol/transport fault), and returned synchronously from
/// [`connect`](crate::connect) only for an immediately rejected request (a
/// malformed URL or subprotocol, or a browser that refuses to construct the
/// socket).
///
/// Every variant carries a human-readable message. The type is `Clone + Send`
/// so it can cross the worker→main-thread boundary on native.
#[derive(Debug, Clone)]
pub enum WsError {
    /// The request was refused before any connection was attempted: the URL
    /// was not a valid `ws://` / `wss://` endpoint, a subprotocol was not an
    /// HTTP token or was offered twice, a header (native) was invalid, or the
    /// platform refused to construct the socket from it.
    InvalidUrl(String),
    /// The connection could not be established (DNS, refused, TLS handshake,
    /// or a rejected upgrade).
    Connect(String),
    /// An outgoing frame could not be sent.
    Send(String),
    /// A transport/protocol fault on an otherwise-open connection.
    Protocol(String),
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WsError::InvalidUrl(msg) => write!(f, "websocket invalid url: {msg}"),
            WsError::Connect(msg) => write!(f, "websocket connect error: {msg}"),
            WsError::Send(msg) => write!(f, "websocket send error: {msg}"),
            WsError::Protocol(msg) => write!(f, "websocket protocol error: {msg}"),
        }
    }
}

impl std::error::Error for WsError {}
